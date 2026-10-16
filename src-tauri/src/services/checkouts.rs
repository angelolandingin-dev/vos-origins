//! Reusable on-disk checkout cache for live reads.
//!
//! Before this module every live command performed a full `git clone` into a
//! fresh temp dir and deleted it on return, so each tab switch re-cloned the
//! target and/or source repository. Checkouts now live at
//! `%APPDATA%\subsystem-integration-manager\checkouts\<key>` (XDG/temp
//! fallback, `VOS_CHECKOUTS_DIR` override for tests): the first use clones
//! with `--no-tags`, later uses `git fetch` the existing clone, and the
//! directory is never deleted by readers. One process-wide mutex per key
//! keeps two commands from cloning the same repo simultaneously, and refresh
//! failures fall back to the stale checkout instead of failing the read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::services::engine::git;
use crate::services::registry::SubsystemEntry;

fn repo_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_for(key: &str) -> Arc<Mutex<()>> {
    let mut map = repo_locks().lock().unwrap_or_else(|err| err.into_inner());
    map.get(key).cloned().unwrap_or_else(|| {
        let slot = Arc::new(Mutex::new(()));
        map.insert(key.to_string(), slot.clone());
        slot
    })
}

/// Root of the checkout cache. Production uses
/// `%APPDATA%\subsystem-integration-manager\checkouts` (XDG config home when
/// `APPDATA` is unset, temp dir as a last resort); tests point
/// `VOS_CHECKOUTS_DIR` at a disposable dir.
pub fn checkouts_root() -> PathBuf {
    if let Ok(dir) = std::env::var("VOS_CHECKOUTS_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir.trim());
        }
    }
    if let Some(base) = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
    {
        return base.join("subsystem-integration-manager").join("checkouts");
    }
    std::env::temp_dir()
        .join("subsystem-integration-manager")
        .join("checkouts")
}

fn force_refresh_requested() -> bool {
    std::env::var("VOS_CHECKOUT_FORCE_REFRESH")
        .map(|value| value == "1")
        .unwrap_or(false)
}

fn fnv1a_hex(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Identity without credentials: `https://user:token@host/…` and the bare
/// `https://host/…` map to the same string, so token rotation never orphans
/// a checkout and tokens never influence the on-disk name.
fn identity_of(raw: &str) -> String {
    let Some(scheme_end) = raw.find("://") else {
        return raw.to_string();
    };
    let after = &raw[scheme_end + 3..];
    let host_path = match after.find('@') {
        Some(at) => &after[at + 1..],
        None => after,
    };
    format!("{}://{host_path}", &raw[..scheme_end])
}

/// Filesystem-safe slug plus a hash of the credential-free identity.
/// Never call with a URL that still needs its token for the name: the
/// token is stripped first, and the result is safe to log.
pub fn slug_for_repo(repo_ref: &str) -> String {
    let clean = identity_of(repo_ref);
    let mut sanitized = String::new();
    for c in clean.chars() {
        if c.is_ascii_alphanumeric() {
            sanitized.push(c.to_ascii_lowercase());
        } else if !sanitized.ends_with('-') {
            sanitized.push('-');
        }
        if sanitized.len() >= 32 {
            break;
        }
    }
    while sanitized.ends_with('-') {
        sanitized.pop();
    }
    if sanitized.is_empty() {
        sanitized.push_str("repo");
    }
    let digest = fnv1a_hex(&clean);
    format!("{sanitized}-{}", &digest[..12])
}

/// Cache key carrying only a content digest of the credential-free repo
/// identity. The cosmetic slug is deliberately omitted: Windows MAX_PATH (260)
/// counts the whole destination, these keys sit under a deep AppData prefix,
/// and some repos ship 140+ character file paths.
pub fn cache_key(prefix: &str, repo_ref: &str) -> String {
    let digest = fnv1a_hex(&identity_of(repo_ref));
    format!("{prefix}-{}", &digest[..12])
}

/// Stable cache key for the main (target) repository.
pub fn target_key(main_repository: &str) -> String {
    cache_key("t", main_repository)
}

/// Stable cache key for a subsystem source repository. Keyed by repository only,
/// so two subsystems pointing at the same repo share one clone.
pub fn source_key(entry: &SubsystemEntry) -> String {
    cache_key("s", &entry.repository)
}

/// A local repo dir passes through untouched (like `VOS_TARGET_REPO_DIR`):
/// it is never copied into the cache. Returns `None` for remotes.
pub fn local_dir_for(raw: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(raw);
    if candidate.is_dir() {
        return Some(candidate);
    }
    if let Some(stripped) = raw.strip_prefix("file://") {
        let local = PathBuf::from(stripped);
        if local.is_dir() {
            return Some(local);
        }
    }
    None
}

fn clone_fresh(dir: &Path, remote: &str) -> AppResult<()> {
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
    if let Some(parent) = dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // `clone_no_tags` carries any `user:token@` credentials via
    // `-c http.extraHeader`, never in argv; errors are sanitized there.
    git::clone_no_tags(remote, dir).map_err(|err| {
        let _ = std::fs::remove_dir_all(dir);
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not clone into the checkout cache ({}).",
                dir.display()
            ),
        )
        .with_remedy(
            "Check the network connection and that `gh` can read the repository, then retry.",
        )
        .with_details(git::sanitize(&err.to_string()))
    })?;
    #[cfg(test)]
    clone_count_for_tests_inner().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
fn clone_count_for_tests_inner() -> &'static std::sync::atomic::AtomicU64 {
    static COUNT: OnceLock<std::sync::atomic::AtomicU64> = OnceLock::new();
    COUNT.get_or_init(|| std::sync::atomic::AtomicU64::new(0))
}

#[cfg(test)]
pub fn clone_count_for_tests() -> u64 {
    clone_count_for_tests_inner().load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
pub fn reset_clone_count_for_tests() {
    clone_count_for_tests_inner().store(0, std::sync::atomic::Ordering::Relaxed);
}

/// A cached checkout is reusable only when it is a real repository with a
/// resolvable `HEAD`. A clone killed mid-transfer leaves `.git` behind with
/// `HEAD -> refs/heads/.invalid` and no objects; trusting it would poison every
/// read until someone deleted the directory by hand.
fn is_usable_checkout(dir: &Path) -> bool {
    if !dir.join(".git").is_dir() {
        return false;
    }
    if let Ok(head) = std::fs::read_to_string(dir.join(".git").join("HEAD")) {
        if head.contains(".invalid") {
            return false;
        }
    }
    matches!(
        git::run(dir, &["rev-parse", "--verify", "--quiet", "HEAD"]),
        Ok(out) if out.status == 0
    )
}

/// How long a cached checkout is considered fresh enough to skip a network
/// `git fetch`. `VOS_CHECKOUT_FETCH_TTL_SECS` overrides it (`0` = always fetch).
const DEFAULT_FETCH_TTL_SECS: u64 = 180;

fn fetch_ttl_secs() -> u64 {
    std::env::var("VOS_CHECKOUT_FETCH_TTL_SECS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_FETCH_TTL_SECS)
}

/// `git fetch` is what makes a warm read cost a network round trip. `FETCH_HEAD`
/// is written by every fetch, so its mtime is the last-fetch stamp.
fn fetch_is_recent(dir: &Path) -> bool {
    let ttl = fetch_ttl_secs();
    if ttl == 0 {
        return false;
    }
    std::fs::metadata(dir.join(".git").join("FETCH_HEAD"))
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .map(|age| age.as_secs() < ttl)
        .unwrap_or(false)
}

/// Force a fetch of an **already-cloned** checkout, ignoring the freshness window.
/// Returns `Ok(false)` when nothing is cached for `key` (callers must not clone
/// implicitly — new repos are cloned on demand when they are actually opened).
pub fn refresh_cached_checkout(key: &str, remote: &str, token: Option<&str>) -> AppResult<bool> {
    let dir = checkouts_root().join(key);
    if !dir.join(".git").is_dir() {
        return Ok(false);
    }
    let slot = lock_for(key);
    let _guard = slot.lock().unwrap_or_else(|err| err.into_inner());
    if !is_usable_checkout(&dir) {
        let _ = std::fs::remove_dir_all(&dir);
        clone_fresh(&dir, remote)?;
        return Ok(true);
    }
    git::fetch_cached(&dir, token).map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!("Could not refresh the cached checkout ({}).", dir.display()),
        )
        .with_remedy("Check the network connection and that `gh` can read the repository.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    Ok(true)
}

/// Reusable checkout for a remote: clone once, `git fetch` on reuse, never
/// delete. A failed refresh keeps the stale checkout (fail-open read), but a
/// broken/partial checkout is discarded and re-cloned. A fetch is skipped while
/// the checkout is fresh (see [`fetch_ttl_secs`]).
/// Only for remotes — pass local dirs through with [`local_dir_for`].
pub fn ensure_remote_checkout(key: &str, remote: &str, token: Option<&str>) -> AppResult<PathBuf> {
    let dir = checkouts_root().join(key);
    let slot = lock_for(key);
    let _guard = slot.lock().unwrap_or_else(|err| err.into_inner());
    if force_refresh_requested() || (dir.exists() && !is_usable_checkout(&dir)) {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if is_usable_checkout(&dir) {
        if !fetch_is_recent(&dir) {
            let _ = git::fetch_cached(&dir, token);
        }
        return Ok(dir);
    }
    clone_fresh(&dir, remote)?;
    Ok(dir)
}

/// Explicit force-refresh path: delete the cached checkout and clone again.
pub fn ensure_remote_checkout_forced(key: &str, remote: &str) -> AppResult<PathBuf> {
    let dir = checkouts_root().join(key);
    let slot = lock_for(key);
    let _guard = slot.lock().unwrap_or_else(|err| err.into_inner());
    clone_fresh(&dir, remote)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_support::env_lock()
    }

    fn unique_dir(prefix: &str) -> PathBuf {
        let n = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "vos-checkouts-test-{prefix}-{n}-{nanos}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init_repo(dir: &Path, branch: &str) {
        std::fs::create_dir_all(dir).expect("mkdir repo");
        git(dir, &["init", "-q", "-b", branch]);
        git(dir, &["config", "user.email", "t@example.com"]);
        git(dir, &["config", "user.name", "T"]);
        git(dir, &["config", "commit.gpgsign", "false"]);
    }

    fn commit_all(dir: &Path, message: &str) -> String {
        git(dir, &["add", "-A"]);
        git(
            dir,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "--no-verify",
                "-q",
                "-m",
                message,
            ],
        );
        git(dir, &["rev-parse", "HEAD"])
    }

    /// Regression: a clone that died mid-transfer leaves `.git/HEAD` pointing at
    /// `refs/heads/.invalid` and no files. Trusting it produced a bogus
    /// "branch does not exist in the target checkout" for every read.
    #[test]
    fn broken_checkout_is_discarded_and_re_cloned() {
        let _env = env_lock();
        let root = unique_dir("heal");
        let _checkouts = EnvRestore::set("VOS_CHECKOUTS_DIR", root.to_str().expect("utf8"));
        let _refresh = EnvRestore::remove("VOS_CHECKOUT_FORCE_REFRESH");

        let remote = unique_dir("heal-remote");
        init_repo(&remote, "master");
        std::fs::write(remote.join("a.txt"), "one").expect("write");
        commit_all(&remote, "one");
        let remote_url = remote.to_string_lossy().to_string();

        let key = "t-heal";
        let first = ensure_remote_checkout(key, &remote_url, None).expect("first clone");
        assert!(is_usable_checkout(&first));
        assert!(
            first.join("a.txt").is_file(),
            "clone should check out files"
        );

        std::fs::remove_dir_all(&first).expect("wipe");
        std::fs::create_dir_all(first.join(".git").join("refs")).expect("mkdir refs");
        std::fs::write(
            first.join(".git").join("HEAD"),
            "ref: refs/heads/.invalid\n",
        )
        .expect("head");

        let healed = ensure_remote_checkout(key, &remote_url, None).expect("heal");
        assert!(
            is_usable_checkout(&healed),
            "the broken cache must be re-cloned"
        );
        assert!(
            healed.join("a.txt").is_file(),
            "the healed checkout must contain the tracked files"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&remote);
    }

    struct EnvRestore {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvRestore {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            // Tests run single-threaded behind the crate-wide
            // `crate::test_support::env_lock` (see above), and
            // this crate is edition 2021 where `set_var` is still safe.
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvRestore {
        fn drop(&mut self) {
            match self.previous.as_deref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn slug_strips_credentials_and_stays_stable() {
        let authed = "https://x-access-token:ghs_secret123@github.com/ORG/hr-management.git";
        let clean = "https://github.com/ORG/hr-management.git";
        assert_eq!(slug_for_repo(authed), slug_for_repo(clean));
        let slug = slug_for_repo(authed);
        assert!(!slug.contains("ghs_secret123"), "{slug}");
        assert!(!slug.contains('@'), "{slug}");
        // Same repo, different token → same slug (no orphaned checkouts).
        let rotated = "https://x-access-token:other_token@github.com/ORG/hr-management.git";
        assert_eq!(slug_for_repo(rotated), slug);
        // Different repos → different slugs.
        assert_ne!(slug_for_repo("https://github.com/ORG/other.git"), slug);
    }

    #[test]
    fn second_ensure_reuses_checkout_and_fetches_instead_of_cloning() {
        let _guard = env_lock();
        let root = unique_dir("root");
        let _root_env = EnvRestore::set("VOS_CHECKOUTS_DIR", &root.to_string_lossy());
        let _refresh_env = EnvRestore::remove("VOS_CHECKOUT_FORCE_REFRESH");
        reset_clone_count_for_tests();

        let origin = unique_dir("origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        let first_sha = commit_all(&origin, "first");

        let key = "t-test-target";
        let first = ensure_remote_checkout(key, &origin.to_string_lossy(), None)
            .expect("first ensure clones");
        assert_eq!(clone_count_for_tests(), 1);
        assert!(first.join(".git").is_dir());
        assert_eq!(first, root.join(key));

        // Sentinel: a re-clone would wipe this untracked file.
        std::fs::write(first.join("SENTINEL.txt"), "keep me\n").expect("sentinel");

        // New commit lands upstream; the cached clone must pick it up by fetch.
        std::fs::write(origin.join("b.txt"), "b\n").expect("write");
        let second_sha = commit_all(&origin, "second");

        let second = ensure_remote_checkout(key, &origin.to_string_lossy(), None)
            .expect("second ensure reuses");
        assert_eq!(second, first, "same checkout dir is reused");
        assert_eq!(clone_count_for_tests(), 1, "no second `git clone` ran");
        assert!(
            first.join("SENTINEL.txt").is_file(),
            "sentinel survives: the dir was not re-cloned"
        );
        let cached_origin = git(&first, &["rev-parse", "origin/master"]);
        assert_eq!(cached_origin, second_sha, "fetch updated the cached refs");
        assert_ne!(first_sha, second_sha);
    }

    #[test]
    fn force_refresh_reclones_and_env_refresh_flag_works() {
        let _guard = env_lock();
        let root = unique_dir("forceroot");
        let _root_env = EnvRestore::set("VOS_CHECKOUTS_DIR", &root.to_string_lossy());
        let _refresh_env = EnvRestore::remove("VOS_CHECKOUT_FORCE_REFRESH");
        reset_clone_count_for_tests();

        let origin = unique_dir("forceorigin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        let _ = commit_all(&origin, "first");

        let key = "t-test-force";
        let first = ensure_remote_checkout(key, &origin.to_string_lossy(), None).expect("clone");
        std::fs::write(first.join("SENTINEL.txt"), "x\n").expect("sentinel");

        let refreshed =
            ensure_remote_checkout_forced(key, &origin.to_string_lossy()).expect("forced");
        assert_eq!(refreshed, first);
        assert_eq!(clone_count_for_tests(), 2);
        assert!(
            !first.join("SENTINEL.txt").exists(),
            "force-refresh re-clones from scratch"
        );

        // The env flag takes the same path through the normal entry point.
        std::fs::write(first.join("SENTINEL.txt"), "y\n").expect("sentinel");
        let _flag = EnvRestore::set("VOS_CHECKOUT_FORCE_REFRESH", "1");
        let via_flag =
            ensure_remote_checkout(key, &origin.to_string_lossy(), None).expect("flag refresh");
        assert_eq!(via_flag, first);
        assert!(!first.join("SENTINEL.txt").exists());
    }

    #[test]
    fn local_dirs_pass_through_and_are_never_cached() {
        let _guard = env_lock();
        let root = unique_dir("passthrough");
        let _root_env = EnvRestore::set("VOS_CHECKOUTS_DIR", &root.to_string_lossy());

        let local = unique_dir("localrepo");
        init_repo(&local, "master");
        assert_eq!(local_dir_for(&local.to_string_lossy()), Some(local.clone()));
        assert!(local_dir_for("https://github.com/ORG/hr.git").is_none());
        // Nothing was cloned into the cache root for a passthrough path.
        assert!(
            std::fs::read_dir(&root)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true),
            "passthrough must not populate the cache"
        );
    }
}

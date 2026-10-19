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
        return base.join("vos-origins").join("checkouts");
    }
    std::env::temp_dir().join("vos-origins").join("checkouts")
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

pub fn target_key(main_repository: &str) -> String {
    cache_key("t", main_repository)
}

pub fn source_key(entry: &SubsystemEntry) -> String {
    cache_key("s", &entry.repository)
}

pub fn sweep_orphaned_checkouts() {
    let Ok(entries) = std::fs::read_dir(checkouts_root()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some((prefix, digest)) = name.split_once('-') else {
            continue;
        };
        if !matches!(prefix, "t" | "s") {
            continue;
        }
        let current_scheme = digest.len() == 12 && digest.chars().all(|c| c.is_ascii_hexdigit());
        if !current_scheme {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

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

pub fn fetch_local_clone_if_stale(dir: &Path, token: Option<&str>) -> bool {
    if !dir.join(".git").exists() {
        return false;
    }
    if fetch_is_recent(dir) {
        return false;
    }
    let _ = git::fetch_local_clone(dir, token);
    true
}

/// What a post-fetch fast-forward accomplished, so "Pull latest" can tell
/// *advanced to a new tip*, *already up to date*, and *fetched but could
/// not advance* apart instead of reporting all three as refreshed.
pub enum RefreshOutcome {
    Advanced(Vec<String>),
    UpToDate,
    HeldBack(String),
    /// Nothing cached under the key, so there was nothing to fetch.
    Missing,
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Why a branch that differs from its upstream without being strictly
/// behind it must be left alone. Callers already know the tips differ.
fn hold_reason(dir: &Path, branch: &str, local: &str, remote: &str) -> String {
    if git::is_ancestor(dir, remote, local) {
        format!("branch '{branch}' has local commits not on its upstream; leaving it alone")
    } else {
        format!("branch '{branch}' has diverged from its upstream; leaving it alone (never force-moves a rewind)")
    }
}

/// Fast-forward every local branch strictly behind its remote-tracking
/// upstream. App-owned scratch clones only: no user state lives here, so
/// every stale branch moves. Branches ahead of or diverged from their
/// upstream are left alone, never force-moved.
fn fast_forward_behind_branches(dir: &Path) -> RefreshOutcome {
    let mut advanced = Vec::new();
    let mut held: Vec<String> = Vec::new();
    for branch in git::list_branches(dir) {
        let Some(upstream) = git::remote_tracking_ref(dir, &branch) else {
            continue;
        };
        let (Some(local), Some(remote)) = (
            git::rev_parse(dir, &format!("refs/heads/{branch}")),
            git::rev_parse(dir, &upstream),
        ) else {
            continue;
        };
        if local == remote {
            continue;
        }
        if !git::is_ancestor(dir, &local, &remote) {
            held.push(hold_reason(dir, &branch, &local, &remote));
            continue;
        }
        if git::update_ref(dir, &format!("refs/heads/{branch}"), &remote, &local).is_ok() {
            advanced.push(format!("{}@{}", branch, short_sha(&remote)));
        }
    }
    if !advanced.is_empty() {
        RefreshOutcome::Advanced(advanced)
    } else if !held.is_empty() {
        RefreshOutcome::HeldBack(held.join("; "))
    } else {
        RefreshOutcome::UpToDate
    }
}

pub fn refresh_local_clone(dir: &Path, token: Option<&str>) -> AppResult<RefreshOutcome> {
    git::fetch_local_clone(dir, token).map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not refresh the local main clone ({}).",
                dir.display()
            ),
        )
        .with_remedy("Check the network connection and that the clone's remote is reachable.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    let Some(branch) = git::current_branch(dir) else {
        return Ok(RefreshOutcome::HeldBack(
            "not on any branch (detached HEAD); leaving it alone".to_string(),
        ));
    };
    let Some(upstream) = git::remote_tracking_ref(dir, &branch) else {
        return Ok(RefreshOutcome::HeldBack(format!(
            "branch '{branch}' has no upstream tracking branch; nothing to advance from"
        )));
    };
    let (Some(local), Some(remote)) = (
        git::rev_parse(dir, &format!("refs/heads/{branch}")),
        git::rev_parse(dir, &upstream),
    ) else {
        return Ok(RefreshOutcome::HeldBack(format!(
            "could not resolve branch '{branch}' or its upstream"
        )));
    };
    if local == remote {
        return Ok(RefreshOutcome::UpToDate);
    }
    if !git::is_ancestor(dir, &local, &remote) {
        return Ok(RefreshOutcome::HeldBack(hold_reason(
            dir, &branch, &local, &remote,
        )));
    }
    if let Err(err) = git::merge_ff_only(dir, &remote) {
        return Ok(RefreshOutcome::HeldBack(format!(
            "could not fast-forward '{branch}': {}",
            git::sanitize(&err.to_string())
        )));
    }
    Ok(RefreshOutcome::Advanced(vec![format!(
        "{}@{}",
        branch,
        short_sha(&remote)
    )]))
}

/// Force a fetch of an **already-cloned** checkout, ignoring the freshness window.
/// Afterwards fast-forwards every local branch strictly behind its upstream
/// (app-owned scratch: no user state to protect). Returns what moved so the
/// caller can report *advanced* apart from *already up to date*.
/// `Missing` when nothing is cached for `key` (callers must not clone
/// implicitly — new repos are cloned on demand when they are actually opened).
pub fn refresh_cached_checkout(
    key: &str,
    remote: &str,
    token: Option<&str>,
) -> AppResult<RefreshOutcome> {
    let dir = checkouts_root().join(key);
    if !dir.join(".git").is_dir() {
        return Ok(RefreshOutcome::Missing);
    }
    let slot = lock_for(key);
    let _guard = slot.lock().unwrap_or_else(|err| err.into_inner());
    if !is_usable_checkout(&dir) {
        let _ = std::fs::remove_dir_all(&dir);
        clone_fresh(&dir, remote)?;
        let tip = git::rev_parse(&dir, "HEAD").map(|sha| short_sha(&sha));
        return Ok(RefreshOutcome::Advanced(vec![format!(
            "re-cloned@{}",
            tip.unwrap_or_default()
        )]));
    }
    git::fetch_cached(&dir, token).map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!("Could not refresh the cached checkout ({}).", dir.display()),
        )
        .with_remedy("Check the network connection and that `gh` can read the repository.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    Ok(fast_forward_behind_branches(&dir))
}

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
        let rotated = "https://x-access-token:other_token@github.com/ORG/hr-management.git";
        assert_eq!(slug_for_repo(rotated), slug);
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

        std::fs::write(first.join("SENTINEL.txt"), "y\n").expect("sentinel");
        let _flag = EnvRestore::set("VOS_CHECKOUT_FORCE_REFRESH", "1");
        let via_flag =
            ensure_remote_checkout(key, &origin.to_string_lossy(), None).expect("flag refresh");
        assert_eq!(via_flag, first);
        assert!(!first.join("SENTINEL.txt").exists());
    }

    #[test]
    fn local_clone_is_fetched_when_stale_and_skipped_when_fresh() {
        let _env = env_lock();
        let root = unique_dir("local-fetch");
        let origin = unique_dir("local-fetch-origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        commit_all(&origin, "first");
        let clone = root.join("clone");
        git(
            &root,
            &[
                "clone",
                "-q",
                origin.to_str().expect("utf8"),
                clone.to_str().expect("utf8"),
            ],
        );

        let _ttl = EnvRestore::set("VOS_CHECKOUT_FETCH_TTL_SECS", "0");
        std::fs::write(origin.join("b.txt"), "b\n").expect("write");
        let second = commit_all(&origin, "second");
        assert!(
            fetch_local_clone_if_stale(&clone, None),
            "an expired TTL must attempt the fetch"
        );
        assert_eq!(
            git(&clone, &["rev-parse", "origin/master"]),
            second,
            "the fetch must advance the clone's remote-tracking ref"
        );

        let _ttl = EnvRestore::set("VOS_CHECKOUT_FETCH_TTL_SECS", "100000");
        std::fs::write(origin.join("c.txt"), "c\n").expect("write");
        let third = commit_all(&origin, "third");
        assert!(
            !fetch_local_clone_if_stale(&clone, None),
            "a fresh clone must skip the network round trip"
        );
        assert_ne!(
            git(&clone, &["rev-parse", "origin/master"]),
            third,
            "a skipped fetch must leave the remote-tracking ref untouched"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn local_clone_fetch_failure_is_swallowed_and_leaves_the_clone_readable() {
        let _env = env_lock();
        let root = unique_dir("local-fetch-fail");
        let clone = root.join("clone");
        init_repo(&clone, "master");
        std::fs::write(clone.join("a.txt"), "a\n").expect("write");
        let sha = commit_all(&clone, "only local");
        git(
            &clone,
            &[
                "remote",
                "add",
                "origin",
                "https://127.0.0.1:1/ORG/does-not-exist.git",
            ],
        );
        let _ttl = EnvRestore::set("VOS_CHECKOUT_FETCH_TTL_SECS", "0");
        assert!(
            fetch_local_clone_if_stale(&clone, None),
            "the fetch is attempted even when it will fail"
        );
        assert_eq!(git(&clone, &["rev-parse", "HEAD"]), sha);
        assert!(clone.join("a.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refresh_local_clone_force_fetches_and_reports_failure() {
        let _env = env_lock();
        let root = unique_dir("local-refresh");
        let origin = unique_dir("local-refresh-origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        commit_all(&origin, "first");
        let clone = root.join("clone");
        git(
            &root,
            &[
                "clone",
                "-q",
                origin.to_str().expect("utf8"),
                clone.to_str().expect("utf8"),
            ],
        );
        std::fs::write(origin.join("b.txt"), "b\n").expect("write");
        let second = commit_all(&origin, "second");
        let _ttl = EnvRestore::set("VOS_CHECKOUT_FETCH_TTL_SECS", "100000");
        refresh_local_clone(&clone, None).expect("force refresh");
        assert_eq!(git(&clone, &["rev-parse", "origin/master"]), second);

        git(
            &clone,
            &[
                "remote",
                "set-url",
                "origin",
                "https://127.0.0.1:1/ORG/does-not-exist.git",
            ],
        );
        assert!(refresh_local_clone(&clone, None).is_err());

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&origin);
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
        assert!(
            std::fs::read_dir(&root)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true),
            "passthrough must not populate the cache"
        );
    }

    #[test]
    fn refresh_cached_checkout_advances_a_behind_local_branch() {
        let _env = env_lock();
        let root = unique_dir("ff-root");
        let _root_env = EnvRestore::set("VOS_CHECKOUTS_DIR", &root.to_string_lossy());
        let _refresh_env = EnvRestore::remove("VOS_CHECKOUT_FORCE_REFRESH");
        reset_clone_count_for_tests();

        let origin = unique_dir("ff-origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        let first = commit_all(&origin, "first");

        let key = "t-ff-behind";
        let cached = ensure_remote_checkout(key, &origin.to_string_lossy(), None).expect("clone");
        assert_eq!(git(&cached, &["rev-parse", "master"]), first);

        std::fs::write(origin.join("b.txt"), "b\n").expect("write");
        let second = commit_all(&origin, "second");

        match refresh_cached_checkout(key, &origin.to_string_lossy(), None).expect("refresh") {
            RefreshOutcome::Advanced(entries) => assert!(
                entries.iter().any(|e| e.starts_with("master@")),
                "must name the advanced branch: {entries:?}"
            ),
            _ => panic!("a behind cache must report Advanced"),
        }
        assert_eq!(git(&cached, &["rev-parse", "master"]), second);
        assert_eq!(git(&cached, &["rev-parse", "origin/master"]), second);
        assert_eq!(
            git::resolve_branch_ref(&cached, "master").as_deref(),
            Some("refs/heads/master"),
            "reads keep resolving the local ref, which must now be the new tip"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn refresh_cached_checkout_leaves_an_ahead_branch_alone() {
        let _env = env_lock();
        let root = unique_dir("ff-ahead-root");
        let _root_env = EnvRestore::set("VOS_CHECKOUTS_DIR", &root.to_string_lossy());
        let _refresh_env = EnvRestore::remove("VOS_CHECKOUT_FORCE_REFRESH");
        reset_clone_count_for_tests();

        let origin = unique_dir("ff-ahead-origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        commit_all(&origin, "first");

        let key = "t-ff-ahead";
        let cached = ensure_remote_checkout(key, &origin.to_string_lossy(), None).expect("clone");
        git(&cached, &["config", "user.email", "t@example.com"]);
        git(&cached, &["config", "user.name", "T"]);
        std::fs::write(cached.join("local.txt"), "local\n").expect("write");
        let local_tip = commit_all(&cached, "local work");

        let outcome =
            refresh_cached_checkout(key, &origin.to_string_lossy(), None).expect("refresh");
        match outcome {
            RefreshOutcome::HeldBack(reason) => assert!(
                reason.contains("local commits"),
                "an ahead branch is held with its reason: {reason}"
            ),
            _ => panic!("an ahead branch must be held back, never force-moved"),
        }
        assert_eq!(
            git(&cached, &["rev-parse", "master"]),
            local_tip,
            "a branch ahead of its upstream must never be force-moved"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn refresh_local_clone_fast_forwards_the_checked_out_branch() {
        let _env = env_lock();
        let root = unique_dir("user-ff");
        let origin = unique_dir("user-ff-origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        commit_all(&origin, "first");
        let clone = root.join("clone");
        git(
            &root,
            &[
                "clone",
                "-q",
                origin.to_str().expect("utf8"),
                clone.to_str().expect("utf8"),
            ],
        );
        std::fs::write(origin.join("b.txt"), "b\n").expect("write");
        let second = commit_all(&origin, "second");

        match refresh_local_clone(&clone, None).expect("refresh") {
            RefreshOutcome::Advanced(entries) => assert!(
                entries.iter().any(|e| e.starts_with("master@")),
                "must name the advanced branch and tip: {entries:?}"
            ),
            _ => panic!("a clean behind clone must report Advanced"),
        }
        assert_eq!(git(&clone, &["rev-parse", "master"]), second);
        assert!(
            clone.join("b.txt").is_file(),
            "the working tree advances with the checked-out branch"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&origin);
    }

    #[test]
    fn refresh_local_clone_leaves_a_divergent_branch_alone_and_reports_it() {
        let _env = env_lock();
        let root = unique_dir("user-diverged");
        let origin = unique_dir("user-diverged-origin");
        init_repo(&origin, "master");
        std::fs::write(origin.join("a.txt"), "a\n").expect("write");
        commit_all(&origin, "first");
        let clone = root.join("clone");
        git(
            &root,
            &[
                "clone",
                "-q",
                origin.to_str().expect("utf8"),
                clone.to_str().expect("utf8"),
            ],
        );
        std::fs::write(origin.join("b.txt"), "b\n").expect("write");
        let second = commit_all(&origin, "second");
        git(&clone, &["config", "user.email", "t@example.com"]);
        git(&clone, &["config", "user.name", "T"]);
        std::fs::write(clone.join("c.txt"), "local\n").expect("write");
        let local_tip = commit_all(&clone, "local work");

        match refresh_local_clone(&clone, None).expect("report, not error") {
            RefreshOutcome::HeldBack(reason) => assert!(
                reason.contains("diverged"),
                "the reason must name the hazard: {reason}"
            ),
            _ => panic!("a divergent branch must be held back, never force-moved"),
        }
        assert_eq!(
            git(&clone, &["rev-parse", "master"]),
            local_tip,
            "a divergent branch must never be force-moved"
        );
        assert_eq!(
            git(&clone, &["rev-parse", "origin/master"]),
            second,
            "the fetch still updates the remote-tracking ref"
        );

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&origin);
    }
}

//! Git plumbing: explicit argv (never shell strings), raw bytes for blobs (spec §6).

use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
pub struct GitOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl GitOutput {
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stdout_trimmed(&self) -> String {
        self.stdout_str().trim().to_string()
    }

    pub fn ok(&self) -> bool {
        self.status == 0
    }
}

#[derive(Debug, Clone)]
pub struct GitError {
    pub args: String,
    pub status: Option<i32>,
    pub stderr: String,
}

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.status {
            Some(status) => write!(
                f,
                "git {} failed (exit {}): {}",
                self.args,
                status,
                self.stderr.trim()
            ),
            None => write!(
                f,
                "git {} could not be started: {}",
                self.args,
                self.stderr.trim()
            ),
        }
    }
}

impl std::error::Error for GitError {}

/// A git process that can never open an interactive credential prompt (Git Credential
/// Manager, askpass, or the terminal): the engine always supplies its own token, so
/// anything else must fail fast instead of popping a sign-in dialog.
fn base_command() -> Command {
    let mut command = crate::platform::hidden("git");
    command
        .arg("-c")
        .arg("credential.helper=")
        // Windows MAX_PATH (260) counts the whole destination path. Cached
        // checkouts sit under a deep AppData prefix and some repos have 140+
        // character file paths, so without this a clone downloads everything and
        // then fails the checkout ("Clone succeeded, but checkout failed").
        .arg("-c")
        .arg("core.longpaths=true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never");
    command
}

/// `Err` = spawn failure; non-zero exit is `Ok` with `status != 0`.
pub fn run(repo_dir: &Path, args: &[&str]) -> Result<GitOutput, GitError> {
    #[cfg(test)]
    count_spawn_for_tests(repo_dir);
    let mut command = base_command();
    command.arg("-C").arg(repo_dir).args(args);
    let output = command.output().map_err(|err| GitError {
        args: args.join(" "),
        status: None,
        stderr: err.to_string(),
    })?;
    Ok(GitOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn run_ok(repo_dir: &Path, args: &[&str]) -> Result<GitOutput, GitError> {
    let out = run(repo_dir, args)?;
    if out.status != 0 {
        return Err(GitError {
            args: args.join(" "),
            status: Some(out.status),
            stderr: out.stderr,
        });
    }
    Ok(out)
}

/// Like [`run`] with bytes on stdin (for `--batch-check` style plumbing).
/// Counts as one spawn for tests, exactly like [`run`].
pub fn run_with_stdin(repo_dir: &Path, args: &[&str], stdin: &[u8]) -> Result<GitOutput, GitError> {
    #[cfg(test)]
    count_spawn_for_tests(repo_dir);
    use std::io::Write;
    let mut command = base_command();
    command.arg("-C").arg(repo_dir).args(args);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|err| GitError {
        args: args.join(" "),
        status: None,
        stderr: err.to_string(),
    })?;
    if let Some(mut pipe) = child.stdin.take() {
        let _ = pipe.write_all(stdin);
    }
    let output = child.wait_with_output().map_err(|err| GitError {
        args: args.join(" "),
        status: None,
        stderr: err.to_string(),
    })?;
    Ok(GitOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

pub fn run_global(args: &[&str]) -> Result<GitOutput, GitError> {
    let output = base_command().args(args).output().map_err(|err| GitError {
        args: args.join(" "),
        status: None,
        stderr: err.to_string(),
    })?;
    Ok(GitOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Test-only: number of `git` spawns issued against `repo_dir` in this
/// process. Keyed by directory so parallel tests in their own sandboxes do
/// not disturb each other's counts. Used by the page-cost regression test
/// to prove N rows cost O(1) spawns rather than O(N).
#[cfg(test)]
pub fn git_spawns_for_tests(repo_dir: &Path) -> u64 {
    spawn_counts_for_tests()
        .lock()
        .map(|counts| {
            counts
                .get(&repo_dir.to_string_lossy().to_string())
                .copied()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

#[cfg(test)]
fn spawn_counts_for_tests() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static COUNTS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    COUNTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
fn count_spawn_for_tests(repo_dir: &Path) {
    if let Ok(mut counts) = spawn_counts_for_tests().lock() {
        *counts
            .entry(repo_dir.to_string_lossy().to_string())
            .or_insert(0) += 1;
    }
}

pub fn rev_parse(repo_dir: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = run(repo_dir, &["rev-parse", "--verify", "--quiet", &spec]).ok()?;
    if out.status == 0 {
        let sha = out.stdout_trimmed();
        if sha.is_empty() {
            None
        } else {
            Some(sha)
        }
    } else {
        None
    }
}

pub fn is_ancestor(repo_dir: &Path, ancestor: &str, descendant: &str) -> bool {
    if rev_parse(repo_dir, ancestor).is_none() || rev_parse(repo_dir, descendant).is_none() {
        return false;
    }
    run(
        repo_dir,
        &["merge-base", "--is-ancestor", ancestor, descendant],
    )
    .map(|out| out.status == 0)
    .unwrap_or(false)
}

pub fn merge_base(repo_dir: &Path, a: &str, b: &str) -> Option<String> {
    let out = run(repo_dir, &["merge-base", a, b]).ok()?;
    if out.status == 0 {
        Some(out.stdout_trimmed())
    } else {
        None
    }
}

pub fn commit_subject(repo_dir: &Path, sha: &str) -> String {
    run_ok(repo_dir, &["show", "-s", "--format=%s", sha])
        .map(|out| out.stdout_trimmed())
        .unwrap_or_default()
}

pub fn commit_message(repo_dir: &Path, sha: &str) -> String {
    run_ok(repo_dir, &["show", "-s", "--format=%B", sha])
        .map(|out| out.stdout_str())
        .unwrap_or_default()
}

/// Committer time of `sha` in epoch seconds (`None` when unresolvable).
/// Used as the sort key that makes unrelated selected SHAs order
/// deterministically (committer epoch, then SHA). Display order only -
/// blob membership decides correctness.
pub fn commit_epoch(repo_dir: &Path, sha: &str) -> Option<i64> {
    let out = run(repo_dir, &["show", "-s", "--format=%ct", sha]).ok()?;
    if out.status != 0 {
        return None;
    }
    out.stdout_trimmed().parse::<i64>().ok()
}

pub fn first_parent(repo_dir: &Path, sha: &str) -> Option<String> {
    rev_parse(repo_dir, &format!("{sha}^1"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaEntry {
    pub status: char,
    pub path: Option<String>,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub score: Option<u32>,
}

impl DeltaEntry {
    pub fn effective_path(&self) -> String {
        self.new_path
            .clone()
            .or_else(|| self.path.clone())
            .unwrap_or_default()
    }

    pub fn is_rename_or_copy(&self) -> bool {
        self.status == 'R' || self.status == 'C'
    }
}

/// Root commits use `diff-tree --root` so they are not misreported as no-ops.
pub fn name_status_delta(
    repo_dir: &Path,
    sha: &str,
) -> Result<(Option<String>, Vec<DeltaEntry>), GitError> {
    match first_parent(repo_dir, sha) {
        Some(parent) => {
            let out = run(
                repo_dir,
                &["diff", "--name-status", "-z", "-M", &parent, sha],
            )?;
            Ok((Some(parent), parse_name_status_z(&out.stdout)))
        }
        None => {
            let out = run(
                repo_dir,
                &[
                    "diff-tree",
                    "--root",
                    "--no-commit-id",
                    "--name-status",
                    "-r",
                    "-z",
                    "-M",
                    sha,
                ],
            )?;
            Ok((None, parse_name_status_z(&out.stdout)))
        }
    }
}

pub fn name_status_range(
    repo_dir: &Path,
    base: &str,
    head: &str,
) -> Result<Vec<DeltaEntry>, GitError> {
    let out = run(repo_dir, &["diff", "--name-status", "-z", "-M", base, head])?;
    Ok(parse_name_status_z(&out.stdout))
}

fn parse_name_status_z(bytes: &[u8]) -> Vec<DeltaEntry> {
    let text = String::from_utf8_lossy(bytes);
    let mut tokens = text.split('\0');
    let mut entries = Vec::new();
    while let Some(status) = tokens.next() {
        if status.is_empty() {
            continue;
        }
        let letter = status.chars().next().unwrap_or(' ');
        if letter == 'R' || letter == 'C' {
            let old_path = tokens.next().unwrap_or("").to_string();
            let new_path = tokens.next().unwrap_or("").to_string();
            let score = status[1..].parse::<u32>().ok();
            entries.push(DeltaEntry {
                status: letter,
                path: None,
                old_path: Some(old_path),
                new_path: Some(new_path),
                score,
            });
        } else {
            let path = tokens.next().unwrap_or("").to_string();
            entries.push(DeltaEntry {
                status: letter,
                path: Some(path),
                old_path: None,
                new_path: None,
                score: None,
            });
        }
    }
    entries
}

pub fn read_blob(repo_dir: &Path, sha: &str, path: &str) -> Option<Vec<u8>> {
    let spec = format!("{sha}:{path}");
    let out = run(repo_dir, &["cat-file", "blob", &spec]).ok()?;
    if out.status == 0 {
        Some(out.stdout)
    } else {
        None
    }
}

pub fn path_mode(repo_dir: &Path, sha: &str, path: &str) -> Option<String> {
    let out = run(repo_dir, &["ls-tree", "-z", sha, "--", path]).ok()?;
    if out.status != 0 {
        return None;
    }
    let text = out.stdout_str();
    if text.is_empty() {
        return None;
    }
    text.split(' ')
        .next()
        .map(|mode| mode.to_string())
        .filter(|m| !m.is_empty())
}

pub fn show_file(repo_dir: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let spec = format!("{rev}:{path}");
    let out = run(repo_dir, &["show", &spec]).ok()?;
    if out.status == 0 {
        Some(out.stdout)
    } else {
        None
    }
}

pub fn file_exists_at(repo_dir: &Path, rev: &str, path: &str) -> bool {
    let spec = format!("{rev}:{path}");
    run(repo_dir, &["cat-file", "-e", &spec])
        .map(|o| o.status == 0)
        .unwrap_or(false)
}

pub fn list_branches(repo_dir: &Path) -> Vec<String> {
    run_ok(repo_dir, &["branch", "--format=%(refname:short)"])
        .map(|out| {
            out.stdout_str()
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// True when `dir` is inside a git work tree / repository. Used to fail
/// closed for `local` delivery before any branch is created: a non-repo
/// path must never be treated as the user's clone.
pub fn is_git_repository(dir: &Path) -> bool {
    run(dir, &["rev-parse", "--git-dir"])
        .map(|out| out.status == 0)
        .unwrap_or(false)
}

/// True when `refs/heads/<branch>` already exists locally. `local` delivery
/// refuses to overwrite it so a re-run never clobbers the user's branch.
pub fn local_branch_exists(repo_dir: &Path, branch: &str) -> bool {
    rev_parse(repo_dir, &format!("refs/heads/{branch}")).is_some()
}

/// Resolve `branch` to a revision name [`rev_parse`] can consume: the local
/// ref (`refs/heads/<branch>`), then the bare name, then any remote-tracking
/// ref. The remote case is required because a fork clone keeps the real
/// branches on a non-`origin` remote (e.g. `upstream`), so
/// `refs/remotes/origin/<branch>` alone would report an existing branch as
/// missing.
pub fn resolve_branch_ref(repo_dir: &Path, branch: &str) -> Option<String> {
    let local = format!("refs/heads/{branch}");
    if rev_parse(repo_dir, &local).is_some() {
        return Some(local);
    }
    if rev_parse(repo_dir, branch).is_some() {
        return Some(branch.to_string());
    }
    remote_tracking_ref(repo_dir, branch)
}

/// First `refs/remotes/<remote>/<branch>` that exists, resolved in ONE
/// `for-each-ref` call (git expands the `*` glob). Remotes are never
/// enumerated in Rust; this runs only after the cheap local checks fail.
fn remote_tracking_ref(repo_dir: &Path, branch: &str) -> Option<String> {
    let pattern = format!("refs/remotes/*/{branch}");
    let out = run(
        repo_dir,
        &["for-each-ref", "--count=1", "--format=%(refname)", &pattern],
    )
    .ok()?;
    if out.status != 0 {
        return None;
    }
    let name = out.stdout_trimmed();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// True when `branch` exists in `repo_dir`: `refs/heads/<branch>` or the bare
/// name resolves, OR any remote-tracking ref matches (a fork clone whose
/// branches live on `upstream`). The remote glob costs one extra call and
/// runs only when the local checks fail.
pub fn branch_exists(repo_dir: &Path, branch: &str) -> bool {
    resolve_branch_ref(repo_dir, branch).is_some()
}

/// Short name of the currently checked-out branch (`None` when detached or
/// unresolvable). `local_direct` delivery uses it to decide between a
/// checkout fast-forward and a ref-only move.
pub fn current_branch(repo_dir: &Path) -> Option<String> {
    let out = run(repo_dir, &["symbolic-ref", "--short", "HEAD"]).ok()?;
    if out.status != 0 {
        return None;
    }
    let name = out.stdout_trimmed();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Current tip of the *local* branch ref (`refs/heads/<branch>`), ignoring
/// remote-tracking refs. `local_direct` delivery pins this before building
/// the commit and re-checks it before moving the ref, so the target can only
/// ever advance fast-forward underneath the new commit.
pub fn local_tip(repo_dir: &Path, branch: &str) -> Option<String> {
    rev_parse(repo_dir, &format!("refs/heads/{branch}"))
}

/// Atomic compare-and-swap of a branch ref: moves `refname` to `new_sha`
/// only when it still points at `old_sha`. Used by `local_direct` delivery
/// when the target is not checked out, so the working tree and the current
/// branch are left completely untouched.
pub fn update_ref(
    repo_dir: &Path,
    refname: &str,
    new_sha: &str,
    old_sha: &str,
) -> Result<(), GitError> {
    run_ok(repo_dir, &["update-ref", refname, new_sha, old_sha])?;
    Ok(())
}

/// Fast-forward the checked-out branch to `commit`. Used by `local_direct`
/// delivery when the target is the checked-out branch (a clean tree is
/// asserted first). `--ff-only` by construction: never `--hard`, never
/// discards user work.
pub fn merge_ff_only(repo_dir: &Path, commit: &str) -> Result<(), GitError> {
    run_ok(repo_dir, &["merge", "--ff-only", commit])?;
    Ok(())
}

/// Quarantine ref, so selected SHAs are present in this object DB.
///
/// The bearer token never lands in argv: any `user:token@` credentials are
/// stripped from the URL and sent via `git -c http.extraHeader=...` instead,
/// and failures carry only the redacted argv + sanitized stderr.
pub fn fetch_source_branch(
    repo_dir: &Path,
    url: &str,
    source_branch: &str,
    remote_name: &str,
) -> Result<String, GitError> {
    let dest_ref = format!("refs/vos-sources/{remote_name}/{source_branch}");
    let refspec = format!("+refs/heads/{source_branch}:{dest_ref}");
    let (clean_url, token) = split_credentials(url);
    let (mut command, mut display) = authed_command(Some(repo_dir), token.as_deref());
    let args = ["fetch", "--no-tags", "--force", &clean_url, &refspec];
    command.args(args);
    display.extend(args.iter().map(|s| s.to_string()));
    run_authed(command, &display)
        .map_err(|err| GitError {
            args: sanitize(&err.args),
            status: err.status,
            stderr: sanitize(&err.stderr),
        })
        .and_then(|out| {
            if out.status != 0 {
                Err(GitError {
                    args: sanitize(&display.join(" ")),
                    status: Some(out.status),
                    stderr: sanitize(&out.stderr),
                })
            } else {
                Ok(dest_ref)
            }
        })
}

pub fn worktree_add(
    repo_dir: &Path,
    worktree_dir: &Path,
    branch: &str,
    start_point: &str,
) -> Result<(), GitError> {
    let worktree = worktree_dir.to_string_lossy().to_string();
    // Force LF checkouts in the engine-owned worktree regardless of the
    // user's `core.autocrlf` so worktree bytes stay identical to blobs.
    // The `-c` flags only affect this invocation; the user's repository
    // config is never mutated — only the new worktree's config below.
    let mut command = base_command();
    command
        .arg("-c")
        .arg("core.autocrlf=false")
        .arg("-c")
        .arg("core.eol=lf")
        .arg("-C")
        .arg(repo_dir)
        .args([
            "worktree",
            "add",
            "-f",
            "-b",
            branch,
            &worktree,
            start_point,
        ]);
    let output = command.output().map_err(|err| GitError {
        args: format!("worktree add -f -b {branch} {worktree} {start_point}"),
        status: None,
        stderr: err.to_string(),
    })?;
    if !output.status.success() {
        return Err(GitError {
            args: format!("worktree add -f -b {branch} {worktree} {start_point}"),
            status: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }
    // Persist LF behaviour for later git operations inside this
    // engine-owned worktree (checkout, commit, add). Never touches `repo_dir`.
    // The `-c core.autocrlf=false -c core.eol=lf` flags above already produced an
    // LF checkout, which is all the engine needs (it writes blob bytes and
    // commits; it never re-checks out). Writing config here would land in the
    // worktree's SHARED config, mutating the owner's repository when the worktree
    // is created off the user's own clone — so it is deliberately not done.
    Ok(())
}

pub fn worktree_remove(repo_dir: &Path, worktree_dir: &Path) {
    let worktree = worktree_dir.to_string_lossy().to_string();
    let _ = run(repo_dir, &["worktree", "remove", "--force", &worktree]);
}

pub fn clone_no_tags(url: &str, dest: &Path) -> Result<(), GitError> {
    let dest_str = dest.to_string_lossy().to_string();
    let (clean_url, token) = split_credentials(url);
    let (mut command, mut display) = authed_command(None, token.as_deref());
    // `-c` after `clone` persists into the new (engine-owned) clone's config
    // so its checkouts stay LF regardless of the user's global autocrlf.
    let args = [
        "clone",
        "-c",
        "core.autocrlf=false",
        "-c",
        "core.eol=lf",
        "--no-tags",
        &clean_url,
        &dest_str,
    ];
    command.args(args);
    display.extend(args.iter().map(|s| s.to_string()));
    let out = run_authed(command, &display).map_err(|err| GitError {
        args: sanitize(&err.args),
        status: err.status,
        stderr: sanitize(&err.stderr),
    })?;
    if out.status != 0 {
        return Err(GitError {
            args: format!("clone {}", sanitize(&clean_url)),
            status: Some(out.status),
            stderr: sanitize(&out.stderr),
        });
    }
    // Backstop for servers/versions that ignore `clone -c`: enforce LF in the
    // engine-owned clone only. Never touches any user repository.
    let _ = run(dest, &["config", "core.autocrlf", "false"]);
    let _ = run(dest, &["config", "core.eol", "lf"]);
    Ok(())
}

pub fn add_all(worktree_dir: &Path) -> Result<(), GitError> {
    run_ok(worktree_dir, &["add", "-A"])?;
    Ok(())
}

pub fn status_porcelain(worktree_dir: &Path) -> String {
    run_ok(worktree_dir, &["status", "--porcelain"])
        .map(|o| o.stdout_str())
        .unwrap_or_default()
}

pub fn update_index_chmod(worktree_dir: &Path, path: &str, executable: bool) {
    let mode = if executable {
        "--chmod=+x"
    } else {
        "--chmod=-x"
    };
    let _ = run(worktree_dir, &["update-index", mode, "--", path]);
}

pub fn commit_with_message(
    worktree_dir: &Path,
    message_file: &Path,
    author_name: &str,
    author_email: &str,
) -> Result<String, GitError> {
    let file = message_file.to_string_lossy().to_string();
    let name = format!("user.name={author_name}");
    let email = format!("user.email={author_email}");
    run_ok(
        worktree_dir,
        &[
            "-c",
            &name,
            "-c",
            &email,
            "commit",
            "--no-verify",
            "--allow-empty",
            "-F",
            &file,
        ],
    )?;
    Ok(rev_parse(worktree_dir, "HEAD").unwrap_or_default())
}

pub fn push(repo_dir: &Path, url: &str, branch: &str) -> Result<(), GitError> {
    let refspec = format!("{branch}:refs/heads/{branch}");
    let (clean_url, token) = split_credentials(url);
    let (mut command, mut display) = authed_command(Some(repo_dir), token.as_deref());
    let args = ["push", &clean_url, &refspec];
    command.args(args);
    display.extend(args.iter().map(|s| s.to_string()));
    let out = run_authed(command, &display).map_err(|err| GitError {
        args: format!("push {branch}"),
        status: err.status,
        stderr: sanitize(&err.stderr),
    })?;
    if out.status != 0 {
        return Err(GitError {
            args: format!("push {branch}"),
            status: Some(out.status),
            stderr: sanitize(&out.stderr),
        });
    }
    Ok(())
}

/// Best-effort refresh for a cached checkout: `git fetch --no-tags --prune
/// origin` so later reads see new branches without re-cloning. The bearer
/// token (when the origin is remote and a token is available) travels via
/// `-c http.extraHeader`, never in argv; failures carry only redacted text.
pub fn fetch_cached(repo_dir: &Path, token: Option<&str>) -> Result<(), GitError> {
    let origin = run(repo_dir, &["remote", "get-url", "origin"])
        .ok()
        .filter(|out| out.status == 0)
        .map(|out| out.stdout_trimmed())
        .unwrap_or_default();
    let remote_http = origin.starts_with("http://") || origin.starts_with("https://");
    let secret = match (remote_http, token.map(str::trim)) {
        (true, Some(secret)) if !secret.is_empty() => Some(secret),
        _ => None,
    };
    let (mut command, mut display) = authed_command(Some(repo_dir), secret);
    let args = ["fetch", "--no-tags", "--prune", "origin"];
    command.args(args);
    display.extend(args.iter().map(|s| s.to_string()));
    let out = run_authed(command, &display).map_err(|err| GitError {
        args: sanitize(&err.args),
        status: err.status,
        stderr: sanitize(&err.stderr),
    })?;
    if out.status != 0 {
        return Err(GitError {
            args: sanitize(&display.join(" ")),
            status: Some(out.status),
            stderr: sanitize(&out.stderr),
        });
    }
    Ok(())
}

/// Split `https://user:token@host/…` into a clean URL plus the bearer token.
/// Local paths and credential-free URLs pass through with no token.
fn split_credentials(url: &str) -> (String, Option<String>) {
    let Some(scheme_end) = url.find("://") else {
        return (url.to_string(), None);
    };
    let after_scheme = &url[scheme_end + 3..];
    let Some(at) = after_scheme.find('@') else {
        return (url.to_string(), None);
    };
    let credentials = &after_scheme[..at];
    let host_and_path = &after_scheme[at + 1..];
    let token = credentials
        .find(':')
        .map(|idx| &credentials[idx + 1..])
        .unwrap_or(credentials);
    if token.is_empty() {
        return (url.to_string(), None);
    }
    (
        format!("{}://{host_and_path}", &url[..scheme_end]),
        Some(token.to_string()),
    )
}

/// GitHub's git endpoints accept `Authorization: Basic base64("x-access-token:<token>")`.
/// `Bearer` is API-only and is rejected by git-http (401), after which git tries to prompt.
fn basic_auth_header(token: &str) -> String {
    format!(
        "Authorization: Basic {}",
        base64_encode(format!("x-access-token:{token}").as_bytes())
    )
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let triple = ((chunk[0] as u32) << 16)
            | ((*chunk.get(1).unwrap_or(&0) as u32) << 8)
            | (*chunk.get(2).unwrap_or(&0) as u32);
        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// `git` with the token carried in `-c http.extraHeader` (never in an argv URL).
/// Returns the command plus the redacted argv for error reports (the token
/// itself is never part of `display`).
fn authed_command(repo_dir: Option<&Path>, token: Option<&str>) -> (Command, Vec<String>) {
    let mut command = base_command();
    let mut display: Vec<String> = Vec::new();
    if let Some(dir) = repo_dir {
        command.arg("-C").arg(dir);
    }
    if let Some(secret) = token {
        command
            .arg("-c")
            .arg(format!("http.extraHeader={}", basic_auth_header(secret)));
        display.push("-c".to_string());
        display.push("http.extraHeader=Authorization: Basic ***".to_string());
    }
    (command, display)
}

fn run_authed(mut command: Command, display: &[String]) -> Result<GitOutput, GitError> {
    let output = command.output().map_err(|err| GitError {
        args: display.join(" "),
        status: None,
        stderr: err.to_string(),
    })?;
    Ok(GitOutput {
        status: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Redacts `https://user:token@host` and `Authorization: Bearer <token>`
/// before text can reach logs, blockers, or the UI.
pub fn sanitize(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(at) = out.find('@') {
        if let Some(scheme) = out[..at].find("://") {
            let creds_start = scheme + 3;
            let creds = &out[creds_start..at];
            if let Some(colon) = creds.find(':') {
                let user = &creds[..colon];
                out = format!("{}://{}:***{}", &out[..scheme], user, &out[at..]);
            }
        }
    }
    // Backstop for `Authorization: Bearer <token>` and `Authorization: Basic <base64>`
    // argv fragments: mask the secret, never the marker.
    for marker in ["Bearer ", "Basic "] {
        redact_after_marker(&mut out, marker);
    }
    out
}

fn redact_after_marker(out: &mut String, marker: &str) {
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find(marker) {
        let secret_start = search_from + rel + marker.len();
        let rest = &out[secret_start..];
        let secret_len = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        if secret_len == 0 || &out[secret_start..secret_start + secret_len] == "***" {
            search_from = secret_start + secret_len;
            continue;
        }
        out.replace_range(secret_start..secret_start + secret_len, "***");
        search_from = secret_start + "***".len();
    }
}

pub fn log_records(repo_dir: &Path, rev: &str, limit: usize) -> Vec<LogRecord> {
    const REC: char = '\u{1e}';
    const FIELD: char = '\u{1f}';
    let fmt = format!("--format=%H{FIELD}%ct{FIELD}%cI{FIELD}%an{FIELD}%s{FIELD}%B{REC}");
    let args = vec!["log", "--reverse", "--topo-order", &fmt, rev];
    let _ = limit;
    let out = match run(repo_dir, &args) {
        Ok(out) if out.status == 0 => out,
        _ => return Vec::new(),
    };
    let text = out.stdout_str();
    let mut records = Vec::new();
    for chunk in text.split(REC) {
        let chunk = chunk.trim_start_matches('\n');
        if chunk.trim().is_empty() {
            continue;
        }
        let mut parts = chunk.split(FIELD);
        let sha = parts.next().unwrap_or("").to_string();
        let committed_at = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
        let committed_iso = parts.next().unwrap_or("").to_string();
        let author = parts.next().unwrap_or("").to_string();
        let subject = parts.next().unwrap_or("").to_string();
        let message = parts.next().unwrap_or("").to_string();
        records.push(LogRecord {
            sha,
            committed_at,
            committed_iso,
            author,
            subject,
            message,
        });
    }
    records
}

#[derive(Debug, Clone)]
pub struct LogRecord {
    pub sha: String,
    pub committed_at: i64,
    pub committed_iso: String,
    pub author: String,
    pub subject: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_redacts_tokens() {
        let url = "https://x-access-token:ghs_secret@github.com/ORG/repo.git";
        let clean = sanitize(url);
        assert!(!clean.contains("ghs_secret"), "{clean}");
        assert!(clean.contains("x-access-token:***@"), "{clean}");
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    /// GitHub's git endpoints reject `Bearer`; the header must be Basic.
    #[test]
    fn basic_auth_header_is_basic_and_hides_the_token() {
        let header = basic_auth_header("gho_secret");
        assert!(header.starts_with("Authorization: Basic "), "{header}");
        assert!(!header.contains("gho_secret"), "raw token leaked: {header}");
    }

    #[test]
    fn sanitize_redacts_basic_auth() {
        let text = "git -c http.extraHeader=Authorization: Basic eC1hY2Nlc3MtdG9rZW46Z2hz_abc";
        let clean = sanitize(text);
        assert!(clean.contains("Basic ***"), "{clean}");
        assert!(!clean.contains("eC1hY2Nlc3MtdG9rZW46Z2hz_abc"), "{clean}");
    }

    #[test]
    fn git_allows_windows_long_paths() {
        let command = base_command();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-c" && pair[1] == "core.longpaths=true"),
            "every git command must allow paths past MAX_PATH: {args:?}"
        );
    }

    #[test]
    fn git_commands_never_open_a_credential_prompt() {
        let command = base_command();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();
        assert!(
            args.iter().any(|arg| arg == "credential.helper="),
            "expected a credential.helper reset, got {args:?}"
        );
        let env_keys: Vec<String> = command
            .get_envs()
            .map(|(key, _)| key.to_string_lossy().to_string())
            .collect();
        assert!(env_keys.iter().any(|key| key == "GIT_TERMINAL_PROMPT"));
        assert!(env_keys.iter().any(|key| key == "GCM_INTERACTIVE"));
    }

    #[test]
    fn sanitize_redacts_bearer_tokens() {
        let text = "git -c http.extraHeader=\"Authorization: Bearer ghs_live_secret\" fetch failed";
        let clean = sanitize(text);
        assert!(!clean.contains("ghs_live_secret"), "{clean}");
        assert!(clean.contains("Bearer ***"), "{clean}");
    }

    #[test]
    fn split_credentials_keeps_local_paths_token_free() {
        let (clean, token) = split_credentials("C:/repos/source");
        assert_eq!(clean, "C:/repos/source");
        assert_eq!(token, None);
        let (clean, token) = split_credentials("https://github.com/ORG/repo.git");
        assert_eq!(clean, "https://github.com/ORG/repo.git");
        assert_eq!(token, None);
        let (clean, token) =
            split_credentials("https://x-access-token:ghs_secret@github.com/ORG/repo.git");
        assert_eq!(clean, "https://github.com/ORG/repo.git");
        assert_eq!(token.as_deref(), Some("ghs_secret"));
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
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

    /// A fork clone keeps the real branches on a non-`origin` remote, so the
    /// existence test must find `refs/remotes/<remote>/<branch>`, not only
    /// `refs/heads/` and `refs/remotes/origin/`.
    #[test]
    fn branch_exists_finds_a_branch_on_a_non_origin_remote() {
        let root = std::env::temp_dir().join(format!(
            "vos-branch-exists-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let upstream = root.join("upstream");
        let clone = root.join("clone");
        std::fs::create_dir_all(&upstream).expect("mkdir upstream");
        run_git(&upstream, &["init", "-q", "-b", "master"]);
        run_git(
            &upstream,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "base",
            ],
        );
        run_git(&upstream, &["branch", "UAT"]);

        // A fork: `origin` carries only `master`; the second remote carries
        // `UAT`, which lands under `refs/remotes/upstream/`.
        run_git(
            &root,
            &[
                "clone",
                "-q",
                "--single-branch",
                "--branch",
                "master",
                upstream.to_str().expect("utf8"),
                clone.to_str().expect("utf8"),
            ],
        );
        run_git(
            &clone,
            &[
                "remote",
                "add",
                "upstream",
                upstream.to_str().expect("utf8"),
            ],
        );
        run_git(&clone, &["fetch", "-q", "upstream"]);

        assert!(
            rev_parse(&clone, "refs/remotes/upstream/UAT").is_some(),
            "precondition: UAT exists only as a remote-tracking ref on upstream"
        );
        assert!(
            rev_parse(&clone, "refs/remotes/origin/UAT").is_none(),
            "precondition: origin does not carry UAT"
        );
        assert!(
            branch_exists(&clone, "UAT"),
            "a branch on a non-origin remote must count as existing"
        );
        assert!(branch_exists(&clone, "master"), "local branch still exists");
        assert!(
            !branch_exists(&clone, "does-not-exist"),
            "an absent branch must not be reported as existing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn fetch_failure_never_surfaces_the_token() {
        let dir = std::env::temp_dir().join(format!(
            "vos-fetch-redact-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        // Unroutable port: fails fast without touching the network.
        let url = "https://x-access-token:ghs_fetch_secret_123@127.0.0.1:1/ORG/repo.git";
        let err = fetch_source_branch(&dir, url, "master", "redact-probe")
            .expect_err("fetch to a dead port must fail");
        let text = err.to_string();
        assert!(!text.contains("ghs_fetch_secret_123"), "{text}");
        assert!(!text.contains("x-access-token"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

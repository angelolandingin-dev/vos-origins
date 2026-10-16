//! Git plumbing: explicit argv (never shell strings), raw bytes for blobs (spec §6).

use std::path::Path;
use std::process::Command;

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

/// `Err` = spawn failure; non-zero exit is `Ok` with `status != 0`.
pub fn run(repo_dir: &Path, args: &[&str]) -> Result<GitOutput, GitError> {
    let mut command = Command::new("git");
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

pub fn run_global(args: &[&str]) -> Result<GitOutput, GitError> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|err| GitError {
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
    run_ok(
        repo_dir,
        &[
            "worktree",
            "add",
            "-f",
            "-b",
            branch,
            &worktree,
            start_point,
        ],
    )?;
    Ok(())
}

pub fn worktree_remove(repo_dir: &Path, worktree_dir: &Path) {
    let worktree = worktree_dir.to_string_lossy().to_string();
    let _ = run(repo_dir, &["worktree", "remove", "--force", &worktree]);
}

pub fn clone_no_tags(url: &str, dest: &Path) -> Result<(), GitError> {
    let dest = dest.to_string_lossy().to_string();
    let (clean_url, token) = split_credentials(url);
    let (mut command, mut display) = authed_command(None, token.as_deref());
    let args = ["clone", "--no-tags", &clean_url, &dest];
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

/// `git` with the bearer token carried in `-c http.extraHeader` (never in an
/// argv URL). Returns the command plus the redacted argv for error reports
/// (the token itself is never part of `display`).
fn authed_command(repo_dir: Option<&Path>, token: Option<&str>) -> (Command, Vec<String>) {
    let mut command = Command::new("git");
    let mut display: Vec<String> = Vec::new();
    if let Some(dir) = repo_dir {
        command.arg("-C").arg(dir);
    }
    if let Some(secret) = token {
        command
            .arg("-c")
            .arg(format!("http.extraHeader=Authorization: Bearer {secret}"));
        display.push("-c".to_string());
        display.push("http.extraHeader=Authorization: Bearer ***".to_string());
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
    // Backstop for `git -c http.extraHeader="Authorization: Bearer <token>"`
    // argv fragments: mask the secret, never the marker.
    let mut search_from = 0;
    while let Some(rel) = out[search_from..].find("Bearer ") {
        let secret_start = search_from + rel + "Bearer ".len();
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
    out
}

pub fn log_records(repo_dir: &Path, rev: &str, limit: usize) -> Vec<LogRecord> {
    const REC: char = '\u{1e}';
    const FIELD: char = '\u{1f}';
    let fmt = format!("--format=%H{FIELD}%ct{FIELD}%cI{FIELD}%s{FIELD}%B{REC}");
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
        let subject = parts.next().unwrap_or("").to_string();
        let message = parts.next().unwrap_or("").to_string();
        records.push(LogRecord {
            sha,
            committed_at,
            committed_iso,
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

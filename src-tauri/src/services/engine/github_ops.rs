//! GitHub access behind [`GitHubOps`]: stub for tests, real `gh` CLI live.
//! Explicit argv only; tokens never land in args, URLs are redacted for logs.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use super::git;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubMode {
    Real,
    Stub,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PullRequestInfo {
    pub number: Option<u64>,
    pub url: Option<String>,
    pub state: String,
    pub base: String,
    pub head: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
    pub stubbed: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub merged_at: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub sha: String,
    pub protected: bool,
}

#[derive(Debug, Clone)]
pub struct CreatePrRequest {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

#[derive(Debug, Clone)]
pub struct PushOutcome {
    pub pushed: bool,
    pub remote: Option<String>,
    pub source: String,
}

pub trait GitHubOps: Send + Sync {
    fn mode(&self) -> GitHubMode;
    fn push_target(&self) -> Option<String>;
    fn push_branch(
        &self,
        repo_dir: &Path,
        branch: &str,
        remote_url: Option<&str>,
    ) -> Result<PushOutcome, String>;
    fn list_open_pull_requests(
        &self,
        repo_slug: &str,
        base: &str,
    ) -> Result<Vec<PullRequestInfo>, String>;
    fn create_pull_request(
        &self,
        repo_slug: &str,
        request: &CreatePrRequest,
    ) -> Result<PullRequestInfo, String>;
    fn resolve_target_tip(&self, repo_slug: &str, branch: &str) -> Result<Option<String>, String>;
    fn list_branches(&self, repo_slug: &str) -> Result<Vec<BranchInfo>, String>;
    fn repo_default_branch(&self, repo_slug: &str) -> Result<Option<String>, String>;
}

/// Open-PR list path: `base`-only filter so an empty `head` ref can never
/// 422 or silently match nothing. `base` is percent-encoded (`release/1.0`
/// must not split the query).
fn open_pulls_path(repo_slug: &str, base: &str, page: u32) -> String {
    format!(
        "/repos/{repo_slug}/pulls?state=open&base={}&per_page=100&page={page}",
        url_encode(base)
    )
}

fn url_encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Never log the result; pass through [`git::sanitize`] first.
pub fn authenticated_remote_url(repo_slug: &str, token: &str) -> String {
    format!("https://x-access-token:{token}@github.com/{repo_slug}.git")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubCall {
    pub name: String,
    pub payload: String,
}

#[derive(Default)]
pub struct StubGitHubOps {
    push_target: Option<String>,
    open_prs: Vec<PullRequestInfo>,
    branches: Vec<BranchInfo>,
    default_branch: Option<String>,
    counter: AtomicU64,
    calls: Mutex<Vec<StubCall>>,
}

impl StubGitHubOps {
    pub fn new() -> Self {
        Self {
            push_target: None,
            open_prs: Vec::new(),
            branches: Vec::new(),
            default_branch: None,
            counter: AtomicU64::new(100),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn with_push_target(mut self, target: impl Into<String>) -> Self {
        self.push_target = Some(target.into());
        self
    }

    pub fn with_open_prs(mut self, prs: Vec<PullRequestInfo>) -> Self {
        self.open_prs = prs;
        self
    }

    pub fn with_branches(mut self, branches: Vec<BranchInfo>) -> Self {
        self.branches = branches;
        self
    }

    pub fn with_default_branch(mut self, branch: impl Into<String>) -> Self {
        self.default_branch = Some(branch.into());
        self
    }

    pub fn calls(&self) -> Vec<StubCall> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }

    pub fn calls_named(&self, name: &str) -> Vec<StubCall> {
        self.calls()
            .into_iter()
            .filter(|c| c.name == name)
            .collect()
    }

    pub fn create_pr_calls(&self) -> Vec<StubCall> {
        self.calls_named("createPullRequest")
    }

    fn record(&self, name: &str, payload: impl Into<String>) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(StubCall {
                name: name.to_string(),
                payload: payload.into(),
            });
        }
    }
}

impl GitHubOps for StubGitHubOps {
    fn mode(&self) -> GitHubMode {
        GitHubMode::Stub
    }

    fn push_target(&self) -> Option<String> {
        self.push_target.clone()
    }

    fn push_branch(
        &self,
        repo_dir: &Path,
        branch: &str,
        remote_url: Option<&str>,
    ) -> Result<PushOutcome, String> {
        let url = remote_url
            .map(str::to_string)
            .or_else(|| self.push_target.clone());
        let Some(url) = url else {
            self.record(
                "pushBranch",
                format!("{branch} (no push target configured)"),
            );
            return Ok(PushOutcome {
                pushed: false,
                remote: None,
                source: "no-push-target".to_string(),
            });
        };
        git::push(repo_dir, &url, branch).map_err(|err| err.to_string())?;
        let source = match (&remote_url, &self.push_target) {
            (Some(url), Some(target)) if url == target => "push-target",
            (Some(_), _) => "explicit",
            (None, _) => "push-target",
        };
        self.record("pushBranch", format!("{branch} -> {}", git::sanitize(&url)));
        Ok(PushOutcome {
            pushed: true,
            remote: Some(git::sanitize(&url)),
            source: source.to_string(),
        })
    }

    fn list_open_pull_requests(
        &self,
        _repo_slug: &str,
        base: &str,
    ) -> Result<Vec<PullRequestInfo>, String> {
        self.record("listOpenPullRequests", base);
        Ok(self
            .open_prs
            .iter()
            .filter(|pr| pr.base == base)
            .cloned()
            .collect())
    }

    fn create_pull_request(
        &self,
        repo_slug: &str,
        request: &CreatePrRequest,
    ) -> Result<PullRequestInfo, String> {
        let number = self.counter.fetch_add(1, Ordering::Relaxed);
        let url = format!("https://github.com/{repo_slug}/pull/{number}");
        self.record(
            "createPullRequest",
            format!("{} -> {} ({})", request.head, request.base, url),
        );
        Ok(PullRequestInfo {
            number: Some(number),
            url: Some(url),
            state: "open".to_string(),
            base: request.base.clone(),
            head: request.head.clone(),
            title: request.title.clone(),
            body: request.body.clone(),
            draft: request.draft,
            stubbed: true,
            created_at: None,
            updated_at: None,
            merged_at: None,
            author: None,
        })
    }

    fn resolve_target_tip(&self, _repo_slug: &str, branch: &str) -> Result<Option<String>, String> {
        self.record("resolveTargetTip", branch);
        Ok(None)
    }

    fn list_branches(&self, _repo_slug: &str) -> Result<Vec<BranchInfo>, String> {
        self.record("listBranches", "all");
        Ok(self.branches.clone())
    }

    fn repo_default_branch(&self, _repo_slug: &str) -> Result<Option<String>, String> {
        self.record("repoDefaultBranch", "default");
        Ok(self.default_branch.clone())
    }
}

pub struct RealGitHubOps {
    token: String,
}

// Branch-protection assumption (documented, not queried): this engine never
// pushes to a protected branch — it only pushes `integration/*` branches and
// opens pull requests whose base equals the requested target branch. Whether
// the target branch is actually protected, and which checks gate merging, is
// therefore enforced by GitHub at PR time, not by this tool. The engine
// deliberately performs no `GET /repos/{slug}/branches/{branch}/protection`
// lookup: a cached or failed lookup could wrongly green-light a direct push,
// while the current design has no direct-push path at all.

impl RealGitHubOps {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
        }
    }

    fn run_gh(&self, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        let mut command = crate::platform::hidden("gh");
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.env("GH_TOKEN", &self.token);
        let mut child = command
            .spawn()
            .map_err(|err| format!("gh could not be started: {err}"))?;
        if let Some(text) = stdin {
            use std::io::Write;
            if let Some(mut handle) = child.stdin.take() {
                handle
                    .write_all(text.as_bytes())
                    .map_err(|err| format!("gh stdin write failed: {err}"))?;
            }
        } else {
            drop(child.stdin.take());
        }
        let output = child
            .wait_with_output()
            .map_err(|err| format!("gh did not complete: {err}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "gh {} failed ({}): {}",
                args.join(" "),
                output.status.code().unwrap_or(-1),
                stderr.trim()
            ));
        }
        Ok(stdout)
    }

    fn parse_pr(value: &serde_json::Value) -> PullRequestInfo {
        let get = |path: &[&str]| -> Option<String> {
            let mut node = value;
            for key in path {
                node = node.get(*key)?;
            }
            node.as_str().map(|s| s.to_string())
        };
        PullRequestInfo {
            number: value.get("number").and_then(serde_json::Value::as_u64),
            url: get(&["html_url"]),
            state: get(&["state"]).unwrap_or_default(),
            base: get(&["base", "ref"]).unwrap_or_default(),
            head: get(&["head", "ref"]).unwrap_or_default(),
            title: get(&["title"]).unwrap_or_default(),
            body: get(&["body"]).unwrap_or_default(),
            draft: value
                .get("draft")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            stubbed: false,
            created_at: get(&["created_at"]),
            updated_at: get(&["updated_at"]),
            merged_at: get(&["merged_at"]),
            author: get(&["user", "login"]),
        }
    }

    fn parse_branch(value: &serde_json::Value) -> Option<BranchInfo> {
        let name = value.get("name")?.as_str()?.to_string();
        let sha = value
            .get("commit")
            .and_then(|c| c.get("sha"))
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        let protected = value
            .get("protected")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        Some(BranchInfo {
            name,
            sha,
            protected,
        })
    }
}

impl GitHubOps for RealGitHubOps {
    fn mode(&self) -> GitHubMode {
        GitHubMode::Real
    }

    fn push_target(&self) -> Option<String> {
        None
    }

    fn push_branch(
        &self,
        repo_dir: &Path,
        branch: &str,
        remote_url: Option<&str>,
    ) -> Result<PushOutcome, String> {
        let Some(url) = remote_url else {
            return Ok(PushOutcome {
                pushed: false,
                remote: None,
                source: "missing-token".to_string(),
            });
        };
        git::push(repo_dir, url, branch).map_err(|err| err.to_string())?;
        Ok(PushOutcome {
            pushed: true,
            remote: Some(git::sanitize(url)),
            source: "authenticated-token".to_string(),
        })
    }

    fn list_open_pull_requests(
        &self,
        repo_slug: &str,
        base: &str,
    ) -> Result<Vec<PullRequestInfo>, String> {
        let mut all = Vec::new();
        // Walk every page so a stale first page can never hide a matching open PR.
        // Filter by `base` only: a `head` filter with an empty ref 422s (or
        // matches nothing), which let duplicate PRs slip through the §8.3 guard.
        for page in 1..=100 {
            let path = open_pulls_path(repo_slug, base, page);
            let raw = self.run_gh(&["api", "--method", "GET", &path], None)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).map_err(|err| format!("gh api pulls: {err}"))?;
            let items = parsed.as_array().cloned().unwrap_or_default();
            let count = items.len();
            all.extend(items.iter().map(Self::parse_pr));
            if count < 100 {
                break;
            }
        }
        Ok(all)
    }

    fn create_pull_request(
        &self,
        repo_slug: &str,
        request: &CreatePrRequest,
    ) -> Result<PullRequestInfo, String> {
        let body = serde_json::json!({
            "title": request.title,
            "head": request.head,
            "base": request.base,
            "body": request.body,
            "draft": request.draft,
        })
        .to_string();
        let path = format!("/repos/{repo_slug}/pulls");
        let raw = self.run_gh(
            &["api", "--method", "POST", &path, "--input", "-"],
            Some(&body),
        )?;
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).map_err(|err| format!("gh api create pull: {err}"))?;
        Ok(Self::parse_pr(&parsed))
    }

    fn resolve_target_tip(&self, repo_slug: &str, branch: &str) -> Result<Option<String>, String> {
        let path = format!("/repos/{repo_slug}/branches/{branch}");
        match self.run_gh(&["api", "--method", "GET", &path], None) {
            Ok(raw) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&raw).map_err(|err| format!("gh api branch: {err}"))?;
                Ok(parsed
                    .get("commit")
                    .and_then(|c| c.get("sha"))
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string()))
            }
            Err(err) if err.contains("404") => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn list_branches(&self, repo_slug: &str) -> Result<Vec<BranchInfo>, String> {
        let mut all = Vec::new();
        for page in 1..=100 {
            let path = format!("/repos/{repo_slug}/branches?per_page=100&page={page}");
            let raw = self.run_gh(&["api", "--method", "GET", &path], None)?;
            let parsed: serde_json::Value =
                serde_json::from_str(&raw).map_err(|err| format!("gh api branches: {err}"))?;
            let items = parsed.as_array().cloned().unwrap_or_default();
            let count = items.len();
            for item in &items {
                if let Some(info) = Self::parse_branch(item) {
                    all.push(info);
                }
            }
            if count < 100 {
                break;
            }
        }
        Ok(all)
    }

    fn repo_default_branch(&self, repo_slug: &str) -> Result<Option<String>, String> {
        let path = format!("/repos/{repo_slug}");
        match self.run_gh(&["api", "--method", "GET", &path], None) {
            Ok(raw) => {
                let parsed: serde_json::Value =
                    serde_json::from_str(&raw).map_err(|err| format!("gh api repo: {err}"))?;
                Ok(parsed
                    .get("default_branch")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()))
            }
            Err(err) if err.contains("404") => Ok(None),
            Err(err) => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_records_calls_and_builds_prs() {
        let stub = StubGitHubOps::new().with_open_prs(vec![PullRequestInfo {
            number: Some(1),
            base: "staging".to_string(),
            head: "integration/hr-x".to_string(),
            ..Default::default()
        }]);
        let prs = stub.list_open_pull_requests("ORG/main", "staging").unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(
            stub.list_open_pull_requests("ORG/main", "master")
                .unwrap()
                .len(),
            0
        );

        let pr = stub
            .create_pull_request(
                "ORG/main",
                &CreatePrRequest {
                    head: "h".into(),
                    base: "staging".into(),
                    title: "t".into(),
                    body: "b".into(),
                    draft: false,
                },
            )
            .unwrap();
        assert_eq!(pr.base, "staging");
        assert!(pr.stubbed);
        assert_eq!(stub.create_pr_calls().len(), 1);
    }

    #[test]
    fn authenticated_url_shape() {
        let url = authenticated_remote_url("ORG/main", "SECRETVALUE");
        assert_eq!(
            url,
            "https://x-access-token:SECRETVALUE@github.com/ORG/main.git"
        );
        assert!(!git::sanitize(&url).contains("SECRETVALUE"));
    }

    #[test]
    fn open_prs_path_filters_by_base_only_with_encoded_base() {
        let path = open_pulls_path("ORG/main", "staging", 1);
        assert_eq!(
            path,
            "/repos/ORG/main/pulls?state=open&base=staging&per_page=100&page=1"
        );
        assert!(!path.contains("head="), "{path}");
        let slashed = open_pulls_path("ORG/main", "release/1.0", 2);
        assert!(
            slashed.contains("base=release%2F1.0"),
            "branch slashes must be encoded: {slashed}"
        );
        assert!(!slashed.contains("head="), "{slashed}");
    }
}

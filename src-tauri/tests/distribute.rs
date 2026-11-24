//! End-to-end distribution: single trunk commit -> single subsystem branch -> pushed PR,
//! against disposable local git repositories so the suite stays offline.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use vos_origins_lib::services::engine::distribute::{self, CommitSnapshot, DistributionPathAction};
use vos_origins_lib::services::engine::github_ops::{
    BranchInfo, CreatePrRequest, GitHubMode, GitHubOps, PullRequestInfo, PushOutcome,
};
use vos_origins_lib::services::registry::SubsystemEntry;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("git {args:?} could not spawn: {err}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).expect("create repo dir");
    git(dir, &["init", "-q", "--initial-branch", branch]);
    git(dir, &["config", "user.name", "Test"]);
    git(dir, &["config", "user.email", "test@example.com"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn write(dir: &Path, path: &str, body: &str) {
    let target = dir.join(path);
    std::fs::create_dir_all(target.parent().expect("path has a parent")).expect("create parent");
    std::fs::write(target, body).expect("write file");
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

fn sandbox(tag: &str) -> PathBuf {
    let base = std::env::var("VOS_ENGINE_SANDBOX")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("vos-rust-engine-sandbox"));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = base.join(format!("{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create sandbox");
    dir
}

#[derive(Default)]
struct PrRecorder {
    requests: Mutex<Vec<CreatePrRequest>>,
}

impl GitHubOps for PrRecorder {
    fn mode(&self) -> GitHubMode {
        GitHubMode::Stub
    }
    fn push_target(&self) -> Option<String> {
        None
    }
    fn push_branch(
        &self,
        _repo_dir: &Path,
        _branch: &str,
        _remote_url: Option<&str>,
    ) -> Result<PushOutcome, String> {
        Ok(PushOutcome {
            pushed: false,
            remote: None,
            source: "test".to_string(),
        })
    }
    fn list_open_pull_requests(
        &self,
        _repo_slug: &str,
        _base: &str,
    ) -> Result<Vec<PullRequestInfo>, String> {
        Ok(Vec::new())
    }
    fn create_pull_request(
        &self,
        _repo_slug: &str,
        request: &CreatePrRequest,
    ) -> Result<PullRequestInfo, String> {
        self.requests.lock().expect("pr lock").push(request.clone());
        Ok(PullRequestInfo {
            number: Some(7),
            url: Some("https://example.test/pull/7".to_string()),
            state: "open".to_string(),
            base: request.base.clone(),
            head: request.head.clone(),
            title: request.title.clone(),
            body: request.body.clone(),
            ..PullRequestInfo::default()
        })
    }
    fn resolve_target_tip(
        &self,
        _repo_slug: &str,
        _branch: &str,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn list_branches(&self, _repo_slug: &str) -> Result<Vec<BranchInfo>, String> {
        Ok(Vec::new())
    }
    fn repo_default_branch(&self, _repo_slug: &str) -> Result<Option<String>, String> {
        Ok(Some("master".to_string()))
    }
}

fn entry() -> SubsystemEntry {
    serde_json::from_value(serde_json::json!({
        "id": "hr-management",
        "name": "HR Management",
        "repository": "ORG/hr-management",
        "sourceBranch": "master",
        "enabled": true,
        "pathStrategy": "preserve-relative-paths",
        "dependencyStrategy": "reconcile",
        "integrationPolicy": {
            "applicationFiles": "selected-commit-delta",
            "packageJson": "merge-dependency-delta",
            "lockfiles": "regenerate-in-target",
            "sharedConfiguration": "review-explicitly",
            "githubWorkflows": "main-owned",
            "secretFiles": "never-import"
        }
    }))
    .expect("valid subsystem entry")
}

#[test]
fn distributes_trunk_commit_paths_into_a_subsystem_pull_request() {
    let root = sandbox("distribute-e2e");

    let main_dir = root.join("main");
    init(&main_dir, "master");
    write(&main_dir, "src/shared.ts", "export const shared = 1;\n");
    write(
        &main_dir,
        "package.json",
        r#"{"name":"main-system","dependencies":{"react":"^18.2.0","zod":"^3.23.0"}}"#,
    );
    commit(&main_dir, "baseline");

    let bare = root.join("hr.git");
    git(
        &root,
        &[
            "init",
            "--bare",
            "-q",
            "--initial-branch",
            "master",
            "hr.git",
        ],
    );
    let seed = root.join("hr-seed");
    init(&seed, "master");
    write(&seed, "src/shared.ts", "export const shared = 1;\n");
    write(
        &seed,
        "src/own.ts",
        "import { useState } from 'react';\nexport const hook = useState;\n",
    );
    write(
        &seed,
        "package.json",
        r#"{"name":"hr-management","dependencies":{"react":"^19.2.0"}}"#,
    );
    commit(&seed, "subsystem baseline");
    let remote = bare.to_string_lossy().replace('\\', "/");
    git(&seed, &["remote", "add", "origin", &remote]);
    git(&seed, &["push", "-q", "origin", "master"]);

    let child_dir = root.join("hr");
    git(&root, &["clone", "-q", &remote, "hr"]);

    write(
        &main_dir,
        "src/shared.ts",
        "import { z } from 'zod';\nexport const shared = z.string();\n",
    );
    let trunk_sha = commit(&main_dir, "main moves ahead");

    let commits = distribute::trunk_commits(&main_dir, "master").expect("trunk commits");
    let selected = commits
        .iter()
        .find(|commit| commit.sha == trunk_sha)
        .expect("the new trunk commit is listed");
    assert_eq!(
        selected
            .paths
            .iter()
            .map(|entry| (entry.path.as_str(), entry.change.as_str()))
            .collect::<Vec<_>>(),
        vec![("src/shared.ts", "modified")]
    );

    let snapshot = CommitSnapshot::load(&main_dir, "master", &trunk_sha).expect("commit snapshot");
    let ops = PrRecorder::default();
    let subsystem = entry();
    let excluded = HashSet::new();
    let plan = distribute::plan(&snapshot, &child_dir, &subsystem, &ops, &excluded);

    let expected_branch = distribute::distribution_branch("master", &trunk_sha);
    assert!(
        plan.blockers.is_empty(),
        "unexpected blockers: {:?}",
        plan.blockers
    );
    assert_eq!(plan.files.len(), 1, "plan: {plan:?}");
    assert_eq!(plan.files[0].path, "src/shared.ts");
    assert_eq!(plan.files[0].action, DistributionPathAction::Update);
    assert_eq!(plan.branch, expected_branch);
    // Scoped deps: `zod` is imported by `src/shared.ts`, while `react` in `src/own.ts`
    // is unrelated to this commit and must not be touched.
    assert_eq!(plan.dependencies.len(), 1, "deps: {:?}", plan.dependencies);
    assert!(plan
        .dependencies
        .iter()
        .any(|dep| dep.name == "zod" && dep.spec == "^3.23.0"));

    let temp_root = root.join("tmp");
    let outcome = distribute::run(distribute::RunInput {
        commit: &snapshot,
        child_dir: &child_dir,
        entry: &subsystem,
        main_repo: "ORG/main-system",
        plan: &plan,
        excluded_paths: &excluded,
        message: Some("distribute from master"),
        attribute_to_account: false,
        account: None,
        temp_root: &temp_root,
        remote_url: &remote,
        github: &ops,
    });

    assert!(outcome.ok, "distribution blockers: {:?}", outcome.blockers);
    assert_eq!(outcome.applied, vec!["src/shared.ts".to_string()]);
    assert!(outcome.deleted.is_empty());
    let pull_request = outcome.pull_request.expect("pull request opened");
    assert_eq!(pull_request.head, expected_branch);
    assert_eq!(pull_request.base, "master");
    assert_eq!(
        ops.requests.lock().expect("pr lock").len(),
        1,
        "exactly one pull request"
    );

    let branch_tip = git(&bare, &["rev-parse", &expected_branch]);
    assert_eq!(Some(branch_tip), outcome.commit_sha);
    assert_eq!(
        git(
            &bare,
            &["show", &format!("{expected_branch}:src/shared.ts")]
        ),
        "import { z } from 'zod';\nexport const shared = z.string();"
    );
    let manifest: serde_json::Value = serde_json::from_str(&git(
        &bare,
        &["show", &format!("{expected_branch}:package.json")],
    ))
    .expect("manifest is json");
    assert_eq!(manifest["dependencies"]["react"], "^19.2.0");
    assert_eq!(manifest["dependencies"]["zod"], "^3.23.0");
    let message = git(&bare, &["log", "-1", "--format=%B", &expected_branch]);
    assert!(
        message.contains("distribute(hr-management): distribute from master"),
        "message: {message}"
    );
    assert!(
        message.contains(&format!("Source-Commit: {trunk_sha}")),
        "message: {message}"
    );
    assert!(
        message.contains("Promotion-Mode: distribute"),
        "message: {message}"
    );
}

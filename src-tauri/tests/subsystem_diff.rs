//! Subsystem-versus-main diff end to end: a partial promotion leaves files
//! behind, and the diff shows them with per-branch statuses from the existing
//! machinery — then the selection resolves to the unpromoted commits plus the
//! pre-filled exclusions, exactly as a careful manual selection would.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use vos_origins_lib::github::models::{CommitStatusKind, SubsystemDiffBucket};
use vos_origins_lib::services::engine::github_ops::StubGitHubOps;
use vos_origins_lib::services::engine::provenance::{build_provenance_message, ProvenanceInput};
use vos_origins_lib::services::registry::Registry;
use vos_origins_lib::services::subsystem_diff::{resolve_selection, subsystem_diff_live};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp(prefix: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "vos-diff-e2e-{prefix}-{n}-{nanos}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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

fn init(dir: &Path, branch: &str) {
    std::fs::create_dir_all(dir).expect("mkdir repo");
    git(dir, &["init", "-q", "-b", branch]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "T"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn write(dir: &Path, rel: &str, content: &str) {
    let abs = dir.join(rel);
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent).expect("mkdir parent");
    }
    std::fs::write(&abs, content).expect("write");
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

fn test_registry() -> Registry {
    let text = r#"{
  "mainRepository": "ORG/main-system",
  "promotion": {
    "allowedTargetBranches": ["staging"],
    "allowedTargetBranchPatterns": [],
    "requirePullRequest": true
  },
  "subsystems": [{
      "id": "mod",
      "name": "Module",
      "repository": "ORG/mod",
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
    }]
}"#;
    Registry::from_json(text, "test").expect("registry")
}

#[test]
fn partial_promotion_end_to_end_missing_behind_identical_with_statuses() {
    let src = tmp("src");
    init(&src, "master");
    write(&src, "f1.txt", "one\n");
    write(&src, "f2.txt", "two\n");
    write(&src, "f3.txt", "three\n");
    write(&src, "g.txt", "v1\n");
    let a = commit_all(&src, "A adds f1 f2 f3 and g");
    write(&src, "g.txt", "v2\n");
    write(&src, "h.txt", "new\n");
    let b = commit_all(&src, "B edits g and adds h");

    let main = tmp("main");
    init(&main, "master");
    write(&main, "base.txt", "base\n");
    let _ = commit_all(&main, "main base");
    git(&main, &["branch", "staging"]);
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "g.txt", "v1\n");
    write(&main, "h.txt", "new\n");
    let message = build_provenance_message(&ProvenanceInput {
        subsystem_id: "mod",
        source_repo: "ORG/mod",
        source_commits: std::slice::from_ref(&b),
        mode: "individual",
        target: "staging",
        subject: "B edits g and adds h",
    });
    let _ = commit_all(&main, &message);

    let registry = test_registry();
    let github = StubGitHubOps::new();
    let diff =
        subsystem_diff_live(&main, &src, &registry, &github, "mod", "staging").expect("diff");
    assert_eq!(diff.subsystem_id, "mod");
    assert_eq!(diff.target_branch, "staging");
    let by_path: HashMap<&str, _> = diff.entries.iter().map(|e| (e.path.as_str(), e)).collect();

    for missing in ["f1.txt", "f2.txt", "f3.txt"] {
        let entry = by_path[missing];
        assert_eq!(entry.bucket, SubsystemDiffBucket::Missing);
        assert_eq!(entry.commits.len(), 1);
        assert_eq!(entry.commits[0].sha, a);
        assert_eq!(entry.commits[0].status.kind, CommitStatusKind::Eligible);
    }
    let g = by_path["g.txt"];
    assert_eq!(g.bucket, SubsystemDiffBucket::Behind);
    assert_eq!(
        g.commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(),
        vec![b.clone(), a.clone()]
    );
    assert_eq!(g.commits[0].status.kind, CommitStatusKind::Integrated);
    assert_eq!(g.commits[1].status.kind, CommitStatusKind::Eligible);
    assert!(g.reverts_newer.is_none(), "an ancestor version never warns");
    // h matches B's version exactly.
    let h = by_path["h.txt"];
    assert_eq!(h.bucket, SubsystemDiffBucket::Identical);

    let mut touches: HashMap<String, Vec<String>> = HashMap::new();
    let mut main_side: HashSet<String> = HashSet::new();
    let mut commit_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for entry in &diff.entries {
        if entry.bucket == SubsystemDiffBucket::MainSideChanged {
            main_side.insert(entry.path.clone());
        }
        let mut shas: Vec<String> = Vec::new();
        for commit in &entry.commits {
            shas.push(commit.sha.clone());
            commit_paths.insert(commit.sha.clone(), commit.touched_paths.clone());
            if !order.contains(&commit.sha) {
                order.push(commit.sha.clone());
            }
        }
        touches.insert(entry.path.clone(), shas);
    }
    order.reverse();
    let promoted: HashSet<String> = [b.clone()].into_iter().collect();
    let selected: HashSet<String> = ["f1.txt", "f2.txt", "f3.txt"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let resolution = resolve_selection(
        &selected,
        &touches,
        &promoted,
        &commit_paths,
        &order,
        &main_side,
    );
    assert_eq!(resolution.commits, vec![a.clone()]);
    assert_eq!(resolution.excluded_paths, vec!["g.txt".to_string()]);
    assert!(resolution.main_side_selected.is_empty());
}

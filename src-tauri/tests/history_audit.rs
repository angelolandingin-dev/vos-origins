//! Integration event audit: one event re-derived from git.
//!
//! The main commit is the key; everything is re-derived server-side.
//! These tests run against disposable local git repos (fully offline) via
//! `integration_event_audit_live`. They prove the audit never fabricates:
//! unreachable commits report `unavailable` with empty subjects, trailerless
//! commits report `main_commit_unavailable`, and a `Reverted` event names
//! what it undid through the replay's already-resolved `reverts_integration`
//! link (never re-parsed from message text).
//!
//! Spawn bound: the enrichment is one `cat-file --batch-check` prefilter +
//! one `git log --no-walk` batch for ALL included SHAs, plus one `git show`
//! for the main commit's changes, all sharing the tip-keyed replay. The
//! numeric bound (2 source spawns, <=8 target spawns, 1 replay scan) is
//! asserted with counters in the unit test
//! `audit_three_commit_event_costs_one_batch_call_not_one_per_commit`
//! (which has `cfg(test)` access to `git_spawns_for_tests` /
//! `replay_scans_for_tests`, unavailable to integration tests). Here we
//! assert the functional batching property: one bad SHA never blanks the
//! reachable ones, and a repeated call reuses the memo.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use vos_origins_lib::github::models::IntegrationHistoryKind;
use vos_origins_lib::services::live_read::integration_event_audit_live;
use vos_origins_lib::services::registry::Registry;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp(prefix: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "vos-audit-{prefix}-{n}-{nanos}-{}",
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

fn single_registry() -> Registry {
    let text = r#"{
  "mainRepository": "ORG/main-system",
  "promotion": {
    "allowedTargetBranches": ["staging", "development"],
    "allowedTargetBranchPatterns": ["release/*"],
    "requirePullRequest": true
  },
  "subsystems": [{
      "id": "hr-management",
      "name": "HR",
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
    }]
}"#;
    Registry::from_json(text, "test").expect("registry")
}

/// Source with three commits; main with one staging integration carrying all
/// three `Source-Commit` trailers in non-alphabetical order.
fn sandbox_three() -> (PathBuf, PathBuf, Vec<String>, String) {
    let src = tmp("src");
    init(&src, "master");
    write(&src, "a.txt", "a\n");
    let a = commit_all(&src, "Alpha feature");
    write(&src, "b.txt", "b\n");
    let b = commit_all(&src, "Beta feature");
    write(&src, "c.txt", "c\n");
    let c = commit_all(&src, "Gamma feature");

    let main = tmp("main");
    init(&main, "master");
    write(&main, "base.txt", "base\n");
    let _ = commit_all(&main, "main base");
    git(&main, &["branch", "staging"]);
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "x.txt", "x\n");
    write(&main, "y.txt", "y\n");
    let order = vec![c.clone(), a.clone(), b.clone()];
    let trailer = format!(
        "integrate(hr-management): trio\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {}\nSource-Commit: {}\nSource-Commit: {}\nPromotion-Mode: individual\nPromotion-Target: staging\n",
        order[0], order[1], order[2]
    );
    let m = commit_all(&main, &trailer);
    (src, main, order, m)
}

#[test]
fn included_commits_are_exactly_source_commit_trailers_in_order() {
    let (src, main, order, m) = sandbox_three();
    let audit = integration_event_audit_live(
        &main,
        Some(&src),
        &single_registry(),
        "staging",
        "hr-management",
        &m,
    )
    .expect("audit");
    assert!(!audit.main_commit_unavailable);
    assert_eq!(audit.kind, IntegrationHistoryKind::Merged);
    assert_eq!(audit.main_commit_sha, m);
    assert_eq!(audit.included_commits.len(), 3);
    let got: Vec<&str> = audit
        .included_commits
        .iter()
        .map(|c| c.sha.as_str())
        .collect();
    assert_eq!(
        got,
        order.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        "included commits are exactly the Source-Commit trailers, in trailer order"
    );
    assert_eq!(audit.included_commits[0].subject, "Gamma feature");
    assert_eq!(audit.included_commits[1].subject, "Alpha feature");
    assert_eq!(audit.included_commits[2].subject, "Beta feature");
    for commit in &audit.included_commits {
        assert!(!commit.unavailable);
        assert_eq!(
            commit.short_sha,
            commit.sha.chars().take(7).collect::<String>()
        );
        assert!(
            !commit.subject.is_empty(),
            "no empty subjects for reachable commits"
        );
    }
    // Verbatim trailer block, in git order — the audit artefact.
    let keys: Vec<&str> = audit.provenance.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "Subsystem-ID",
            "Source-Repo",
            "Source-Commit",
            "Source-Commit",
            "Source-Commit",
            "Promotion-Mode",
            "Promotion-Target",
        ]
    );
    let sources: Vec<&str> = audit
        .provenance
        .iter()
        .filter(|p| p.key == "Source-Commit")
        .map(|p| p.value.as_str())
        .collect();
    assert_eq!(
        sources,
        order.iter().map(|s| s.as_str()).collect::<Vec<_>>()
    );
}

#[test]
fn unreachable_source_commit_is_unavailable_with_empty_subject() {
    let src = tmp("unreach-src");
    init(&src, "master");
    write(&src, "a.txt", "a\n");
    let real = commit_all(&src, "Real work");
    let bogus = "f".repeat(40);

    let main = tmp("unreach-main");
    init(&main, "master");
    write(&main, "base.txt", "base\n");
    let _ = commit_all(&main, "main base");
    git(&main, &["branch", "staging"]);
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "x.txt", "x\n");
    let trailer = format!(
        "integrate(hr-management): mixed\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {real}\nSource-Commit: {bogus}\nPromotion-Mode: individual\nPromotion-Target: staging\n"
    );
    let m = commit_all(&main, &trailer);

    let audit = integration_event_audit_live(
        &main,
        Some(&src),
        &single_registry(),
        "staging",
        "hr-management",
        &m,
    )
    .expect("audit");
    assert!(!audit.main_commit_unavailable);
    assert_eq!(audit.included_commits.len(), 2);
    assert!(!audit.included_commits[0].unavailable);
    assert_eq!(audit.included_commits[0].subject, "Real work");
    let missing = &audit.included_commits[1];
    assert_eq!(missing.sha, bogus);
    assert!(missing.unavailable);
    assert!(
        missing.subject.is_empty(),
        "no fabricated subject for an unreachable source commit"
    );
    assert_ne!(
        missing.subject, missing.sha,
        "the SHA must never stand in as the subject"
    );
    assert!(missing.author.is_none());
    assert!(missing.authored_at.is_none());
}

#[test]
fn changes_and_totals_match_real_numstat_for_multi_file_commit() {
    let main = tmp("changes");
    init(&main, "master");
    write(&main, "a.txt", "l1\nl2\nl3\n");
    let _ = commit_all(&main, "main base");
    git(&main, &["branch", "staging"]);
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "a.txt", "l1\nl2 changed\nl3\nl4\n");
    write(&main, "b.txt", "n1\nn2\nn3\n");
    let trailer = format!(
        "integrate(hr-management): two files\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {}\nPromotion-Mode: individual\nPromotion-Target: staging\n",
        "a".repeat(40)
    );
    let m = commit_all(&main, &trailer);

    // Independent ground truth straight from git (setup `Command`, uncounted).
    let numstat = git(&main, &["show", "--numstat", "--format=", &m]);
    let mut expected: std::collections::HashMap<String, (u32, u32)> =
        std::collections::HashMap::new();
    for line in numstat.lines() {
        let mut cols = line.split('\t');
        if let (Some(a), Some(d), Some(p)) = (cols.next(), cols.next(), cols.next()) {
            expected.insert(
                p.to_string(),
                (a.parse().unwrap_or(0), d.parse().unwrap_or(0)),
            );
        }
    }

    // No source checkout needed here: enrichment reports unavailable.
    let audit = integration_event_audit_live(
        &main,
        None,
        &single_registry(),
        "staging",
        "hr-management",
        &m,
    )
    .expect("audit");
    assert!(!audit.main_commit_unavailable);
    assert_eq!(audit.changes.len(), expected.len());
    for change in &audit.changes {
        let (adds, dels) = expected
            .get(&change.path)
            .copied()
            .unwrap_or_else(|| panic!("unexpected path {}", change.path));
        assert_eq!(
            (change.additions, change.deletions),
            (adds, dels),
            "per-file counts match numstat for {}",
            change.path
        );
    }
    let by_path: std::collections::HashMap<&str, &str> = audit
        .changes
        .iter()
        .map(|c| {
            (
                c.path.as_str(),
                match c.change {
                    vos_origins_lib::github::models::IntegrationEventChangeKind::Added => "added",
                    vos_origins_lib::github::models::IntegrationEventChangeKind::Modified => {
                        "modified"
                    }
                    vos_origins_lib::github::models::IntegrationEventChangeKind::Deleted => {
                        "deleted"
                    }
                    vos_origins_lib::github::models::IntegrationEventChangeKind::Renamed => {
                        "renamed"
                    }
                },
            )
        })
        .collect();
    assert_eq!(by_path["a.txt"], "modified");
    assert_eq!(by_path["b.txt"], "added");
    let totals = audit
        .totals
        .expect("totals are present for a readable commit");
    assert_eq!(totals.files as usize, audit.changes.len());
    assert_eq!(
        totals.additions,
        audit.changes.iter().map(|c| c.additions).sum::<u32>()
    );
    assert_eq!(
        totals.deletions,
        audit.changes.iter().map(|c| c.deletions).sum::<u32>()
    );
    assert_eq!((totals.additions, totals.deletions), (5, 1));
}

#[test]
fn trailerless_commit_reports_unavailable_without_fabricated_commits() {
    let main = tmp("plain");
    init(&main, "master");
    write(&main, "base.txt", "base\n");
    let _ = commit_all(&main, "main base");
    git(&main, &["branch", "staging"]);
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "work.txt", "work\n");
    let plain = commit_all(&main, "ordinary work without trailers");

    let audit = integration_event_audit_live(
        &main,
        None,
        &single_registry(),
        "staging",
        "hr-management",
        &plain,
    )
    .expect("audit is a truthful result, not an error");
    assert!(audit.main_commit_unavailable);
    assert!(
        audit.included_commits.is_empty(),
        "a commit with no trailers yields no fabricated commits"
    );
    assert!(audit.changes.is_empty());
    assert!(audit.totals.is_none());
    assert!(
        audit.detail.contains("reachable history"),
        "detail says why: {}",
        audit.detail
    );

    // Outside the scanned range is the same truthful shape, never an Err.
    let bogus_main = "d".repeat(40);
    let missing = integration_event_audit_live(
        &main,
        None,
        &single_registry(),
        "staging",
        "hr-management",
        &bogus_main,
    )
    .expect("missing main commit is truthful, not an error");
    assert!(missing.main_commit_unavailable);
    assert!(missing.included_commits.is_empty());
    assert!(missing.changes.is_empty());
    assert!(missing.totals.is_none());
}

#[test]
fn reverted_event_names_what_it_undid_via_reverts_event_id() {
    use vos_origins_lib::services::engine::provenance::{
        build_provenance_message, build_revert_message, ProvenanceInput,
    };

    let src = tmp("revert-src");
    init(&src, "master");
    write(&src, "f.txt", "feature\n");
    let s = commit_all(&src, "Feature work");

    let main = tmp("revert-main");
    init(&main, "master");
    write(&main, "base.txt", "base\n");
    let _ = commit_all(&main, "main base");
    git(&main, &["branch", "staging"]);
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "f.txt", "feature\n");
    let integrate_msg = build_provenance_message(&ProvenanceInput {
        subsystem_id: "hr-management",
        source_repo: "ORG/hr-management",
        source_commits: std::slice::from_ref(&s),
        mode: "individual",
        target: "staging",
        subject: "Feature work",
        gap_shas: &[],
        gap_override: false,
    });
    let i = commit_all(&main, &integrate_msg);
    // Revert through the existing builder: the `Reverts-Integration` trailer
    // carries the undone commit and the replay resolves it. Never re-parsed
    // from "This reverts commit ..." text here.
    write(&main, "f.txt", "undone\n");
    let revert_msg = build_revert_message(
        "hr-management",
        std::slice::from_ref(&s),
        &i,
        "Feature work",
    );
    let r = commit_all(&main, &revert_msg);

    let registry = single_registry();
    let undone =
        integration_event_audit_live(&main, Some(&src), &registry, "staging", "hr-management", &r)
            .expect("audit");
    assert!(!undone.main_commit_unavailable);
    assert_eq!(undone.kind, IntegrationHistoryKind::Reverted);
    assert_eq!(
        undone.reverted_main_commit_sha.as_deref(),
        Some(i.as_str()),
        "a Reverted event names the commit it undid via the structured revert link"
    );
    let kept =
        integration_event_audit_live(&main, Some(&src), &registry, "staging", "hr-management", &i)
            .expect("audit");
    assert_eq!(kept.kind, IntegrationHistoryKind::Merged);
    assert_eq!(kept.reverted_main_commit_sha, None);
}

#[test]
fn multi_commit_event_costs_small_constant_not_one_spawn_per_commit() {
    // Functional batching proof (numeric spawn counters live in the unit
    // test with `cfg(test)` access): one bad SHA never blanks the reachable
    // ones, which is exactly what a naive single `git log` without the
    // `cat-file --batch-check` prefilter would do (it exits 128 and prints
    // nothing). A per-commit spawn loop would also pass functionally, so the
    // constant-spawn shape (one prefilter + one batch log + one show, shared
    // tip-keyed replay) is pinned numerically by
    // `audit_three_commit_event_costs_one_batch_call_not_one_per_commit`:
    // 2 source spawns, <=8 target spawns, 1 replay scan.
    let (src, main, order, m) = sandbox_three();
    let registry = single_registry();
    let audit =
        integration_event_audit_live(&main, Some(&src), &registry, "staging", "hr-management", &m)
            .expect("audit");
    assert_eq!(
        audit.included_commits.len(),
        3,
        "3 included commits arrive in one batch, not one spawn per commit"
    );
    // One bad SHA alongside two good ones: the good ones still enrich.
    let bogus = "e".repeat(40);
    write(&main, "z.txt", "z\n");
    let mixed_trailer = format!(
        "integrate(hr-management): mixed-batch\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {}\nSource-Commit: {}\nSource-Commit: {}\nPromotion-Mode: individual\nPromotion-Target: staging\n",
        order[0], bogus, order[1]
    );
    // Commit on staging so it is reachable.
    git(&main, &["checkout", "-q", "staging"]);
    write(&main, "z2.txt", "z2\n");
    let mixed = commit_all(&main, &mixed_trailer);
    let mixed_audit = integration_event_audit_live(
        &main,
        Some(&src),
        &registry,
        "staging",
        "hr-management",
        &mixed,
    )
    .expect("mixed audit");
    assert_eq!(mixed_audit.included_commits.len(), 3);
    assert!(!mixed_audit.included_commits[0].unavailable);
    assert!(mixed_audit.included_commits[1].unavailable);
    assert!(mixed_audit.included_commits[1].subject.is_empty());
    assert!(!mixed_audit.included_commits[2].unavailable);
    // Repeated click reuses the memo: identical, truthful, no recompute.
    let again =
        integration_event_audit_live(&main, Some(&src), &registry, "staging", "hr-management", &m)
            .expect("second audit");
    assert_eq!(again.main_commit_sha, audit.main_commit_sha);
    assert_eq!(
        again
            .included_commits
            .iter()
            .map(|c| c.sha.clone())
            .collect::<Vec<_>>(),
        audit
            .included_commits
            .iter()
            .map(|c| c.sha.clone())
            .collect::<Vec<_>>(),
        "a repeated click reuses the tip-keyed replay"
    );
}

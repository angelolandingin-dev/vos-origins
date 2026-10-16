//! Dry-run mergeability preflight: per-file `clean` / `merged` / `conflict` /
//! `skipped` computed from the same blobs and the same three-way merge the real
//! apply uses — without touching either repository.
//!
//! Read-only by construction: every git operation here is a read (`rev-parse`,
//! `cat-file`, `ls-tree`, `log`, `show`, `merge-file`). This module never calls
//! `worktree_add`, never creates a branch or a commit, and the only files it
//! writes are `merge-file` scratch files under a temp dir that is always
//! removed before returning.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::apply;
use super::classify::{self, PathStatus};
use super::git;
use crate::github::models::PromotionMode;
use crate::github::models::PromotionOnConflict;
use crate::services::registry::IntegrationPolicy;

/// Per-file mergeability, mirroring the real apply's branches:
/// - `clean`: applies without a merge (new file, target already matches the
///   base, or target already matches the source);
/// - `merged`: both sides moved and the three-way merge succeeded;
/// - `conflict`: the three-way merge conflicts (or the engine would refuse the
///   path outright, e.g. the add-over-existing recency guard);
/// - `skipped`: dependency manifest/lockfile paths, which the engine merges or
///   regenerates rather than writes (plus main-owned/secret paths, which the
///   engine never imports).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightOutcome {
    Clean,
    Merged,
    Conflict,
    Skipped,
}

impl PreflightOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            PreflightOutcome::Clean => "clean",
            PreflightOutcome::Merged => "merged",
            PreflightOutcome::Conflict => "conflict",
            PreflightOutcome::Skipped => "skipped",
        }
    }

    fn severity(self) -> u8 {
        match self {
            PreflightOutcome::Skipped => 0,
            PreflightOutcome::Clean => 1,
            PreflightOutcome::Merged => 2,
            PreflightOutcome::Conflict => 3,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreflightFile {
    pub path: String,
    pub outcome: PreflightOutcome,
}

#[derive(Debug, Clone, Default)]
pub struct PreflightReport {
    pub files: Vec<PreflightFile>,
    pub clean: usize,
    pub merged: usize,
    pub conflict: usize,
    pub skipped: usize,
    pub conflicts: Vec<String>,
}

/// Read-only inputs for [`compute_preflight`] (mirrors `EngineRequest`).
pub struct PreflightRequest<'a> {
    pub policy: &'a IntegrationPolicy,
    pub commits: &'a [String],
    pub mode: PromotionMode,
    pub base_sha: Option<&'a str>,
    pub target_branch: &'a str,
    /// Mirrors the real apply's `on_conflict`: with `take_subsystem`, paths
    /// the policy would resolve report as `merged` instead of `conflict`, so
    /// preflight and the real apply never disagree. Guards the policy never
    /// bypasses (unsafe paths, classification, dependencies) still report
    /// `conflict`/`skipped` exactly like the engine fails.
    pub on_conflict: PromotionOnConflict,
}

#[derive(Debug, Clone)]
pub enum PreflightError {
    UnknownTargetBranch(String),
    UnknownCommit(String),
    InvalidRange(String),
    Scratch(String),
}

impl PreflightError {
    pub fn message(&self) -> String {
        match self {
            PreflightError::UnknownTargetBranch(branch) => {
                format!("Target branch '{branch}' does not exist in the target checkout.")
            }
            PreflightError::UnknownCommit(sha) => {
                format!("Source commit '{sha}' was not found in the source checkout.")
            }
            PreflightError::InvalidRange(message) | PreflightError::Scratch(message) => {
                message.clone()
            }
        }
    }
}

/// Per-file mergeability for the selected deltas against the target branch tip.
/// Reads blobs from `source_dir` (base/theirs) and `target_dir` (ours); writes
/// nothing to either and leaves no branch, commit, or worktree behind.
pub fn compute_preflight(
    source_dir: &Path,
    target_dir: &Path,
    request: &PreflightRequest<'_>,
    scratch_root: &Path,
) -> Result<PreflightReport, PreflightError> {
    let target_tip = resolve_target_tip(target_dir, request.target_branch)
        .ok_or_else(|| PreflightError::UnknownTargetBranch(request.target_branch.to_string()))?;

    let plan = build_delta_plan(source_dir, request)?;

    let scratch = unique_dir(scratch_root, "vos-preflight");
    std::fs::create_dir_all(&scratch).map_err(|err| {
        PreflightError::Scratch(format!("Could not create preflight scratch dir: {err}"))
    })?;
    let report = evaluate_plan(
        source_dir,
        target_dir,
        request.policy,
        &plan,
        &target_tip,
        &scratch,
        request.on_conflict,
    );
    let _ = std::fs::remove_dir_all(&scratch);
    Ok(report)
}

#[derive(Debug, Clone)]
struct PlannedDelta {
    sha: String,
    base_sha: Option<String>,
    entries: Vec<git::DeltaEntry>,
}

/// First-parent deltas for the selected SHAs (or the baseline range in
/// cumulative mode) — the same delta resolution the real apply uses.
fn build_delta_plan(
    source_dir: &Path,
    request: &PreflightRequest<'_>,
) -> Result<Vec<PlannedDelta>, PreflightError> {
    if request.mode == PromotionMode::Cumulative {
        if request.commits.len() != 1 {
            return Err(PreflightError::InvalidRange(
                "Cumulative mode requires exactly one endpoint SHA.".to_string(),
            ));
        }
        let endpoint = request.commits[0].clone();
        let base = request.base_sha.filter(|s| !s.is_empty()).ok_or_else(|| {
            PreflightError::InvalidRange("Cumulative mode requires a baseline SHA.".to_string())
        })?;
        if git::rev_parse(source_dir, &endpoint).is_none() {
            return Err(PreflightError::UnknownCommit(endpoint.clone()));
        }
        if git::rev_parse(source_dir, base).is_none() {
            return Err(PreflightError::UnknownCommit(base.to_string()));
        }
        if !git::is_ancestor(source_dir, base, &endpoint) {
            return Err(PreflightError::InvalidRange(format!(
                "Baseline {base} is not an ancestor of {endpoint}."
            )));
        }
        let entries = git::name_status_range(source_dir, base, &endpoint)
            .map_err(|err| PreflightError::InvalidRange(err.to_string()))?;
        return Ok(vec![PlannedDelta {
            sha: endpoint.clone(),
            base_sha: Some(base.to_string()),
            entries,
        }]);
    }
    let mut plan = Vec::with_capacity(request.commits.len());
    for sha in request.commits {
        if git::rev_parse(source_dir, sha).is_none() {
            return Err(PreflightError::UnknownCommit(sha.clone()));
        }
        let (parent, entries) = git::name_status_delta(source_dir, sha)
            .map_err(|err| PreflightError::InvalidRange(err.to_string()))?;
        plan.push(PlannedDelta {
            sha: sha.clone(),
            base_sha: parent,
            entries,
        });
    }
    Ok(plan)
}

fn evaluate_plan(
    source_dir: &Path,
    target_dir: &Path,
    policy: &IntegrationPolicy,
    plan: &[PlannedDelta],
    target_tip: &str,
    scratch: &Path,
    on_conflict: PromotionOnConflict,
) -> PreflightReport {
    // A path touched by several selected commits reports its worst outcome: a
    // single conflicting delta fails the real promotion, so it must surface.
    let mut by_path: HashMap<String, PreflightOutcome> = HashMap::new();
    for delta in plan {
        for entry in &delta.entries {
            for rel_path in expanded_paths(entry) {
                let outcome = evaluate_path(
                    source_dir,
                    target_dir,
                    policy,
                    delta,
                    entry,
                    &rel_path,
                    target_tip,
                    scratch,
                    on_conflict,
                );
                by_path
                    .entry(rel_path)
                    .and_modify(|kept| {
                        if outcome.severity() > kept.severity() {
                            *kept = outcome;
                        }
                    })
                    .or_insert(outcome);
            }
        }
    }
    let mut files: Vec<PreflightFile> = by_path
        .into_iter()
        .map(|(path, outcome)| PreflightFile { path, outcome })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut report = PreflightReport::default();
    for file in &files {
        match file.outcome {
            PreflightOutcome::Clean => report.clean += 1,
            PreflightOutcome::Merged => report.merged += 1,
            PreflightOutcome::Conflict => {
                report.conflict += 1;
                report.conflicts.push(file.path.clone());
            }
            PreflightOutcome::Skipped => report.skipped += 1,
        }
    }
    report.conflicts.sort();
    report.files = files;
    report
}

/// The worktree paths one delta entry applies to — the same expansion the real
/// apply uses (renames touch both sides, copies only materialize the new path).
fn expanded_paths(entry: &git::DeltaEntry) -> Vec<String> {
    if entry.status == 'D' {
        entry.path.clone().into_iter().collect()
    } else if entry.status == 'R' {
        entry
            .old_path
            .clone()
            .into_iter()
            .chain(entry.new_path.clone())
            .collect()
    } else if entry.status == 'C' {
        entry.new_path.clone().into_iter().collect()
    } else {
        entry.path.clone().into_iter().collect()
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_path(
    source_dir: &Path,
    target_dir: &Path,
    policy: &IntegrationPolicy,
    delta: &PlannedDelta,
    entry: &git::DeltaEntry,
    rel_path: &str,
    target_tip: &str,
    scratch: &Path,
    on_conflict: PromotionOnConflict,
) -> PreflightOutcome {
    if rel_path.is_empty() {
        return PreflightOutcome::Conflict;
    }
    // Same mode lookup as the engine's classifier: deletes and rename sources
    // read the mode from the baseline, everything else from the source commit.
    let mode_opt = if entry.status == 'D'
        || (entry.is_rename_or_copy() && Some(rel_path) == entry.old_path.as_deref())
    {
        delta
            .base_sha
            .as_deref()
            .and_then(|base| git::path_mode(source_dir, base, rel_path))
    } else {
        git::path_mode(source_dir, &delta.sha, rel_path)
    };
    let classification = classify::classify_path(rel_path, policy, mode_opt.as_deref());
    match classification.status {
        // Deleting the root manifest is never automatic (same rule as the engine).
        PathStatus::DependencyMerge if entry.status == 'D' => {
            return PreflightOutcome::Conflict;
        }
        PathStatus::DependencyMerge
        | PathStatus::Regenerate
        | PathStatus::MainOwned
        | PathStatus::SecretExcluded => return PreflightOutcome::Skipped,
        PathStatus::ExplicitReview | PathStatus::Rejected => {
            return PreflightOutcome::Conflict;
        }
        PathStatus::Apply => {}
    }

    // Same blob inputs as the real apply: base/theirs from the source checkout,
    // ours from the target branch tip. Deletes and rename sources have no
    // `theirs` blob, exactly like `apply_one` sees them.
    let theirs_opt = if entry.status == 'D'
        || (entry.is_rename_or_copy() && Some(rel_path) == entry.old_path.as_deref())
    {
        None
    } else {
        git::read_blob(source_dir, &delta.sha, rel_path)
    };
    let base_opt = delta
        .base_sha
        .as_deref()
        .and_then(|base| git::read_blob(source_dir, base, rel_path));
    let ours_opt = git::read_blob(target_dir, target_tip, rel_path);
    blob_outcome(
        source_dir,
        target_dir,
        scratch,
        &delta.sha,
        target_tip,
        rel_path,
        theirs_opt,
        base_opt,
        ours_opt,
        on_conflict,
    )
}

/// The blob-level branches of `apply_one`, without any worktree writes: the
/// equality short-circuits are identical, and divergence falls through to the
/// engine's own [`apply::three_way_merge`].
#[allow(clippy::too_many_arguments)]
fn blob_outcome(
    source_dir: &Path,
    target_dir: &Path,
    scratch: &Path,
    source_sha: &str,
    target_tip: &str,
    rel_path: &str,
    theirs_opt: Option<Vec<u8>>,
    base_opt: Option<Vec<u8>>,
    ours_opt: Option<Vec<u8>>,
    on_conflict: PromotionOnConflict,
) -> PreflightOutcome {
    // Same CRLF-insensitive text comparison as the apply path; binaries keep
    // exact byte semantics and are never normalised.
    let text_path = !theirs_opt.as_ref().is_some_and(|b| apply::is_binary(b))
        && !base_opt.as_ref().is_some_and(|b| apply::is_binary(b))
        && !ours_opt.as_ref().is_some_and(|b| apply::is_binary(b));
    let normalised = |bytes: &[u8]| -> Vec<u8> {
        if text_path {
            apply::strip_cr(bytes)
        } else {
            bytes.to_vec()
        }
    };
    let theirs_norm_opt = theirs_opt.as_ref().map(|b| normalised(b));
    let base_norm_opt = base_opt.as_ref().map(|b| normalised(b));
    let ours_norm_opt = ours_opt.as_ref().map(|b| normalised(b));

    // The source deleted the path.
    if theirs_opt.is_none() {
        if ours_opt.is_none() {
            return PreflightOutcome::Clean;
        }
        if base_opt.is_some() && ours_norm_opt == base_norm_opt {
            return PreflightOutcome::Clean;
        }
        // Delete-vs-diverge: the real apply deletes under `take_subsystem`.
        return take_or_conflict(on_conflict);
    }
    let theirs_norm = theirs_norm_opt.unwrap_or_default();

    // The target already matches the source.
    if ours_norm_opt.as_deref() == Some(theirs_norm.as_slice()) {
        return PreflightOutcome::Clean;
    }
    // The target does not carry the path: the add applies without a merge.
    if ours_opt.is_none() {
        return PreflightOutcome::Clean;
    }
    let ours_norm = ours_norm_opt.unwrap_or_default();
    // Add over an existing target file: allowed only when the target copy is
    // not newer than the source commit (same recency rule as the engine), or
    // unconditionally under `take_subsystem` (same as the real apply).
    if base_opt.is_none() {
        let allowed =
            add_overwrite_allowed(source_dir, target_dir, target_tip, source_sha, rel_path);
        return if allowed {
            PreflightOutcome::Clean
        } else {
            take_or_conflict(on_conflict)
        };
    }
    let base_norm = base_norm_opt.unwrap_or_default();
    // The target still matches the source baseline: the delta applies cleanly.
    if ours_norm == base_norm {
        return PreflightOutcome::Clean;
    }
    // The source left the content unchanged (e.g. a mode-only delta): the real
    // apply keeps the target's bytes without merging — even for binaries.
    if theirs_norm == base_norm {
        return PreflightOutcome::Clean;
    }
    // Both sides moved: the engine's own three-way merge decides (including
    // its whitespace-only auto-resolution, shared with the real apply).
    match apply::three_way_merge(scratch, rel_path, &ours_norm, &base_norm, &theirs_norm) {
        apply::MergeOutcome::Ok(content) => {
            if normalised(&content) == ours_norm {
                PreflightOutcome::Clean
            } else {
                PreflightOutcome::Merged
            }
        }
        apply::MergeOutcome::Conflict(_) => take_or_conflict(on_conflict),
    }
}

/// A would-be `conflict` under `take_subsystem` resolves the way the real
/// apply does (subsystem version wins — a change), so preflight reports it as
/// `merged` instead of `conflict`. Under `fail` it stays a conflict.
fn take_or_conflict(on_conflict: PromotionOnConflict) -> PreflightOutcome {
    if on_conflict == PromotionOnConflict::TakeSubsystem {
        PreflightOutcome::Merged
    } else {
        PreflightOutcome::Conflict
    }
}

/// Same recency rule as the engine's add-over-existing guard: an add may
/// overwrite the target copy only when the target branch never carried the
/// path or the source commit is at least as new. Blobs come from the source
/// checkout here (the engine reads them from its fetched object store instead).
fn add_overwrite_allowed(
    source_dir: &Path,
    target_dir: &Path,
    target_tip: &str,
    source_sha: &str,
    rel_path: &str,
) -> bool {
    let source_epoch = git::commit_epoch(source_dir, source_sha);
    match git::last_change_epoch(target_dir, target_tip, rel_path) {
        None => true,
        Some(target) => source_epoch.map(|source| source >= target).unwrap_or(false),
    }
}

fn resolve_target_tip(target_dir: &Path, branch: &str) -> Option<String> {
    [
        format!("refs/heads/{branch}"),
        format!("refs/remotes/origin/{branch}"),
        branch.to_string(),
    ]
    .into_iter()
    .find_map(|candidate| git::rev_parse(target_dir, &candidate))
}

fn unique_dir(root: &Path, prefix: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    root.join(format!("{prefix}-{pid}-{nanos}-{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::registry::Registry;
    use std::process::Command;

    static SANDBOX_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp(prefix: &str) -> PathBuf {
        let n = SANDBOX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "vos-preflight-{prefix}-{n}-{nanos}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        git_with_dates(dir, args, None)
    }

    fn git_with_dates(dir: &Path, args: &[&str], date: Option<&str>) -> String {
        let mut command = Command::new("git");
        command.arg("-C").arg(dir).args(args);
        if let Some(stamp) = date {
            command.env("GIT_AUTHOR_DATE", stamp);
            command.env("GIT_COMMITTER_DATE", stamp);
        }
        let out = command.output().expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init(dir: &Path) {
        std::fs::create_dir_all(dir).expect("mkdir repo");
        git(dir, &["init", "-q", "-b", "master"]);
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
        commit_all_at(dir, message, None)
    }

    fn commit_all_at(dir: &Path, message: &str, date: Option<&str>) -> String {
        git(dir, &["add", "-A"]);
        git_with_dates(
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
            date,
        );
        git(dir, &["rev-parse", "HEAD"])
    }

    fn test_registry() -> Registry {
        let text = r#"{
  "mainRepository": "ORG/main-system",
  "promotion": {
    "allowedTargetBranches": ["target"],
    "allowedTargetBranchPatterns": [],
    "requirePullRequest": true
  },
  "subsystems": [{
      "id": "hr-management",
      "name": "HR",
      "repository": "ORG/hr-management",
      "sourceBranch": "source",
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

    fn check(repo: &Path, shas: &[String]) -> PreflightReport {
        let registry = test_registry();
        let scratch = tmp("scratch");
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: shas,
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
        };
        let report = compute_preflight(repo, repo, &request, &scratch).expect("preflight computes");
        let _ = std::fs::remove_dir_all(&scratch);
        report
    }

    fn outcome_of(report: &PreflightReport, path: &str) -> PreflightOutcome {
        report
            .files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} missing from preflight"))
            .outcome
    }

    /// Base commit plus `source` / `target` branches diverging from it.
    fn diverging_repo() -> PathBuf {
        let repo = tmp("repo");
        init(&repo);
        write(&repo, "app.txt", "base\n");
        write(&repo, "doc.txt", "v1\n");
        let _ = commit_all(&repo, "base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        repo
    }

    #[test]
    fn clean_commit_reports_all_clean() {
        let repo = diverging_repo();
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "app.txt", "line1\nline2 source\n");
        write(&repo, "new-file.txt", "fresh\n");
        let sha = commit_all(&repo, "source work");
        // The target branch never touched these paths.

        let report = check(&repo, &[sha]);
        assert_eq!(report.conflicts, Vec::<String>::new());
        assert_eq!(report.conflict, 0);
        assert_eq!(report.merged, 0);
        assert_eq!(report.skipped, 0);
        assert_eq!(report.clean, 2, "{report:?}");
        assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Clean);
        assert_eq!(outcome_of(&report, "new-file.txt"), PreflightOutcome::Clean);
    }

    #[test]
    fn conflicting_path_is_listed() {
        let repo = diverging_repo();
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "app.txt", "target edit\n");
        let _ = commit_all(&repo, "target work");
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "app.txt", "source edit\n");
        let sha = commit_all(&repo, "source work");

        let report = check(&repo, &[sha]);
        assert_eq!(report.conflicts, vec!["app.txt".to_string()]);
        assert_eq!(report.conflict, 1);
        assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Conflict);
    }

    #[test]
    fn disjoint_edits_merge() {
        let repo = tmp("repo");
        init(&repo);
        write(&repo, "app.txt", "a\nb\nc\nd\ne\nf\n");
        let _ = commit_all(&repo, "base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "app.txt", "A\nb\nc\nd\ne\nf\n");
        let _ = commit_all(&repo, "target head");
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "app.txt", "a\nb\nc\nd\ne\nF\n");
        let sha = commit_all(&repo, "source tail");

        let report = check(&repo, &[sha]);
        assert_eq!(report.conflicts, Vec::<String>::new());
        assert_eq!(report.merged, 1, "{report:?}");
        assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Merged);
    }

    #[test]
    fn manifest_and_lockfile_are_skipped() {
        let repo = tmp("repo");
        init(&repo);
        write(
            &repo,
            "package.json",
            "{\"name\":\"x\",\"dependencies\":{\"a\":\"^1.0.0\"}}\n",
        );
        write(&repo, "package-lock.json", "{}\n");
        let _ = commit_all(&repo, "base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        git(&repo, &["checkout", "-q", "source"]);
        write(
            &repo,
            "package.json",
            "{\"name\":\"x\",\"dependencies\":{\"a\":\"^2.0.0\"}}\n",
        );
        write(&repo, "package-lock.json", "{\"bumped\":true}\n");
        let sha = commit_all(&repo, "bump dep");

        let report = check(&repo, &[sha]);
        assert_eq!(report.conflicts, Vec::<String>::new());
        assert_eq!(report.skipped, 2, "{report:?}");
        assert_eq!(
            outcome_of(&report, "package.json"),
            PreflightOutcome::Skipped
        );
        assert_eq!(
            outcome_of(&report, "package-lock.json"),
            PreflightOutcome::Skipped
        );
    }

    #[test]
    fn identical_target_is_clean() {
        let repo = diverging_repo();
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "doc.txt", "v2\n");
        let sha = commit_all(&repo, "source doc");
        // The target carries byte-identical content from its own commit.
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "doc.txt", "v2\n");
        let _ = commit_all(&repo, "target doc");

        let report = check(&repo, &[sha]);
        assert_eq!(report.conflicts, Vec::<String>::new());
        assert_eq!(outcome_of(&report, "doc.txt"), PreflightOutcome::Clean);
    }

    #[test]
    fn add_over_existing_recency_refuses_newer_target() {
        let repo = tmp("repo");
        init(&repo);
        write(&repo, "base.txt", "base\n");
        let _ = commit_all(&repo, "base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        // The source adds both files in one older commit.
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "newer.txt", "source\n");
        write(&repo, "older.txt", "source\n");
        let sha = commit_all_at(&repo, "source adds", Some("2024-06-01T00:00:00Z"));
        // The target adds its own newer copy of one file and an older copy of
        // the other; only the newer copy must refuse the add.
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "newer.txt", "target\n");
        let _ = commit_all_at(&repo, "target newer", Some("2025-01-01T00:00:00Z"));
        write(&repo, "older.txt", "target\n");
        let _ = commit_all_at(&repo, "target older", Some("2020-01-01T00:00:00Z"));

        let report = check(&repo, &[sha]);
        assert_eq!(report.conflicts, vec!["newer.txt".to_string()]);
        assert_eq!(outcome_of(&report, "newer.txt"), PreflightOutcome::Conflict);
        assert_eq!(outcome_of(&report, "older.txt"), PreflightOutcome::Clean);
    }

    #[test]
    fn preflight_mutates_nothing() {
        let repo = diverging_repo();
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "app.txt", "target edit\n");
        let _ = commit_all(&repo, "target work");
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "app.txt", "source edit\n");
        let sha = commit_all(&repo, "source work");

        let branches_before = git(&repo, &["branch", "--format=%(refname:short)"]);
        let status_before = git(&repo, &["status", "--porcelain"]);
        let worktrees_before = git(&repo, &["worktree", "list", "--porcelain"]);
        assert!(
            !branches_before.contains("integration/"),
            "fixture must start clean"
        );

        let registry = test_registry();
        let scratch = tmp("scratch");
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: &[sha],
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        assert_eq!(report.conflict, 1);
        let leftovers: Vec<_> = std::fs::read_dir(&scratch).expect("read scratch").collect();
        assert!(
            leftovers.is_empty(),
            "preflight must remove its merge scratch dir"
        );

        assert_eq!(
            git(&repo, &["branch", "--format=%(refname:short)"]),
            branches_before,
            "preflight must not create a branch"
        );
        assert_eq!(
            git(&repo, &["status", "--porcelain"]),
            status_before,
            "preflight must not touch the working tree"
        );
        assert_eq!(
            git(&repo, &["worktree", "list", "--porcelain"]),
            worktrees_before,
            "preflight must leave no worktree behind"
        );
        assert!(
            !git(&repo, &["branch", "--format=%(refname:short)"]).contains("integration/"),
            "no integration branch may exist afterwards"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}

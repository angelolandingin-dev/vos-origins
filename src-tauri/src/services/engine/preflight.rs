//! Dry-run copy-semantics preflight: per-file `clean` / `conflict` /
//! `skipped` computed from the same blobs and the same blob-membership rules
//! the real apply uses — without touching either repository.
//!
//! Read-only by construction: every git operation here is a read (`rev-parse`,
//! `cat-file`, `ls-tree`, `log`, `show`, `rev-list`, `diff`). This module never
//! calls `worktree_add`, never creates a branch or a commit, and writes no
//! files at all (copy semantics needs no `merge-file` scratch).
//!
//! `three_way_merge` has zero call sites on this path by construction (G3).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::apply;
use super::blobset;
use super::classify::{self, PathStatus};
use super::git;
use crate::github::models::PromotionMode;
use crate::github::models::PromotionOnConflict;
use crate::services::registry::IntegrationPolicy;

/// Per-file copy outcome, mirroring the real apply's branches:
/// - `clean`: copies without refusal (new file, target behind-or-equal via
///   blob membership, target already matches source, or source-unchanged);
/// - `merged`: retained for wire compat, never emitted (copy never merges);
/// - `conflict`: the engine would refuse the path outright
///   (`NON_CONTIGUOUS_PATH_OVERLAP`, `TARGET_DIVERGED`,
///   `GLOBAL_FILE_PROTECTED`, unsafe paths);
/// - `skipped`: dependency manifest/lockfile paths, which the engine merges or
///   regenerates rather than writes (plus main-owned/secret paths, which the
///   engine never imports, plus `excludedPaths`).
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
/// The registry snapshot is pinned once per call (addendum C): `policy`,
/// `global_files`, `allow_gaps` and `excluded_paths` come from the same
/// snapshot the apply would use, so the two cannot disagree because the file
/// changed mid-run.
pub struct PreflightRequest<'a> {
    pub policy: &'a IntegrationPolicy,
    pub commits: &'a [String],
    pub mode: PromotionMode,
    pub base_sha: Option<&'a str>,
    pub target_branch: &'a str,
    /// Mirrors the real apply's `on_conflict`. Copy semantics ignores it for
    /// correctness (refusals are fail-closed and never bypassed); the field
    /// stays for wire compat.
    pub on_conflict: PromotionOnConflict,
    /// Wire `allowGaps` (default false).
    pub allow_gaps: bool,
    /// Wire `excludedPaths` (default empty). Excluded paths report `Skipped`.
    pub excluded_paths: &'a [String],
    /// Snapshot of `globalFiles` (exact paths or directory prefixes).
    pub global_files: &'a [String],
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
/// Copy semantics (addendum §7): the apply copies unconditionally, never
/// merges — so preflight never merges either. `three_way_merge` has zero call
/// sites on this path by construction. Outcomes use the same vocabulary the
/// apply reports: `Clean` = will write/delete/leave-unchanged without refusal
/// (maps to apply `Written`/`Deleted`/`Unchanged`); `Conflict` = the apply
/// would refuse (`NON_CONTIGUOUS_PATH_OVERLAP`, `TARGET_DIVERGED`,
/// `GLOBAL_FILE_PROTECTED`); `Skipped` = dependency/main-owned/secret or
/// `excludedPaths`. `Merged` is retained for wire compat but never emitted.
pub fn compute_preflight(
    source_dir: &Path,
    target_dir: &Path,
    request: &PreflightRequest<'_>,
    scratch_root: &Path,
) -> Result<PreflightReport, PreflightError> {
    let _ = scratch_root;
    let target_tip = resolve_target_tip(target_dir, request.target_branch)
        .ok_or_else(|| PreflightError::UnknownTargetBranch(request.target_branch.to_string()))?;

    let plan = build_delta_plan(source_dir, request)?;

    // No scratch dir: copy semantics needs no `merge-file` temp files.
    let report = evaluate_plan(
        source_dir,
        target_dir,
        request.policy,
        &plan,
        &target_tip,
        request.allow_gaps,
        request.excluded_paths,
        request.global_files,
    );
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

#[allow(clippy::too_many_arguments)]
fn evaluate_plan(
    source_dir: &Path,
    target_dir: &Path,
    policy: &IntegrationPolicy,
    plan: &[PlannedDelta],
    target_tip: &str,
    allow_gaps: bool,
    excluded_paths: &[String],
    global_files: &[String],
) -> PreflightReport {
    // Copy semantics, same rules as the engine (addendum §7 + E3):
    // blob membership decides, dates never do. ONE ls-tree per revision for
    // ALL paths. A path touched by several commits reports its worst outcome.
    let excluded: HashSet<String> = excluded_paths.iter().cloned().collect();
    // Ordered selection for blob sets (plan is already ancestry-ordered by
    // build_delta_plan mirroring build_plan).
    let ordered: Vec<(String, Option<String>)> = plan
        .iter()
        .map(|d| (d.sha.clone(), d.base_sha.clone()))
        .collect();
    // All apply-candidate paths (classification pre-pass for Skipped).
    let mut candidate_paths: Vec<String> = Vec::new();
    for delta in plan {
        for entry in &delta.entries {
            for p in expanded_paths(entry) {
                if !candidate_paths.contains(&p) {
                    candidate_paths.push(p);
                }
            }
        }
    }
    candidate_paths.sort();
    let blob_sets = blobset::collect_blob_sets(source_dir, &ordered, &candidate_paths);
    let target_tree = blobset::tree_blob_map(target_dir, target_tip);
    let oldest_tree: HashMap<String, String> = match ordered.first().and_then(|(_, b)| b.as_deref())
    {
        Some(base) => blobset::tree_blob_map(source_dir, base),
        None => HashMap::new(),
    };
    // Newest selected toucher per path (for gap content-awareness).
    let mut newest_toucher: HashMap<String, String> = HashMap::new();
    for delta in plan {
        for entry in &delta.entries {
            for p in expanded_paths(entry) {
                newest_toucher.insert(p, delta.sha.clone());
            }
        }
    }
    // Skipped SHAs between oldest base and newest tip (same range rule as the
    // engine). Computed once; per-path touch map below.
    let mut skipped_touch: HashMap<String, Vec<String>> = HashMap::new();
    // Whether the request mode even has gaps (cumulative never does).
    let has_range = plan.len() > 1 && ordered.first().and_then(|(_, b)| b.as_deref()).is_some();
    if has_range {
        if let (Some(oldest_b), Some(newest_sha)) = (
            ordered.first().and_then(|(_, b)| b.as_deref()),
            ordered.last().map(|(s, _)| s.as_str()),
        ) {
            let selected: HashSet<&str> = ordered.iter().map(|(s, _)| s.as_str()).collect();
            if let Ok(out) = git::run(
                source_dir,
                &[
                    "rev-list",
                    "--topo-order",
                    "--reverse",
                    &format!("{oldest_b}..{newest_sha}"),
                ],
            ) {
                if out.status == 0 {
                    for line in out.stdout_str().lines() {
                        let sha = line.trim().to_string();
                        if sha.is_empty() || selected.contains(sha.as_str()) {
                            continue;
                        }
                        if let Ok((_, entries)) = git::name_status_delta(source_dir, &sha) {
                            for e in &entries {
                                for p in expanded_paths(e) {
                                    skipped_touch.entry(p).or_default().push(sha.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut by_path: HashMap<String, PreflightOutcome> = HashMap::new();
    for delta in plan {
        for entry in &delta.entries {
            for rel_path in expanded_paths(entry) {
                let outcome = evaluate_path_copy(
                    source_dir,
                    target_dir,
                    policy,
                    delta,
                    entry,
                    &rel_path,
                    target_tip,
                    &target_tree,
                    &blob_sets,
                    &oldest_tree,
                    &newest_toucher,
                    &skipped_touch,
                    allow_gaps,
                    &excluded,
                    global_files,
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
fn evaluate_path_copy(
    source_dir: &Path,
    target_dir: &Path,
    policy: &IntegrationPolicy,
    delta: &PlannedDelta,
    entry: &git::DeltaEntry,
    rel_path: &str,
    target_tip: &str,
    target_tree: &HashMap<String, String>,
    blob_sets: &HashMap<String, HashSet<Option<String>>>,
    oldest_tree: &HashMap<String, String>,
    newest_toucher: &HashMap<String, String>,
    skipped_touch: &HashMap<String, Vec<String>>,
    allow_gaps: bool,
    excluded: &HashSet<String>,
    global_files: &[String],
) -> PreflightOutcome {
    if rel_path.is_empty() {
        return PreflightOutcome::Conflict;
    }
    if excluded.contains(rel_path) {
        return PreflightOutcome::Skipped;
    }
    // Same mode lookup as the engine's classifier.
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

    // Same blob inputs as the real apply (CRLF-insensitive text compare).
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
    // Note: `ours` comes from the target tree map (ONE ls-tree), not a
    // per-path `cat-file`. Content equality below uses CRLF-stripped bytes.
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

    // Gap refusal (content-aware, same rule as the engine): a skipped SHA
    // touching this path with a material (non-undone) effect conflicts unless
    // allowGaps. Undone effects (newest == oldest base) skip harmlessly.
    if !allow_gaps {
        if let Some(touchers) = skipped_touch.get(rel_path) {
            if !touchers.is_empty() {
                let newest_blob = newest_toucher
                    .get(rel_path)
                    .and_then(|s| blobset::tree_blob_map(source_dir, s).get(rel_path).cloned());
                let oldest_blob = oldest_tree.get(rel_path).cloned();
                if newest_blob != oldest_blob {
                    return PreflightOutcome::Conflict;
                }
            }
        }
    }

    // globalFiles guard (same rule as the engine).
    if is_global_match(rel_path, global_files) {
        let target_blob = target_tree.get(rel_path).cloned();
        let allowed = match blob_sets.get(rel_path) {
            Some(set) => set.contains(&target_blob),
            None => target_blob.is_none(),
        };
        if !allowed {
            return PreflightOutcome::Conflict;
        }
    }

    // Overwrite refusal by blob membership (same rule as the engine).
    // `theirs == None` deletes are included: set = {base, None}.
    {
        let target_blob = target_tree.get(rel_path).cloned();
        let allowed = match blob_sets.get(rel_path) {
            Some(set) => set.contains(&target_blob),
            None => target_blob.is_none(),
        };
        if !allowed {
            return PreflightOutcome::Conflict;
        }
    }

    // Copy semantics below (no merge — `three_way_merge` is never called):
    // each branch mirrors `apply_one`'s equality short-circuits.
    if theirs_opt.is_none() {
        // Source deleted: allowed above means target absent or matching base.
        return PreflightOutcome::Clean;
    }
    let theirs_norm = theirs_norm_opt.unwrap_or_default();
    if ours_norm_opt.as_deref() == Some(theirs_norm.as_slice()) {
        return PreflightOutcome::Clean;
    }
    if ours_opt.is_none() {
        return PreflightOutcome::Clean;
    }
    let ours_norm = ours_norm_opt.unwrap_or_default();
    let base_norm = base_norm_opt.unwrap_or_default();
    if ours_norm == base_norm {
        return PreflightOutcome::Clean;
    }
    if theirs_norm == base_norm {
        return PreflightOutcome::Clean;
    }
    // Both sides moved with membership allowed (target behind-or-equal):
    // the copy overwrites — Clean (the apply reports Written/Replaced).
    // No `Merged`: copy never merges (G3).
    PreflightOutcome::Clean
}

fn is_global_match(path: &str, global_files: &[String]) -> bool {
    let norm = path.replace('\\', "/");
    for entry in global_files {
        let e = entry.replace('\\', "/");
        let e = e.trim().trim_end_matches('/').to_string();
        if e.is_empty() {
            continue;
        }
        if norm == e || norm.starts_with(&format!("{e}/")) {
            return true;
        }
    }
    false
}

/// REMOVED: `take_or_conflict`, `add_overwrite_allowed`, and the entire
/// `three_way_merge` branch. Copy semantics never merges: disjoint edits that
/// three-way would have called `Merged` now report `Clean` (will be replaced)
/// when blob membership allows, else `Conflict` (would refuse). Dates never
/// decide — blob membership does (addendum E3).
fn resolve_target_tip(target_dir: &Path, branch: &str) -> Option<String> {
    [
        format!("refs/heads/{branch}"),
        format!("refs/remotes/origin/{branch}"),
        branch.to_string(),
    ]
    .into_iter()
    .find_map(|candidate| git::rev_parse(target_dir, &candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::registry::Registry;
    use std::path::PathBuf;
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
        let empty: Vec<String> = Vec::new();
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: shas,
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
            allow_gaps: false,
            excluded_paths: &empty,
            global_files: &empty,
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
    fn disjoint_edits_no_longer_merge_copy_refuses_diverged() {
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

        // Copy semantics, blob membership (§3, §7): the target copy (`A…`)
        // matches neither the oldest base nor the source tip, so it has
        // diverged and preflight refuses — never `Merged`. Three-way would
        // have merged these disjoint edits; the copy does not merge (G3).
        let report = check(&repo, &[sha]);
        assert_eq!(report.merged, 0, "copy never merges: {report:?}");
        assert_eq!(report.conflict, 1, "{report:?}");
        assert_eq!(report.conflicts, vec!["app.txt".to_string()]);
        assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Conflict);
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
    fn add_over_existing_membership_refuses_any_diverged_target() {
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
        // The target adds its own copies (newer and older dates): blob
        // membership (§3) refuses BOTH — dates never decide (E3). Timestamps
        // are display order only; the target blobs match neither the oldest
        // base (absent) nor the source tip.
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "newer.txt", "target\n");
        let _ = commit_all_at(&repo, "target newer", Some("2025-01-01T00:00:00Z"));
        write(&repo, "older.txt", "target\n");
        let _ = commit_all_at(&repo, "target older", Some("2020-01-01T00:00:00Z"));

        let report = check(&repo, &[sha]);
        assert_eq!(report.merged, 0, "copy never merges: {report:?}");
        assert_eq!(
            report.conflicts,
            vec!["newer.txt".to_string(), "older.txt".to_string()]
        );
        assert_eq!(outcome_of(&report, "newer.txt"), PreflightOutcome::Conflict);
        assert_eq!(outcome_of(&report, "older.txt"), PreflightOutcome::Conflict);
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
        let empty: Vec<String> = Vec::new();
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: &[sha],
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
            allow_gaps: false,
            excluded_paths: &empty,
            global_files: &empty,
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        assert_eq!(report.conflict, 1);
        // Copy semantics writes no scratch files (no merge-file temps).
        let leftovers: Vec<_> = std::fs::read_dir(&scratch).expect("read scratch").collect();
        assert!(
            leftovers.is_empty(),
            "preflight must leave no scratch files (copy needs none)"
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

    /// Preflight and apply agree (§7, G3) for: a modified path (target
    /// behind), a divergent path (target not in blob set), a delete, an
    /// add-over-existing, and an identical no-op. Copy semantics: no `Merged`,
    /// `three_way_merge` has zero call sites above. When preflight says
    /// `Clean`, the apply writes the same bytes (`Written`/`Deleted`/
    /// `Unchanged`); when it says `Conflict`, the engine would refuse
    /// (`TARGET_DIVERGED`) and the apply is never run — they agree.
    #[test]
    fn preflight_and_apply_agree_on_five_shapes() {
        use crate::services::engine::apply::{apply_file_delta, ApplyOptions, ApplyOutcome};
        use crate::services::engine::blobset;

        let repo = tmp("agree");
        init(&repo);
        write(&repo, "mod.txt", "base\n");
        write(&repo, "div.txt", "base\n");
        write(&repo, "del.txt", "delete me\n");
        write(&repo, "same.txt", "same\n");
        let _ = commit_all(&repo, "base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        // Source: modify mod/div/same, delete del, add new.
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "mod.txt", "source mod\n");
        write(&repo, "div.txt", "source div\n");
        write(&repo, "same.txt", "same v2\n");
        std::fs::remove_file(repo.join("del.txt")).expect("remove del");
        write(&repo, "new.txt", "brand new\n");
        let sha = commit_all(&repo, "source work");
        // Target: leave mod at base (behind), diverge div + new, keep del at
        // base (so delete applies), match same (identical no-op).
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "div.txt", "target divergent\n");
        write(&repo, "new.txt", "target other\n");
        write(&repo, "same.txt", "same v2\n");
        let _ = commit_all(&repo, "target work");

        let registry = test_registry();
        let scratch = tmp("agree-scratch");
        let empty: Vec<String> = Vec::new();
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: std::slice::from_ref(&sha),
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
            allow_gaps: false,
            excluded_paths: &empty,
            global_files: &empty,
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        assert_eq!(report.merged, 0, "copy never merges: {report:?}");
        // Modified (behind: target == base, in set) → Clean.
        assert_eq!(outcome_of(&report, "mod.txt"), PreflightOutcome::Clean);
        // Divergent (target matches neither base nor tip) → Conflict.
        assert_eq!(outcome_of(&report, "div.txt"), PreflightOutcome::Conflict);
        // Delete (target == base, in set {base, None}) → Clean.
        assert_eq!(outcome_of(&report, "del.txt"), PreflightOutcome::Clean);
        // Add-over-existing diverged → Conflict.
        assert_eq!(outcome_of(&report, "new.txt"), PreflightOutcome::Conflict);
        // Identical no-op (target == source) → Clean.
        assert_eq!(outcome_of(&report, "same.txt"), PreflightOutcome::Clean);

        // Blob membership behind the verdicts (E3: dates never decide).
        let target_tip = git(&repo, &["rev-parse", "target"]);
        let parent = git(&repo, &["rev-parse", &format!("{sha}^")]);
        let ordered: Vec<(String, Option<String>)> = vec![(sha.clone(), Some(parent.clone()))];
        for path in ["mod.txt", "div.txt", "del.txt", "new.txt", "same.txt"] {
            let set = blobset::collect_blob_set(&repo, &ordered, path);
            let target_blob = blobset::blob_id(&repo, &target_tip, path);
            let allowed = set.contains(&target_blob);
            let preflight = outcome_of(&report, path);
            assert_eq!(
                allowed,
                preflight == PreflightOutcome::Clean,
                "blob membership must match preflight for {path}"
            );
        }
        let _ = ordered;

        // For Clean paths, the apply writes the same bytes preflight predicts.
        // Seed a worktree at the target tip and apply the source delta.
        let wt = tmp("agree-wt");
        let target_tip_sha = git(&repo, &["rev-parse", "target"]);
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--detach",
                wt.to_str().unwrap(),
                &target_tip_sha,
            ],
        );
        // Apply mod.txt (modified): must be Written with source bytes.
        let entry_mod = crate::services::engine::git::DeltaEntry {
            status: 'M',
            path: Some("mod.txt".to_string()),
            old_path: None,
            new_path: None,
            score: None,
        };
        let set = apply_file_delta(
            &repo,
            &wt,
            &sha,
            Some(&parent),
            &entry_mod,
            &scratch,
            &ApplyOptions::default(),
        )
        .expect("mod applies");
        assert!(set
            .results
            .iter()
            .any(|r| r.path == "mod.txt" && r.outcome == ApplyOutcome::Written));
        assert_eq!(
            std::fs::read(wt.join("mod.txt")).unwrap(),
            b"source mod\n",
            "applied bytes equal the source blob (preflight Clean)"
        );
        // Apply same.txt (identical): must be Unchanged.
        let entry_same = crate::services::engine::git::DeltaEntry {
            status: 'M',
            path: Some("same.txt".to_string()),
            old_path: None,
            new_path: None,
            score: None,
        };
        let set = apply_file_delta(
            &repo,
            &wt,
            &sha,
            Some(&parent),
            &entry_same,
            &scratch,
            &ApplyOptions::default(),
        )
        .expect("same applies");
        assert!(set
            .results
            .iter()
            .any(|r| r.path == "same.txt" && r.outcome == ApplyOutcome::Unchanged));
        // Apply del.txt (delete): must be Deleted.
        let entry_del = crate::services::engine::git::DeltaEntry {
            status: 'D',
            path: Some("del.txt".to_string()),
            old_path: None,
            new_path: None,
            score: None,
        };
        let set = apply_file_delta(
            &repo,
            &wt,
            &sha,
            Some(&parent),
            &entry_del,
            &scratch,
            &ApplyOptions::default(),
        )
        .expect("delete applies");
        assert!(set
            .results
            .iter()
            .any(|r| r.path == "del.txt" && r.outcome == ApplyOutcome::Deleted));
        assert!(!wt.join("del.txt").exists());
        let _ = std::fs::remove_dir_all(&scratch);
        let _ = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "remove", "--force", wt.to_str().unwrap()])
            .output();
    }
}

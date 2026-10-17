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
///   blob membership, target already matches source, source-unchanged, or a
///   diverged target copy the subsystem's version overwrites — promotion is a
///   copy and the subsystem's copy always wins);
/// - `merged`: retained for wire compat, never emitted (copy never merges);
/// - `conflict`: the engine would still refuse the path outright
///   (`GLOBAL_FILE_PROTECTED`, unsafe paths);
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
    /// Set when writing this path would move it backwards (warning only).
    pub reverts_newer: Option<crate::github::models::RevertsNewerWork>,
}

#[derive(Debug, Clone, Default)]
pub struct PreflightReport {
    pub files: Vec<PreflightFile>,
    pub clean: usize,
    pub merged: usize,
    pub conflict: usize,
    pub skipped: usize,
    pub conflicts: Vec<String>,
    /// Contract pin (dialog depends on it): `conflicts` names only paths the
    /// engine would still refuse (global-file protection, unsafe
    /// paths) — a diverged target copy is NOT a conflict: the subsystem's
    /// copy overwrites it, so it reports `clean` and is listed in `replaced`
    /// instead. `replaced` names every diverged path the promotion will
    /// overwrite, under either policy. Paths refused regardless of policy
    /// (unsafe, secret/main-owned, global-file protected, dependency,
    /// lockfile) never enter `replaced`.
    pub replaced: Vec<String>,
    /// Paths skipped via `excludedPaths` (subset of `skipped`).
    pub excluded: Vec<String>,
}

/// Read-only inputs for [`compute_preflight`] (mirrors `EngineRequest`).
/// The registry snapshot is pinned once per call (addendum C): `policy`,
/// `global_files` and `excluded_paths` come from the same
/// snapshot the apply would use, so the two cannot disagree because the file
/// changed mid-run.
pub struct PreflightRequest<'a> {
    pub policy: &'a IntegrationPolicy,
    pub commits: &'a [String],
    pub mode: PromotionMode,
    pub base_sha: Option<&'a str>,
    pub target_branch: &'a str,
    /// Wire `onConflict`. Retained for wire compatibility only: promotion is
    /// a copy and the subsystem's copy always wins, so a diverged target
    /// copy is overwritten (and reported in `replaced`) under either value.
    /// Never bypasses unsafe paths, secret/main-owned files, global-file
    /// protection, dependency or lockfile guards.
    pub on_conflict: PromotionOnConflict,
    /// Wire `excludedPaths` (default empty). Excluded paths report `Skipped`.
    pub excluded_paths: &'a [String],
    /// Snapshot of `globalFiles` (exact paths or directory prefixes).
    pub global_files: &'a [String],
    /// The source branch tip the selection is promoted from. Feeds the
    /// "reverts newer work" warning: a write moves a path backwards when the
    /// target blob is reachable from this tip but not from the selected commit
    /// producing the write. `None` skips the check (no warning, never a
    /// refusal).
    pub source_tip: Option<&'a str>,
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
/// apply reports: `Clean` = will write/delete/leave-unchanged/overwrite
/// without refusal (maps to apply `Written`/`Deleted`/`Unchanged`/replaced);
/// `Conflict` = the apply would refuse (`GLOBAL_FILE_PROTECTED`, unsafe
/// paths); `Skipped` = dependency/
/// main-owned/secret or `excludedPaths`. `Merged` is retained for wire compat
/// but never emitted.
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
        request.excluded_paths,
        request.global_files,
        request.source_tip,
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
    excluded_paths: &[String],
    global_files: &[String],
    source_tip: Option<&str>,
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
    // Shared overwrite-allowance (the SAME function the engine calls): range
    // membership widened by one batched selected-history probe for all paths,
    // so preview and apply cannot disagree. Scoped to the selected ancestry —
    // main-system-only bytes committed on the target side are not reachable
    // from it, so they still refuse.
    let history_tips: Vec<String> = ordered.iter().map(|(sha, _)| sha.clone()).collect();
    let overwrite_ok = blobset::overwrite_allowed(
        source_dir,
        &blob_sets,
        &target_tree,
        &candidate_paths,
        &history_tips,
    );
    // Newest selected toucher per path (for the reverts-newer warning).
    let mut newest_toucher: HashMap<String, String> = HashMap::new();
    for delta in plan {
        for entry in &delta.entries {
            for p in expanded_paths(entry) {
                newest_toucher.insert(p, delta.sha.clone());
            }
        }
    }
    let mut by_path: HashMap<String, PreflightOutcome> = HashMap::new();
    let mut replaced: Vec<String> = Vec::new();
    let mut excluded_hit: Vec<String> = Vec::new();
    // "Reverts newer work" warning (information only, never a refusal): one
    // `rev-list --objects <write>..<tip>` per distinct selected write commit
    // (bounded by the selection size), intersected in memory with the target
    // blobs. A path warns when the target holds content from a LATER
    // subsystem commit than the selected commit producing the write.
    let newer_by_write: HashMap<String, HashSet<String>> = match source_tip {
        Some(tip) => {
            let mut writes: Vec<String> = newest_toucher.values().cloned().collect();
            writes.sort();
            writes.dedup();
            blobset::blobs_after(source_dir, &writes, tip)
        }
        None => HashMap::new(),
    };
    let tip_str = source_tip.unwrap_or("");
    // path -> target blob for Clean non-delete writes that move backwards.
    let mut warned_paths: HashMap<String, String> = HashMap::new();
    for delta in plan {
        for entry in &delta.entries {
            for rel_path in expanded_paths(entry) {
                // `candidate_paths` was built from this same expansion, so the
                // map always has the path; `false` is fail-closed if not.
                let allowed = overwrite_ok.get(&rel_path).copied().unwrap_or(false);
                let outcome = evaluate_path_copy(
                    source_dir,
                    target_dir,
                    policy,
                    delta,
                    entry,
                    &rel_path,
                    target_tip,
                    &excluded,
                    global_files,
                    allowed,
                    &mut replaced,
                );
                if excluded.contains(&rel_path) && !excluded_hit.contains(&rel_path) {
                    excluded_hit.push(rel_path.clone());
                }
                // Backwards-write candidate (warning only, never a refusal):
                // a Clean non-delete write whose target blob is reachable
                // from the source tip but not from the selected commit
                // producing it — main already holds a later version.
                let is_delete_side = entry.status == 'D'
                    || (entry.is_rename_or_copy()
                        && Some(rel_path.as_str()) == entry.old_path.as_deref());
                if outcome == PreflightOutcome::Clean && !is_delete_side {
                    if let (Some(write), Some(target_blob)) =
                        (newest_toucher.get(&rel_path), target_tree.get(&rel_path))
                    {
                        let newer = newer_by_write
                            .get(write)
                            .is_some_and(|set| set.contains(target_blob));
                        if newer {
                            warned_paths
                                .entry(rel_path.clone())
                                .or_insert_with(|| target_blob.clone());
                        }
                    }
                }
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
    let mut files: Vec<PreflightFile> = {
        // One naming lookup per warned path (zero when nothing warns): the
        // newer commit is part of the warning so the operator can verify it.
        // Unresolvable falls back to a nameless warning rather than a guess.
        let mut warned: Vec<(&String, &String)> = warned_paths.iter().collect();
        warned.sort_by(|a, b| a.0.cmp(b.0));
        let mut named: HashMap<String, crate::github::models::RevertsNewerWork> = HashMap::new();
        for (path, blob) in warned {
            let (sha, subject) = blobset::newest_commit_for_blob(source_dir, blob, tip_str, path)
                .unwrap_or_default();
            named.insert(
                (*path).clone(),
                crate::github::models::RevertsNewerWork { sha, subject },
            );
        }
        by_path
            .into_iter()
            .map(|(path, outcome)| {
                let reverts_newer = named.remove(&path);
                PreflightFile {
                    path,
                    outcome,
                    reverts_newer,
                }
            })
            .collect()
    };
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
    replaced.sort();
    replaced.dedup();
    report.replaced = replaced;
    excluded_hit.sort();
    report.excluded = excluded_hit;
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
    excluded: &HashSet<String>,
    global_files: &[String],
    overwrite_ok: bool,
    replaced: &mut Vec<String>,
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

    // globalFiles guard (same shared allowance as the engine). Protected
    // app-shell files stay refused regardless of policy: `take_subsystem`
    // never rescues them, so they never enter `replaced`.
    if is_global_match(rel_path, global_files) && !overwrite_ok {
        return PreflightOutcome::Conflict;
    }

    // Overwrite on the shared allowance (same rule as the engine).
    // `theirs == None` deletes are included: set = {base, None}.
    // Promotion is a copy and the subsystem's copy always wins: a diverged
    // target copy is overwritten, so it reports `clean` here and is recorded
    // in `replaced` for the dialog's overwrite preview — never `conflict`.
    if !overwrite_ok {
        if !replaced.iter().any(|p| p == rel_path) {
            replaced.push(rel_path.to_string());
        }
        return PreflightOutcome::Clean;
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
/// Tip of the target branch the preflight evaluates against: the shared branch
/// resolver (local ref, then bare name, then ANY remote-tracking ref — see
/// [`git::resolve_branch_ref`]) resolved to a commit SHA, so the preview and
/// the engine agree on the same tip object.
fn resolve_target_tip(target_dir: &Path, branch: &str) -> Option<String> {
    git::resolve_branch_ref(target_dir, branch)
        .and_then(|reference| git::rev_parse(target_dir, &reference))
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
            excluded_paths: &empty,
            global_files: &empty,
            source_tip: None,
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
    fn diverged_path_is_overwritten_not_listed_as_conflict() {
        let repo = diverging_repo();
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "app.txt", "target edit\n");
        let _ = commit_all(&repo, "target work");
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "app.txt", "source edit\n");
        let sha = commit_all(&repo, "source work");

        // Promotion is a copy: the subsystem's copy always wins. A diverged
        // target copy reports `clean` and is recorded as replaced — never a
        // preflight conflict.
        let report = check(&repo, &[sha]);
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(report.conflict, 0);
        assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Clean);
        assert_eq!(report.replaced, vec!["app.txt".to_string()]);
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
        // diverged — and the subsystem's copy overwrites it. Never `Merged`
        // (G3), never `Conflict`: divergence is not a refusal.
        let report = check(&repo, &[sha]);
        assert_eq!(report.merged, 0, "copy never merges: {report:?}");
        assert_eq!(report.conflict, 0, "{report:?}");
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Clean);
        assert_eq!(report.replaced, vec!["app.txt".to_string()]);
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
    fn add_over_existing_membership_diverged_target_is_overwritten() {
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
            report.conflict, 0,
            "divergence is overwritten, not refused: {report:?}"
        );
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(
            report.replaced,
            vec!["newer.txt".to_string(), "older.txt".to_string()]
        );
        assert_eq!(outcome_of(&report, "newer.txt"), PreflightOutcome::Clean);
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
        let empty: Vec<String> = Vec::new();
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: &[sha],
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
            excluded_paths: &empty,
            global_files: &empty,
            source_tip: None,
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        assert_eq!(report.conflict, 0, "divergence is overwritten: {report:?}");
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(report.replaced, vec!["app.txt".to_string()]);
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
    /// `three_way_merge` has zero call sites above. The subsystem's copy
    /// always wins, so every shape reports `Clean` — diverged paths additionally
    /// appear in `replaced`. When preflight says `Clean`, the apply writes the
    /// same bytes (`Written`/`Deleted`/`Unchanged`).
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
            excluded_paths: &empty,
            global_files: &empty,
            source_tip: None,
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        assert_eq!(report.merged, 0, "copy never merges: {report:?}");
        assert_eq!(report.conflict, 0, "divergence is overwritten: {report:?}");
        assert!(report.conflicts.is_empty(), "{report:?}");
        // Modified (behind: target == base, in set) → Clean.
        assert_eq!(outcome_of(&report, "mod.txt"), PreflightOutcome::Clean);
        // Divergent (target matches neither base nor tip) → Clean + replaced.
        assert_eq!(outcome_of(&report, "div.txt"), PreflightOutcome::Clean);
        // Delete (target == base, in set {base, None}) → Clean.
        assert_eq!(outcome_of(&report, "del.txt"), PreflightOutcome::Clean);
        // Add-over-existing diverged → Clean + replaced.
        assert_eq!(outcome_of(&report, "new.txt"), PreflightOutcome::Clean);
        // Identical no-op (target == source) → Clean.
        assert_eq!(outcome_of(&report, "same.txt"), PreflightOutcome::Clean);
        assert_eq!(
            report.replaced,
            vec!["div.txt".to_string(), "new.txt".to_string()]
        );

        // Blob membership behind the verdicts (E3: dates never decide):
        // membership-allowed paths report Clean without audit, while
        // membership-failed paths report Clean AND appear in `replaced`
        // (the subsystem's copy overwrites them).
        let target_tip = git(&repo, &["rev-parse", "target"]);
        let parent = git(&repo, &["rev-parse", &format!("{sha}^")]);
        let ordered: Vec<(String, Option<String>)> = vec![(sha.clone(), Some(parent.clone()))];
        for path in ["mod.txt", "div.txt", "del.txt", "new.txt", "same.txt"] {
            let set = blobset::collect_blob_set(&repo, &ordered, path);
            let target_blob = blobset::blob_id(&repo, &target_tip, path);
            let allowed = set.contains(&target_blob);
            let preflight = outcome_of(&report, path);
            assert_eq!(
                preflight,
                PreflightOutcome::Clean,
                "every shape copies without refusal: {path}"
            );
            assert_eq!(
                allowed,
                !report.replaced.iter().any(|p| p == path),
                "membership decides the audit bucket for {path}"
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

    /// Dialog contract pin: divergence is NOT a conflict under either policy.
    /// The subsystem's copy always wins, so a diverged path reports `clean`
    /// and is recorded in `replaced` whether `on_conflict` is `fail` or
    /// `take_subsystem`. `conflicts` stays empty in both cases — only true
    /// refusals (global protection, unsafe paths) land there.
    #[test]
    fn divergence_is_clean_and_replaced_under_both_policies_never_a_conflict() {
        let repo = diverging_repo();
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "app.txt", "target edit\n");
        let _ = commit_all(&repo, "target work");
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "app.txt", "source edit\n");
        let sha = commit_all(&repo, "source work");

        for on_conflict in [
            PromotionOnConflict::Fail,
            PromotionOnConflict::TakeSubsystem,
        ] {
            let registry = test_registry();
            let scratch = tmp("scratch");
            let empty: Vec<String> = Vec::new();
            let request = PreflightRequest {
                policy: &registry.subsystems[0].integration_policy,
                commits: std::slice::from_ref(&sha),
                mode: PromotionMode::Individual,
                base_sha: None,
                target_branch: "target",
                on_conflict,
                excluded_paths: &empty,
                global_files: &empty,
                source_tip: None,
            };
            let report =
                compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
            assert!(
                report.conflicts.is_empty(),
                "divergence must never be a conflict under {on_conflict:?}: {report:?}"
            );
            assert_eq!(report.conflict, 0);
            assert_eq!(outcome_of(&report, "app.txt"), PreflightOutcome::Clean);
            assert_eq!(
                report.replaced,
                vec!["app.txt".to_string()],
                "the overwrite is recorded under {on_conflict:?}"
            );
            let _ = std::fs::remove_dir_all(&scratch);
        }
    }

    /// Selecting an older commit while the target already holds a later
    /// subsystem version warns — naming the newer commit — without refusing:
    /// the write would move the path backwards.
    #[test]
    fn selecting_older_commit_over_newer_target_warns_with_name() {
        let repo = tmp("reverts");
        init(&repo);
        write(&repo, "f.txt", "v1\n");
        let a = commit_all(&repo, "A base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "f.txt", "v2\n");
        let b = commit_all(&repo, "B second version");
        // The target independently holds v2 (same bytes as B).
        git(&repo, &["checkout", "-q", "target"]);
        write(&repo, "f.txt", "v2\n");
        let _ = commit_all(&repo, "target takes v2");

        let registry = test_registry();
        let scratch = tmp("reverts-scratch");
        let empty: Vec<String> = Vec::new();
        let tip = git(&repo, &["rev-parse", "source"]);
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: std::slice::from_ref(&a),
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
            excluded_paths: &empty,
            global_files: &empty,
            source_tip: Some(tip.as_str()),
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        let file = report
            .files
            .iter()
            .find(|f| f.path == "f.txt")
            .expect("f.txt in preflight");
        assert_eq!(file.outcome, PreflightOutcome::Clean, "{report:?}");
        let warn = file
            .reverts_newer
            .as_ref()
            .expect("writing v1 over the target's v2 warns");
        assert_eq!(warn.sha, b, "the warning names the newer commit");
        assert_eq!(warn.subject, "B second version");
        // Warning only: never a conflict, never a refusal.
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(report.conflict, 0);
        assert_eq!(report.replaced, vec!["f.txt".to_string()]);

        // Without a source tip the check is skipped — still no refusal, and
        // no warning rather than a guessed one.
        let request = PreflightRequest {
            source_tip: None,
            ..request
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        let file = report
            .files
            .iter()
            .find(|f| f.path == "f.txt")
            .expect("f.txt in preflight");
        assert_eq!(file.outcome, PreflightOutcome::Clean);
        assert!(file.reverts_newer.is_none());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Selecting the newer commit while the target holds the older version is
    /// simply behind: the write moves the path forwards, so it never warns.
    #[test]
    fn selecting_newer_commit_over_older_target_does_not_warn() {
        let repo = tmp("forward");
        init(&repo);
        write(&repo, "f.txt", "v1\n");
        let _a = commit_all(&repo, "A base");
        git(&repo, &["branch", "source"]);
        git(&repo, &["branch", "target"]);
        git(&repo, &["checkout", "-q", "source"]);
        write(&repo, "f.txt", "v2\n");
        let b = commit_all(&repo, "B second version");
        // The target still holds v1: an ancestor version, not newer work.

        let registry = test_registry();
        let scratch = tmp("forward-scratch");
        let empty: Vec<String> = Vec::new();
        let tip = git(&repo, &["rev-parse", "source"]);
        let request = PreflightRequest {
            policy: &registry.subsystems[0].integration_policy,
            commits: std::slice::from_ref(&b),
            mode: PromotionMode::Individual,
            base_sha: None,
            target_branch: "target",
            on_conflict: PromotionOnConflict::Fail,
            excluded_paths: &empty,
            global_files: &empty,
            source_tip: Some(tip.as_str()),
        };
        let report =
            compute_preflight(&repo, &repo, &request, &scratch).expect("preflight computes");
        let file = report
            .files
            .iter()
            .find(|f| f.path == "f.txt")
            .expect("f.txt in preflight");
        assert_eq!(file.outcome, PreflightOutcome::Clean, "{report:?}");
        assert!(
            file.reverts_newer.is_none(),
            "a forwards write over an ancestor version never warns: {report:?}"
        );
        assert!(
            report.replaced.is_empty(),
            "a behind target needs no overwrite record: {report:?}"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The preflight tip resolver must agree with the engine: a branch that
    /// exists only as a non-`origin` remote-tracking ref resolves, so the
    /// preview is computed against the same tip the promotion would use.
    #[test]
    fn resolve_target_tip_accepts_a_branch_only_on_a_non_origin_remote() {
        let root = tmp("non-origin-tip");
        let upstream = root.join("upstream");
        let clone = root.join("clone");
        init(&upstream);
        write(&upstream, "base.txt", "base\n");
        let _ = commit_all(&upstream, "base");
        git(&upstream, &["branch", "UAT"]);
        let uat = git(&upstream, &["rev-parse", "UAT"]);

        // A fork: `origin` carries only `master`; the second remote carries
        // `UAT`.
        git(
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
        git(
            &clone,
            &[
                "remote",
                "add",
                "upstream",
                upstream.to_str().expect("utf8"),
            ],
        );
        git(&clone, &["fetch", "-q", "upstream"]);

        assert!(
            git::rev_parse(&clone, "refs/remotes/upstream/UAT").is_some(),
            "precondition: UAT exists only as a remote-tracking ref on upstream"
        );
        assert!(
            git::rev_parse(&clone, "refs/remotes/origin/UAT").is_none(),
            "precondition: origin does not carry UAT"
        );
        assert_eq!(
            resolve_target_tip(&clone, "UAT"),
            Some(uat),
            "the preflight tip must resolve a branch on a non-origin remote"
        );
        assert_eq!(resolve_target_tip(&clone, "does-not-exist"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}

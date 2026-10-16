//! App-driven promotion engine (`ARCHITECTURE.md` §4).
//! Fail closed; never pushes to a protected branch, never adds files
//! beyond the integration PR.

pub mod apply;
pub mod classify;
pub mod deps;
pub mod git;
pub mod github_ops;
pub mod history;
pub mod lockfile;
pub mod preflight;
pub mod provenance;
pub mod semver;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::github::models::PromotionDelivery;
use crate::github::models::PromotionMode;
use crate::github::models::PromotionOnConflict;
use crate::services::registry::{
    assert_full_sha, assert_safe_branch_name, DependencyStrategy, Registry, SubsystemEntry,
};

use classify::PathStatus;
use deps::{DepAdd, DepUpdate, MovedDep, RemovedDep, ReviewNote};
use github_ops::{CreatePrRequest, GitHubOps, PullRequestInfo, PushOutcome};
use lockfile::{LockfileGenerator, LockfileOutcome};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blocker {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
}

impl Blocker {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct EngineRequest {
    pub subsystem_id: String,
    pub commits: Vec<String>,
    pub mode: PromotionMode,
    pub base_sha: Option<String>,
    pub target_branch: String,
    /// `local` builds the branch + commit in the target clone and stops
    /// (no push, no PR); `local_direct` commits the same delta straight onto
    /// the local target branch (no push, no PR — pushing stays manual);
    /// `worktree` writes the same delta into the user's own checkout of the
    /// target branch and stages it (no branch, no commit, no push, no PR);
    /// `push_pr` is today's push-then-PR behaviour.
    /// Defaults to `local_direct` when omitted on the wire.
    pub delivery: PromotionDelivery,
    /// `fail` (default) refuses every unmergeable path; `take_subsystem`
    /// resolves merge conflicts, both-sides-touched binaries, delete-vs-diverge,
    /// and recency-refused adds with the subsystem's version and reports every
    /// such path. Never bypasses path-safety, classification, dependency,
    /// lockfile, or NOOP guards.
    pub on_conflict: PromotionOnConflict,
    /// The promotion commit's own message (required; an empty one is refused so
    /// a multi-commit promotion is never labelled with only the last subject).
    pub message: String,
}

impl EngineRequest {
    /// Test helper: `push_pr` delivery without touching the new field.
    #[cfg(test)]
    pub fn for_push_pr(
        subsystem_id: &str,
        commits: Vec<String>,
        mode: PromotionMode,
        base_sha: Option<String>,
        target_branch: &str,
    ) -> Self {
        Self {
            subsystem_id: subsystem_id.to_string(),
            commits,
            mode,
            base_sha,
            target_branch: target_branch.to_string(),
            delivery: PromotionDelivery::PushPr,
            on_conflict: PromotionOnConflict::Fail,
            message: "test promotion".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EngineOptions {
    pub target_repo_dir: PathBuf,
    pub temp_root: PathBuf,
    pub push_remote_url: Option<String>,
    pub token: Option<String>,
    pub allow_shared_config: bool,
    pub allow_removals: bool,
    pub nonce: String,
    pub push: bool,
    pub create_pr: bool,
}

pub struct EngineDeps<'a> {
    pub github: &'a dyn GitHubOps,
    pub lockfile: &'a dyn LockfileGenerator,
    pub source_url: &'a dyn Fn(&SubsystemEntry) -> Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ChangeSummary {
    pub applied: Vec<String>,
    pub updated: Vec<String>,
    pub merged: Vec<String>,
    pub deleted: Vec<String>,
    pub main_owned_skipped: Vec<String>,
    pub secret_excluded: Vec<String>,
    pub regenerated: Vec<String>,
    /// Paths resolved via `on_conflict = take_subsystem` (the subsystem's
    /// version won instead of failing). Also surfaced as a dedicated
    /// `conflict_taken_from_subsystem` warning.
    pub conflict_taken: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct DependencySummary {
    pub added: Vec<DepAdd>,
    pub updated: Vec<DepUpdate>,
    pub removed: Vec<RemovedDep>,
    pub moved: Vec<MovedDep>,
    pub review: Vec<ReviewNote>,
    pub noop: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct VerificationSummary {
    pub mode: String,
    pub order: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PromotionReport {
    pub ok: bool,
    pub result: Option<String>,
    pub blockers: Vec<Blocker>,
    pub warnings: Vec<Blocker>,
    pub integration_branch: Option<String>,
    pub start_point: Option<String>,
    pub commit_sha: Option<String>,
    pub pull_request: Option<PullRequestInfo>,
    pub push: Option<PushOutcome>,
    pub provenance_message: Option<String>,
    pub changes: ChangeSummary,
    pub dependencies: DependencySummary,
    pub lockfile: Option<LockfileOutcome>,
    pub verification: VerificationSummary,
    pub worktree: Option<PathBuf>,
    pub temp_dir: Option<PathBuf>,
    /// `worktree` delivery only: every path the promotion wrote or deleted,
    /// staged in the user's own checkout (`git add` / `git rm`). Empty for
    /// every other delivery mode. The command layer builds the follow-up
    /// `git commit` pathspec from this, so it must stay exactly the staged set.
    pub staged_paths: Vec<String>,
}

impl PromotionReport {
    fn new() -> Self {
        Self {
            ok: false,
            result: None,
            blockers: Vec::new(),
            warnings: Vec::new(),
            integration_branch: None,
            start_point: None,
            commit_sha: None,
            pull_request: None,
            push: None,
            provenance_message: None,
            changes: ChangeSummary::default(),
            dependencies: DependencySummary::default(),
            lockfile: None,
            verification: VerificationSummary::default(),
            worktree: None,
            temp_dir: None,
            staged_paths: Vec::new(),
        }
    }

    pub fn blocker_codes(&self) -> Vec<String> {
        self.blockers.iter().map(|b| b.code.clone()).collect()
    }

    pub fn has_blocker(&self, code: &str) -> bool {
        self.blockers.iter().any(|b| b.code == code)
    }
}

struct Bail;
type StepResult = Result<(), Bail>;

#[derive(Debug, Clone)]
struct ClassifiedEntry {
    delta: git::DeltaEntry,
    path: String,
    status: PathStatus,
    reason: String,
}

#[derive(Debug, Clone)]
struct VerifiedEntry {
    sha: String,
    base_sha: Option<String>,
    entries: Vec<git::DeltaEntry>,
}

#[derive(Debug, Clone)]
struct PlannedEntry {
    sha: String,
    base_sha: Option<String>,
    entries: Vec<ClassifiedEntry>,
}

fn short(sha: &str) -> String {
    sha.chars().take(12).collect()
}

/// Expected/policy failures become blockers, never panics.
pub fn run_promotion(
    registry: &Registry,
    request: &EngineRequest,
    options: &EngineOptions,
    deps: &EngineDeps<'_>,
) -> PromotionReport {
    let mut report = PromotionReport::new();
    let mut worktree: Option<PathBuf> = None;

    let outcome = run_inner(registry, request, options, deps, &mut report, &mut worktree);

    if let Some(dir) = worktree.as_ref() {
        if dir.exists() {
            git::worktree_remove(&options.target_repo_dir, dir);
            let _ = std::fs::remove_dir_all(dir);
        }
    }
    report.worktree = None;

    if outcome.is_err() {
        // A failed promotion must not leave the temporary `integration/*`
        // ref sitting at the target tip (especially under a persistent
        // `VOS_TARGET_REPO_DIR` clone): the worktree is gone, so the local
        // branch ref is deleted too. A branch already pushed before a later
        // failure (e.g. PR creation) stays on the remote for diagnosis; only
        // the local ref is cleaned here.
        if let Some(branch) = report.integration_branch.as_deref() {
            let _ = git::run(
                &options.target_repo_dir,
                &["branch", "--delete", "--force", branch],
            );
        }
    }

    if outcome.is_ok() {
        report.ok = true;
        report.result = Some("promoted".to_string());
    }
    report
}

#[allow(clippy::too_many_arguments)]
fn run_inner(
    registry: &Registry,
    request: &EngineRequest,
    options: &EngineOptions,
    deps: &EngineDeps<'_>,
    report: &mut PromotionReport,
    worktree: &mut Option<PathBuf>,
) -> StepResult {
    if crate::services::registry::assert_subsystem_id(&request.subsystem_id).is_err() {
        report.blockers.push(Blocker::new(
            "SUBSYSTEM_INVALID",
            format!("Invalid subsystem id shape: {:?}", request.subsystem_id),
        ));
        return Err(Bail);
    }
    let Some(subsystem) = registry.find(&request.subsystem_id) else {
        report.blockers.push(Blocker::new(
            "SUBSYSTEM_NOT_REGISTERED",
            format!(
                "'{}' is not present in the tool registry.",
                request.subsystem_id
            ),
        ));
        return Err(Bail);
    };
    if !subsystem.enabled {
        report.blockers.push(Blocker::new(
            "SUBSYSTEM_DISABLED",
            format!("Subsystem \"{}\" is registered but disabled.", subsystem.id),
        ));
        return Err(Bail);
    }

    if let Err(err) = assert_safe_branch_name(&request.target_branch, "target_branch") {
        report
            .blockers
            .push(Blocker::new("TARGET_INVALID", err.message));
        return Err(Bail);
    }
    let decision = registry.is_target_allowed(&request.target_branch);
    if !decision.allowed {
        report.blockers.push(Blocker::new(
            "TARGET_NOT_ALLOWED",
            format!(
                "Target branch \"{}\" is not permitted by policy ({}).",
                request.target_branch, decision.reason
            ),
        ));
        return Err(Bail);
    }

    if request.commits.is_empty() {
        report.blockers.push(Blocker::new(
            "COMMIT_VERIFICATION_FAILED",
            "No source commit SHAs were selected.",
        ));
        return Err(Bail);
    }
    let mut seen: Vec<&str> = Vec::new();
    for sha in &request.commits {
        if let Err(err) = assert_full_sha(sha, "commits[]") {
            report
                .blockers
                .push(Blocker::new("COMMIT_VERIFICATION_FAILED", err.message));
            return Err(Bail);
        }
        if seen.contains(&sha.as_str()) {
            report.blockers.push(Blocker::new(
                "COMMIT_VERIFICATION_FAILED",
                format!("commits contains a duplicate SHA: {sha}"),
            ));
            return Err(Bail);
        }
        seen.push(sha);
    }

    if matches!(
        request.delivery,
        PromotionDelivery::Local | PromotionDelivery::LocalDirect | PromotionDelivery::Worktree
    ) && !git::is_git_repository(&options.target_repo_dir)
    {
        report.blockers.push(Blocker::new(
            "LOCAL_REPO_INVALID",
            format!(
                "The local target path '{}' is not a git repository.",
                options.target_repo_dir.display()
            ),
        ));
        return Err(Bail);
    }

    let target_ref = resolve_local_ref(&options.target_repo_dir, &request.target_branch);
    let Some(target_tip) = target_ref else {
        match deps
            .github
            .resolve_target_tip(&registry.main_repository, &request.target_branch)
        {
            Ok(Some(_)) => {
                report.blockers.push(Blocker::new(
                    "TARGET_NOT_LOCAL",
                    format!(
                        "Target branch \"{}\" exists remotely but is not available locally for patching.",
                        request.target_branch
                    ),
                ));
            }
            _ => {
                report.blockers.push(Blocker::new(
                    "TARGET_NOT_FOUND",
                    format!(
                        "Target branch \"{}\" does not exist locally or remotely.",
                        request.target_branch
                    ),
                ));
            }
        }
        return Err(Bail);
    };
    report.start_point = Some(target_tip.clone());

    // `local_direct` commits straight onto `refs/heads/<target>`, so the
    // local ref must exist now: a remote-only tip has nowhere to land.
    if request.delivery == PromotionDelivery::LocalDirect
        && git::local_tip(&options.target_repo_dir, &request.target_branch).is_none()
    {
        report.blockers.push(Blocker::new(
            "TARGET_NOT_LOCAL",
            format!(
                "Target branch \"{}\" has no local ref for a direct commit; check it out (or fetch it) in the local clone first, then re-promote.",
                request.target_branch
            ),
        ));
        return Err(Bail);
    }

    // `worktree` delivery stages into the user's own checkout, so the local
    // ref must exist, it must be the currently checked-out branch (otherwise
    // the files would belong to a different branch), and the tree must be
    // clean (otherwise the promotion would mix with uncommitted work). Fail
    // closed before any branch or worktree is created: nothing is touched.
    if request.delivery == PromotionDelivery::Worktree
        && git::local_tip(&options.target_repo_dir, &request.target_branch).is_none()
    {
        report.blockers.push(Blocker::new(
            "TARGET_NOT_LOCAL",
            format!(
                "Target branch \"{}\" has no local ref to stage onto; check it out (or fetch it) in the local clone first, then re-promote.",
                request.target_branch
            ),
        ));
        return Err(Bail);
    }
    if request.delivery == PromotionDelivery::Worktree {
        match git::current_branch(&options.target_repo_dir) {
            Some(current) if current == request.target_branch => {}
            other => {
                let actual = other.unwrap_or_else(|| "a detached HEAD".to_string());
                report.blockers.push(Blocker::new(
                    "TARGET_NOT_CHECKED_OUT",
                    format!(
                        "Target branch \"{}\" is not checked out in '{}' (currently on {actual}); a worktree promotion writes into the working tree, so it would land on the wrong branch. Switch with `git checkout {}` in '{}', then re-promote. Nothing was changed.",
                        request.target_branch,
                        options.target_repo_dir.display(),
                        request.target_branch,
                        options.target_repo_dir.display(),
                    ),
                ));
                return Err(Bail);
            }
        }
        if !git::status_porcelain(&options.target_repo_dir)
            .trim()
            .is_empty()
        {
            report.blockers.push(Blocker::new(
                "DIRTY_WORKTREE",
                format!(
                    "Your checkout of \"{}\" in '{}' has uncommitted changes; refusing to mix a promotion with them. Commit or stash them first (`git status` in '{}'), then re-promote. Nothing was changed.",
                    request.target_branch,
                    options.target_repo_dir.display(),
                    options.target_repo_dir.display(),
                ),
            ));
            return Err(Bail);
        }
    }

    let duplicates = history::detect_branch_duplicates(
        &options.target_repo_dir,
        &request.target_branch,
        &subsystem.id,
        &request.commits,
        &registry.main_repository,
        deps.github,
    );
    if !duplicates.active_shas.is_empty() {
        report.blockers.push(Blocker::new(
            "DUPLICATE_ACTIVE",
            format!(
                "SHA(s) already integrated on \"{}\": {}.",
                request.target_branch,
                duplicates.active_shas.join(", ")
            ),
        ));
        return Err(Bail);
    }
    if let Some(err) = &duplicates.pr_lookup_error {
        report.blockers.push(Blocker::new(
            "PR_LOOKUP_FAILED",
            format!(
                "Could not list open pull requests for base \"{}\"; refusing to guess whether a promotion is already pending ({err}).",
                request.target_branch
            ),
        ));
        return Err(Bail);
    }
    if !duplicates.open_prs.is_empty() {
        report.blockers.push(Blocker::new(
            "PR_ALREADY_OPEN",
            format!(
                "An open integration PR already targets \"{}\" for these SHA(s).",
                request.target_branch
            ),
        ));
        return Err(Bail);
    }

    let Some(source_url) = (deps.source_url)(subsystem) else {
        report.blockers.push(Blocker::new(
            "SOURCE_URL_UNRESOLVED",
            "No source URL resolver configured; refusing to guess a remote.",
        ));
        return Err(Bail);
    };
    let source_ref = match git::fetch_source_branch(
        &options.target_repo_dir,
        &source_url,
        &subsystem.source_branch,
        &subsystem.id,
    ) {
        Ok(reference) => reference,
        Err(err) => {
            // The token never reaches argv (`http.extraHeader`), but any git
            // error is still passed through `sanitize()` before it can reach
            // a blocker or the UI.
            let clean = git::sanitize(&err.to_string());
            report.blockers.push(Blocker::new(
                "COMMIT_VERIFICATION_FAILED",
                format!(
                    "Could not fetch the source branch \"{}\": {}",
                    subsystem.source_branch, clean
                ),
            ));
            return Err(Bail);
        }
    };

    let verified_plan = match build_plan(&options.target_repo_dir, &source_ref, request, report) {
        Ok(plan) => plan,
        Err(()) => return Err(Bail),
    };

    let mut classified_plan: Vec<PlannedEntry> = Vec::new();
    for planned in &verified_plan {
        let mut entries = Vec::new();
        for delta in &planned.entries {
            let (status, path, reason) = classify_entry(
                &options.target_repo_dir,
                &subsystem.integration_policy,
                &planned.sha,
                planned.base_sha.as_deref(),
                delta,
            );
            entries.push(ClassifiedEntry {
                delta: delta.clone(),
                path,
                status,
                reason,
            });
        }
        classified_plan.push(PlannedEntry {
            sha: planned.sha.clone(),
            base_sha: planned.base_sha.clone(),
            entries,
        });
    }
    report.verification = VerificationSummary {
        mode: request.mode.as_str().to_string(),
        order: request.commits.clone(),
    };

    for planned in &classified_plan {
        for entry in &planned.entries {
            match entry.status {
                PathStatus::Rejected => report.blockers.push(Blocker::new(
                    "UNSAFE_PATH",
                    format!("Rejected unsafe path \"{}\": {}", entry.path, entry.reason),
                )),
                PathStatus::ExplicitReview if !options.allow_shared_config => {
                    report.blockers.push(Blocker::new(
                            "SHARED_CONFIG_REQUIRES_REVIEW",
                            format!("Shared/global config changed: \"{}\". Requires explicit review (enable allowSharedConfig to proceed).", entry.path),
                        ));
                }
                _ => {}
            }
        }
    }
    if !report.blockers.is_empty() {
        return Err(Bail);
    }
    if options.allow_shared_config {
        let shared: Vec<&str> = classified_plan
            .iter()
            .flat_map(|p| p.entries.iter())
            .filter(|e| e.status == PathStatus::ExplicitReview)
            .map(|e| e.path.as_str())
            .collect();
        if !shared.is_empty() {
            report.warnings.push(Blocker::new(
                "SHARED_CONFIG_EXPLICITLY_ALLOWED",
                format!(
                    "Shared config applied under explicit approval: {}",
                    shared.join(", ")
                ),
            ));
        }
    }

    let branch_name = integration_branch_name(
        &subsystem.id,
        &request.commits,
        &request.target_branch,
        &options.nonce,
    );
    if git::local_branch_exists(&options.target_repo_dir, &branch_name) {
        report.blockers.push(Blocker::new(
            "BRANCH_ALREADY_EXISTS",
            format!(
                "Integration branch '{branch_name}' already exists in '{}'.",
                options.target_repo_dir.display()
            ),
        ));
        return Err(Bail);
    }
    let worktree_dir = options.temp_root.join(format!(
        "vos-integration-{}",
        sanitize_nonce(&options.nonce)
    ));
    if let Err(err) = git::worktree_add(
        &options.target_repo_dir,
        &worktree_dir,
        &branch_name,
        &target_tip,
    ) {
        report.blockers.push(Blocker::new(
            "ENGINE_ERROR",
            format!("Could not create the integration worktree: {err}"),
        ));
        return Err(Bail);
    }
    *worktree = Some(worktree_dir.clone());
    report.integration_branch = Some(branch_name.clone());

    let mut mode_ops: Vec<apply::ModeOp> = Vec::new();
    let mut lockfile_touched: Vec<String> = Vec::new();
    let mut pkg_pairs: Vec<(String, Option<String>)> = Vec::new();

    for planned in &classified_plan {
        for entry in &planned.entries {
            match entry.status {
                PathStatus::Apply | PathStatus::ExplicitReview => {
                    let target_tip = report.start_point.clone().unwrap_or_default();
                    let apply_options = apply::ApplyOptions {
                        allowed_add_overwrite: allowed_add_overwrite(
                            &options.target_repo_dir,
                            &target_tip,
                            &planned.sha,
                            planned.base_sha.as_deref(),
                            &entry.delta,
                        ),
                        on_conflict: request.on_conflict,
                    };
                    match apply::apply_file_delta(
                        &options.target_repo_dir,
                        &worktree_dir,
                        &planned.sha,
                        planned.base_sha.as_deref(),
                        &entry.delta,
                        &options.temp_root,
                        &apply_options,
                    ) {
                        Ok(set) => {
                            mode_ops.extend(set.mode_ops);
                            report
                                .changes
                                .conflict_taken
                                .extend(set.taken_from_subsystem);
                            for result in set.results {
                                match result.outcome {
                                    apply::ApplyOutcome::Written => {
                                        if result.reason.as_deref()
                                            == Some(apply::ADD_OVER_EXISTING_REASON)
                                        {
                                            report.changes.updated.push(result.path)
                                        } else {
                                            report.changes.applied.push(result.path)
                                        }
                                    }
                                    apply::ApplyOutcome::Merged => {
                                        report.changes.merged.push(result.path)
                                    }
                                    apply::ApplyOutcome::Deleted => {
                                        report.changes.deleted.push(result.path)
                                    }
                                    apply::ApplyOutcome::Unchanged => {}
                                }
                            }
                        }
                        Err(conflict) => {
                            report.blockers.push(
                                Blocker::new("PATCH_CONFLICT", conflict.message)
                                    .with_details(conflict.kind),
                            );
                            return Err(Bail);
                        }
                    }
                }
                PathStatus::DependencyMerge => {
                    pkg_pairs.push((planned.sha.clone(), planned.base_sha.clone()))
                }
                PathStatus::Regenerate => {
                    let name = entry
                        .path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&entry.path)
                        .to_string();
                    if !lockfile_touched.contains(&name) {
                        lockfile_touched.push(name);
                    }
                }
                PathStatus::MainOwned => report.changes.main_owned_skipped.push(entry.path.clone()),
                PathStatus::SecretExcluded => {
                    report.changes.secret_excluded.push(entry.path.clone())
                }
                PathStatus::Rejected => {}
            }
        }
    }

    report.changes.conflict_taken.sort();
    report.changes.conflict_taken.dedup();
    if !report.changes.conflict_taken.is_empty() {
        report.warnings.push(Blocker::new(
            "conflict_taken_from_subsystem",
            format!(
                "On-conflict policy `take_subsystem` replaced {} path(s) with the subsystem's version: {}. Review the replaced files before merging.",
                report.changes.conflict_taken.len(),
                report.changes.conflict_taken.join(", ")
            ),
        ));
    }

    let manifest_path = worktree_dir.join("package.json");
    let original_manifest_text = std::fs::read_to_string(&manifest_path).ok();
    let mut target_manifest: serde_json::Value = original_manifest_text
        .as_deref()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let mut deps_changed = false;
    let mut delta_names: Vec<String> = Vec::new();
    if subsystem.dependency_strategy == DependencyStrategy::Reconcile && !pkg_pairs.is_empty() {
        let deltas: Vec<deps::DependencyDelta> = pkg_pairs
            .iter()
            .map(|(sha, base)| {
                let new_pkg = read_manifest(&options.target_repo_dir, sha);
                let old_pkg = base
                    .as_deref()
                    .map(|b| read_manifest(&options.target_repo_dir, b))
                    .unwrap_or_else(|| serde_json::json!({}));
                deps::compute_dependency_delta(&old_pkg, &new_pkg)
            })
            .collect();
        let reconciled =
            deps::reconcile_dependencies(&deltas, &target_manifest, options.allow_removals);
        report.dependencies = DependencySummary {
            added: reconciled.added.clone(),
            updated: reconciled.updated.clone(),
            removed: reconciled.removed.clone(),
            moved: reconciled.moved.clone(),
            review: reconciled.review.clone(),
            noop: reconciled.noop.clone(),
        };
        for blocker in &reconciled.blockers {
            report.blockers.push(blocker.clone());
        }
        if !report.blockers.is_empty() {
            return Err(Bail);
        }
        deps_changed = !(reconciled.added.is_empty()
            && reconciled.updated.is_empty()
            && reconciled.removed.is_empty()
            && reconciled.moved.is_empty());
        if deps_changed {
            target_manifest = reconciled.manifest.clone();
            let text = stringify_manifest(&target_manifest, original_manifest_text.as_deref());
            if let Err(err) = std::fs::write(&manifest_path, text) {
                report.blockers.push(Blocker::new(
                    "ENGINE_ERROR",
                    format!("Could not write package.json: {err}"),
                ));
                return Err(Bail);
            }
        }
        delta_names = reconciled.added.iter().map(|a| a.name.clone()).collect();
        delta_names.extend(reconciled.moved.iter().map(|m| m.name.clone()));
    }

    {
        let mut code_files: Vec<deps::CodeFile> = Vec::new();
        for planned in &classified_plan {
            for entry in &planned.entries {
                if entry.status != PathStatus::Apply {
                    continue;
                }
                if !deps::is_code_path(&entry.path) {
                    continue;
                }
                if let Some(bytes) =
                    git::read_blob(&options.target_repo_dir, &planned.sha, &entry.path)
                {
                    code_files.push(deps::CodeFile {
                        path: entry.path.clone(),
                        content: String::from_utf8_lossy(&bytes).to_string(),
                    });
                }
            }
        }
        for missing in deps::check_import_prerequisites(&code_files, &target_manifest, &delta_names)
        {
            report.blockers.push(Blocker::new(
                "MISSING_DEPENDENCY_PREREQUISITE",
                format!(
                    "Selected commit(s) import \"{}\" which is neither in the target manifest nor added by the selected delta. An unselected prerequisite commit likely must be selected explicitly.",
                    missing.name
                ),
            ));
        }
        if !report.blockers.is_empty() {
            return Err(Bail);
        }
    }

    let should_regenerate = deps_changed || !lockfile_touched.is_empty();
    if should_regenerate {
        let outcome = deps.lockfile.generate(&worktree_dir);
        if !outcome.ok && !outcome.skipped {
            report.blockers.push(Blocker::new(
                "LOCKFILE_REGENERATION_FAILED",
                format!(
                    "Lockfile regeneration failed ({}): {}",
                    outcome
                        .manager
                        .clone()
                        .unwrap_or_else(|| "unknown".to_string()),
                    outcome.output.clone().unwrap_or_default()
                ),
            ));
            report.lockfile = Some(outcome);
            return Err(Bail);
        }
        if outcome.skipped {
            report.blockers.push(Blocker::new(
                "LOCKFILE_MANAGER_UNSUPPORTED",
                format!(
                    "The selected delta changes dependencies but the target lockfile could not be regenerated ({}): {}.",
                    outcome.manager.clone().unwrap_or_else(|| "unknown manager".to_string()),
                    outcome.reason.clone().unwrap_or_else(|| "unsupported or absent target lockfile".to_string())
                ),
            ));
            report.lockfile = Some(outcome);
            return Err(Bail);
        }
        report.changes.regenerated.push(
            outcome
                .manager
                .clone()
                .unwrap_or_else(|| "none".to_string()),
        );
        report.lockfile = Some(outcome);
    }

    let regenerated_really = report
        .lockfile
        .as_ref()
        .map(|l| should_regenerate && !l.skipped && !l.stubbed)
        .unwrap_or(false);
    let any_change = !(report.changes.applied.is_empty()
        && report.changes.updated.is_empty()
        && report.changes.merged.is_empty()
        && report.changes.deleted.is_empty()
        && report.changes.conflict_taken.is_empty())
        || deps_changed
        || regenerated_really;
    if !any_change {
        report.blockers.push(Blocker::new(
            "NOOP_PROMOTION",
            "The selected commit delta produced no applicable changes; refusing to record a no-op integration.",
        ));
        return Err(Bail);
    }

    // A promotion carries its own message so a multi-commit promotion is never
    // labelled with only the last source commit's subject.
    let subject = request.message.trim().to_string();
    if subject.is_empty() {
        report.blockers.push(Blocker::new(
            "MESSAGE_REQUIRED",
            "Give the promotion its own commit message; the source commits are recorded as provenance trailers.",
        ));
        return Err(Bail);
    }
    // `worktree` delivery: the delta built above (identical copy application,
    // dependency-delta merge, and lockfile regeneration) is written into the
    // user's own checkout of the target branch and staged — exactly the paths
    // the promotion wrote or deleted, nothing else. No branch is created,
    // nothing is committed (so no provenance trailers are written), nothing
    // is pushed, and no PR is opened. Status is derived from reachable
    // history, so a worktree promotion leaves every commit status exactly as
    // it was; the user reviews with `git diff --staged` and commits
    // themselves. The temporary branch and worktree are always removed.
    if request.delivery == PromotionDelivery::Worktree {
        match stage_into_own_worktree(
            &options.target_repo_dir,
            &worktree_dir,
            &target_tip,
            &request.target_branch,
            report,
            &mode_ops,
        ) {
            Ok(staged) => {
                report.staged_paths = staged;
            }
            Err(blocker) => {
                report.blockers.push(blocker);
                return Err(Bail);
            }
        }

        // The changes are staged in the user's checkout: drop the temporary
        // branch and worktree so no `integration/*` ref is left behind. The
        // worktree goes first (a branch checked out in a linked worktree
        // cannot be deleted), and `*worktree` is cleared so the outer cleanup
        // does not redo it.
        git::worktree_remove(&options.target_repo_dir, &worktree_dir);
        let _ = std::fs::remove_dir_all(&worktree_dir);
        *worktree = None;
        if git::run(
            &options.target_repo_dir,
            &["branch", "--delete", "--force", &branch_name],
        )
        .map(|out| out.status != 0)
        .unwrap_or(true)
        {
            report.warnings.push(Blocker::new(
                "TEMP_BRANCH_LEFTOVER",
                format!(
                    "The promotion was staged on \"{}\" but the temporary branch '{branch_name}' could not be deleted; remove it manually (`git branch -D {branch_name}`).",
                    request.target_branch,
                ),
            ));
        }
        report.integration_branch = None;
        return Ok(());
    }

    let message = provenance::build_provenance_message(&provenance::ProvenanceInput {
        subsystem_id: &subsystem.id,
        source_repo: &subsystem.repository,
        source_commits: &request.commits,
        mode: request.mode.as_str(),
        target: &request.target_branch,
        subject: &subject,
    });
    report.provenance_message = Some(message.clone());
    let message_file = options
        .temp_root
        .join(format!("vos-msg-{}.txt", sanitize_nonce(&options.nonce)));
    match provenance::write_provenance_commit(
        &worktree_dir,
        &message,
        &message_file,
        "vos-integration-bot",
        "vos-integration-bot@users.noreply.github.com",
        &mode_ops,
    ) {
        Ok(sha) => report.commit_sha = Some(sha),
        Err(err) => {
            report.blockers.push(Blocker::new(
                "ENGINE_ERROR",
                format!("Could not create the provenance commit: {err}"),
            ));
            return Err(Bail);
        }
    }

    let commit_sha = report.commit_sha.clone().unwrap_or_default();
    if !git::is_ancestor(&options.target_repo_dir, &target_tip, &commit_sha) {
        report.blockers.push(Blocker::new(
            "INTEGRITY_BRANCH_ANCESTRY",
            "Integration commit is not descended from the target tip; aborting.",
        ));
        return Err(Bail);
    }

    // `local_direct` delivery: the commit built above (identical delta
    // application, provenance trailers, conflict handling, and `on_conflict`
    // policy) lands straight onto the local target branch. Never pushes,
    // never opens a PR. Fail-closed: the target tip is re-checked against
    // the pinned tip, a dirty checkout of the target refuses, and the
    // temporary branch is always removed.
    if request.delivery == PromotionDelivery::LocalDirect {
        let repo = &options.target_repo_dir;
        let refname = format!("refs/heads/{}", request.target_branch);
        match git::local_tip(repo, &request.target_branch) {
            Some(current) if current == target_tip => {}
            Some(current) => {
                report.blockers.push(Blocker::new(
                    "TARGET_MOVED",
                    format!(
                        "Target branch \"{}\" moved while the promotion was being built (expected {}, now {}); nothing was committed to it. Retry the promotion to rebuild against the new tip.",
                        request.target_branch,
                        short(&target_tip),
                        short(&current),
                    ),
                ));
                return Err(Bail);
            }
            None => {
                report.blockers.push(Blocker::new(
                    "TARGET_MOVED",
                    format!(
                        "Target branch \"{}\" lost its local ref while the promotion was being built; nothing was committed to it. Retry the promotion to rebuild against the current state.",
                        request.target_branch,
                    ),
                ));
                return Err(Bail);
            }
        }

        let checked_out =
            git::current_branch(repo).is_some_and(|current| current == request.target_branch);
        if checked_out {
            if !git::status_porcelain(repo).trim().is_empty() {
                report.blockers.push(Blocker::new(
                    "DIRTY_WORKTREE",
                    format!(
                        "Your checkout of \"{}\" has uncommitted changes; refusing to fast-forward it. Commit or stash them first (`git status` in '{}'), then re-promote. Nothing was committed.",
                        request.target_branch,
                        repo.display(),
                    ),
                ));
                return Err(Bail);
            }
            if let Err(err) = git::merge_ff_only(repo, &commit_sha) {
                report.blockers.push(Blocker::new(
                    "TARGET_MOVED",
                    format!(
                        "Could not fast-forward the \"{}\" checkout to the new commit ({}); the target may have moved. Nothing further was changed — retry the promotion.",
                        request.target_branch,
                        git::sanitize(&err.to_string()),
                    ),
                ));
                return Err(Bail);
            }
        } else if let Err(err) = git::update_ref(repo, &refname, &commit_sha, &target_tip) {
            report.blockers.push(Blocker::new(
                "TARGET_MOVED",
                format!(
                    "Target branch \"{}\" moved while the promotion was being built ({}); nothing was committed to it. Retry the promotion to rebuild against the new tip.",
                    request.target_branch,
                    git::sanitize(&err.to_string()),
                ),
            ));
            return Err(Bail);
        }

        // The commit is on the target: drop the temporary branch and worktree
        // so no `integration/*` ref is left behind. The worktree goes first
        // (a branch checked out in a linked worktree cannot be deleted), and
        // `*worktree` is cleared so the outer cleanup does not redo it.
        git::worktree_remove(repo, &worktree_dir);
        let _ = std::fs::remove_dir_all(&worktree_dir);
        *worktree = None;
        if git::run(repo, &["branch", "--delete", "--force", &branch_name])
            .map(|out| out.status != 0)
            .unwrap_or(true)
        {
            report.warnings.push(Blocker::new(
                "TEMP_BRANCH_LEFTOVER",
                format!(
                    "The promotion commit landed on \"{}\" but the temporary branch '{branch_name}' could not be deleted; remove it manually (`git branch -D {branch_name}`).",
                    request.target_branch,
                ),
            ));
        }
        report.integration_branch = None;
        return Ok(());
    }

    // `local` delivery stops here: the branch + provenance commit stay in the
    // target clone (worktree removed, branch kept). No push, no PR — the user
    // pushes explicitly. This guard is delivery-based, not flag-based, so even
    // a caller passing `push: true` cannot push in local mode.
    if request.delivery == PromotionDelivery::Local {
        return Ok(());
    }

    if options.push {
        let mut url = options
            .push_remote_url
            .clone()
            .or_else(|| deps.github.push_target());
        if url.is_none() && deps.github.mode() == github_ops::GitHubMode::Real {
            match options.token.as_deref() {
                Some(token) if !token.is_empty() => {
                    url = Some(github_ops::authenticated_remote_url(
                        &registry.main_repository,
                        token,
                    ));
                }
                _ => {
                    report.blockers.push(Blocker::new(
                        "PUSH_FAILED",
                        "No authenticated push target is available (missing-token); refusing to push to an unauthenticated remote.",
                    ));
                    return Err(Bail);
                }
            }
        }
        match deps
            .github
            .push_branch(&options.target_repo_dir, &branch_name, url.as_deref())
        {
            Ok(outcome) => {
                if !outcome.pushed && outcome.source == "no-push-target" {
                    report.blockers.push(Blocker::new(
                        "PUSH_FAILED",
                        "No authenticated push target is available; refusing to push to an unauthenticated remote.",
                    ));
                    return Err(Bail);
                }
                report.push = Some(outcome);
            }
            Err(err) => {
                report.blockers.push(Blocker::new("PUSH_FAILED", err));
                return Err(Bail);
            }
        }
    }

    if options.create_pr {
        let body = build_pr_body(subsystem, request, report, &branch_name);
        let pr_request = CreatePrRequest {
            head: branch_name.clone(),
            base: request.target_branch.clone(),
            title: format!("integrate({}): {}", subsystem.id, subject),
            body,
            draft: false,
        };
        match deps
            .github
            .create_pull_request(&registry.main_repository, &pr_request)
        {
            Ok(pr) => report.pull_request = Some(pr),
            Err(err) => {
                report
                    .blockers
                    .push(Blocker::new("PR_CREATION_FAILED", err));
                return Err(Bail);
            }
        }
    }

    Ok(())
}

fn sanitize_nonce(nonce: &str) -> String {
    nonce
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Prefix shared by branch naming and duplicate detection.
pub fn integration_branch_prefix(subsystem_id: &str, commits: &[String]) -> String {
    let first = commits.first().cloned().unwrap_or_default();
    format!("integration/{}-{}", subsystem_id, short(&first))
}

pub fn integration_branch_name(
    subsystem_id: &str,
    commits: &[String],
    target_branch: &str,
    nonce: &str,
) -> String {
    let safe_target: String = target_branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!(
        "{}-{}-{}",
        integration_branch_prefix(subsystem_id, commits),
        safe_target,
        sanitize_nonce(nonce)
    )
}

/// Recency guard for adds-over-existing (subsystem split history).
///
/// The subsystem repos were split from the main system, so files that exist
/// in BOTH appear as adds in the subsystem's fresh history. An add may
/// overwrite the target copy only when that copy is NOT newer than the
/// source commit: allowed when the target branch never carried the path
/// (`target_epoch.is_none()`) or the source commit is at least as new
/// (`source_epoch >= target_epoch`). Computed per add-path; `apply`
/// enforces the resulting set. Modified (`M`) paths are untouched and keep
/// the three-way merge.
fn allowed_add_overwrite(
    objects_repo: &Path,
    target_tip: &str,
    source_sha: &str,
    base_sha: Option<&str>,
    delta: &git::DeltaEntry,
) -> HashSet<String> {
    let mut paths: Vec<&str> = Vec::new();
    if delta.status == 'D' {
        if let Some(path) = delta.path.as_deref() {
            paths.push(path);
        }
    } else if delta.is_rename_or_copy() {
        if let Some(old) = delta.old_path.as_deref() {
            paths.push(old);
        }
        if let Some(new) = delta.new_path.as_deref() {
            paths.push(new);
        }
    } else if let Some(path) = delta.path.as_deref() {
        paths.push(path);
    }
    let source_epoch = git::commit_epoch(objects_repo, source_sha);
    let mut allowed = HashSet::new();
    for rel_path in paths {
        // Only adds (absent from the source baseline, present in the source
        // commit) can take the add-over-existing path in `apply`.
        let base_blob = base_sha.and_then(|sha| git::read_blob(objects_repo, sha, rel_path));
        if base_blob.is_some() {
            continue;
        }
        if git::read_blob(objects_repo, source_sha, rel_path).is_none() {
            continue;
        }
        let target_epoch = git::last_change_epoch(objects_repo, target_tip, rel_path);
        let overwrite_ok = match target_epoch {
            None => true,
            Some(target) => source_epoch.map(|source| source >= target).unwrap_or(false),
        };
        if overwrite_ok {
            allowed.insert(rel_path.to_string());
        }
    }
    allowed
}

fn resolve_local_ref(repo_dir: &Path, branch: &str) -> Option<String> {
    for candidate in [
        format!("refs/heads/{branch}"),
        format!("refs/remotes/origin/{branch}"),
        branch.to_string(),
    ] {
        if let Some(sha) = git::rev_parse(repo_dir, &candidate) {
            return Some(sha);
        }
    }
    None
}

/// Manifest/lockfile paths the engine may rewrite outside the copied delta
/// (dependency-delta merge, lockfile regeneration). Compared byte-for-byte
/// between the built worktree and the pinned target tip so a `worktree`
/// promotion stages a regenerated lockfile or rewritten manifest exactly
/// when it changed — a half-applied manifest would be worse than no
/// promotion at all.
const WORKTREE_DRIFT_PATHS: [&str; 4] = [
    "package.json",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
];

/// Copy the built delta from the temporary worktree into the user's own
/// checkout of the target branch and stage exactly the written/deleted paths
/// (`git add` / `git rm`). The preconditions (checked-out target, clean
/// tree) are re-checked here because the tree may have moved while the
/// promotion was being built. Returns the staged paths, sorted. Any failure
/// rolls the working tree back to the pinned tip and reports a blocker:
/// nothing is committed on any path.
fn stage_into_own_worktree(
    repo: &Path,
    worktree_dir: &Path,
    target_tip: &str,
    target_branch: &str,
    report: &PromotionReport,
    mode_ops: &[apply::ModeOp],
) -> Result<Vec<String>, Blocker> {
    match git::current_branch(repo) {
        Some(current) if current == target_branch => {}
        other => {
            let actual = other.unwrap_or_else(|| "a detached HEAD".to_string());
            return Err(Blocker::new(
                "TARGET_NOT_CHECKED_OUT",
                format!(
                    "Target branch \"{target_branch}\" stopped being the checked-out branch of '{}' while the promotion was being built (now on {actual}); nothing was staged. Switch back with `git checkout {target_branch}`, then re-promote.",
                    repo.display(),
                ),
            ));
        }
    }
    if !git::status_porcelain(repo).trim().is_empty() {
        return Err(Blocker::new(
            "DIRTY_WORKTREE",
            format!(
                "Your checkout of \"{target_branch}\" in '{}' gained uncommitted changes while the promotion was being built; refusing to mix a promotion with them. Commit or stash them first (`git status` in '{}'), then re-promote. Nothing was staged.",
                repo.display(),
                repo.display(),
            ),
        ));
    }

    let mut written: HashSet<String> = HashSet::new();
    written.extend(report.changes.applied.iter().cloned());
    written.extend(report.changes.updated.iter().cloned());
    written.extend(report.changes.merged.iter().cloned());
    written.extend(report.changes.conflict_taken.iter().cloned());
    let mut deleted: HashSet<String> = report.changes.deleted.iter().cloned().collect();
    for rel in WORKTREE_DRIFT_PATHS {
        let built = std::fs::read(worktree_dir.join(rel)).ok();
        let pinned = git::read_blob(repo, target_tip, rel);
        match (built, pinned) {
            (Some(bytes), Some(tip)) if bytes != tip => {
                written.insert(rel.to_string());
            }
            (Some(_), None) => {
                written.insert(rel.to_string());
            }
            (None, Some(_)) => {
                deleted.insert(rel.to_string());
            }
            _ => {}
        }
    }

    for rel in written.iter() {
        if let Err(err) = write_repo_file(repo, worktree_dir, rel) {
            rollback_worktree_stage(repo, target_tip, &written, &deleted);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not write \"{rel}\" into '{}': {err}. The promotion was aborted and the working tree was rolled back; nothing was staged and nothing was committed.",
                    repo.display(),
                ),
            ));
        }
    }
    for rel in deleted.iter() {
        let abs = repo.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if abs.exists() && std::fs::remove_file(&abs).is_err() {
            rollback_worktree_stage(repo, target_tip, &written, &deleted);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not delete \"{rel}\" from '{}'. The promotion was aborted and the working tree was rolled back; nothing was staged and nothing was committed.",
                    repo.display(),
                ),
            ));
        }
    }

    // One `git add` for every written path (plain, never `-f`: a path the
    // target ignores stays unstaged exactly as the commit flow's `add -A`
    // would leave it) and one `git rm` for every deletion.
    if !written.is_empty() {
        let mut sorted: Vec<&str> = written.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut args: Vec<&str> = vec!["add", "--"];
        args.extend(sorted);
        if let Err(err) = git::run_ok(repo, &args) {
            rollback_worktree_stage(repo, target_tip, &written, &deleted);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not stage the promoted files in '{}' ({}); the promotion was aborted and the working tree was rolled back. Nothing was committed.",
                    repo.display(),
                    git::sanitize(&err.to_string()),
                ),
            ));
        }
    }
    if !deleted.is_empty() {
        let mut sorted: Vec<&str> = deleted.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        let mut args: Vec<&str> = vec!["rm", "--"];
        args.extend(sorted);
        if let Err(err) = git::run_ok(repo, &args) {
            rollback_worktree_stage(repo, target_tip, &written, &deleted);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not stage the promoted deletions in '{}' ({}); the promotion was aborted and the working tree was rolled back. Nothing was committed.",
                    repo.display(),
                    git::sanitize(&err.to_string()),
                ),
            ));
        }
    }
    for op in mode_ops {
        if written.contains(&op.path) {
            git::update_index_chmod(repo, &op.path, op.executable);
        }
    }

    // The stage must contain exactly the promotion's paths: anything else
    // means something raced the promotion into the index.
    let expected: HashSet<String> = written.union(&deleted).cloned().collect();
    let staged = match git::run(repo, &["diff", "--cached", "--name-only", "-z"]) {
        Ok(out) if out.status == 0 => out
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect::<HashSet<_>>(),
        _ => {
            rollback_worktree_stage(repo, target_tip, &written, &deleted);
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not inspect the index in '{}'; the promotion was aborted and the working tree was rolled back. Nothing was committed.",
                    repo.display(),
                ),
            ));
        }
    };
    if staged != expected {
        rollback_worktree_stage(repo, target_tip, &written, &deleted);
        return Err(Blocker::new(
            "ENGINE_ERROR",
            format!(
                "The index in '{}' does not match the promoted paths after staging; the promotion was aborted and the working tree was rolled back. Nothing was committed.",
                repo.display(),
            ),
        ));
    }

    let mut staged_paths: Vec<String> = expected.into_iter().collect();
    staged_paths.sort();
    Ok(staged_paths)
}

fn write_repo_file(repo: &Path, worktree_dir: &Path, rel: &str) -> Result<(), String> {
    apply::assert_real_path_within(repo, rel).map_err(|err| err.to_string())?;
    let from = worktree_dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    let bytes =
        std::fs::read(&from).map_err(|err| format!("could not read the built file: {err}"))?;
    let abs = repo.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("could not create {}: {err}", parent.display()))?;
    }
    std::fs::write(&abs, &bytes)
        .map_err(|err| format!("could not write {}: {err}", abs.display()))?;
    Ok(())
}

/// Best-effort rollback for a failed `worktree` stage: unstage everything
/// and restore every touched path from the pinned target tip (or delete it
/// when the tip never carried it). The tree was verified clean before the
/// stage, so every change present is the promotion's own.
fn rollback_worktree_stage(
    repo: &Path,
    target_tip: &str,
    written: &HashSet<String>,
    deleted: &HashSet<String>,
) {
    let mut paths: Vec<&str> = written
        .iter()
        .chain(deleted.iter())
        .map(String::as_str)
        .collect();
    paths.sort_unstable();
    if !paths.is_empty() {
        let mut args: Vec<&str> = vec!["reset", "-q", "HEAD", "--"];
        args.extend(paths.iter().copied());
        let _ = git::run(repo, &args);
    }
    for rel in written.iter().chain(deleted.iter()) {
        match git::read_blob(repo, target_tip, rel) {
            Some(bytes) => {
                let abs = repo.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                if let Some(parent) = abs.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&abs, &bytes);
            }
            None => {
                let abs = repo.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
                let _ = std::fs::remove_file(&abs);
            }
        }
    }
}

fn read_manifest(repo_dir: &Path, sha: &str) -> serde_json::Value {
    git::read_blob(repo_dir, sha, "package.json")
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn stringify_manifest(manifest: &serde_json::Value, original_text: Option<&str>) -> String {
    let indent = original_text
        .and_then(|text| {
            let idx = text.find('\n')?;
            let rest = &text[idx + 1..];
            if !rest.starts_with('"') {
                let ws: String = rest
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                if !ws.is_empty() {
                    return Some(ws);
                }
            }
            None
        })
        .unwrap_or_else(|| "  ".to_string());
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut buf, formatter);
    use serde::Serialize;
    if manifest.serialize(&mut serializer).is_ok() {
        let mut text = String::from_utf8_lossy(&buf).to_string();
        text.push('\n');
        text
    } else {
        format!("{manifest}\n")
    }
}

fn classify_entry(
    objects_repo: &Path,
    policy: &crate::services::registry::IntegrationPolicy,
    sha: &str,
    base_sha: Option<&str>,
    entry: &git::DeltaEntry,
) -> (PathStatus, String, String) {
    if entry.status == 'D' {
        let path = entry.path.clone().unwrap_or_default();
        let mode = base_sha.and_then(|base| git::path_mode(objects_repo, base, &path));
        let classification = classify::classify_path(&path, policy, mode.as_deref());
        let mut status = classification.status;
        let mut reason = classification.reason;
        if status == PathStatus::DependencyMerge {
            status = PathStatus::Rejected;
            reason = "deleting the root manifest is never automatic".to_string();
        }
        return (status, classification.path, reason);
    }
    if entry.is_rename_or_copy() {
        let old_path = entry.old_path.clone().unwrap_or_default();
        let new_path = entry.new_path.clone().unwrap_or_default();
        let mode_from = base_sha.and_then(|base| git::path_mode(objects_repo, base, &old_path));
        let mode_to = git::path_mode(objects_repo, sha, &new_path);
        let from = classify::classify_path(&old_path, policy, mode_from.as_deref());
        let to = classify::classify_path(&new_path, policy, mode_to.as_deref());
        let status = if from.status == PathStatus::Rejected || to.status == PathStatus::Rejected {
            PathStatus::Rejected
        } else {
            to.status
        };
        let reason = format!("{}: {}; {}: {}", from.path, from.reason, to.path, to.reason);
        return (status, to.path, reason);
    }
    let path = entry.path.clone().unwrap_or_default();
    let mode = git::path_mode(objects_repo, sha, &path);
    let classification = classify::classify_path(&path, policy, mode.as_deref());
    (
        classification.status,
        classification.path,
        classification.reason,
    )
}

fn build_plan(
    repo_dir: &Path,
    source_ref: &str,
    request: &EngineRequest,
    report: &mut PromotionReport,
) -> Result<Vec<VerifiedEntry>, ()> {
    let source_tip = match git::rev_parse(repo_dir, source_ref) {
        Some(tip) => tip,
        None => {
            report.blockers.push(Blocker::new(
                "COMMIT_VERIFICATION_FAILED",
                format!("Source branch ref not found: {source_ref}"),
            ));
            return Err(());
        }
    };
    let fail = |report: &mut PromotionReport, message: String| {
        report
            .blockers
            .push(Blocker::new("COMMIT_VERIFICATION_FAILED", message));
    };

    if request.mode == PromotionMode::Cumulative {
        if request.commits.len() != 1 {
            fail(
                report,
                "cumulative mode requires exactly one endpoint SHA in commits".to_string(),
            );
            return Err(());
        }
        let endpoint = request.commits[0].clone();
        let Some(base_sha) = request.base_sha.clone() else {
            fail(report, "cumulative mode requires base_sha".to_string());
            return Err(());
        };
        if let Err(err) = assert_full_sha(&base_sha, "base_sha") {
            fail(report, err.message);
            return Err(());
        }
        if git::rev_parse(repo_dir, &endpoint).is_none() {
            fail(
                report,
                format!("endpoint SHA not found in fetched history: {endpoint}"),
            );
            return Err(());
        }
        if !git::is_ancestor(repo_dir, &endpoint, &source_tip) {
            fail(
                report,
                format!("endpoint {endpoint} is not reachable from source branch {source_ref}"),
            );
            return Err(());
        }
        if !git::is_ancestor(repo_dir, &base_sha, &source_tip) {
            fail(
                report,
                format!("base_sha {base_sha} is not reachable from source branch {source_ref}"),
            );
            return Err(());
        }
        if !git::is_ancestor(repo_dir, &base_sha, &endpoint) {
            fail(report, format!("base_sha {base_sha} is not an ancestor of {endpoint}; cumulative range is invalid"));
            return Err(());
        }
        let entries = match git::name_status_range(repo_dir, &base_sha, &endpoint) {
            Ok(entries) => entries,
            Err(err) => {
                fail(report, err.to_string());
                return Err(());
            }
        };
        return Ok(vec![VerifiedEntry {
            sha: endpoint,
            base_sha: Some(base_sha),
            entries,
        }]);
    }

    for sha in &request.commits {
        if git::rev_parse(repo_dir, sha).is_none() {
            fail(report, format!("SHA not found in fetched history: {sha}"));
            return Err(());
        }
        if !git::is_ancestor(repo_dir, sha, &source_tip) {
            fail(
                report,
                format!("SHA {sha} is not reachable from the configured protected source branch ({source_ref})"),
            );
            return Err(());
        }
    }
    // The selection order decides nothing: a copy takes each path from the
    // newest SELECTED commit that touches it, which is an ancestry question, not
    // an input-order one. Sort ancestor-first instead of refusing a selection
    // the user happened to make out of order.
    let mut ordered: Vec<String> = Vec::with_capacity(request.commits.len());
    for sha in &request.commits {
        if ordered.iter().any(|placed| placed == sha) {
            continue;
        }
        let at = ordered
            .iter()
            .position(|placed| git::is_ancestor(repo_dir, sha, placed))
            .unwrap_or(ordered.len());
        ordered.insert(at, sha.clone());
    }

    let mut plan = Vec::new();
    for sha in &ordered {
        let (parent, entries) = match git::name_status_delta(repo_dir, sha) {
            Ok(value) => value,
            Err(err) => {
                fail(report, err.to_string());
                return Err(());
            }
        };
        plan.push(VerifiedEntry {
            sha: sha.clone(),
            base_sha: parent,
            entries,
        });
    }
    Ok(plan)
}

fn build_pr_body(
    subsystem: &SubsystemEntry,
    request: &EngineRequest,
    report: &PromotionReport,
    branch_name: &str,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("## Subsystem promotion: `{}`", subsystem.id));
    lines.push(String::new());
    lines.push(format!(
        "Integration branch `{branch_name}` → base **`{}`** (PR base equals the requested target branch).",
        request.target_branch
    ));
    lines.push(String::new());
    lines.push(format!(
        "**Mode:** {} · **Source repo:** {} · **Source branch:** {}",
        request.mode.as_str(),
        subsystem.repository,
        subsystem.source_branch
    ));
    lines.push(String::new());
    lines.push("### Source commits (in applied order)".to_string());
    for sha in &request.commits {
        lines.push(format!("- `{sha}`"));
    }
    lines.push(String::new());
    // Machine-readable metadata block read back by `integration-history` to match an open PR
    // against EACH requested source SHA.
    lines.push(format!("Subsystem-ID: {}", subsystem.id));
    lines.push(format!("Source-Repo: {}", subsystem.repository));
    for sha in &request.commits {
        lines.push(format!("Source-Commit: {sha}"));
    }
    lines.push(format!("Promotion-Target: {}", request.target_branch));
    lines.push(String::new());

    if !report.dependencies.added.is_empty()
        || !report.dependencies.updated.is_empty()
        || !report.dependencies.removed.is_empty()
        || !report.dependencies.moved.is_empty()
    {
        lines.push("### Dependency changes".to_string());
        for d in &report.dependencies.added {
            lines.push(format!("- added `{}` {}", d.name, d.range));
        }
        for d in &report.dependencies.updated {
            lines.push(format!("- updated `{}`: {} → {}", d.name, d.from, d.to));
        }
        for d in &report.dependencies.moved {
            lines.push(format!(
                "- moved `{}`: {} → {} ({})",
                d.name, d.from_section, d.to_section, d.range
            ));
        }
        for d in &report.dependencies.removed {
            lines.push(format!("- removed `{}` (was {})", d.name, d.previous_range));
        }
        lines.push(String::new());
    }
    if !report.changes.main_owned_skipped.is_empty() {
        lines.push("### Not imported (main-owned)".to_string());
        for path in &report.changes.main_owned_skipped {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    if !report.changes.conflict_taken.is_empty() {
        lines.push("### Resolved via on-conflict policy (subsystem version wins)".to_string());
        for path in &report.changes.conflict_taken {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    if !report.changes.secret_excluded.is_empty() {
        lines.push("### Not imported (secret-looking)".to_string());
        for path in &report.changes.secret_excluded {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    let _ = (report, branch_name);
    lines.push(
        "> This PR only integrates the explicitly selected source commits. Unselected earlier/later commits are never imported automatically."
            .to_string(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_naming_is_deterministic_and_safe() {
        let shas = vec!["a".repeat(40)];
        assert_eq!(
            integration_branch_prefix("hr-management", &shas),
            format!("integration/hr-management-{}", "a".repeat(12))
        );
        let name = integration_branch_name("hr-management", &shas, "release/1.0", "n once");
        assert_eq!(
            name,
            format!(
                "integration/hr-management-{}-release-1.0-n-once",
                "a".repeat(12)
            )
        );
    }
}

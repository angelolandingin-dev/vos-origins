//! App-driven promotion engine (`ARCHITECTURE.md` §4).
//! Fail closed; never pushes to a protected branch, never adds files
//! beyond the integration PR.

pub mod apply;
pub mod blobset;
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

use crate::github::gh_cli::GitHubAccount;
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
    pub delivery: PromotionDelivery,
    /// Wire `onConflict` (`fail` | `take_subsystem`). Retained for wire
    /// compatibility only: promotion is a copy and the subsystem's copy
    /// always wins, so a diverged target copy is overwritten (and reported
    /// as replaced) under either value. Never bypasses path-safety,
    /// classification, dependency, lockfile, or NOOP guards.
    pub on_conflict: PromotionOnConflict,
    /// The promotion commit's own message (required; an empty one is refused so
    /// a multi-commit promotion is never labelled with only the last subject).
    pub message: String,
    pub excluded_paths: Vec<String>,
}

impl EngineRequest {
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
            excluded_paths: Vec::new(),
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
    pub account: fn() -> Option<GitHubAccount>,
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
    pub conflict_taken: Vec<String>,
    /// Copy-semantics reporting (§5): paths whose target copy was overwritten
    /// (`updated` + `conflict_taken` mirror, kept as the canonical list).
    pub replaced: Vec<String>,
    pub unchanged: Vec<String>,
    pub flagged: Vec<String>,
    pub excluded: Vec<String>,
    pub ignored_skipped: Vec<String>,
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

/// Per-checkout lock serializing concurrent local runs over
/// `refs/vos-sources/*` (addendum D). The fetch writes that ref namespace
/// into the checkout; two concurrent runs on the same `target_repo_dir`
/// would race over it. One process-wide mutex per canonical checkout dir
/// narrows (but see the post-commit PR re-check comment: concurrent runs
/// still race over the remote — that window is narrowed, not closed).
fn vos_sources_lock_for(dir: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let map = LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let key = dir
        .canonicalize()
        .unwrap_or_else(|_| dir.to_path_buf())
        .to_string_lossy()
        .to_string();
    let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
    guard.get(&key).cloned().unwrap_or_else(|| {
        let slot = Arc::new(Mutex::new(()));
        guard.insert(key, slot.clone());
        slot
    })
}

/// Best-effort deletion of the tool's `refs/vos-sources/*` namespace from the
/// checkout after the run. The engine fetches the source branch into that
/// namespace inside `VOS_TARGET_REPO_DIR` — which is the user's personal
/// clone — so leaving it behind pollutes their ref list. Failures are silent:
/// a leftover ref is untracked history, never a correctness issue.
fn delete_vos_sources_refs(repo_dir: &Path) {
    let Ok(out) = git::run(
        repo_dir,
        &["for-each-ref", "--format=%(refname)", "refs/vos-sources/"],
    ) else {
        return;
    };
    if out.status != 0 {
        return;
    }
    for line in out.stdout_str().lines() {
        let name = line.trim();
        if name.is_empty() || !name.starts_with("refs/vos-sources/") {
            continue;
        }
        let _ = git::run(repo_dir, &["update-ref", "-d", name]);
    }
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

/// `git worktree list --porcelain` check: is `branch` checked out in a linked
/// worktree whose path differs from `repo_dir`? `local_direct` refuses the
/// `update-ref` fast path in that case — moving the ref from here would
/// desync that worktree.
fn is_branch_checked_out_elsewhere(repo_dir: &Path, branch: &str) -> bool {
    let Ok(out) = git::run(repo_dir, &["worktree", "list", "--porcelain"]) else {
        return false;
    };
    if out.status != 0 {
        return false;
    }
    let wanted = format!("refs/heads/{branch}");
    let mut current_path: Option<String> = None;
    let repo_canon = repo_dir
        .canonicalize()
        .unwrap_or_else(|_| repo_dir.to_path_buf())
        .to_string_lossy()
        .to_string();
    for line in out.stdout_str().lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = Some(p.trim().to_string());
        } else if line.trim() == format!("branch {wanted}") {
            if let Some(p) = current_path.as_deref() {
                let this_canon = std::path::PathBuf::from(p)
                    .canonicalize()
                    .map(|c| c.to_string_lossy().to_string())
                    .unwrap_or_else(|_| p.to_string());
                // Same checkout (or case/sep differences on Windows) is fine;
                // a *different* linked worktree refuses.
                if this_canon.to_lowercase() != repo_canon.to_lowercase() {
                    return true;
                }
            }
        }
    }
    false
}

/// Expected/policy failures become blockers, never panics.
pub fn run_promotion(
    registry: &Registry,
    request: &EngineRequest,
    options: &EngineOptions,
    deps: &EngineDeps<'_>,
) -> PromotionReport {
    // Pin the checkout for the whole run: concurrent local runs on the same
    // repo race over `refs/vos-sources/*` (fetch writes that namespace into
    // the checkout). The per-checkout mutex serializes runs in this process.
    // Cross-process runs can still race — that window is documented, not
    // claimed handled (addendum D).
    let checkout_lock = vos_sources_lock_for(&options.target_repo_dir);
    let _checkout_guard = checkout_lock.lock().unwrap_or_else(|e| e.into_inner());
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

    // Housekeeping (§10): the fetch wrote `refs/vos-sources/*` into the
    // user's own clone (`VOS_TARGET_REPO_DIR`). Delete the tool's namespace
    // after the run — success or failure — so their ref list stays clean.
    delete_vos_sources_refs(&options.target_repo_dir);

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

    // `registry` (and thus `subsystem`, its `integration_policy` and
    // `global_files`) was read ONCE at the start of this run and this
    // snapshot is used for both the preflight-equivalent checks below
    // (overwrite / global) and the apply that follows. The file is never
    // re-read mid-run, so the two cannot disagree because the file changed
    // mid-run. The standalone preflight command likewise loads the registry
    // once per call.
    let excluded_set: std::collections::HashSet<String> = request
        .excluded_paths
        .iter()
        .map(|p| p.replace('\\', "/"))
        .collect();
    {
        let mut excluded: Vec<String> = excluded_set.iter().cloned().collect();
        excluded.sort();
        report.changes.excluded = excluded;
    }

    let apply_paths: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        for planned in &classified_plan {
            for entry in &planned.entries {
                if !matches!(entry.status, PathStatus::Apply | PathStatus::ExplicitReview) {
                    continue;
                }
                if excluded_set.contains(&entry.path) {
                    continue;
                }
                if !v.contains(&entry.path) {
                    v.push(entry.path.clone());
                }
            }
        }
        v.sort();
        v
    };

    let ordered_selection: Vec<(String, Option<String>)> = verified_plan
        .iter()
        .map(|v| (v.sha.clone(), v.base_sha.clone()))
        .collect();
    let ordered_ref: Vec<(String, Option<String>)> = ordered_selection.clone();

    // ONE ls-tree per revision for ALL paths (optimisation note §3).
    let blob_sets = crate::services::engine::blobset::collect_blob_sets(
        &options.target_repo_dir,
        &ordered_ref,
        &apply_paths,
    );
    let target_tree =
        crate::services::engine::blobset::tree_blob_map(&options.target_repo_dir, &target_tip);
    let oldest_base = ordered_selection.first().and_then(|(_, b)| b.clone());
    let oldest_tree: std::collections::HashMap<String, String> = match oldest_base.as_deref() {
        Some(base) => {
            crate::services::engine::blobset::tree_blob_map(&options.target_repo_dir, base)
        }
        None => std::collections::HashMap::new(),
    };
    let history_tips: Vec<String> = ordered_selection
        .iter()
        .map(|(sha, _)| sha.clone())
        .collect();
    let overwrite_ok = crate::services::engine::blobset::overwrite_allowed(
        &options.target_repo_dir,
        &blob_sets,
        &target_tree,
        &apply_paths,
        &history_tips,
    );
    let mut conflict_takeover: Vec<String> = Vec::new();

    {
        let global_files = subsystem.global_files.clone().unwrap_or_default();
        if !global_files.is_empty() {
            let mut violations: Vec<String> = Vec::new();
            for path in &apply_paths {
                if !is_global_match(path, &global_files) {
                    continue;
                }
                if !overwrite_ok.get(path).copied().unwrap_or(false) {
                    violations.push(path.clone());
                }
            }
            violations.sort();
            if !violations.is_empty() {
                report.blockers.push(Blocker::new(
                    "GLOBAL_FILE_PROTECTED",
                    format!(
                        "Protected app-shell file(s) would be overwritten: {}. The target copy has diverged from every source version in this selection. Remedy: adapt {} in the subsystem first, then re-promote (or exclude it via excludedPaths if the shell copy is canonical).",
                        violations.join(", "),
                        if violations.len() == 1 {
                            format!("'{}'", violations[0])
                        } else {
                            "these paths".to_string()
                        }
                    ),
                ));
                return Err(Bail);
            }
        }
    }

    {
        for path in &apply_paths {
            let global_files = subsystem.global_files.clone().unwrap_or_default();
            if is_global_match(path, &global_files) {
                continue;
            }
            if overwrite_ok.get(path).copied().unwrap_or(false) {
                continue;
            }
            if !conflict_takeover.contains(path) {
                conflict_takeover.push(path.clone());
            }
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
                    // Exclusion (§4): skipped here, reported as excluded.
                    if excluded_set.contains(&entry.path) {
                        continue;
                    }
                    // Blob membership (§3) already decided correctness above;
                    // dates never gate the write (addendum E3). The recency
                    // guard is intentionally dead: pass an empty set.
                    let apply_options = apply::ApplyOptions {
                        allowed_add_overwrite: std::collections::HashSet::new(),
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
                                            report.changes.updated.push(result.path.clone());
                                            report.changes.replaced.push(result.path);
                                        } else {
                                            report.changes.applied.push(result.path.clone());
                                        }
                                    }
                                    apply::ApplyOutcome::Merged => {
                                        report.changes.merged.push(result.path)
                                    }
                                    apply::ApplyOutcome::Deleted => {
                                        report.changes.deleted.push(result.path)
                                    }
                                    apply::ApplyOutcome::Unchanged => {
                                        report.changes.unchanged.push(result.path)
                                    }
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

    conflict_takeover.sort();
    report
        .changes
        .conflict_taken
        .extend(conflict_takeover.iter().cloned());
    report.changes.conflict_taken.sort();
    report.changes.conflict_taken.dedup();
    if !report.changes.conflict_taken.is_empty() {
        report.warnings.push(Blocker::new(
            "conflict_taken_from_subsystem",
            format!(
                "Subsystem copy replaced {} diverged path(s) with the subsystem's version (listed under Replaced). Review the replaced files before merging.",
                report.changes.conflict_taken.len(),
            ),
        ));
    }

    {
        report.changes.applied.sort();
        report.changes.applied.dedup();
        report.changes.updated.sort();
        report.changes.updated.dedup();
        report.changes.deleted.sort();
        report.changes.deleted.dedup();
        report.changes.unchanged.sort();
        report.changes.unchanged.dedup();
        report.changes.replaced.sort();
        report.changes.replaced.dedup();
        for p in report.changes.updated.iter() {
            if !report.changes.replaced.contains(p) {
                report.changes.replaced.push(p.clone());
            }
        }
        for p in conflict_takeover.iter() {
            if !report.changes.replaced.contains(p) {
                report.changes.replaced.push(p.clone());
            }
        }
        report.changes.replaced.sort();
        let mut flagged: Vec<String> = Vec::new();
        for p in report.changes.replaced.iter() {
            let target_blob = target_tree.get(p).cloned();
            let base_blob = oldest_tree.get(p).cloned();
            if target_blob != base_blob {
                flagged.push(p.clone());
            }
        }
        flagged.sort();
        flagged.dedup();
        report.changes.flagged = flagged;
        report.changes.excluded.sort();
        report.changes.excluded.dedup();
        if !report.changes.flagged.is_empty() {
            report.warnings.push(Blocker::new(
                "flagged_overlap",
                format!(
                    "Cross-subsystem overlap: {} replaced path(s) whose target copy had independently diverged from the commit's base (listed under Replaced). Review before merging.",
                    report.changes.flagged.len(),
                ),
            ));
        }
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
        && report.changes.conflict_taken.is_empty()
        && report.changes.replaced.is_empty())
        || deps_changed
        || regenerated_really;
    if !any_change {
        report.blockers.push(Blocker::new(
            "NOOP_PROMOTION",
            "The selected commit delta produced no applicable changes; refusing to record a no-op integration.",
        ));
        return Err(Bail);
    }

    let subject = request.message.trim().to_string();
    if subject.is_empty() {
        report.blockers.push(Blocker::new(
            "MESSAGE_REQUIRED",
            "Give the promotion its own commit message; the source commits are recorded as provenance trailers.",
        ));
        return Err(Bail);
    }
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

    let mut message = provenance::build_provenance_message(&provenance::ProvenanceInput {
        subsystem_id: &subsystem.id,
        source_repo: &subsystem.repository,
        source_commits: &request.commits,
        mode: request.mode.as_str(),
        target: &request.target_branch,
        subject: &subject,
    });
    if !conflict_takeover.is_empty() {
        message.push_str(&format!(
            "Promotion-Replaced: {}\n",
            conflict_takeover.join(", ")
        ));
    }
    report.provenance_message = Some(message.clone());
    let message_file = options
        .temp_root
        .join(format!("vos-msg-{}.txt", sanitize_nonce(&options.nonce)));
    let attribute_to_account = registry.promotion.attribute_to_account;
    let account = if attribute_to_account {
        (deps.account)()
    } else {
        None
    };
    if attribute_to_account && account.is_none() {
        report.warnings.push(Blocker::new(
            "ACCOUNT_ATTRIBUTION_UNAVAILABLE",
            "Could not resolve the authenticated GitHub account, so the promotion commit was authored by the bot identity instead. Run `gh auth login --hostname github.com` (or uncheck \"Attribute promotions to my GitHub account\") to stop this warning.",
        ));
    }
    let author = provenance::commit_author(attribute_to_account, account.as_ref());
    match provenance::write_provenance_commit(
        &worktree_dir,
        &message,
        &message_file,
        &author.name,
        &author.email,
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

    // Post-commit duplicate re-check (§10): the early guard ran before the
    // build; two concurrent runs can both pass it, then both commit. Re-check
    // open PRs (and active SHAs) after the commit and before any push or
    // ref move, so the second committer fails closed instead of pushing a
    // duplicate. BEST-EFFORT: it narrows the window between two concurrent
    // runs, it does not close it — a true close needs cross-machine locking,
    // which is explicitly out of scope. A push/merge that lands between this
    // check and the push below can still duplicate; the remote remains the
    // final arbiter (a manual push is never blocked).
    {
        let recheck = history::detect_branch_duplicates(
            &options.target_repo_dir,
            &request.target_branch,
            &subsystem.id,
            &request.commits,
            &registry.main_repository,
            deps.github,
        );
        if !recheck.active_shas.is_empty() {
            // The SHAs became integrated while we built (a concurrent run
            // landed first). Our commit is on the temp branch only — nothing
            // reached the target — so fail closed here.
            report.blockers.push(Blocker::new(
                "DUPLICATE_ACTIVE",
                format!(
                    "SHA(s) became integrated on \"{}\" while this promotion was being built: {}. Re-promoting the same SHA is refused post-commit; nothing was pushed.",
                    request.target_branch,
                    recheck.active_shas.join(", ")
                ),
            ));
            return Err(Bail);
        }
        if let Some(err) = &recheck.pr_lookup_error {
            report.blockers.push(Blocker::new(
                "PR_LOOKUP_FAILED",
                format!(
                    "Could not re-list open pull requests for base \"{}\" post-commit; refusing to push a possible duplicate ({err}).",
                    request.target_branch
                ),
            ));
            return Err(Bail);
        }
        if !recheck.open_prs.is_empty() {
            report.blockers.push(Blocker::new(
                "PR_ALREADY_OPEN",
                format!(
                    "An open integration PR for these SHA(s) appeared while this promotion was being built (base \"{}\"). Re-promoting is refused post-commit; nothing was pushed.",
                    request.target_branch
                ),
            ));
            return Err(Bail);
        }
    }

    if request.delivery == PromotionDelivery::LocalDirect {
        // Linked-worktree guard (§10): refuse when `git worktree list` shows
        // the target branch checked out in a different linked worktree. An
        // `update-ref` from here would desync that worktree (and a
        // fast-forward there would need to run there, not here).
        if !git::current_branch(&options.target_repo_dir)
            .is_some_and(|c| c == request.target_branch)
            && is_branch_checked_out_elsewhere(&options.target_repo_dir, &request.target_branch)
        {
            report.blockers.push(Blocker::new(
                "TARGET_CHECKED_OUT_ELSEWHERE",
                format!(
                    "Target branch \"{}\" is checked out in a different linked worktree; refusing to move its ref from '{}'. Finish or remove that worktree (`git worktree list`), or promote with the target checked out here, then re-promote. Nothing was committed to it.",
                    request.target_branch,
                    options.target_repo_dir.display(),
                ),
            ));
            return Err(Bail);
        }
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

    // Pre-push duplicate re-check (§10, best-effort — same caveat as above:
    // narrows the window, does not close it; a manual push is never blocked).
    if options.push || options.create_pr {
        let recheck = history::detect_branch_duplicates(
            &options.target_repo_dir,
            &request.target_branch,
            &subsystem.id,
            &request.commits,
            &registry.main_repository,
            deps.github,
        );
        if !recheck.active_shas.is_empty() || !recheck.open_prs.is_empty() {
            if !recheck.active_shas.is_empty() {
                report.blockers.push(Blocker::new(
                    "DUPLICATE_ACTIVE",
                    format!(
                        "SHA(s) became integrated on \"{}\" before push: {}. Re-promoting the same SHA is refused pre-push; nothing was pushed.",
                        request.target_branch,
                        recheck.active_shas.join(", ")
                    ),
                ));
            } else {
                report.blockers.push(Blocker::new(
                    "PR_ALREADY_OPEN",
                    format!(
                        "An open integration PR for these SHA(s) appeared before push (base \"{}\"). Nothing was pushed.",
                        request.target_branch
                    ),
                ));
            }
            return Err(Bail);
        }
        if let Some(err) = &recheck.pr_lookup_error {
            report.blockers.push(Blocker::new(
                "PR_LOOKUP_FAILED",
                format!(
                    "Could not re-list open pull requests pre-push for base \"{}\"; refusing to push a possible duplicate ({err}).",
                    request.target_branch
                ),
            ));
            return Err(Bail);
        }
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

/// REMOVED: recency guard for adds-over-existing (timestamps).
/// Replaced by blob membership (§3): for each touched path, the shared
/// allowance decides whether the target copy is behind-or-equal, and either
/// way the subsystem's copy is written (a diverged copy is overwritten and
/// reported as replaced). Dates determine display order only, never
/// correctness (addendum E3). The function is deleted, not deprecated, so no
/// caller can accidentally consult timestamps.
fn resolve_local_ref(repo_dir: &Path, branch: &str) -> Option<String> {
    git::resolve_branch_ref(repo_dir, branch)
        .and_then(|reference| git::rev_parse(repo_dir, &reference))
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
    report: &mut PromotionReport,
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
    written.extend(report.changes.replaced.iter().cloned());
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

    // Ignored-path skip (§9): `git check-ignore` for each written path
    // BEFORE staging. Ignored paths are reported as a skip and never forced
    // with `-f` (the `add` below is plain, never `-f`). CRLF-normalised files
    // pass: this check is names-only, content equality was already decided on
    // CRLF-stripped bytes in the apply.
    {
        if !written.is_empty() {
            let mut sorted: Vec<&str> = written.iter().map(String::as_str).collect();
            sorted.sort_unstable();
            let mut args: Vec<&str> = vec!["check-ignore", "--"];
            args.extend(sorted);
            if let Ok(out) = git::run(repo, &args) {
                if out.status == 0 {
                    let ignored: Vec<String> = out
                        .stdout_str()
                        .lines()
                        .map(|l| l.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !ignored.is_empty() {
                        for p in &ignored {
                            written.remove(p);
                        }
                        let mut v = ignored.clone();
                        v.sort();
                        report.changes.ignored_skipped.extend(v.clone());
                        report.warnings.push(Blocker::new(
                            "WORKTREE_IGNORED",
                            format!(
                                "{} path(s) are ignored by the target repository and were not staged: {}. Nothing was forced with `-f`.",
                                v.len(),
                                v.join(", ")
                            ),
                        ));
                    }
                }
            }
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
    if let Err(blocker) = check_staged_matches(repo, &expected) {
        rollback_worktree_stage(repo, target_tip, &written, &deleted);
        return Err(blocker);
    }

    let mut staged_paths: Vec<String> = expected.into_iter().collect();
    staged_paths.sort();
    Ok(staged_paths)
}

/// Verify the index contains exactly `expected` after staging. Returns the
/// staged set on match, else a blocker that NAMES the extra/missing paths
/// (§9, second addendum E): a bare `ENGINE_ERROR` reads as tool breakage.
/// Extracted so tests can fire it by behaviour (stage an extra file, call
/// this, assert the error names it) — the promotion path above calls it too.
fn check_staged_matches(repo: &Path, expected: &HashSet<String>) -> Result<(), Blocker> {
    let staged = match git::run(repo, &["diff", "--cached", "--name-only", "-z"]) {
        Ok(out) if out.status == 0 => out
            .stdout
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s).to_string())
            .collect::<HashSet<_>>(),
        _ => {
            return Err(Blocker::new(
                "ENGINE_ERROR",
                format!(
                    "Could not inspect the index in '{}'; the promotion was aborted and the working tree was rolled back. Nothing was committed.",
                    repo.display(),
                ),
            ));
        }
    };
    if staged != *expected {
        // Name the mismatch: extra (staged but not expected) and missing
        // (expected but not staged) so the operator sees exactly what raced
        // the index.
        let mut extra_sorted: Vec<String> = staged.difference(expected).cloned().collect();
        extra_sorted.sort();
        let mut missing_sorted: Vec<String> = expected.difference(&staged).cloned().collect();
        missing_sorted.sort();
        return Err(Blocker::new(
            "STAGED_MISMATCH",
            format!(
                "The index in '{}' does not match the promoted paths after staging (extra staged: [{}]; missing: [{}]); the promotion was aborted and the working tree was rolled back. Nothing was committed.",
                repo.display(),
                extra_sorted.join(", "),
                missing_sorted.join(", "),
            ),
        ));
    }
    Ok(())
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
    // Determinism (addendum E3, §10): unrelated SHAs (neither is an ancestor
    // of the other) sort by committer timestamp, then SHA — so
    // last-writer-wins can't flip with click order. Dates determine display
    // order only, never correctness (blob membership decides).
    let mut ordered: Vec<String> = Vec::with_capacity(request.commits.len());
    let mut by_time: Vec<(String, i64)> = request
        .commits
        .iter()
        .map(|sha| {
            let epoch = git::commit_epoch(repo_dir, sha).unwrap_or(0);
            (sha.clone(), epoch)
        })
        .collect();
    by_time.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    for (sha, _) in &by_time {
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
    lines.push(format!("Subsystem-ID: {}", subsystem.id));
    lines.push(format!("Source-Repo: {}", subsystem.repository));
    for sha in &request.commits {
        lines.push(format!("Source-Commit: {sha}"));
    }
    lines.push(format!("Promotion-Target: {}", request.target_branch));
    lines.push(String::new());

    // Reporting §5: replaced / deleted / unchanged / flagged / excluded.
    if !report.changes.replaced.is_empty() {
        lines.push("### Replaced (target copy overwritten)".to_string());
        for path in &report.changes.replaced {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    if !report.changes.deleted.is_empty() {
        lines.push("### Deleted".to_string());
        for path in &report.changes.deleted {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    if !report.changes.unchanged.is_empty() {
        lines.push("### Unchanged (target already matched source)".to_string());
        for path in &report.changes.unchanged {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    if !report.changes.flagged.is_empty() {
        lines.push("### Flagged (cross-subsystem overlap — review)".to_string());
        for path in &report.changes.flagged {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    if !report.changes.excluded.is_empty() {
        lines.push("### Excluded via excludedPaths".to_string());
        for path in &report.changes.excluded {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }

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
    if !report.changes.secret_excluded.is_empty() {
        lines.push("### Not imported (secret-looking)".to_string());
        for path in &report.changes.secret_excluded {
            lines.push(format!("- `{path}`"));
        }
        lines.push(String::new());
    }
    let _ = (report, branch_name);
    lines.push(
        "> This PR integrates the selected source commits' changes. A skipped commit's edits to files no selected commit touches are not written; files copied whole may still carry a skipped commit's edits to those same files."
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

    /// `STAGED_MISMATCH` fires by behaviour (second addendum E): inject an
    /// extra staged file alongside a missing expected path and assert the
    /// produced error names both. Same helper the promotion path calls.
    #[test]
    fn staged_mismatch_names_extra_and_missing_paths() {
        use std::collections::HashSet;
        use std::process::Command;

        let dir = std::env::temp_dir().join(format!(
            "vos-staged-mismatch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .output()
                .expect("spawn git");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-q", "-b", "master"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "T"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("keep.txt"), "keep\n").expect("write");
        git(&["add", "-A"]);
        git(&[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "--no-verify",
            "-q",
            "-m",
            "base",
        ]);
        std::fs::write(dir.join("keep.txt"), "keep\n").expect("write");
        std::fs::write(dir.join("extra.txt"), "injected race\n").expect("write");
        git(&["add", "--", "keep.txt", "extra.txt"]);
        let mut expected = HashSet::new();
        expected.insert("keep.txt".to_string());
        expected.insert("missing.txt".to_string());
        let err = check_staged_matches(&dir, &expected).expect_err("must mismatch");
        assert_eq!(err.code, "STAGED_MISMATCH", "{err:?}");
        assert!(
            err.message.contains("extra.txt"),
            "must name the extra staged path: {}",
            err.message
        );
        assert!(
            err.message.contains("missing.txt"),
            "must name the missing path: {}",
            err.message
        );
        assert_ne!(err.code, "ENGINE_ERROR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn promotion_resolver_accepts_a_branch_only_on_a_non_origin_remote() {
        use std::process::Command;

        let root = std::env::temp_dir().join(format!(
            "vos-promote-ref-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let upstream = root.join("upstream");
        let clone = root.join("clone");
        let git = |dir: &Path, args: &[&str]| {
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
        };
        std::fs::create_dir_all(&upstream).expect("mkdir upstream");
        git(&upstream, &["init", "-q", "-b", "master"]);
        git(
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
        git(&upstream, &["branch", "UAT"]);
        let uat_sha = git(&upstream, &["rev-parse", "UAT"]);

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
            resolve_local_ref(&clone, "UAT"),
            Some(uat_sha),
            "the promotion path must accept a branch that lives only on a non-origin remote"
        );
        let master_sha = git(&clone, &["rev-parse", "refs/heads/master"]);
        assert_eq!(resolve_local_ref(&clone, "master"), Some(master_sha));
        assert_eq!(resolve_local_ref(&clone, "does-not-exist"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}

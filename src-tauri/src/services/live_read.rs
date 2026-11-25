use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::{self, now_rfc3339};
use crate::github::models::{
    ChangedFile, CommitDetails, CommitDiff, CommitPage, CommitQuery, CommitStatus,
    CommitStatusKind, CommitSummary, DataSource, DependencyChange, DependencyChangeKind,
    DependencySection, DiffHunk, DiffLine, DiffLineKind, FileChangeKind, FileIntegrationPolicy,
    IntegrationEventAudit, IntegrationEventChange, IntegrationEventChangeKind,
    IntegrationEventCommit, IntegrationEventProvenance, IntegrationEventTotals,
    IntegrationEvidence, IntegrationHistory, IntegrationHistoryEntry, IntegrationHistoryKind,
    IntegrationPr, IntegrationStatus, MainCommitDiff, MainCommitFileDiff, PrState, PullRequestRef,
    ReviewState, SubsystemPolicy, SubsystemSummary, TargetBranch, TargetBranchList,
    TargetBranchPolicy,
};
use crate::services::engine::github_ops::{BranchInfo, GitHubOps, PullRequestInfo};
use crate::services::engine::history::BranchStatusKind;
use crate::services::engine::{classify, deps, git, history};
use crate::services::registry::{Registry, SubsystemEntry};

pub const MAX_DIFF_LINES: usize = 1500;
const MAX_SOURCE_COMMITS: usize = 2000;
const WORKFLOW_FILE: &str = "promote-subsystem.yml";

pub fn use_fixtures() -> bool {
    std::env::var("VOS_USE_FIXTURES")
        .map(|v| v == "1")
        .unwrap_or(false)
}

pub fn load_live_registry() -> AppResult<Registry> {
    Registry::load(Registry::app_data_override_path().as_deref())
}

/// Classify a `gh`/network failure into a typed [`AppError`]. Never logs tokens.
pub fn gh_error_to_app(raw: String) -> AppError {
    let clean = git::sanitize(&raw);
    let lower = clean.to_lowercase();
    if lower.contains("gh could not be started")
        || lower.contains("gh was not found")
        || (lower.contains("gh") && lower.contains("no such file"))
    {
        return AppError::new(
            ErrorCode::MissingPrerequisite,
            "GitHub CLI (gh) was not found on this machine.",
        )
        .with_remedy(
            "Install GitHub CLI from https://cli.github.com, run `gh auth login`, then retry. Live reads stay disabled until then.",
        )
        .with_details(clean);
    }
    if lower.contains("rate limit")
        || lower.contains("secondary rate")
        || lower.contains("abuse")
        || lower.contains("429")
    {
        return AppError::new(
            ErrorCode::RateLimited,
            "GitHub rate-limited this request.",
        )
        .with_remedy("Wait a minute and retry. If it persists, check `gh api rate_limit` for the reset time.")
        .with_details(clean)
        .retryable();
    }
    if lower.contains("could not resolve")
        || lower.contains("failed to connect")
        || lower.contains("connection refused")
        || lower.contains("network is unreachable")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("offline")
        || lower.contains("no internet")
    {
        return AppError::new(
            ErrorCode::Offline,
            "The GitHub request failed: the network looks unreachable.",
        )
        .with_remedy("Check the network connection, then retry. `VOS_TARGET_REPO_DIR` reads of cached clones keep working offline.")
        .with_details(clean)
        .retryable();
    }
    if lower.contains("401")
        || lower.contains("bad credentials")
        || lower.contains("requires authentication")
        || lower.contains("not authenticated")
    {
        return AppError::new(
            ErrorCode::Unauthenticated,
            "GitHub rejected the credentials.",
        )
        .with_remedy("Run `gh auth login --hostname github.com` (or set GH_TOKEN) and retry.")
        .with_details(clean);
    }
    if lower.contains("403") && lower.contains("scope") {
        return AppError::new(
            ErrorCode::InsufficientScope,
            "The GitHub login lacks a scope required to read this repository.",
        )
        .with_remedy("Run `gh auth refresh -h github.com -s repo` and retry.")
        .with_details(clean);
    }
    if lower.contains("404") {
        return AppError::new(
            ErrorCode::RepositoryInaccessible,
            "GitHub reports this repository or branch does not exist (404).",
        )
        .with_remedy("Confirm the registry OWNER/REPO slugs and that `gh` can read them (`gh repo view OWNER/REPO`).")
        .with_details(clean);
    }
    AppError::new(
        ErrorCode::RepositoryInaccessible,
        "The GitHub request failed.",
    )
    .with_remedy(
        "Confirm `gh auth status` works and the registry repositories are readable, then retry.",
    )
    .with_details(clean)
}

fn no_token_error() -> AppError {
    AppError::new(
        ErrorCode::Unauthenticated,
        "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
    )
    .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
}

fn subsystem_not_found(id: &str, main_repo: &str) -> AppError {
    AppError::invalid_registry(format!(
        "'{id}' is not present in the tool registry (main repository {main_repo})."
    ))
    .with_remedy(
        "Register the subsystem first; the registry override lives at %APPDATA%\\vos-origins\\registry.json.",
    )
}

fn disabled_error(id: &str) -> AppError {
    AppError::invalid_registry(format!("Subsystem '{id}' is registered but disabled."))
        .with_remedy("Re-enable it through a reviewed registry change, then refresh.")
}

fn branch_not_permitted(branch: &str, reason: &str) -> AppError {
    AppError::new(
        ErrorCode::BranchNotPermitted,
        format!("Target branch '{branch}' is not permitted ({reason})."),
    )
    .with_remedy("Choose a target branch that exists and is allowed by promotion.allowedTargetBranches / allowedTargetBranchPatterns.")
}

fn policy_strings(entry: &SubsystemEntry) -> SubsystemPolicy {
    use crate::services::registry::{
        ApplicationFiles, DependencyStrategy as DepStrat, GithubWorkflowPolicy, LockfilePolicy,
        PackageJsonPolicy, PathStrategy, SecretFilePolicy, SharedConfigPolicy,
    };
    let application_files = match entry.integration_policy.application_files {
        ApplicationFiles::SelectedCommitDelta => "selected-commit-delta",
    };
    let package_json = match entry.integration_policy.package_json {
        PackageJsonPolicy::MergeDependencyDelta => "merge-dependency-delta",
        PackageJsonPolicy::ExplicitReview => "explicit-review",
    };
    let lockfiles = match entry.integration_policy.lockfiles {
        LockfilePolicy::RegenerateInTarget => "regenerate-in-target",
    };
    let shared_configuration = match entry.integration_policy.shared_configuration {
        SharedConfigPolicy::ReviewExplicitly => "review-explicitly",
        SharedConfigPolicy::MergeControlled => "merge-controlled",
    };
    let github_workflows = match entry.integration_policy.github_workflows {
        GithubWorkflowPolicy::MainOwned => "main-owned",
    };
    let secret_files = match entry.integration_policy.secret_files {
        SecretFilePolicy::NeverImport => "never-import",
    };
    let _ = PathStrategy::PreserveRelativePaths;
    let _ = DepStrat::Reconcile;
    SubsystemPolicy {
        application_files: application_files.to_string(),
        package_json: package_json.to_string(),
        lockfiles: lockfiles.to_string(),
        shared_configuration: shared_configuration.to_string(),
        github_workflows: github_workflows.to_string(),
        secret_files: secret_files.to_string(),
    }
}

pub fn subsystem_summaries(registry: &Registry) -> Vec<SubsystemSummary> {
    let now = now_rfc3339();
    let mut out: Vec<SubsystemSummary> = registry
        .subsystems
        .iter()
        .map(|entry| {
            let mut allowed = registry.promotion.allowed_target_branches.clone();
            for pattern in &registry.promotion.allowed_target_branch_patterns {
                if !allowed.iter().any(|b| b == pattern) {
                    allowed.push(pattern.clone());
                }
            }
            let path_strategy = match entry.path_strategy {
                crate::services::registry::PathStrategy::PreserveRelativePaths => {
                    "preserve-relative-paths"
                }
            };
            let dependency_strategy = match entry.dependency_strategy {
                crate::services::registry::DependencyStrategy::Reconcile => "reconcile",
                crate::services::registry::DependencyStrategy::Disabled => "none",
            };
            SubsystemSummary {
                id: entry.id.clone(),
                name: entry.name.clone(),
                repository: entry.repository.clone(),
                source_branch: entry.source_branch.clone(),
                enabled: entry.enabled,
                description: format!(
                    "{} — {} ({})",
                    entry.name, entry.repository, entry.source_branch
                ),
                connectivity: crate::github::models::ConnectivityState::Connected,
                connectivity_message: None,
                policy: policy_strings(entry),
                path_strategy: path_strategy.to_string(),
                dependency_strategy: dependency_strategy.to_string(),
                last_synced_at: now.clone(),
                registered_at: now.clone(),
                registration_pr_number: None,
                registration_pr_url: None,
                allowed_target_branches: allowed,
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn find_live_subsystem<'a>(
    registry: &'a Registry,
    subsystem_id: &str,
) -> AppResult<&'a SubsystemEntry> {
    if subsystem_id.trim().is_empty() {
        return Err(
            AppError::invalid_registry("No subsystem was specified.").with_remedy(
                "Pick a registered subsystem from the Subsystems or Commit Browser screen.",
            ),
        );
    }
    let entry = registry
        .find(subsystem_id)
        .ok_or_else(|| subsystem_not_found(subsystem_id, &registry.main_repository))?;
    if !entry.enabled {
        return Err(disabled_error(&entry.id));
    }
    Ok(entry)
}

pub(crate) fn require_allowed_target(registry: &Registry, branch: &str) -> AppResult<String> {
    if branch.trim().is_empty() {
        return Err(AppError::invalid_request("No target branch was specified.")
            .with_remedy("Pick a target branch from the branch selector."));
    }
    let decision = registry.is_target_allowed(branch);
    if !decision.allowed {
        return Err(branch_not_permitted(branch, &decision.reason));
    }
    Ok(branch.to_string())
}

pub fn resolve_target_checkout(
    main_repository: &str,
    local_override: Option<&str>,
    token: Option<&str>,
) -> AppResult<(PathBuf, Option<PathBuf>)> {
    // The registry (Settings → localMainRepositoryPath) wins over the
    // environment: an explicitly configured directory must be honoured by
    // reads too, not only by the promotion engine, otherwise the app clones
    // the main repo into the cache while the user's own clone sits unused.
    let configured = local_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("VOS_TARGET_REPO_DIR")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        });
    if let Some(path) = configured {
        if path.is_dir() {
            crate::services::checkouts::fetch_local_clone_if_stale(&path, token);
            return Ok((path, None));
        }
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "The configured main-system directory '{}' is not a directory.",
                path.display()
            ),
        )
        .with_remedy("Set the local clone in Settings (localMainRepositoryPath) or VOS_TARGET_REPO_DIR to your local main-system git clone, or clear it to use the managed checkout."));
    }
    if let Some(local) = crate::services::checkouts::local_dir_for(main_repository) {
        crate::services::checkouts::fetch_local_clone_if_stale(&local, token);
        return Ok((local, None));
    }
    let token = token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(no_token_error)?;
    let key = crate::services::checkouts::target_key(main_repository);
    let url = crate::services::engine::github_ops::authenticated_remote_url(main_repository, token);
    let dir = crate::services::checkouts::ensure_remote_checkout(&key, &url, Some(token))?;
    Ok((dir, None))
}

pub fn resolve_source_checkout(
    entry: &SubsystemEntry,
    resolver: &dyn Fn(&SubsystemEntry) -> Option<String>,
    token: Option<&str>,
) -> AppResult<(PathBuf, Option<PathBuf>)> {
    let raw = resolver(entry).ok_or_else(|| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!("No source URL is configured for subsystem '{}'.", entry.id),
        )
        .with_remedy("Check the registry repository slug for this subsystem.")
    })?;
    if let Some(local) = crate::services::checkouts::local_dir_for(&raw) {
        return Ok((local, None));
    }
    let key = crate::services::checkouts::source_key(entry);
    let dir = crate::services::checkouts::ensure_remote_checkout(&key, &raw, token)?;
    Ok((dir, None))
}

struct StatusEnrichment {
    primary: history::ReplayResult,
    remote: Option<history::ReplayResult>,
    local: Option<history::ReplayResult>,
}

fn status_enrichment_for(repo_dir: &Path, queried_branch: &str) -> StatusEnrichment {
    let local_ref = format!("refs/heads/{queried_branch}");
    let primary = history::resolve_ref(repo_dir, queried_branch)
        .map(|reference| history::cached_replay(repo_dir, &reference))
        .unwrap_or_default();
    StatusEnrichment {
        remote: git::remote_tracking_ref(repo_dir, queried_branch)
            .and_then(|reference| history::cached_replay_if_resolvable(repo_dir, &reference)),
        local: history::cached_replay_if_resolvable(repo_dir, &local_ref),
        primary,
    }
}

fn evidence_for(
    detail: Option<&history::CommitDetail>,
    integration_commit: &str,
    queried_branch: &str,
    promotion_target: Option<&str>,
) -> IntegrationEvidence {
    let target = promotion_target.unwrap_or(queried_branch).to_string();
    let inherited = if target == queried_branch {
        None
    } else {
        Some(target.clone())
    };
    let iso = detail
        .map(|d| d.committed_iso.clone())
        .filter(|s| !s.is_empty());
    let subject = detail.map(|d| d.subject.clone());
    let message = detail
        .map(|d| d.message.trim().to_string())
        .filter(|m| !m.is_empty());
    IntegrationEvidence {
        main_commit_sha: integration_commit.to_string(),
        merged_at: iso.unwrap_or_else(now_rfc3339),
        message: message
            .or(subject)
            .unwrap_or_else(|| format!("Integration {integration_commit}")),
        pull_request_number: None,
        target_branch: queried_branch.to_string(),
        inherited_from_branch: inherited,
    }
}

/// Last-fetch time for the checkout (`FETCH_HEAD` mtime — filesystem metadata
/// only, no git spawn). Used for the `Unknown (stale, as of <time>)` label
/// when the remote-tracking ref cannot be read (§8).
fn fetch_time_str(repo_dir: &Path) -> String {
    let fetch_head = repo_dir.join(".git").join("FETCH_HEAD");
    // Bare repos and worktrees differ: try `.git/FETCH_HEAD`, then `FETCH_HEAD`.
    let candidates = [fetch_head, repo_dir.join("FETCH_HEAD")];
    for candidate in &candidates {
        if let Ok(meta) = std::fs::metadata(candidate) {
            if let Ok(modified) = meta.modified() {
                let secs = modified
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
                    return dt.to_rfc3339();
                }
            }
        }
    }
    "unknown fetch time".to_string()
}

fn branch_results_to_statuses(
    repo_dir: &Path,
    queried_branch: &str,
    results: &[history::BranchStatusResult],
) -> Vec<CommitStatus> {
    let enrichment = status_enrichment_for(repo_dir, queried_branch);
    results
        .iter()
        .map(|result| branch_result_to_status_with(repo_dir, &enrichment, queried_branch, result))
        .collect()
}

fn branch_result_to_status_with(
    repo_dir: &Path,
    enrichment: &StatusEnrichment,
    queried_branch: &str,
    result: &history::BranchStatusResult,
) -> CommitStatus {
    match result.status {
        BranchStatusKind::Integrated => {
            // Verify trailers actually exist on the claimed commit (addendum A).
            // A squash-merge or history rewrite strips them: absence of
            // trailers is NOT evidence of no promotion — it is Unknown/needs
            // verification, never Eligible. Message parsing is a hint for the
            // claim; the trailer block is the verification (and blob
            // membership decides divergence elsewhere — E2). The message comes
            // from the branch walk's harvested details, not a per-row
            // `git show`: a commit absent from the walk reads as empty, which
            // can only yield Unknown here, never a fabricated Integrated.
            if let Some(commit) = result.integration_commit.as_deref() {
                let msg = enrichment
                    .primary
                    .details
                    .get(commit)
                    .map(|d| d.message.as_str())
                    .unwrap_or("");
                let parsed = crate::services::engine::provenance::parse_trailers(msg);
                let has_subsystem = parsed.first("Subsystem-ID").is_some();
                let has_source = parsed
                    .get("Source-Commit")
                    .iter()
                    .any(|s| s == &result.sha);
                if !has_subsystem || !has_source {
                    return CommitStatus::of(
                        CommitStatusKind::Unknown,
                        queried_branch,
                        Some(format!(
                            "A promotion was claimed via {} but its provenance trailers are missing (squash-merge strips them). Needs verification — absence of trailers is not evidence of no promotion, so this is Unknown, never Eligible.",
                            commit.chars().take(7).collect::<String>(),
                        )),
                        None,
                    );
                }
            }
            let evidence = result.integration_commit.as_deref().map(|commit| {
                evidence_for(
                    enrichment.primary.details.get(commit),
                    commit,
                    queried_branch,
                    result.promotion_target.as_deref(),
                )
            });
            let mut detail = match &evidence {
                Some(ev) => format!(
                    "Integration commit {} recorded this source SHA on {}.",
                    ev.main_commit_sha.chars().take(7).collect::<String>(),
                    queried_branch
                ),
                None => format!("Recorded as integrated on {queried_branch}."),
            };
            let mut label_override: Option<String> = None;
            // Remote-tracking resolution (§8, G4): Integrated iff reachable
            // from the RESOLVED remote-tracking ref (any remote, never a
            // hardcoded `origin`); local-only iff reachable from
            // refs/heads/<branch> alone; stale when the remote ref can't be
            // read. Additive: kind stays Integrated for local-only (label
            // carries the qualifier); stale without local proof is Unknown.
            // Never demote a known-remote Integrated on stale data — if the
            // commit is reachable locally we keep Integrated (local-only),
            // not Unknown.
            if let Some(commit) = result.integration_commit.as_deref() {
                let in_remote = enrichment
                    .remote
                    .as_ref()
                    .is_some_and(|replay| replay.reachable.contains(commit));
                if in_remote {
                } else {
                    let in_local = enrichment
                        .local
                        .as_ref()
                        .is_some_and(|replay| replay.reachable.contains(commit));
                    if in_local {
                        label_override = Some("Integrated (local only, unpushed)".to_string());
                        detail = format!(
                            "{detail} Reachable from the local '{queried_branch}' only; the remote-tracking ref does not contain it (unpushed). Push when ready."
                        );
                    } else if enrichment.remote.is_none() {
                        return CommitStatus {
                            kind: CommitStatusKind::Unknown,
                            label: "Unknown (stale)".to_string(),
                            target_branch: queried_branch.to_string(),
                            detail: Some(format!(
                                "Unknown (stale, as of {}): no remote-tracking ref for '{queried_branch}' can be read and the commit is not reachable locally. A squash-merge strips trailers, so absence proves nothing — needs verification, never Eligible.",
                                fetch_time_str(repo_dir),
                            )),
                            evidence,
                        };
                    }
                }
            }
            let mut status = CommitStatus::of(
                CommitStatusKind::Integrated,
                queried_branch,
                Some(detail),
                evidence,
            );
            if let Some(label) = label_override {
                status.label = label;
            }
            status
        }
        BranchStatusKind::Reverted => {
            let evidence = result.integration_commit.as_deref().map(|commit| {
                evidence_for(
                    enrichment.primary.details.get(commit),
                    commit,
                    queried_branch,
                    result.promotion_target.as_deref(),
                )
            });
            let reverted = result
                .reverted_by
                .as_deref()
                .map(|s| format!(" Reverted by {}.", s.chars().take(7).collect::<String>()))
                .unwrap_or_default();
            CommitStatus::of(
                CommitStatusKind::Reverted,
                queried_branch,
                Some(format!(
                    "Previously integrated and then reverted on this target branch.{reverted} A deliberate re-promotion opens a new integration PR."
                )),
                evidence,
            )
        }
        BranchStatusKind::Eligible => CommitStatus::of(
            CommitStatusKind::Eligible,
            queried_branch,
            Some(format!(
                "Reachable from the protected source branch. No successful integration found for {queried_branch}. Trailers absent — a squash-merge strips them; the content labels Applied / Divergent are the cross-check."
            )),
            None,
        ),
        BranchStatusKind::Unknown => CommitStatus::unknown(
            queried_branch,
            "This SHA could not be verified against the target branch history.",
        ),
    }
}

fn open_pr_status(pr: &PullRequestInfo, queried_branch: &str) -> CommitStatus {
    let number = pr.number.unwrap_or(0);
    CommitStatus::of(
        CommitStatusKind::PrOpen,
        queried_branch,
        Some(format!(
            "Integration PR #{number} targets {queried_branch}. Not merged, so not integrated."
        )),
        None,
    )
}

pub fn integration_statuses_with_prs(
    target_dir: &Path,
    registry: &Registry,
    open_prs: &[PullRequestInfo],
    target_branch: &str,
    subsystem_id: Option<&str>,
    shas: &[String],
) -> AppResult<Vec<IntegrationStatus>> {
    let target_branch = require_allowed_target(registry, target_branch)?;
    if !git::branch_exists(target_dir, &target_branch) {
        return Err(branch_not_permitted(
            &target_branch,
            "branch does not exist in the target checkout",
        ));
    }
    let ids: Vec<String> = match subsystem_id {
        Some(id) => vec![find_live_subsystem(registry, id)?.id.clone()],
        None => registry.subsystems.iter().map(|s| s.id.clone()).collect(),
    };
    // Fail closed: an unreadable PR list must not hide a pending promotion.
    // (`integration_statuses_live` fetches the list; callers that already hold
    // it — commit details — pass it in. An empty slice here means "no open
    // PRs", never "lookup failed".)
    let checked_at = now_rfc3339();
    let enrichment = status_enrichment_for(target_dir, &target_branch);
    let mut out = Vec::new();
    for id in ids {
        let results = history::compute_branch_status(target_dir, &target_branch, &id, shas);
        for result in results {
            let mut status =
                branch_result_to_status_with(target_dir, &enrichment, &target_branch, &result);
            // Overlay: an open PR referencing this SHA reports PrOpen —
            // but only when the reachable history does not already prove
            // Integrated/Reverted. A push or PR never mints Integrated.
            let base_kind = status.kind;
            if matches!(
                base_kind,
                CommitStatusKind::Eligible | CommitStatusKind::Unknown
            ) {
                let matched = open_prs.iter().find(|pr| {
                    history::pr_references_requested_shas(
                        pr,
                        &id,
                        std::slice::from_ref(&result.sha),
                    )
                });
                if let Some(pr) = matched {
                    status = open_pr_status(pr, &target_branch);
                }
            }
            out.push(IntegrationStatus {
                subsystem_id: id.clone(),
                source_sha: result.sha.clone(),
                target_branch: target_branch.clone(),
                status,
                checked_at: checked_at.clone(),
            });
        }
    }
    Ok(out)
}

/// Core status computation against an already-resolved target checkout.
/// `Integrated` only from reachable trailers; open PRs overlay as `PrOpen`.
/// Fail closed: an unreadable PR list must not hide a pending promotion.
pub fn integration_statuses_live(
    target_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    target_branch: &str,
    subsystem_id: Option<&str>,
    shas: &[String],
) -> AppResult<Vec<IntegrationStatus>> {
    // Fetched here (rather than by the caller) so the fail-closed mapping
    // stays in one place: a PR-lookup failure errors, it never yields an
    // empty overlay that could hide a pending promotion.
    let open_prs = github
        .list_open_pull_requests(&registry.main_repository, target_branch)
        .map_err(gh_error_to_app)?;
    integration_statuses_with_prs(
        target_dir,
        registry,
        &open_prs,
        target_branch,
        subsystem_id,
        shas,
    )
}

fn target_policy(registry: &Registry) -> TargetBranchPolicy {
    TargetBranchPolicy {
        allowed_target_branches: registry.promotion.allowed_target_branches.clone(),
        allowed_target_branch_patterns: registry.promotion.allowed_target_branch_patterns.clone(),
        require_pull_request: registry.promotion.require_pull_request,
        allow_direct_push: false,
        workflow_file: WORKFLOW_FILE.to_string(),
    }
}

pub fn target_branches_live(
    registry: &Registry,
    github: &dyn GitHubOps,
    selected: Option<&str>,
    target_dir: Option<&Path>,
) -> AppResult<TargetBranchList> {
    let remote: Vec<BranchInfo> = github
        .list_branches(&registry.main_repository)
        .map_err(gh_error_to_app)?;
    if remote.is_empty() {
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "No branches were returned for {}.",
                registry.main_repository
            ),
        )
        .with_remedy("Confirm the repository exists and `gh` can read it."));
    }
    let default_branch = github
        .repo_default_branch(&registry.main_repository)
        .map_err(gh_error_to_app)?
        .unwrap_or_else(|| "master".to_string());
    let mut branches: Vec<TargetBranch> = Vec::new();
    for info in &remote {
        let decision = registry.is_target_allowed(&info.name);
        let missing_from_checkout =
            decision.allowed && target_dir.is_some_and(|dir| !git::branch_exists(dir, &info.name));
        let allowed = decision.allowed && !missing_from_checkout;
        // Best-effort open-PR count; a lookup failure leaves 0 rather than
        // failing the whole branch list.
        let open_count = github
            .list_open_pull_requests(&registry.main_repository, &info.name)
            .map(|prs| prs.len() as u32)
            .unwrap_or(0);
        branches.push(TargetBranch {
            name: info.name.clone(),
            head_sha: info.sha.clone(),
            protected: info.protected,
            is_default: info.name == default_branch,
            allowed,
            disallowed_reason: if allowed {
                None
            } else if missing_from_checkout {
                Some("Branch does not exist in the target checkout.".to_string())
            } else {
                Some(format!(
                    "Not permitted by registry policy ({}).",
                    decision.reason
                ))
            },
            open_pr_count: open_count,
            active_promotions: open_count,
        });
    }
    let policy_order: Vec<&str> = registry
        .promotion
        .allowed_target_branches
        .iter()
        .map(String::as_str)
        .collect();
    branches.sort_by(|a, b| {
        let rank = |name: &str| {
            policy_order
                .iter()
                .position(|candidate| *candidate == name)
                .unwrap_or(usize::MAX)
        };
        (rank(&a.name), a.name.as_str()).cmp(&(rank(&b.name), b.name.as_str()))
    });
    let selected_name = selected
        .map(str::to_string)
        .filter(|s| branches.iter().any(|b| &b.name == s && b.allowed))
        .or_else(|| {
            branches
                .iter()
                .find(|b| b.name == fixtures::DEFAULT_TARGET_BRANCH && b.allowed)
                .map(|b| b.name.clone())
        })
        .or_else(|| branches.iter().find(|b| b.allowed).map(|b| b.name.clone()))
        .or_else(|| branches.first().map(|b| b.name.clone()))
        .unwrap_or_else(|| fixtures::DEFAULT_TARGET_BRANCH.to_string());
    Ok(TargetBranchList {
        target_branch: selected_name,
        main_repository: registry.main_repository.clone(),
        registry_branch: "tool-registry".to_string(),
        branches,
        policy: target_policy(registry),
        fetched_at: now_rfc3339(),
        data_source: DataSource::Live,
    })
}

fn parse_pr_subsystem(body: &str, head: &str) -> Option<String> {
    for raw in body.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Subsystem-ID:") {
            let candidate = rest.split_whitespace().next().unwrap_or("");
            if crate::services::registry::assert_subsystem_id(candidate).is_ok() {
                return Some(candidate.to_string());
            }
        }
    }
    if let Some(rest) = head.strip_prefix("integration/") {
        let id = rest.split('-').next().unwrap_or("");
        if !id.is_empty() && crate::services::registry::assert_subsystem_id(id).is_ok() {
            return Some(id.to_string());
        }
    }
    None
}

fn is_integration_pr(pr: &PullRequestInfo) -> bool {
    if pr.head.starts_with("integration/") {
        return true;
    }
    pr.body.contains("Subsystem-ID:") && pr.body.contains("Source-Commit:")
}

fn pr_state_of(pr: &PullRequestInfo) -> PrState {
    if pr.merged_at.as_deref().is_some_and(|s| !s.is_empty()) {
        return PrState::Merged;
    }
    match pr.state.to_lowercase().as_str() {
        "open" => PrState::Open,
        "merged" => PrState::Merged,
        "closed" => PrState::Closed,
        _ => PrState::Open,
    }
}

fn integration_pr_from_info(
    pr: &PullRequestInfo,
    main_repository: &str,
    fallback_subsystem: Option<&str>,
) -> Option<IntegrationPr> {
    if !is_integration_pr(pr) {
        return None;
    }
    let subsystem_id = parse_pr_subsystem(&pr.body, &pr.head)
        .or_else(|| fallback_subsystem.map(str::to_string))
        .unwrap_or_default();
    if subsystem_id.is_empty() {
        return None;
    }
    let source_shas = history::extract_source_commits(&pr.body);
    let number = pr.number.unwrap_or(0);
    let provenance_ok = parse_pr_subsystem(&pr.body, &pr.head).is_some() && !source_shas.is_empty();
    Some(IntegrationPr {
        number,
        title: if pr.title.is_empty() {
            format!("Integration {subsystem_id}")
        } else {
            pr.title.clone()
        },
        state: pr_state_of(pr),
        draft: pr.draft,
        review_state: ReviewState::Unknown,
        base_branch: pr.base.clone(),
        head_branch: pr.head.clone(),
        subsystem_id,
        source_shas,
        created_at: pr.created_at.clone().unwrap_or_else(now_rfc3339),
        updated_at: pr.updated_at.clone().unwrap_or_else(now_rfc3339),
        merged_at: pr.merged_at.clone(),
        merged_by: None,
        merge_commit_sha: None,
        author: pr.author.clone().unwrap_or_else(|| "unknown".to_string()),
        html_url: pr
            .url
            .clone()
            .unwrap_or_else(|| format!("https://github.com/{main_repository}/pull/{number}")),
        provenance_ok,
        provenance_note: if provenance_ok {
            None
        } else {
            Some(
                "The PR body is missing machine-readable Subsystem-ID / Source-Commit trailers; status matching falls back to the branch name."
                    .to_string(),
            )
        },
    })
}

pub fn integration_prs_live(
    registry: &Registry,
    github: &dyn GitHubOps,
    target_branch: &str,
    subsystem_id: Option<&str>,
) -> AppResult<Vec<IntegrationPr>> {
    let target_branch = require_allowed_target(registry, target_branch)?;
    if let Some(id) = subsystem_id {
        find_live_subsystem(registry, id)?;
    }
    let raw = github
        .list_open_pull_requests(&registry.main_repository, &target_branch)
        .map_err(gh_error_to_app)?;
    let mut out: Vec<IntegrationPr> = raw
        .iter()
        .filter_map(|pr| integration_pr_from_info(pr, &registry.main_repository, subsystem_id))
        .filter(|pr| pr.base_branch == target_branch)
        .filter(|pr| subsystem_id.map(|id| pr.subsystem_id == id).unwrap_or(true))
        .collect();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(out)
}

fn resolve_history_branch(
    target_dir: &Path,
    registry: &Registry,
    requested: Option<&str>,
) -> AppResult<String> {
    match requested {
        Some(branch) => {
            let allowed = require_allowed_target(registry, branch)?;
            if !git::branch_exists(target_dir, &allowed) {
                return Err(branch_not_permitted(
                    &allowed,
                    "branch does not exist in the target checkout",
                ));
            }
            Ok(allowed)
        }
        None => {
            let first = registry
                .promotion
                .allowed_target_branches
                .first()
                .cloned()
                .unwrap_or_else(|| fixtures::DEFAULT_TARGET_BRANCH.to_string());
            Ok(first)
        }
    }
}

pub fn integration_history_live(
    target_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    target_branch: Option<&str>,
    subsystem_id: Option<&str>,
    limit: u32,
) -> AppResult<IntegrationHistory> {
    let branch = resolve_history_branch(target_dir, registry, target_branch)?;
    if let Some(id) = subsystem_id {
        find_live_subsystem(registry, id)?;
    }
    let reference = git::resolve_branch_ref(target_dir, &branch).ok_or_else(|| {
        branch_not_permitted(&branch, "branch does not exist in the target checkout")
    })?;
    let replayed = history::cached_replay(target_dir, &reference);
    let mut entries: Vec<IntegrationHistoryEntry> = replayed
        .history
        .iter()
        .filter(|event| {
            subsystem_id
                .map(|id| event.subsystem.as_deref() == Some(id))
                .unwrap_or(true)
        })
        .map(|event| {
            let (kind, detail) = match event.kind {
                history::EventType::Integrate => {
                    (IntegrationHistoryKind::Merged, event.subject.clone())
                }
                history::EventType::Revert => (
                    IntegrationHistoryKind::Reverted,
                    format!(
                        "Reverted {}. {}",
                        event
                            .reverts_integration
                            .clone()
                            .unwrap_or_else(|| "an integration".to_string()),
                        event.subject
                    ),
                ),
                history::EventType::Restore => {
                    (IntegrationHistoryKind::Restored, event.subject.clone())
                }
            };
            IntegrationHistoryEntry {
                id: event.commit.clone(),
                kind,
                subsystem_id: event
                    .subsystem
                    .clone()
                    .unwrap_or_else(|| subsystem_id.unwrap_or("unknown").to_string()),
                source_shas: event.source_commits.clone(),
                target_branch: event.target.clone().unwrap_or_else(|| branch.clone()),
                main_commit_sha: Some(event.commit.clone()),
                pull_request_number: None,
                actor: if event.author.trim().is_empty() {
                    "unknown".to_string()
                } else {
                    event.author.clone()
                },
                occurred_at: if event.committed_iso.is_empty() {
                    now_rfc3339()
                } else {
                    event.committed_iso.clone()
                },
                detail,
                inherited_from_branch: None,
            }
        })
        .collect();
    let open_prs = github
        .list_open_pull_requests(&registry.main_repository, &branch)
        .map_err(gh_error_to_app)?;
    for pr in &open_prs {
        let Some(mapped) = integration_pr_from_info(pr, &registry.main_repository, subsystem_id)
        else {
            continue;
        };
        if mapped.base_branch != branch {
            continue;
        }
        if subsystem_id
            .map(|id| mapped.subsystem_id != id)
            .unwrap_or(false)
        {
            continue;
        }
        entries.push(IntegrationHistoryEntry {
            id: format!("pr-{}", mapped.number),
            kind: IntegrationHistoryKind::PendingPr,
            subsystem_id: mapped.subsystem_id.clone(),
            source_shas: mapped.source_shas.clone(),
            target_branch: mapped.base_branch.clone(),
            main_commit_sha: None,
            pull_request_number: Some(mapped.number),
            actor: mapped.author.clone(),
            occurred_at: mapped.created_at.clone(),
            detail: mapped.title.clone(),
            inherited_from_branch: None,
        });
    }
    entries.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
    let total = entries.len() as u32;
    let capped = limit.clamp(1, 200) as usize;
    entries.truncate(capped);
    Ok(IntegrationHistory {
        target_branch: branch,
        subsystem_id: subsystem_id.map(str::to_string),
        entries,
        total,
        fetched_at: now_rfc3339(),
        data_source: DataSource::Live,
    })
}

pub fn cached_source_dir(registry: &Registry, subsystem_id: &str) -> AppResult<Option<PathBuf>> {
    let entry = find_live_subsystem(registry, subsystem_id)?;
    if let Some(local) = crate::services::checkouts::local_dir_for(&entry.repository) {
        return Ok(Some(local));
    }
    let dir = crate::services::checkouts::checkouts_root()
        .join(crate::services::checkouts::source_key(entry));
    if !dir.join(".git").is_dir() {
        return Ok(None);
    }
    if git::rev_parse(&dir, "HEAD").is_none() {
        return Ok(None);
    }
    Ok(Some(dir))
}

/// Verbatim trailing `Key: value` block of a commit message, in git order.
/// No allowlist and no SHA validation: this is the audit artefact, surfaced
/// exactly as written. It shares the trailing-block rule with
/// [`provenance::parse_trailers`] but never feeds a structured decision —
/// `source_shas` and the revert link below come only from the already-parsed
/// event fields, so the two cannot disagree.
fn verbatim_trailer_block(message: &str) -> Vec<(String, String)> {
    let lines: Vec<&str> = message.split('\n').collect();
    let mut end = lines.len();
    while end > 0 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    let mut block: Vec<&str> = Vec::new();
    let mut i = end;
    while i > 0 {
        let line = lines[i - 1];
        let Some(idx) = line.find(':') else {
            break;
        };
        if idx == 0
            || !line[..idx]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            break;
        }
        block.push(line);
        i -= 1;
    }
    block.reverse();
    let mut out = Vec::new();
    for line in block {
        let idx = line.find(':').unwrap_or(0);
        out.push((line[..idx].to_string(), line[idx + 1..].trim().to_string()));
    }
    out
}

fn unavailable_commit(sha: &str) -> IntegrationEventCommit {
    IntegrationEventCommit {
        sha: sha.to_string(),
        short_sha: sha.chars().take(7).collect(),
        subject: String::new(),
        author: None,
        authored_at: None,
        unavailable: true,
    }
}

/// One batched enrichment for ALL included SHAs (constant spawns, never one
/// per commit): first a single `cat-file --batch-check` prefilter, then one
/// `git log --no-walk` for the survivors. The prefilter is required because
/// `git log` exits 128 on ANY bad object and prints nothing, so one
/// unreachable SHA would otherwise blank the reachable ones. SHAs absent
/// from the output are unreachable in the local clone and report
/// `unavailable` — absence of a record is not evidence.
fn enrich_included_commits(
    source_dir: Option<&Path>,
    shas: &[String],
) -> Vec<IntegrationEventCommit> {
    let Some(dir) = source_dir else {
        return shas.iter().map(|sha| unavailable_commit(sha)).collect();
    };
    if shas.is_empty() {
        return Vec::new();
    }
    let stdin = shas.join("\n");
    let mut live: Vec<&String> = Vec::new();
    if let Ok(out) = git::run_with_stdin(dir, &["cat-file", "--batch-check"], stdin.as_bytes()) {
        if out.status == 0 {
            let mut missing: HashSet<String> = HashSet::new();
            for line in out.stdout_str().lines() {
                let mut cols = line.split_whitespace();
                if let (Some(sha), Some(kind)) = (cols.next(), cols.next()) {
                    if kind == "missing" {
                        missing.insert(sha.to_string());
                    }
                }
            }
            live = shas.iter().filter(|s| !missing.contains(*s)).collect();
        }
    }
    let mut found: HashMap<String, (String, String, String)> = HashMap::new();
    if !live.is_empty() {
        let mut args: Vec<String> = vec![
            "log".to_string(),
            "--no-walk".to_string(),
            "--format=%H%x1f%s%x1f%an%x1f%aI%x1e".to_string(),
        ];
        args.extend(live.iter().map(|s| (*s).clone()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        if let Ok(out) = git::run(dir, &refs) {
            if out.status == 0 {
                for record in out.stdout_str().split('\u{1e}') {
                    let record = record.trim_start_matches('\n').trim();
                    if record.is_empty() {
                        continue;
                    }
                    let mut parts = record.split('\u{1f}');
                    if let (Some(sha), Some(subject), Some(author), Some(at)) =
                        (parts.next(), parts.next(), parts.next(), parts.next())
                    {
                        if !sha.is_empty() {
                            found.insert(
                                sha.to_string(),
                                (subject.to_string(), author.to_string(), at.to_string()),
                            );
                        }
                    }
                }
            }
        }
    }
    shas.iter()
        .map(|sha| match found.get(sha) {
            Some((subject, author, at)) => IntegrationEventCommit {
                sha: sha.clone(),
                short_sha: sha.chars().take(7).collect(),
                subject: subject.clone(),
                author: if author.is_empty() {
                    None
                } else {
                    Some(author.clone())
                },
                authored_at: if at.is_empty() {
                    None
                } else {
                    Some(at.clone())
                },
                unavailable: false,
            },
            None => unavailable_commit(sha),
        })
        .collect()
}

fn rename_new_keys(path: &str) -> Vec<String> {
    if let (Some(open), Some(close)) = (path.find('{'), path.find('}')) {
        if open < close {
            let prefix = &path[..open];
            let suffix = &path[close + 1..];
            let inside = &path[open + 1..close];
            if let Some((_, new_part)) = inside.split_once(" => ") {
                return vec![format!("{prefix}{new_part}{suffix}")];
            }
        }
    }
    path.split(" => ")
        .last()
        .map(|s| vec![s.trim().to_string()])
        .unwrap_or_default()
}

/// The main commit's own changed paths AND its identifying metadata in ONE
/// `git show` spawn: a one-line `%H/%s/%an/%aI` header joined with the raw
/// status letters and numstat counts (same `--numstat` line grammar as
/// [`numstat_map`], which stays the single-commit helper for details).
/// `None` when the commit cannot be read; an empty-change commit yields
/// `Some` with zero totals, which is truthful, not unavailable. The header is
/// validated (first field must be the requested SHA) and the subject is kept
/// verbatim — never the SHA standing in as the subject.
fn main_commit_changes(
    target_dir: &Path,
    sha: &str,
) -> Option<(
    Vec<IntegrationEventChange>,
    IntegrationEventTotals,
    IntegrationEventCommit,
)> {
    let out = git::run(
        target_dir,
        &[
            "show",
            "--format=%H%x1f%s%x1f%an%x1f%aI",
            "--numstat",
            "--raw",
            "-M",
            sha,
        ],
    )
    .ok()?;
    if out.status != 0 {
        return None;
    }
    let text = out.stdout_str();
    let mut lines = text.lines();
    // Exactly one header line: `%s`/`%an` never emit newlines, so a subject
    // containing a tab cannot leak into the numstat parse below.
    let header = lines.next().unwrap_or("");
    let mut header_parts = header.split('\u{1f}');
    let (Some(header_sha), Some(subject), Some(author), Some(authored_at)) = (
        header_parts.next(),
        header_parts.next(),
        header_parts.next(),
        header_parts.next(),
    ) else {
        return None;
    };
    if header_sha != sha {
        return None;
    }
    let main_commit = IntegrationEventCommit {
        sha: sha.to_string(),
        short_sha: sha.chars().take(7).collect(),
        subject: subject.to_string(),
        author: if author.is_empty() {
            None
        } else {
            Some(author.to_string())
        },
        authored_at: if authored_at.is_empty() {
            None
        } else {
            Some(authored_at.to_string())
        },
        unavailable: false,
    };
    let mut stats: HashMap<String, (u32, u32)> = HashMap::new();
    let mut raws: Vec<(char, String)> = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix(':') {
            let Some((meta, paths)) = rest.split_once('\t') else {
                continue;
            };
            let status = meta.split_whitespace().last().unwrap_or("");
            let letter = status.chars().next().unwrap_or('M');
            let mut sides = paths.split('\t');
            let (Some(first), second) = (sides.next(), sides.next()) else {
                continue;
            };
            let display = if letter == 'R' || letter == 'C' {
                second.unwrap_or(first).to_string()
            } else {
                first.to_string()
            };
            if display.is_empty() {
                continue;
            }
            raws.push((letter, display));
        } else {
            let mut cols = line.split('\t');
            if let (Some(adds), Some(dels), Some(path)) = (cols.next(), cols.next(), cols.next()) {
                if path.is_empty() {
                    continue;
                }
                let counts = (
                    adds.parse::<u32>().unwrap_or(0),
                    dels.parse::<u32>().unwrap_or(0),
                );
                stats.insert(path.to_string(), counts);
                if path.contains(" => ") {
                    for key in rename_new_keys(path) {
                        stats.entry(key).or_insert(counts);
                    }
                }
            }
        }
    }
    let mut changes: Vec<IntegrationEventChange> = Vec::new();
    let (mut total_add, mut total_del) = (0u32, 0u32);
    for (letter, path) in &raws {
        let (adds, dels) = stats.get(path).copied().unwrap_or((0, 0));
        total_add += adds;
        total_del += dels;
        changes.push(IntegrationEventChange {
            path: path.clone(),
            change: match letter {
                'A' => IntegrationEventChangeKind::Added,
                'D' => IntegrationEventChangeKind::Deleted,
                'R' => IntegrationEventChangeKind::Renamed,
                _ => IntegrationEventChangeKind::Modified,
            },
            additions: adds,
            deletions: dels,
        });
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    let totals = IntegrationEventTotals {
        files: changes.len() as u32,
        additions: total_add,
        deletions: total_del,
    };
    Some((changes, totals, main_commit))
}

fn unavailable_audit(
    branch: &str,
    subsystem_id: &str,
    main_commit_sha: &str,
    detail: String,
) -> IntegrationEventAudit {
    IntegrationEventAudit {
        main_commit_sha: main_commit_sha.to_string(),
        main_commit: None,
        kind: IntegrationHistoryKind::Merged,
        subsystem_id: subsystem_id.to_string(),
        target_branch: branch.to_string(),
        actor: "unknown".to_string(),
        occurred_at: now_rfc3339(),
        detail,
        pull_request_number: None,
        inherited_from_branch: None,
        included_commits: Vec::new(),
        changes: Vec::new(),
        totals: None,
        provenance: Vec::new(),
        reverted_main_commit_sha: None,
        main_commit_unavailable: true,
        data_source: DataSource::Live,
    }
}

/// One integration event re-derived from git: the event resolves through the
/// same tip-keyed [`history::cached_replay`] the history list uses (so the two
/// agree on kind/actor/date/detail), `includedCommits` are exactly the
/// `Source-Commit` trailers already parsed onto the event, `changes` come
/// from one `git show`, and `mainCommit` (subject/author/authored date)
/// rides that same `git show` as a one-line format header — no extra spawn.
/// A missing/unreachable event is an ordinary truthful result, never an
/// `Err`.
pub fn integration_event_audit_live(
    target_dir: &Path,
    source_dir: Option<&Path>,
    registry: &Registry,
    target_branch: &str,
    subsystem_id: &str,
    main_commit_sha: &str,
) -> AppResult<IntegrationEventAudit> {
    let branch = require_allowed_target(registry, target_branch)?;
    find_live_subsystem(registry, subsystem_id)?;
    let wanted = main_commit_sha.trim();
    // The ref NAME (not its SHA) keys the tip-keyed replay, exactly like the
    // history list and status reads, so all three share one scan per tip.
    let Some(reference) = history::resolve_ref(target_dir, &branch) else {
        return Err(branch_not_permitted(
            &branch,
            "branch does not exist in the target checkout",
        ));
    };
    let replayed = history::cached_replay(target_dir, &reference);
    let Some(full) = git::rev_parse(target_dir, wanted) else {
        return Ok(unavailable_audit(
            &branch,
            subsystem_id,
            wanted,
            format!(
                "{wanted} could not be resolved in the target checkout: it is outside the scanned range or was stripped by a squash. Nothing was inferred; changes and included commits are empty."
            ),
        ));
    };
    let Some(event) = replayed
        .history
        .iter()
        .find(|e| e.commit == full && e.subsystem.as_deref() == Some(subsystem_id))
    else {
        return Ok(unavailable_audit(
            &branch,
            subsystem_id,
            &full,
            format!(
                "{} was not found in the reachable history of '{branch}' for subsystem '{subsystem_id}': no integration event trailers reference it here (a squash-merge strips trailers, or it is outside the scanned range). Nothing was inferred; changes and included commits are empty.",
                full.chars().take(7).collect::<String>(),
            ),
        ));
    };
    let (kind, detail) = match event.kind {
        history::EventType::Integrate => (IntegrationHistoryKind::Merged, event.subject.clone()),
        history::EventType::Revert => (
            IntegrationHistoryKind::Reverted,
            format!(
                "Reverted {}. {}",
                event
                    .reverts_integration
                    .clone()
                    .unwrap_or_else(|| "an integration".to_string()),
                event.subject
            ),
        ),
        history::EventType::Restore => (IntegrationHistoryKind::Restored, event.subject.clone()),
    };
    let reverted_main_commit_sha = if matches!(event.kind, history::EventType::Revert) {
        event.reverts_integration.clone()
    } else {
        None
    };
    let provenance: Vec<IntegrationEventProvenance> = replayed
        .details
        .get(&event.commit)
        .map(|d| verbatim_trailer_block(&d.message))
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| IntegrationEventProvenance { key, value })
        .collect();
    let included_commits = enrich_included_commits(source_dir, &event.source_commits);
    let (changes, totals, main_commit) = match main_commit_changes(target_dir, &event.commit) {
        Some((changes, totals, main_commit)) => (changes, Some(totals), Some(main_commit)),
        None => {
            return Ok(IntegrationEventAudit {
                main_commit_sha: event.commit.clone(),
                // The commit body could not be read, so its metadata cannot
                // be described either: null, never fabricated.
                main_commit: None,
                kind,
                subsystem_id: event
                    .subsystem
                    .clone()
                    .unwrap_or_else(|| subsystem_id.to_string()),
                target_branch: event.target.clone().unwrap_or_else(|| branch.clone()),
                actor: if event.author.trim().is_empty() {
                    "unknown".to_string()
                } else {
                    event.author.clone()
                },
                occurred_at: if event.committed_iso.is_empty() {
                    now_rfc3339()
                } else {
                    event.committed_iso.clone()
                },
                detail: format!(
                    "{} resolved as an event but its commit body could not be read. Nothing was inferred; changes are empty.",
                    event.commit.chars().take(7).collect::<String>(),
                ),
                pull_request_number: None,
                inherited_from_branch: None,
                included_commits,
                changes: Vec::new(),
                totals: None,
                provenance,
                reverted_main_commit_sha,
                main_commit_unavailable: true,
                data_source: DataSource::Live,
            });
        }
    };
    Ok(IntegrationEventAudit {
        main_commit_sha: event.commit.clone(),
        main_commit,
        kind,
        subsystem_id: event
            .subsystem
            .clone()
            .unwrap_or_else(|| subsystem_id.to_string()),
        target_branch: event.target.clone().unwrap_or_else(|| branch.clone()),
        actor: if event.author.trim().is_empty() {
            "unknown".to_string()
        } else {
            event.author.clone()
        },
        occurred_at: if event.committed_iso.is_empty() {
            now_rfc3339()
        } else {
            event.committed_iso.clone()
        },
        detail,
        pull_request_number: None,
        inherited_from_branch: None,
        included_commits,
        changes,
        totals,
        provenance,
        reverted_main_commit_sha,
        main_commit_unavailable: false,
        data_source: DataSource::Live,
    })
}

pub(crate) struct SourceCommit {
    pub(crate) sha: String,
    pub(crate) subject: String,
    pub(crate) author_name: String,
    pub(crate) authored_at: String,
    pub(crate) committed_at: String,
    pub(crate) committed_unix: i64,
    pub(crate) parents: Vec<String>,
    pub(crate) files_changed: u32,
    pub(crate) additions: u32,
    pub(crate) deletions: u32,
    pub(crate) touches_manifest: bool,
    pub(crate) deltas: Vec<(String, String, char)>,
}

pub(crate) fn parse_source_log(repo_dir: &Path, branch: &str) -> AppResult<Vec<SourceCommit>> {
    source_log(repo_dir, branch, false)
}

fn source_log(repo_dir: &Path, branch: &str, first_parent: bool) -> AppResult<Vec<SourceCommit>> {
    const REC: char = '\u{1e}';
    const FIELD: char = '\u{1f}';
    let fmt = format!(
        "--format=%H{FIELD}%P{FIELD}%an{FIELD}%ae{FIELD}%aI{FIELD}%cI{FIELD}%ct{FIELD}%s{REC}"
    );
    let max_count = format!("--max-count={MAX_SOURCE_COMMITS}");
    let mut args = vec![
        "log",
        max_count.as_str(),
        &fmt,
        "--raw",
        // Full blob ids: `history_of` compares content identity across repos.
        "--no-abbrev",
        // Rename rows carry two paths after one tab, which the row parser
        // cannot split; as A+D pairs every consumer sees real paths.
        "--no-renames",
    ];
    if first_parent {
        args.push("--first-parent");
    }
    args.push(branch);
    let out = git::run(repo_dir, &args).map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not read source branch '{branch}': {}",
                git::sanitize(&err.to_string())
            ),
        )
        .with_remedy("Confirm the subsystem sourceBranch exists and is readable.")
    })?;
    if out.status != 0 {
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Could not read source branch '{branch}': {}",
                git::sanitize(&out.stderr)
            ),
        )
        .with_remedy("Confirm the subsystem sourceBranch exists and is readable."));
    }
    // `git log --raw` interleaves one format header per commit with its raw
    // change rows. Headers always contain FIELD separators; change rows never
    // do, so scan line-wise instead of splitting on REC, and keep each commit's
    // blob ids for the content-level classification below.
    let text = out.stdout_str();
    let mut commits: Vec<SourceCommit> = Vec::new();
    let mut current: Option<SourceCommit> = None;
    let flush = |current: &mut Option<SourceCommit>, commits: &mut Vec<SourceCommit>| {
        if let Some(commit) = current.take() {
            if commit.sha.len() == 40 || commit.sha.len() == 64 {
                commits.push(commit);
            }
        }
    };
    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches([REC, '\r']).to_string();
        if line.contains(FIELD) {
            flush(&mut current, &mut commits);
            let mut parts = line.split(FIELD);
            let sha = parts.next().unwrap_or("").trim().to_string();
            if sha.is_empty() {
                continue;
            }
            let parents_raw = parts.next().unwrap_or("");
            let author_name = parts.next().unwrap_or("").to_string();
            let _author_email = parts.next().unwrap_or("");
            let authored_at = parts.next().unwrap_or("").to_string();
            let committed_at = parts.next().unwrap_or("").to_string();
            let committed_unix = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
            let subject = parts.next().unwrap_or("").to_string();
            let parents: Vec<String> = parents_raw.split_whitespace().map(str::to_string).collect();
            current = Some(SourceCommit {
                sha,
                subject,
                author_name,
                authored_at,
                committed_at,
                committed_unix,
                parents,
                files_changed: 0,
                additions: 0,
                deletions: 0,
                touches_manifest: false,
                deltas: Vec::new(),
            });
        } else if let Some(commit) = current.as_mut() {
            let row = line.trim();
            let Some((meta, path)) = row.split_once('\t') else {
                continue;
            };
            let mut cols = meta.split(' ');
            let (Some(_old_mode), Some(_new_mode), Some(_old_sha), Some(new_sha), Some(status)) = (
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
                cols.next(),
            ) else {
                continue;
            };
            let Some(kind) = status.chars().next() else {
                continue;
            };
            commit.files_changed += 1;
            if path == "package.json" || path.ends_with("/package.json") {
                commit.touches_manifest = true;
            }
            commit
                .deltas
                .push((path.to_string(), new_sha.to_string(), kind));
        }
    }
    flush(&mut current, &mut commits);
    Ok(commits)
}

fn relative_now(iso: &str) -> String {
    let now = chrono::Utc::now();
    match fixtures::parse_time(iso) {
        Some(then) => fixtures::relative_time(then, now),
        None => "unknown".to_string(),
    }
}

fn promotion_note_for(status: &CommitStatus) -> Option<String> {
    match status.kind {
        CommitStatusKind::Eligible => Some(
            "Promoting applies the first-parent delta of this exact SHA only. Files copied whole may include a skipped commit's edits to those same files; a skipped commit's edits to files no selected commit touches are not written."
                .to_string(),
        ),
        CommitStatusKind::Integrated => Some(
            "Already recorded as integrated on this target branch. Re-promoting the same SHA to the same branch is blocked; a different branch is still allowed."
                .to_string(),
        ),
        CommitStatusKind::Reverted => Some(
            "Previously integrated and then reverted on this target branch. A deliberate re-promotion opens a new integration PR."
                .to_string(),
        ),
        CommitStatusKind::PrOpen => Some(
            "An integration PR for this SHA is already open. There is no need to dispatch another run.".to_string(),
        ),
        _ => None,
    }
}

/// `path -> blob sha` for a revision, in one call. Blob ids are
/// content-addressed, so equal ids mean byte-identical files: comparing the
/// source and target trees is an exact divergence check with no content
/// diffing — and no add-vs-modify ambiguity, which the subsystem's split
/// history otherwise creates.
fn tree_blobs(repo_dir: &Path, rev: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(out) = git::run(repo_dir, &["ls-tree", "-r", rev]) else {
        return map;
    };
    if out.status != 0 {
        return map;
    }
    for line in out.stdout_str().lines() {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        if let Some(sha) = meta.split(' ').nth(2) {
            map.insert(path.to_string(), sha.to_string());
        }
    }
    map
}

fn latest_toucher_index(commits: &[SourceCommit]) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    for (index, commit) in commits.iter().enumerate() {
        for (path, _, _) in &commit.deltas {
            map.entry(path.clone()).or_insert(index);
        }
    }
    map
}

/// Paths the engine never writes wholesale: the manifest is merged as a
/// dependency delta and lockfiles are deferred to the deployment (which
/// regenerates them), so recency must not treat them as "the target is newer".
fn is_dependency_path(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "package.json"
            | "package-lock.json"
            | "npm-shrinkwrap.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
    )
}

/// Only paths the promotion could write may vote on commit status.
/// Lockfiles, main-owned, secret and rejected paths never reach the commit,
/// so they are excluded from the comparison.
fn is_status_voting(status: classify::PathStatus) -> bool {
    matches!(
        status,
        classify::PathStatus::Apply
            | classify::PathStatus::ExplicitReview
            | classify::PathStatus::DependencyMerge
    )
}

fn all_superseded(paths: &[&String], latest: &HashMap<String, usize>, index: usize) -> bool {
    !paths.is_empty()
        && paths
            .iter()
            .all(|path| latest.get(*path).is_some_and(|newer| *newer < index))
}

/// Epoch of the target branch's most recent change per path (bounded scan).
/// Compared against a source commit's date it answers "which side is newer?"
/// for DISPLAY ORDER ONLY — never correctness (blob membership decides, E3).
/// Returns `(map, truncated)`: `truncated` is true when the 800-commit scan
/// hit its limit. A truncated scan that finds no hit is `Unknown` plus the
/// truncation flag, never `Integrated` (E3) — history is deeper than the scan
/// and dates may be skewed.
fn target_path_dates(target_dir: &Path, target_branch: &str) -> (HashMap<String, i64>, bool) {
    const LIMIT: usize = 800;
    let mut map: HashMap<String, i64> = HashMap::new();
    let Ok(out) = git::run(
        target_dir,
        &[
            "log",
            "--max-count=800",
            "--format=\u{1e}%ct",
            "--name-only",
            target_branch,
        ],
    ) else {
        return (map, false);
    };
    if out.status != 0 {
        return (map, false);
    }
    let mut commits_seen = 0usize;
    for chunk in out.stdout_str().split('\u{1e}') {
        let mut lines = chunk.lines();
        let Some(epoch) = lines
            .next()
            .map(str::trim)
            .and_then(|s| s.parse::<i64>().ok())
        else {
            continue;
        };
        commits_seen += 1;
        for path in lines {
            let path = path.trim();
            if !path.is_empty() {
                map.entry(path.to_string()).or_insert(epoch);
            }
        }
    }
    (map, commits_seen >= LIMIT)
}

/// `+`/`-` counts for the returned page only: `--numstat` across a whole history
/// costs ~0.8s because it line-diffs every commit, while one `--no-walk` pass
/// over page-sized input costs ~0.05s.
fn numstat_totals(repo_dir: &Path, shas: &[&str]) -> HashMap<String, (u32, u32)> {
    let mut map = HashMap::new();
    if shas.is_empty() {
        return map;
    }
    let mut args: Vec<String> = vec![
        "log".to_string(),
        "--no-walk".to_string(),
        "--numstat".to_string(),
        "--format=%x1e%H".to_string(),
    ];
    args.extend(shas.iter().map(|sha| (*sha).to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let Ok(out) = git::run(repo_dir, &refs) else {
        return map;
    };
    if out.status != 0 {
        return map;
    }
    for chunk in out.stdout_str().split('\u{1e}') {
        let mut lines = chunk.lines();
        let Some(sha) = lines.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let (mut adds, mut dels) = (0u32, 0u32);
        for row in lines {
            let mut cols = row.trim().split('\t');
            let (Some(add), Some(del), Some(path)) = (cols.next(), cols.next(), cols.next()) else {
                continue;
            };
            if path.is_empty() {
                continue;
            }
            adds += add.parse::<u32>().unwrap_or(0);
            dels += del.parse::<u32>().unwrap_or(0);
        }
        map.insert(sha.to_string(), (adds, dels));
    }
    map
}

pub fn commit_page_live(
    target_dir: &Path,
    source_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    query: &CommitQuery,
) -> AppResult<CommitPage> {
    let entry = find_live_subsystem(registry, &query.subsystem_id)?;
    let target_branch = require_allowed_target(registry, &query.target_branch)?;
    if !git::branch_exists(target_dir, &target_branch) {
        return Err(branch_not_permitted(
            &target_branch,
            "branch does not exist in the target checkout",
        ));
    }
    let commits = parse_source_log(source_dir, &entry.source_branch)?;
    // Comparing the two trees by blob id is an exact divergence check — no
    // content diffing, and no add-vs-modify ambiguity.
    let source_tree = tree_blobs(source_dir, &entry.source_branch);
    let target_blobs = tree_blobs(target_dir, &target_branch);
    let path_latest = latest_toucher_index(&commits);
    let (target_dates, path_dates_truncated) = target_path_dates(target_dir, &target_branch);
    let shas: Vec<String> = commits.iter().map(|c| c.sha.clone()).collect();
    let results = history::compute_branch_status(target_dir, &target_branch, &entry.id, &shas);
    let converted = branch_results_to_statuses(target_dir, &target_branch, &results);
    let mut status_by_sha: HashMap<String, CommitStatus> = HashMap::new();
    for (result, status) in results.iter().zip(converted) {
        status_by_sha.insert(result.sha.clone(), status);
    }
    let open_prs = github
        .list_open_pull_requests(&registry.main_repository, &target_branch)
        .map_err(gh_error_to_app)?;
    let mut open_by_sha: HashMap<String, PullRequestInfo> = HashMap::new();
    for pr in &open_prs {
        for sha in history::extract_source_commits(&pr.body) {
            open_by_sha.entry(sha).or_insert_with(|| pr.clone());
        }
    }
    let search = query.search.trim().to_lowercase();
    let mut rows: Vec<CommitSummary> = Vec::new();
    for (index, commit) in commits.iter().enumerate() {
        if let Some(wanted) = query.shas.as_ref() {
            let hit = wanted
                .iter()
                .any(|sha| commit.sha.starts_with(sha) || sha == &commit.sha);
            if !hit {
                continue;
            }
        }
        if !search.is_empty()
            && !commit.subject.to_lowercase().contains(&search)
            && !commit.sha.to_lowercase().starts_with(&search)
            && !commit.author_name.to_lowercase().contains(&search)
        {
            continue;
        }
        let branch_status = status_by_sha.remove(&commit.sha);
        let mut status = match branch_status {
            Some(status) => status,
            None => CommitStatus::unknown(&target_branch, "Status unavailable for this SHA."),
        };
        if matches!(
            status.kind,
            CommitStatusKind::Eligible | CommitStatusKind::Unknown
        ) && open_by_sha.contains_key(&commit.sha)
        {
            if let Some(pr) = open_by_sha.get(&commit.sha) {
                status = open_pr_status(pr, &target_branch);
            }
        }
        if matches!(
            status.kind,
            CommitStatusKind::Eligible | CommitStatusKind::Unknown
        ) {
            let out_of_sync: Vec<&String> = commit
                .deltas
                .iter()
                // A deletion leaves nothing to compare: `None == None` would
                // read a removal the target never saw as "already applied", so
                // an add+delete bundle could not be handed over as a pair. Count
                // it as promotable — re-applying an absent file is a no-op.
                .filter(|(path, _, kind)| {
                    *kind == 'D' || {
                        let voted = classify::classify_path(path, &entry.integration_policy, None);
                        is_status_voting(voted.status)
                            && source_tree.get(path.as_str()) != target_blobs.get(path.as_str())
                    }
                })
                .map(|(path, _, _)| path)
                .collect();
            // Manifest and lockfile paths are delta-merged or deferred to the
            // deployment (which regenerates the lockfile), never written
            // wholesale (spec §6.4/§7), so a newer copy on the target does
            // not invalidate this commit — reconciliation decides those.
            let recency_paths: Vec<&String> = out_of_sync
                .iter()
                .copied()
                .filter(|path| !is_dependency_path(path))
                .collect();
            if out_of_sync.is_empty() {
                status = CommitStatus::of(
                    CommitStatusKind::Applied,
                    &target_branch,
                    Some(
                        "Every promotable file this commit changes already matches the target branch; deferred or never-imported paths are not counted. There is nothing to promote."
                            .to_string(),
                    ),
                    None,
                );
            } else if all_superseded(&out_of_sync, &path_latest, index) {
                status = CommitStatus::of(
                    CommitStatusKind::Superseded,
                    &target_branch,
                    Some(
                        "A newer commit on the source branch changes the same files; promoting this one would re-apply stale content."
                            .to_string(),
                    ),
                    None,
                );
            } else if !recency_paths.is_empty()
                && recency_paths.iter().all(|path| {
                    target_dates
                        .get(*path)
                        .is_some_and(|epoch| *epoch > commit.committed_unix)
                })
            {
                status = CommitStatus::of(
                    CommitStatusKind::Divergent,
                    &target_branch,
                    Some(
                        "The target branch's copies of every promotable file this commit changes were themselves changed more recently; promoting would revert newer work. (The dependency manifest is exempt — it is merged; deferred or never-imported paths are not counted.)"
                            .to_string(),
                    ),
                    None,
                );
            } else if path_dates_truncated
                && matches!(
                    status.kind,
                    CommitStatusKind::Eligible | CommitStatusKind::Unknown
                )
                && !out_of_sync.is_empty()
            {
                // Fail-closed (E3): history is deeper than the 800-commit scan
                // and dates may be skewed. A truncated scan that finds no
                // Divergent hit proves nothing — report Unknown plus the
                // truncation flag, never Integrated (and never a confident
                // Eligible). Dates determine display order only.
                status = CommitStatus::of(
                    CommitStatusKind::Unknown,
                    &target_branch,
                    Some(
                        "The 800-commit target history scan hit its limit without finding a newer change; history is deeper than the scan, so this SHA needs verification (Unknown, not Eligible).".to_string(),
                    ),
                    None,
                );
            }
        }
        if let Some(filter) = query.status_filter {
            if status.kind != filter {
                continue;
            }
        }
        let short: String = commit.sha.chars().take(7).collect();
        rows.push(CommitSummary {
            sha: commit.sha.clone(),
            short_sha: short,
            subject: commit.subject.clone(),
            author_name: commit.author_name.clone(),
            author_login: None,
            authored_at: commit.authored_at.clone(),
            committed_at: commit.committed_at.clone(),
            relative_time: relative_now(&commit.committed_at),
            status,
            parents: commit.parents.clone(),
            is_merge: commit.parents.len() > 1,
            eligible: true,
            files_changed: commit.files_changed,
            additions: commit.additions,
            deletions: commit.deletions,
            tags: Vec::new(),
            touches_manifest: commit.touches_manifest,
        });
    }
    match query.sort {
        crate::github::models::CommitSort::Newest => {
            rows.sort_by(|a, b| b.committed_at.cmp(&a.committed_at));
        }
        crate::github::models::CommitSort::Oldest => {
            rows.sort_by(|a, b| a.committed_at.cmp(&b.committed_at));
        }
    }
    let total = rows.len() as u32;
    let page_size = query.page_size.clamp(5, 200);
    let page = query.page.max(1);
    let start = ((page - 1) * page_size) as usize;
    let mut paged: Vec<CommitSummary> = rows
        .into_iter()
        .skip(start)
        .take(page_size as usize)
        .collect();
    let page_shas: Vec<&str> = paged.iter().map(|row| row.sha.as_str()).collect();
    let stats = numstat_totals(source_dir, &page_shas);
    for row in &mut paged {
        if let Some((adds, dels)) = stats.get(&row.sha) {
            row.additions = *adds;
            row.deletions = *dels;
        }
    }
    Ok(CommitPage {
        target_branch,
        source_branch: entry.source_branch.clone(),
        subsystem_id: entry.id.clone(),
        total,
        page,
        page_size,
        commits: paged,
        fetched_at: now_rfc3339(),
        data_source: DataSource::Live,
        path_dates_truncated,
    })
}

fn numstat_map(repo_dir: &Path, sha: &str) -> HashMap<String, (u32, u32, bool)> {
    let mut map = HashMap::new();
    let out = git::run(repo_dir, &["show", "--numstat", "--format=", sha]);
    let Ok(out) = out else {
        return map;
    };
    if out.status != 0 {
        return map;
    }
    for line in out.stdout_str().lines() {
        let mut cols = line.split('\t');
        let (Some(adds), Some(dels), Some(path)) = (cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        let binary = adds == "-" || dels == "-";
        let additions = adds.parse::<u32>().unwrap_or(0);
        let deletions = dels.parse::<u32>().unwrap_or(0);
        map.insert(path.to_string(), (additions, deletions, binary));
        if path.contains(" => ") {
            let parts: Vec<&str> = path.split(" => ").collect();
            if let Some(last) = parts.last() {
                map.insert((*last).to_string(), (additions, deletions, binary));
            }
            if let Some(first) = parts.first() {
                map.insert((*first).to_string(), (additions, deletions, binary));
            }
        }
    }
    map
}

fn file_policy_for(
    entry: &SubsystemEntry,
    repo_dir: &Path,
    sha: &str,
    base: Option<&str>,
    delta_path: &str,
    status_char: char,
) -> (FileIntegrationPolicy, Option<String>) {
    let mode = if status_char == 'D' {
        base.and_then(|b| git::path_mode(repo_dir, b, delta_path))
    } else {
        git::path_mode(repo_dir, sha, delta_path)
    };
    let classification =
        classify::classify_path(delta_path, &entry.integration_policy, mode.as_deref());
    let policy = match classification.status {
        classify::PathStatus::Apply => FileIntegrationPolicy::Apply,
        classify::PathStatus::DependencyMerge => FileIntegrationPolicy::ReviewRequired,
        classify::PathStatus::Regenerate | classify::PathStatus::LockfileDeferred => {
            FileIntegrationPolicy::RegenerateInTarget
        }
        classify::PathStatus::MainOwned => FileIntegrationPolicy::MainOwned,
        classify::PathStatus::SecretExcluded | classify::PathStatus::Rejected => {
            FileIntegrationPolicy::NeverImport
        }
        classify::PathStatus::ExplicitReview => FileIntegrationPolicy::ReviewRequired,
    };
    (policy, Some(classification.reason))
}

fn dependency_section_of(section: &str) -> DependencySection {
    match section {
        "devDependencies" => DependencySection::DevDependencies,
        "peerDependencies" => DependencySection::PeerDependencies,
        "optionalDependencies" => DependencySection::OptionalDependencies,
        _ => DependencySection::Dependencies,
    }
}

fn dependency_changes_for(repo_dir: &Path, sha: &str, base: Option<&str>) -> Vec<DependencyChange> {
    let new_pkg = git::read_blob(repo_dir, sha, "package.json")
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let old_pkg = base
        .and_then(|b| git::read_blob(repo_dir, b, "package.json"))
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if old_pkg == new_pkg {
        return Vec::new();
    }
    let delta = deps::compute_dependency_delta(&old_pkg, &new_pkg);
    let mut out = Vec::new();
    for add in delta.added {
        out.push(DependencyChange {
            name: add.name,
            section: dependency_section_of(&add.section),
            change: DependencyChangeKind::Added,
            from_range: None,
            to_range: Some(add.range),
            conflict: false,
            note: None,
        });
    }
    for update in delta.updated {
        out.push(DependencyChange {
            name: update.name,
            section: dependency_section_of(&update.section),
            change: DependencyChangeKind::Updated,
            from_range: Some(update.from),
            to_range: Some(update.to),
            conflict: false,
            note: None,
        });
    }
    for removed in delta.removed {
        out.push(DependencyChange {
            name: removed.name,
            section: dependency_section_of(&removed.section),
            change: DependencyChangeKind::Removed,
            from_range: Some(removed.range),
            to_range: None,
            conflict: false,
            note: None,
        });
    }
    for moved in delta.moved {
        out.push(DependencyChange {
            name: moved.name.clone(),
            section: dependency_section_of(&moved.to_section),
            change: DependencyChangeKind::Updated,
            from_range: Some(moved.from),
            to_range: Some(moved.to),
            conflict: false,
            note: Some(format!(
                "Moved from {} to {}.",
                moved.from_section, moved.to_section
            )),
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub fn commit_details_live(
    target_dir: &Path,
    source_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    subsystem_id: &str,
    sha: &str,
    target_branch: &str,
) -> AppResult<CommitDetails> {
    let entry = find_live_subsystem(registry, subsystem_id)?;
    let target_branch = require_allowed_target(registry, target_branch)?;
    if git::rev_parse(source_dir, sha).is_none() {
        return Err(AppError::not_found(format!(
            "Commit '{sha}' was not found on {}.",
            entry.repository
        ))
        .with_remedy(
            "Refresh the commit list; the branch may have moved or the SHA may be mistyped.",
        ));
    }
    let show = git::run(
        source_dir,
        &[
            "show",
            "-s",
            "--format=%H\x1f%P\x1f%an\x1f%ae\x1f%aI\x1f%cI\x1f%s\x1f%b",
            sha,
        ],
    )
    .map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!("Could not read commit '{sha}'."),
        )
        .with_remedy("Refresh and retry.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    if show.status != 0 {
        return Err(
            AppError::not_found(format!("Commit '{sha}' was not found."))
                .with_remedy("Refresh the commit list and try again."),
        );
    }
    let line = show.stdout_str().lines().next().unwrap_or("").to_string();
    let mut parts = line.split('\u{1f}');
    let full_sha = parts.next().unwrap_or(sha).to_string();
    let parents_raw = parts.next().unwrap_or("");
    let author_name = parts.next().unwrap_or("").to_string();
    let _author_email = parts.next().unwrap_or("");
    let authored_at = parts.next().unwrap_or("").to_string();
    let committed_at = parts.next().unwrap_or("").to_string();
    let subject = parts.next().unwrap_or("").to_string();
    let body = {
        let marker = format!("{subject}\x1f");
        show.stdout_str()
            .split_once(&marker)
            .map(|(_, tail)| tail.trim().to_string())
            .unwrap_or_default()
    };
    let parents: Vec<String> = parents_raw.split_whitespace().map(str::to_string).collect();
    let is_merge = parents.len() > 1;
    // Single `name_status_delta` pass: the entries carry rename metadata and
    // are reused below (a second call would re-spawn `rev-parse` + `diff`).
    let (base_sha, deltas) = git::name_status_delta(source_dir, &full_sha).map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!("Could not read the changed files for '{sha}'."),
        )
        .with_remedy("Refresh and retry.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    let stats = numstat_map(source_dir, &full_sha);
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut total_add = 0_u32;
    let mut total_del = 0_u32;
    for delta in &deltas {
        let (path, previous, change) = match delta.status {
            'A' => (
                delta.path.clone().unwrap_or_default(),
                None,
                FileChangeKind::Added,
            ),
            'D' => (
                delta.path.clone().unwrap_or_default(),
                None,
                FileChangeKind::Deleted,
            ),
            'R' => (
                delta.new_path.clone().unwrap_or_default(),
                delta.old_path.clone(),
                FileChangeKind::Renamed,
            ),
            'C' => (
                delta.new_path.clone().unwrap_or_default(),
                delta.old_path.clone(),
                FileChangeKind::Added,
            ),
            'T' => (
                delta.path.clone().unwrap_or_default(),
                None,
                FileChangeKind::Modified,
            ),
            _ => (
                delta.path.clone().unwrap_or_default(),
                None,
                FileChangeKind::Modified,
            ),
        };
        if path.is_empty() {
            continue;
        }
        let (adds, dels, binary) = stats.get(&path).copied().unwrap_or((0, 0, false));
        total_add += adds;
        total_del += dels;
        let (policy, note) = file_policy_for(
            entry,
            source_dir,
            &full_sha,
            base_sha.as_deref(),
            &path,
            delta.status,
        );
        files.push(ChangedFile {
            path,
            previous_path: previous,
            change,
            additions: adds,
            deletions: dels,
            binary,
            policy,
            policy_note: note,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let dependencies = dependency_changes_for(source_dir, &full_sha, base_sha.as_deref());
    // One open-PR listing serves both the status overlay and the related-PRs
    // section below: a second `gh api` round trip for the same (repo, branch)
    // pair inside one click can only add latency, never newer data.
    // Fail-closed exactly like before: when the listing fails, the status
    // degrades to Unknown and the related section stays empty (previously the
    // status call errored into Unknown and the related call failed open).
    let open_prs = github.list_open_pull_requests(&registry.main_repository, &target_branch);
    let status = match &open_prs {
        Ok(prs) => integration_statuses_with_prs(
            target_dir,
            registry,
            prs,
            &target_branch,
            Some(&entry.id),
            std::slice::from_ref(&full_sha),
        )
        .map(|mut v| {
            v.pop()
                .map(|s| s.status)
                .unwrap_or_else(|| CommitStatus::unknown(&target_branch, "Status unavailable."))
        })
        .unwrap_or_else(|_| {
            CommitStatus::unknown(&target_branch, "Status unavailable for this SHA.")
        }),
        Err(_) => CommitStatus::unknown(&target_branch, "Status unavailable for this SHA."),
    };
    let mut related: Vec<PullRequestRef> = Vec::new();
    if let Ok(prs) = &open_prs {
        for pr in prs {
            if history::pr_references_requested_shas(pr, &entry.id, std::slice::from_ref(&full_sha))
            {
                related.push(PullRequestRef {
                    number: pr.number.unwrap_or(0),
                    title: pr.title.clone(),
                    state: PrState::Open,
                    base_branch: pr.base.clone(),
                    html_url: pr.url.clone().unwrap_or_else(|| {
                        format!(
                            "https://github.com/{}/pull/{}",
                            registry.main_repository,
                            pr.number.unwrap_or(0)
                        )
                    }),
                    repository: registry.main_repository.clone(),
                });
            }
        }
    }
    let short: String = full_sha.chars().take(7).collect();
    let files_changed = files.len() as u32;
    Ok(CommitDetails {
        subsystem_id: entry.id.clone(),
        target_branch: target_branch.clone(),
        sha: full_sha.clone(),
        short_sha: short,
        subject: if subject.is_empty() {
            "(no subject)".to_string()
        } else {
            subject
        },
        body,
        author_name: if author_name.is_empty() {
            "unknown".to_string()
        } else {
            author_name
        },
        author_login: None,
        authored_at: if authored_at.is_empty() {
            committed_at.clone()
        } else {
            authored_at
        },
        committed_at: committed_at.clone(),
        relative_time: relative_now(&committed_at),
        status: status.clone(),
        tags: Vec::new(),
        files,
        files_changed,
        additions: total_add,
        deletions: total_del,
        dependencies,
        related_pull_requests: related,
        parents,
        is_merge,
        source_url: format!("https://github.com/{}/commit/{full_sha}", entry.repository),
        note: promotion_note_for(&status),
    })
}

fn parse_hunk_header(header: &str) -> (u32, u32) {
    let mut old_start = 1_u32;
    let mut new_start = 1_u32;
    for token in header.split_whitespace() {
        if let Some(rest) = token.strip_prefix('-') {
            let num = rest.split(',').next().unwrap_or("1");
            old_start = num.parse::<u32>().unwrap_or(1);
        } else if let Some(rest) = token.strip_prefix('+') {
            let num = rest.split(',').next().unwrap_or("1");
            new_start = num.parse::<u32>().unwrap_or(1);
        }
    }
    (old_start, new_start)
}

/// Metadata lines `git show -p` emits around hunks (`diff --git`, `index`,
/// `---`/`+++`, mode/rename headers, binary markers). Skipped only BEFORE a
/// file's first hunk: inside a hunk a removed line whose content starts with
/// `-- ` renders as `--- …` and must never be mistaken for a header.
fn is_diff_metadata(raw: &str) -> bool {
    raw.starts_with("diff --git")
        || raw.starts_with("index ")
        || raw.starts_with("--- ")
        || raw.starts_with("+++ ")
        || raw.starts_with("new file mode")
        || raw.starts_with("deleted file mode")
        || raw.starts_with("old mode")
        || raw.starts_with("new mode")
        || raw.starts_with("similarity index")
        || raw.starts_with("dissimilarity index")
        || raw.starts_with("rename ")
        || raw.starts_with("copy ")
        || raw.starts_with("Binary files")
}

/// Parse unified-diff hunk lines into hunks plus `+`/`−` counts. Shared by the
/// single-file diff and the whole-commit diff so the two can never disagree on
/// what a hunk is or how many lines it added.
pub(crate) fn parse_unified_hunks<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> (Vec<DiffHunk>, u32, u32) {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_line = 1_u32;
    let mut new_line = 1_u32;
    let mut additions = 0_u32;
    let mut deletions = 0_u32;
    for raw in lines {
        if current.is_none() && is_diff_metadata(raw) {
            continue;
        }
        if let Some(header) = raw.strip_prefix("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            let (old_start, new_start) = parse_hunk_header(header);
            old_line = old_start;
            new_line = new_start;
            current = Some(DiffHunk {
                header: format!("@@{header}"),
                lines: Vec::new(),
            });
            continue;
        }
        if raw.starts_with('\\') {
            continue;
        }
        let Some(hunk) = current.as_mut() else {
            continue;
        };
        if let Some(content) = raw.strip_prefix('+') {
            hunk.lines.push(DiffLine {
                kind: DiffLineKind::Added,
                old_line: None,
                new_line: Some(new_line),
                text: content.to_string(),
            });
            new_line += 1;
            additions += 1;
        } else if let Some(content) = raw.strip_prefix('-') {
            hunk.lines.push(DiffLine {
                kind: DiffLineKind::Removed,
                old_line: Some(old_line),
                new_line: None,
                text: content.to_string(),
            });
            old_line += 1;
            deletions += 1;
        } else if let Some(content) = raw.strip_prefix(' ') {
            hunk.lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(old_line),
                new_line: Some(new_line),
                text: content.to_string(),
            });
            old_line += 1;
            new_line += 1;
        } else if raw.is_empty() {
            hunk.lines.push(DiffLine {
                kind: DiffLineKind::Context,
                old_line: Some(old_line),
                new_line: Some(new_line),
                text: String::new(),
            });
            old_line += 1;
            new_line += 1;
        }
    }
    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }
    (hunks, additions, deletions)
}

/// Bound hunks to [`MAX_DIFF_LINES`] total lines across the file, returning
/// whether anything was dropped. Shared so single-file and whole-commit diffs
/// bound identically.
pub(crate) fn bound_hunks(hunks: &mut [DiffHunk]) -> bool {
    let mut budget = MAX_DIFF_LINES;
    let mut truncated = false;
    for hunk in hunks.iter_mut() {
        if budget == 0 {
            truncated = true;
            hunk.lines.clear();
            continue;
        }
        if hunk.lines.len() > budget {
            hunk.lines.truncate(budget);
            budget = 0;
            truncated = true;
        } else {
            budget -= hunk.lines.len();
        }
    }
    truncated
}

pub(crate) fn truncation_note() -> String {
    format!("Response bounded at {MAX_DIFF_LINES} lines to keep the window responsive; open the file on GitHub for the rest.")
}

/// Bounded unified diff for one path from the source repo.
pub fn commit_diff_live(
    source_dir: &Path,
    registry: &Registry,
    subsystem_id: &str,
    sha: &str,
    file_path: Option<&str>,
) -> AppResult<CommitDiff> {
    let entry = find_live_subsystem(registry, subsystem_id)?;
    if git::rev_parse(source_dir, sha).is_none() {
        return Err(AppError::not_found(format!(
            "Commit '{sha}' was not found on {}.",
            entry.repository
        ))
        .with_remedy("Refresh the commit list and try again."));
    }
    let deltas = git::name_status_delta(source_dir, sha)
        .map(|(_, entries)| entries)
        .map_err(|err| {
            AppError::new(
                ErrorCode::RepositoryInaccessible,
                "Could not read the changed files for this commit.",
            )
            .with_remedy("Refresh and retry.")
            .with_details(git::sanitize(&err.to_string()))
        })?;
    let mut available: Vec<String> = deltas
        .iter()
        .map(|d| {
            d.new_path
                .clone()
                .or_else(|| d.path.clone())
                .unwrap_or_default()
        })
        .filter(|p| !p.is_empty())
        .collect();
    available.sort();
    available.dedup();
    if available.is_empty() {
        return Err(AppError::not_found("This commit has no file changes.")
            .with_remedy("Pick a different commit from the list."));
    }
    let chosen = match file_path {
        Some(requested) => {
            if requested.trim().is_empty() {
                available.first().cloned().unwrap_or_default()
            } else {
                // Reject traversal before touching git.
                let safe = classify::inspect_path_safety(requested).map_err(|reason| {
                    AppError::invalid_request(format!(
                        "Refusing unsafe path '{requested}': {reason}."
                    ))
                    .with_remedy("Choose a file listed under 'Files Changed'.")
                })?;
                if !available.iter().any(|p| p == &safe) {
                    return Err(AppError::not_found(format!(
                        "No diff is available for '{requested}' in commit {sha}."
                    ))
                    .with_remedy(
                        "Choose a file listed under 'Files Changed' in the commit details panel.",
                    ));
                }
                safe
            }
        }
        None => available.first().cloned().unwrap_or_default(),
    };
    let full_sha = git::rev_parse(source_dir, sha).unwrap_or_else(|| sha.to_string());
    let out = git::run(
        source_dir,
        &[
            "show",
            "--format=",
            "--no-color",
            "--unified=3",
            &full_sha,
            "--",
            &chosen,
        ],
    )
    .map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            "Could not read the diff.",
        )
        .with_remedy("Refresh and retry.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    if out.status != 0 {
        return Err(AppError::not_found(format!(
            "No diff is available for '{chosen}' in commit {sha}."
        ))
        .with_remedy("Choose a file listed under 'Files Changed'."));
    }
    let text = out.stdout_str();
    if text.contains("Binary files") && text.contains("differ") {
        return Ok(CommitDiff {
            subsystem_id: entry.id.clone(),
            sha: full_sha.clone(),
            file_path: chosen.clone(),
            language: fixtures::language_for(&chosen).to_string(),
            hunks: Vec::new(),
            total_lines: 0,
            additions: 0,
            deletions: 0,
            binary: true,
            truncated: false,
            source_url: format!(
                "https://github.com/{}/commit/{full_sha}#diff-{}",
                entry.repository, chosen
            ),
            note: Some("Binary file: no text diff is shown.".to_string()),
        });
    }
    let (mut hunks, additions, deletions) = parse_unified_hunks(text.lines());
    if hunks.is_empty() {
        return Err(AppError::not_found(format!(
            "No diff is available for '{chosen}' in commit {sha}."
        ))
        .with_remedy("Choose a file listed under 'Files Changed' in the commit details panel."));
    }
    let total_lines: u32 = hunks.iter().map(|h| h.lines.len() as u32).sum();
    let truncated = bound_hunks(&mut hunks);
    Ok(CommitDiff {
        subsystem_id: entry.id.clone(),
        sha: full_sha.clone(),
        file_path: chosen.clone(),
        language: fixtures::language_for(&chosen).to_string(),
        hunks,
        total_lines,
        additions,
        deletions,
        binary: false,
        truncated,
        source_url: format!(
            "https://github.com/{}/commit/{full_sha}#diff-{}",
            entry.repository, chosen
        ),
        note: if truncated {
            Some(truncation_note())
        } else {
            None
        },
    })
}

/// Path for a `---`/`+++` diff header line, with git's `a/`/`b/` prefix
/// stripped. `None` for `/dev/null` (the other side names the file). Git is
/// invoked with `core.quotePath=false`, so the path arrives verbatim.
fn diff_header_path(line: &str) -> Option<String> {
    let rest = line
        .strip_prefix("+++ ")
        .or_else(|| line.strip_prefix("--- "))?;
    let rest = rest.trim_end();
    if rest == "/dev/null" {
        return None;
    }
    let path = rest
        .strip_prefix("b/")
        .or_else(|| rest.strip_prefix("a/"))
        .unwrap_or(rest);
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

struct RawFilePatch<'a> {
    /// Resolved display path; `+++` (the new side) wins, `---` covers a
    /// deletion. `None` for a mode-only change that emits no path headers.
    path: Option<String>,
    binary: bool,
    body: Vec<&'a str>,
}

/// Split a whole-commit patch body into per-file slices in one pass. Renames
/// resolve to the new path (the audit's `changes` list uses the same side), and
/// a deletion keeps its `---` path because `+++` is `/dev/null`.
fn split_commit_patch(text: &str) -> Vec<RawFilePatch<'_>> {
    let mut files: Vec<RawFilePatch> = Vec::new();
    let mut current: Option<RawFilePatch> = None;
    for raw in text.lines() {
        if raw.starts_with("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            current = Some(RawFilePatch {
                path: None,
                binary: false,
                body: Vec::new(),
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        // A pure rename (identical content) emits no `---`/`+++` lines, only
        // `rename to <new>`: recover the new path so it still keys a diff.
        if let Some(rest) = raw.strip_prefix("rename to ") {
            let renamed = rest.trim();
            if !renamed.is_empty() {
                file.path = Some(renamed.to_string());
            }
        }
        let is_new_side = raw.starts_with("+++ ");
        let is_old_side = raw.starts_with("--- ");
        if is_new_side || (is_old_side && file.path.is_none()) {
            if let Some(path) = diff_header_path(raw) {
                file.path = Some(path);
            }
        }
        if raw.starts_with("Binary files") && raw.ends_with("differ") {
            file.binary = true;
        }
        file.body.push(raw);
    }
    if let Some(file) = current.take() {
        files.push(file);
    }
    files
}

/// The main commit's per-file diff in ONE `git show` spawn: every changed
/// file's bounded hunks keyed by path, so the audit page renders the same
/// files-column + selected-file diff the commit browser uses without a fetch
/// per file. The main commit lives in the MAIN repository, so `main_commit_sha`
/// resolves in the target checkout, never a subsystem.
///
/// `--first-parent -m` is required: a merge commit's plain `git show -p` is
/// empty, while the audit's own `changes` (and the promotion engine's
/// "first parent of the merge" content) are the first-parent delta — the two
/// must describe the same change.
pub fn main_commit_diff_live(
    target_dir: &Path,
    registry: &Registry,
    target_branch: &str,
    main_commit_sha: &str,
) -> AppResult<MainCommitDiff> {
    require_allowed_target(registry, target_branch)?;
    let wanted = main_commit_sha.trim();
    let full = git::rev_parse(target_dir, wanted).ok_or_else(|| {
        AppError::not_found(format!(
            "Commit '{wanted}' was not found in the target checkout."
        ))
        .with_remedy("Refresh the integration history and try again.")
    })?;
    let out = git::run(
        target_dir,
        &[
            "-c",
            "core.quotePath=false",
            "show",
            "--format=",
            "--no-color",
            "--unified=3",
            "-M",
            "--first-parent",
            "-m",
            "--root",
            &full,
        ],
    )
    .map_err(|err| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            "Could not read the commit diff.",
        )
        .with_remedy("Refresh and retry.")
        .with_details(git::sanitize(&err.to_string()))
    })?;
    if out.status != 0 {
        return Err(
            AppError::not_found(format!("No diff is available for commit {wanted}."))
                .with_remedy("Refresh the integration history and try again."),
        );
    }
    let text = out.stdout_str();
    let mut files: Vec<MainCommitFileDiff> = Vec::new();
    for raw in split_commit_patch(&text) {
        let Some(path) = raw.path else {
            continue;
        };
        let (mut hunks, additions, deletions) = parse_unified_hunks(raw.body.iter().copied());
        let total_lines: u32 = hunks.iter().map(|h| h.lines.len() as u32).sum();
        let truncated = bound_hunks(&mut hunks);
        files.push(MainCommitFileDiff {
            language: fixtures::language_for(&path).to_string(),
            path: path.clone(),
            hunks,
            total_lines,
            additions,
            deletions,
            binary: raw.binary,
            truncated,
            source_url: format!(
                "https://github.com/{}/commit/{full}#diff-{path}",
                registry.main_repository
            ),
            note: if raw.binary {
                Some("Binary file: no text diff is shown.".to_string())
            } else if truncated {
                Some(truncation_note())
            } else {
                None
            },
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(MainCommitDiff {
        main_commit_sha: full,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::engine::github_ops::{BranchInfo, PullRequestInfo, StubGitHubOps};

    fn registry_json(subsystems: &str) -> String {
        format!(
            r#"{{
  "mainRepository": "ORG/main-system",
  "promotion": {{
    "allowedTargetBranches": ["staging", "development"],
    "allowedTargetBranchPatterns": ["release/*"],
    "requirePullRequest": true
  }},
  "subsystems": [{subsystems}]
}}"#
        )
    }

    fn subsystem_json(id: &str, name: &str, repo: &str, branch: &str, enabled: bool) -> String {
        format!(
            r#"{{
      "id": "{id}",
      "name": "{name}",
      "repository": "{repo}",
      "sourceBranch": "{branch}",
      "enabled": {enabled},
      "pathStrategy": "preserve-relative-paths",
      "dependencyStrategy": "reconcile",
      "integrationPolicy": {{
        "applicationFiles": "selected-commit-delta",
        "packageJson": "merge-dependency-delta",
        "lockfiles": "regenerate-in-target",
        "sharedConfiguration": "review-explicitly",
        "githubWorkflows": "main-owned",
        "secretFiles": "never-import"
      }}
    }}"#
        )
    }

    fn test_registry_two() -> Registry {
        let subsystems = format!(
            "{},{}",
            subsystem_json("hr-management", "HR", "ORG/hr-management", "master", true),
            subsystem_json("payroll", "Payroll", "ORG/payroll", "master", true)
        );
        Registry::from_json(&registry_json(&subsystems), "test").expect("registry")
    }

    #[test]
    fn registry_backed_subsystems_come_from_registry() {
        let registry = test_registry_two();
        let summaries = subsystem_summaries(&registry);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].name, "HR");
        assert_eq!(summaries[1].name, "Payroll");
        let hr = summaries.iter().find(|s| s.id == "hr-management").unwrap();
        assert_eq!(hr.repository, "ORG/hr-management");
        assert_eq!(hr.source_branch, "master");
        assert!(hr.enabled);
        assert!(!summaries.iter().any(|s| s.id == "inventory-management"));
        assert!(!summaries.iter().any(|s| s.id == "document-management"));
    }

    static SANDBOX_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn tmp(prefix: &str) -> PathBuf {
        let n = SANDBOX_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "vos-live-{prefix}-{n}-{nanos}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
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
        let subsystems = subsystem_json("hr-management", "HR", "ORG/hr-management", "master", true);
        Registry::from_json(&registry_json(&subsystems), "test").expect("registry")
    }

    struct TtlGuard {
        previous: Option<String>,
    }

    impl TtlGuard {
        fn set(value: &str) -> Self {
            let previous = std::env::var("VOS_CHECKOUT_FETCH_TTL_SECS").ok();
            std::env::set_var("VOS_CHECKOUT_FETCH_TTL_SECS", value);
            Self { previous }
        }
    }

    impl Drop for TtlGuard {
        fn drop(&mut self) {
            match self.previous.as_deref() {
                Some(value) => std::env::set_var("VOS_CHECKOUT_FETCH_TTL_SECS", value),
                None => std::env::remove_var("VOS_CHECKOUT_FETCH_TTL_SECS"),
            }
        }
    }

    fn cloned_remote_sandbox(prefix: &str) -> (PathBuf, PathBuf, PathBuf) {
        let origin = tmp(&format!("{prefix}-origin"));
        init(&origin, "master");
        write(&origin, "base.txt", "base\n");
        let _ = commit_all(&origin, "main base");
        git(&origin, &["branch", "staging"]);

        let holder = tmp(&format!("{prefix}-holder"));
        let clone = holder.join("main-system");
        git(
            &holder,
            &[
                "clone",
                "-q",
                origin.to_str().expect("utf8"),
                clone.to_str().expect("utf8"),
            ],
        );
        (origin, holder, clone)
    }

    fn sandbox_status() -> (PathBuf, PathBuf, String, String, Registry) {
        let src = tmp("src");
        init(&src, "master");
        write(&src, "a.txt", "a\n");
        write(&src, "package.json", "{\"name\":\"x\"}\n");
        let _a0 = commit_all(&src, "A base");
        let a = git(&src, &["rev-parse", "HEAD"]);
        write(&src, "b.txt", "b feature\n");
        let _b0 = commit_all(&src, "B feature");
        let b = git(&src, &["rev-parse", "HEAD"]);

        let main = tmp("main");
        init(&main, "master");
        write(&main, "base.txt", "base\n");
        let _base = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);
        git(&main, &["branch", "development"]);
        git(&main, &["checkout", "-q", "staging"]);
        write(&main, "b.txt", "b feature\n");
        let trailer = format!(
            "integrate(hr-management): B feature\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {b}\nPromotion-Mode: individual\nPromotion-Target: staging\n"
        );
        let _i = commit_all(&main, &trailer);
        git(&main, &["checkout", "-q", "master"]);
        (src, main, a, b, single_registry())
    }

    #[test]
    fn live_status_integrated_eligible_and_pr_open() {
        let (_src, main, a, b, registry) = sandbox_status();
        let empty = StubGitHubOps::new();
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            &[a.clone(), b.clone()],
        )
        .expect("statuses");
        assert_eq!(statuses.len(), 2);
        let by_sha: HashMap<&str, &IntegrationStatus> = statuses
            .iter()
            .map(|s| (s.source_sha.as_str(), s))
            .collect();
        assert_eq!(
            by_sha[&b.as_str()].status.kind,
            CommitStatusKind::Integrated,
            "B has a real trailer commit on staging"
        );
        assert_eq!(
            by_sha[&a.as_str()].status.kind,
            CommitStatusKind::Eligible,
            "A has no integration"
        );
        let pr = PullRequestInfo {
            number: Some(7),
            base: "staging".to_string(),
            head: format!(
                "integration/hr-management-{}-staging-x",
                &a[..12.min(a.len())]
            ),
            title: "integrate(hr-management): A".to_string(),
            body: format!(
                "Subsystem-ID: hr-management\nSource-Commit: {a}\nPromotion-Target: staging\n"
            ),
            ..Default::default()
        };
        let with_pr = StubGitHubOps::new().with_open_prs(vec![pr]);
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &with_pr,
            "staging",
            Some("hr-management"),
            &[a.clone(), b.clone()],
        )
        .expect("statuses with pr");
        let by_sha: HashMap<&str, &IntegrationStatus> = statuses
            .iter()
            .map(|s| (s.source_sha.as_str(), s))
            .collect();
        assert_eq!(
            by_sha[&a.as_str()].status.kind,
            CommitStatusKind::PrOpen,
            "open-PR-only SHA must be PrOpen"
        );
        assert_ne!(
            by_sha[&a.as_str()].status.kind,
            CommitStatusKind::Integrated,
            "a push/PR must never mint Integrated"
        );
        assert_eq!(
            by_sha[&b.as_str()].status.kind,
            CommitStatusKind::Integrated
        );
        let _ = _src;
    }

    #[test]
    fn a_deletion_stays_promotable_when_the_target_lacks_the_file() {
        let (src, main, _a, _b, registry) = sandbox_status();
        std::fs::remove_file(src.join("b.txt")).expect("remove b.txt");
        let _dropped = commit_all(&src, "Drop b.txt");
        let drop_sha = git(&src, &["rev-parse", "HEAD"]);
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &StubGitHubOps::new(),
            "development",
            Some("hr-management"),
            std::slice::from_ref(&drop_sha),
        )
        .expect("statuses");
        assert_eq!(statuses.len(), 1);
        assert_ne!(
            statuses[0].status.kind,
            CommitStatusKind::Applied,
            "absent on both sides proves nothing; a removal must stay promotable"
        );
        assert_eq!(statuses[0].status.kind, CommitStatusKind::Eligible);
    }

    #[test]
    fn target_branch_policy_filtering_marks_allowed_and_reason() {
        let registry = single_registry();
        let checkout = tmp("branch-list-policy");
        init(&checkout, "master");
        write(&checkout, "x.txt", "x\n");
        let _ = commit_all(&checkout, "base");
        git(&checkout, &["branch", "staging"]);
        git(&checkout, &["branch", "release/2026-01"]);
        let github = StubGitHubOps::new()
            .with_branches(vec![
                BranchInfo {
                    name: "staging".to_string(),
                    sha: "a".repeat(40),
                    protected: true,
                },
                BranchInfo {
                    name: "feature/evil".to_string(),
                    sha: "b".repeat(40),
                    protected: false,
                },
                BranchInfo {
                    name: "release/2026-01".to_string(),
                    sha: "c".repeat(40),
                    protected: true,
                },
            ])
            .with_default_branch("staging");
        let list = target_branches_live(&registry, &github, Some("staging"), Some(&checkout))
            .expect("branches");
        assert_eq!(list.data_source, DataSource::Live);
        let by_name: HashMap<&str, &TargetBranch> =
            list.branches.iter().map(|b| (b.name.as_str(), b)).collect();
        assert!(by_name["staging"].allowed);
        assert!(
            by_name["release/2026-01"].allowed,
            "pattern release/* allows it"
        );
        assert!(
            !by_name["feature/evil"].allowed,
            "policy must reject feature/evil"
        );
        let reason = by_name["feature/evil"]
            .disallowed_reason
            .clone()
            .unwrap_or_default();
        assert!(
            reason.contains("registry policy") || reason.contains("Not permitted"),
            "excluded branches carry the policy reason: {reason}"
        );
    }

    #[test]
    fn policy_allowed_branch_missing_from_checkout_is_listed_not_allowed() {
        let registry = single_registry();
        let checkout = tmp("branch-list-missing");
        init(&checkout, "master");
        write(&checkout, "x.txt", "x\n");
        let _ = commit_all(&checkout, "base");
        let github = StubGitHubOps::new()
            .with_branches(vec![
                BranchInfo {
                    name: "staging".to_string(),
                    sha: "a".repeat(40),
                    protected: true,
                },
                BranchInfo {
                    name: "master".to_string(),
                    sha: "b".repeat(40),
                    protected: true,
                },
            ])
            .with_default_branch("master");
        let list = target_branches_live(&registry, &github, Some("staging"), Some(&checkout))
            .expect("a missing branch must not fail the whole list");
        let by_name: HashMap<&str, &TargetBranch> =
            list.branches.iter().map(|b| (b.name.as_str(), b)).collect();
        assert!(
            by_name.contains_key("staging"),
            "the absent branch is still listed, not dropped"
        );
        assert!(
            !by_name["staging"].allowed,
            "policy-allowed but absent from the checkout → not allowed"
        );
        let reason = by_name["staging"]
            .disallowed_reason
            .clone()
            .unwrap_or_default();
        assert!(
            reason.contains("checkout"),
            "the reason names the checkout: {reason}"
        );
    }

    /// The Dashboard bug end-to-end: a fork clone keeps the real target branch
    /// on a non-`origin` remote, so a read of that branch must resolve
    /// `refs/remotes/upstream/<branch>` instead of erroring.
    #[test]
    fn status_read_resolves_a_branch_that_lives_only_on_a_non_origin_remote() {
        let (_src, main, _a, b, registry) = sandbox_status();
        // Move `staging` out of the local refs into a non-origin
        // remote-tracking ref, exactly like a fork clone whose real branches
        // live on `upstream`.
        git(
            &main,
            &[
                "update-ref",
                "refs/remotes/upstream/staging",
                "refs/heads/staging",
            ],
        );
        git(&main, &["branch", "-D", "staging"]);
        assert!(git::rev_parse(&main, "refs/heads/staging").is_none());
        assert!(git::rev_parse(&main, "refs/remotes/origin/staging").is_none());
        assert!(git::rev_parse(&main, "refs/remotes/upstream/staging").is_some());
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &StubGitHubOps::new(),
            "staging",
            Some("hr-management"),
            std::slice::from_ref(&b),
        )
        .expect("a branch on a non-origin remote must resolve, not error");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].target_branch, "staging");
    }

    #[test]
    fn fixture_flag_and_target_dir_seam() {
        let _env = crate::test_support::env_lock();
        std::env::remove_var("VOS_USE_FIXTURES");
        assert!(!use_fixtures());
        std::env::set_var("VOS_USE_FIXTURES", "1");
        assert!(use_fixtures());
        let world = crate::fixtures::FixtureWorld::build();
        assert!(
            world.subsystems.len() >= 5,
            "fixture world keeps its subsystems"
        );
        std::env::remove_var("VOS_USE_FIXTURES");
        assert!(!use_fixtures());
        // Default live mode works when only VOS_TARGET_REPO_DIR is set
        // (no token, no clone): the local dir passes straight through.
        let dir = tmp("target-seam");
        init(&dir, "master");
        write(&dir, "x.txt", "x\n");
        let _ = commit_all(&dir, "base");
        std::env::set_var("VOS_TARGET_REPO_DIR", &dir);
        let (resolved, cleanup) =
            resolve_target_checkout("ORG/main-system", None, None).expect("local dir seam");
        assert_eq!(resolved, dir);
        assert!(cleanup.is_none());
        std::env::remove_var("VOS_TARGET_REPO_DIR");
    }

    #[test]
    fn live_history_prs_and_commits_come_from_git_and_github() {
        let (src, main, a, b, registry) = sandbox_status();
        let empty = StubGitHubOps::new();
        let history = integration_history_live(
            &main,
            &registry,
            &empty,
            Some("staging"),
            Some("hr-management"),
            50,
        )
        .expect("history");
        assert_eq!(history.data_source, DataSource::Live);
        assert!(
            history
                .entries
                .iter()
                .any(|e| e.kind == IntegrationHistoryKind::Merged && e.source_shas.contains(&b)),
            "reachable trailer integration must appear as Merged"
        );
        let good = PullRequestInfo {
            number: Some(11),
            base: "staging".to_string(),
            head: "integration/hr-management-abc-staging-x".to_string(),
            title: "integrate(hr-management): B".to_string(),
            body: format!(
                "Subsystem-ID: hr-management\nSource-Commit: {b}\nPromotion-Target: staging\n"
            ),
            ..Default::default()
        };
        let noise = PullRequestInfo {
            number: Some(12),
            base: "staging".to_string(),
            head: "feature/random".to_string(),
            title: "random".to_string(),
            body: "nothing to see here".to_string(),
            ..Default::default()
        };
        let github = StubGitHubOps::new().with_open_prs(vec![good, noise]);
        let prs = integration_prs_live(&registry, &github, "staging", Some("hr-management"))
            .expect("prs");
        assert_eq!(prs.len(), 1, "non-integration PRs are excluded");
        assert_eq!(prs[0].subsystem_id, "hr-management");
        assert!(prs[0].source_shas.contains(&b));
        assert!(prs[0].provenance_ok);
        let query = CommitQuery {
            subsystem_id: "hr-management".to_string(),
            target_branch: "staging".to_string(),
            page: 1,
            page_size: 50,
            search: String::new(),
            status_filter: None,
            sort: crate::github::models::CommitSort::Newest,
            shas: None,
        };
        let page = commit_page_live(&main, &src, &registry, &empty, &query).expect("page");
        assert_eq!(page.data_source, DataSource::Live);
        assert_eq!(page.total, 2);
        assert!(page.commits.iter().any(|c| c.sha == a));
        assert!(page.commits.iter().any(|c| c.sha == b));
        let details = commit_details_live(
            &main,
            &src,
            &registry,
            &empty,
            "hr-management",
            &b,
            "staging",
        )
        .expect("details");
        assert_eq!(details.sha, b);
        assert!(
            details.files.iter().any(|f| f.path == "b.txt"),
            "changed files carry add/mod/delete flags"
        );
        let diff =
            commit_diff_live(&src, &registry, "hr-management", &b, Some("b.txt")).expect("diff");
        assert_eq!(diff.file_path, "b.txt");
        assert!(!diff.hunks.is_empty());
        assert!(!diff.truncated);
        assert!(diff.total_lines > 0);
    }

    #[test]
    fn cached_statuses_second_call_hits_cache_without_git_or_gh_work() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let (_src, main, a, b, registry) = sandbox_status();
        let state = crate::AppState::new();
        let github = StubGitHubOps::new();
        let shas = vec![a.clone(), b.clone()];
        let key = crate::commands::commits::statuses_cache_key(
            "live",
            "staging",
            Some("hr-management"),
            &shas,
        );
        let computes = AtomicU64::new(0);
        let first: Vec<IntegrationStatus> = crate::commands::cached_json(&state, &key, || {
            computes.fetch_add(1, Ordering::Relaxed);
            integration_statuses_live(
                &main,
                &registry,
                &github,
                "staging",
                Some("hr-management"),
                &shas,
            )
        })
        .expect("statuses");
        assert_eq!(first.len(), 2);
        let second: Vec<IntegrationStatus> = crate::commands::cached_json(&state, &key, || {
            computes.fetch_add(1, Ordering::Relaxed);
            integration_statuses_live(
                &main,
                &registry,
                &github,
                "staging",
                Some("hr-management"),
                &shas,
            )
        })
        .expect("cached statuses");
        assert_eq!(second.len(), 2);
        assert_eq!(
            computes.load(Ordering::Relaxed),
            1,
            "the second identical read must not recompute"
        );
        assert_eq!(
            github.calls_named("listOpenPullRequests").len(),
            1,
            "no extra `gh` work on a cache hit"
        );
        let _ = _src;
    }

    #[test]
    fn cached_details_second_call_hits_cache_without_git_or_gh_work() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let (src, main, _a, b, registry) = sandbox_status();
        let state = crate::AppState::new();
        let github = StubGitHubOps::new();
        let key =
            crate::commands::commits::details_cache_key("live", "hr-management", &b, "staging");
        let computes = AtomicU64::new(0);
        let first: CommitDetails = crate::commands::cached_json(&state, &key, || {
            computes.fetch_add(1, Ordering::Relaxed);
            commit_details_live(
                &main,
                &src,
                &registry,
                &github,
                "hr-management",
                &b,
                "staging",
            )
        })
        .expect("details");
        assert_eq!(first.sha, b);
        assert!(first.files.iter().any(|f| f.path == "b.txt"));
        let pr_calls_before = github.calls_named("listOpenPullRequests").len();
        let second: CommitDetails = crate::commands::cached_json(&state, &key, || {
            computes.fetch_add(1, Ordering::Relaxed);
            commit_details_live(
                &main,
                &src,
                &registry,
                &github,
                "hr-management",
                &b,
                "staging",
            )
        })
        .expect("cached details");
        assert_eq!(second.sha, b);
        assert_eq!(
            computes.load(Ordering::Relaxed),
            1,
            "the second identical read must not recompute"
        );
        assert_eq!(
            github.calls_named("listOpenPullRequests").len(),
            pr_calls_before,
            "no extra `gh` work on a cache hit"
        );
    }

    /// G4: `local_direct` unpushed → `Integrated (local only, unpushed)`;
    /// after push (remote contains) → plain `Integrated`; unreadable remote →
    /// `Unknown (stale, as of <time>)`, never demoting known-remote Integrated.
    #[test]
    fn local_only_unpushed_pushed_and_stale_labels() {
        let (_src, main, _a, b, registry) = sandbox_status();
        let empty = StubGitHubOps::new();
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            std::slice::from_ref(&b),
        )
        .expect("statuses");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status.kind, CommitStatusKind::Integrated);
        assert_eq!(
            statuses[0].status.label, "Integrated (local only, unpushed)",
            "unpushed local commit must carry the additive label"
        );
        git(
            &main,
            &["update-ref", "refs/remotes/origin/staging", "staging"],
        );
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            std::slice::from_ref(&b),
        )
        .expect("statuses after push");
        assert_eq!(statuses[0].status.kind, CommitStatusKind::Integrated);
        assert_eq!(statuses[0].status.label, "Integrated");
        let stale = fetch_time_str(&main);
        assert!(!stale.is_empty());
    }

    #[test]
    fn resolve_target_checkout_fetches_a_stale_user_clone() {
        let _env = crate::test_support::env_lock();
        let (origin, _holder, clone) = cloned_remote_sandbox("fetch");
        let clone_str = clone.to_string_lossy().to_string();

        {
            let _ttl = TtlGuard::set("0");
            write(&origin, "b.txt", "b\n");
            let upstream = commit_all(&origin, "second");
            let (resolved, _) = resolve_target_checkout("ORG/main-system", Some(&clone_str), None)
                .expect("resolve");
            assert_eq!(resolved, clone, "the configured clone is what reads use");
            assert_eq!(
                git(&resolved, &["rev-parse", "origin/master"]),
                upstream,
                "the read path must fetch the user clone when stale"
            );
        }

        {
            let _ttl = TtlGuard::set("100000");
            write(&origin, "c.txt", "c\n");
            let third = commit_all(&origin, "third");
            let (resolved, _) = resolve_target_checkout("ORG/main-system", Some(&clone_str), None)
                .expect("resolve");
            assert_ne!(
                git(&resolved, &["rev-parse", "origin/master"]),
                third,
                "a warm read must skip the fetch"
            );
        }
    }

    #[test]
    fn fetch_failure_leaves_the_read_stale_but_working() {
        let _env = crate::test_support::env_lock();
        let (_src, main, _a, b, registry) = sandbox_status();
        git(
            &main,
            &[
                "remote",
                "add",
                "origin",
                "https://127.0.0.1:1/ORG/main-system.git",
            ],
        );
        let _ttl = TtlGuard::set("0");
        let (resolved, _) =
            resolve_target_checkout("ORG/main-system", Some(&main.to_string_lossy()), None)
                .expect("a fetch failure must not error the read");
        assert_eq!(resolved, main);
        let statuses = integration_statuses_live(
            &resolved,
            &registry,
            &StubGitHubOps::new(),
            "staging",
            Some("hr-management"),
            std::slice::from_ref(&b),
        )
        .expect("the stale clone still answers");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status.kind, CommitStatusKind::Integrated);
    }

    #[test]
    fn local_only_label_survives_a_fetch() {
        let _env = crate::test_support::env_lock();
        let src = tmp("lbl-src");
        init(&src, "master");
        write(&src, "a.txt", "a\n");
        let _ = commit_all(&src, "A base");
        write(&src, "b.txt", "b feature\n");
        let b = commit_all(&src, "B feature");

        let (_origin, _holder, clone) = cloned_remote_sandbox("lbl");
        git(&clone, &["checkout", "-q", "staging"]);
        write(&clone, "b.txt", "b feature\n");
        let trailer = format!(
            "integrate(hr-management): B feature\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {b}\nPromotion-Mode: individual\nPromotion-Target: staging\n"
        );
        let _ = commit_all(&clone, &trailer);
        let integration = git(&clone, &["rev-parse", "HEAD"]);

        let _ttl = TtlGuard::set("0");
        let (resolved, _) =
            resolve_target_checkout("ORG/main-system", Some(&clone.to_string_lossy()), None)
                .expect("resolve");
        assert!(
            git::rev_parse(&resolved, "refs/remotes/origin/staging").is_some(),
            "the fetch must have populated the remote-tracking ref"
        );
        assert!(
            !git::is_ancestor(&resolved, &integration, "refs/remotes/origin/staging"),
            "the integration commit is local-only, absent from the remote"
        );

        let statuses = integration_statuses_live(
            &resolved,
            &single_registry(),
            &StubGitHubOps::new(),
            "staging",
            Some("hr-management"),
            std::slice::from_ref(&b),
        )
        .expect("statuses");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status.kind, CommitStatusKind::Integrated);
        assert_eq!(
            statuses[0].status.label, "Integrated (local only, unpushed)",
            "a fetch must not collapse the local-only distinction"
        );
    }

    #[test]
    fn integration_on_a_non_origin_remote_reports_integrated_not_stale() {
        let (_src, main, _a, b, registry) = sandbox_status();
        // Move `staging` out of the local refs into a non-origin
        // remote-tracking ref, exactly like a fork clone whose real branch
        // lives on `upstream`, and leave no `origin` ref behind.
        git(
            &main,
            &[
                "update-ref",
                "refs/remotes/upstream/staging",
                "refs/heads/staging",
            ],
        );
        git(&main, &["branch", "-D", "staging"]);
        assert!(git::rev_parse(&main, "refs/heads/staging").is_none());
        assert!(git::rev_parse(&main, "refs/remotes/origin/staging").is_none());
        assert!(git::rev_parse(&main, "refs/remotes/upstream/staging").is_some());

        let empty = StubGitHubOps::new();
        let statuses = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            std::slice::from_ref(&b),
        )
        .expect("a branch on a non-origin remote must resolve, not error");
        assert_eq!(statuses.len(), 1);
        assert_eq!(
            statuses[0].status.kind,
            CommitStatusKind::Integrated,
            "reachable from a non-origin remote-tracking ref must stay Integrated"
        );
        assert_eq!(
            statuses[0].status.label, "Integrated",
            "on a remote ⇒ plain Integrated, never Unknown (stale)"
        );
    }

    #[test]
    fn squash_stripped_trailers_report_unknown_never_eligible() {
        use crate::services::engine::history::{BranchStatusKind, BranchStatusResult};
        let dir = tmp("squash");
        init(&dir, "master");
        write(&dir, "x.txt", "x\n");
        let _ = commit_all(&dir, "base");
        git(&dir, &["branch", "staging"]);
        write(&dir, "x.txt", "squashed content\n");
        let fake = commit_all(&dir, "squashed integration without trailers");
        let result = BranchStatusResult {
            sha: "a".repeat(40),
            status: BranchStatusKind::Integrated,
            integration_commit: Some(fake),
            promotion_target: Some("staging".to_string()),
            reverted_by: None,
        };
        let status = branch_results_to_statuses(&dir, "staging", std::slice::from_ref(&result))
            .pop()
            .expect("one status per result");
        assert_eq!(status.kind, CommitStatusKind::Unknown);
        assert!(
            status
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("Needs verification"),
            "must say needs verification: {:?}",
            status.detail
        );
        assert!(
            !matches!(status.kind, CommitStatusKind::Eligible),
            "never fall back to Eligible"
        );
    }

    /// G6: a >800-commit scan sets the truncation flag (E3: dates are display
    /// order only; a truncated no-hit scan is Unknown, never Integrated).
    #[test]
    fn over_800_commit_scan_sets_truncation_flag() {
        let dir = tmp("trunc");
        init(&dir, "master");
        write(&dir, "x.txt", "x\n");
        let _ = commit_all(&dir, "base");
        git(&dir, &["branch", "staging"]);
        git(&dir, &["checkout", "-q", "staging"]);
        for i in 0..801 {
            git(
                &dir,
                &["commit", "--allow-empty", "-q", "-m", &format!("empty {i}")],
            );
        }
        let (_map, truncated) = target_path_dates(&dir, "staging");
        assert!(truncated, "801 commits must trip the 800 scan limit");
        let small = tmp("trunc-small");
        init(&small, "master");
        write(&small, "x.txt", "x\n");
        let _ = commit_all(&small, "base");
        let (_, truncated_small) = target_path_dates(&small, "master");
        assert!(!truncated_small);
    }

    #[test]
    fn status_scan_is_shared_across_reads_on_unchanged_tip() {
        let (src, main, a, b, registry) = sandbox_status();
        let scans = || crate::services::engine::history::replay_scans_for_tests(&main, "staging");
        let empty = StubGitHubOps::new();
        let first = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            &[a.clone(), b.clone()],
        )
        .expect("statuses");
        assert_eq!(scans(), 1, "the first read walks the history once");
        let details = commit_details_live(
            &main,
            &src,
            &registry,
            &empty,
            "hr-management",
            &b,
            "staging",
        )
        .expect("details");
        assert_eq!(details.sha, b);
        assert_eq!(
            scans(),
            1,
            "details on the same tip must reuse the scan, not walk history again"
        );
        let by_sha: HashMap<&str, &IntegrationStatus> =
            first.iter().map(|s| (s.source_sha.as_str(), s)).collect();
        assert_eq!(by_sha[&b.as_str()].status.kind, details.status.kind);
        git(&main, &["checkout", "-q", "staging"]);
        write(&main, "new.txt", "new\n");
        let _ = commit_all(&main, "ordinary newcomer");
        let second = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            &[a.clone(), b.clone()],
        )
        .expect("statuses after tip move");
        assert_eq!(
            scans(),
            1,
            "a moved tip must recompute its own scan, never reuse the old tip's replay"
        );
        let by_sha: HashMap<&str, &IntegrationStatus> =
            second.iter().map(|s| (s.source_sha.as_str(), s)).collect();
        assert_eq!(
            by_sha[&b.as_str()].status.kind,
            CommitStatusKind::Integrated,
            "the trailer integration survives newer ordinary commits"
        );
        assert_eq!(
            by_sha[&a.as_str()].status.kind,
            CommitStatusKind::Eligible,
            "an unintegrated SHA stays eligible after the tip moves"
        );
    }

    #[test]
    fn page_of_rows_costs_one_branch_walk_not_one_per_row() {
        let src = tmp("page-src");
        init(&src, "master");
        let mut shas = Vec::new();
        for i in 0..10 {
            write(&src, &format!("f{i}.txt"), &format!("content {i}\n"));
            shas.push(commit_all(&src, &format!("feature {i}")));
        }
        let main = tmp("page-main");
        init(&main, "master");
        write(&main, "base.txt", "base\n");
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);
        git(&main, &["checkout", "-q", "staging"]);
        for (i, sha) in shas.iter().enumerate() {
            write(&main, &format!("f{i}.txt"), &format!("content {i}\n"));
            let trailer = format!(
                "integrate(hr-management): feature {i}\n\nSubsystem-ID: hr-management\nSource-Repo: ORG/hr-management\nSource-Commit: {sha}\nPromotion-Mode: individual\nPromotion-Target: staging\n"
            );
            let _ = commit_all(&main, &trailer);
        }
        let registry = single_registry();
        let empty = StubGitHubOps::new();

        let statuses = integration_statuses_live(
            &main,
            &registry,
            &empty,
            "staging",
            Some("hr-management"),
            &shas,
        )
        .expect("statuses");
        assert_eq!(statuses.len(), 10);
        for s in &statuses {
            assert_eq!(
                s.status.kind,
                CommitStatusKind::Integrated,
                "batched enrichment must not lose an integration: {}",
                s.source_sha
            );
            assert!(
                s.status.evidence.is_some(),
                "evidence survives batching: {}",
                s.source_sha
            );
        }
        assert_eq!(
            crate::services::engine::history::replay_scans_for_tests(&main, "staging"),
            1,
            "10 rows must cost one branch walk, not one per row"
        );
        let spawns = crate::services::engine::git::git_spawns_for_tests(&main);
        assert!(
            spawns <= 16,
            "10 integrated rows cost {spawns} git spawns, expected O(1) from one shared walk"
        );
        let history = integration_history_live(
            &main,
            &registry,
            &empty,
            Some("staging"),
            Some("hr-management"),
            50,
        )
        .expect("history");
        let merged: Vec<_> = history
            .entries
            .iter()
            .filter(|e| e.kind == IntegrationHistoryKind::Merged)
            .collect();
        assert_eq!(merged.len(), 10);
        for entry in &merged {
            assert_eq!(
                entry.actor, "T",
                "author harvested by the walk, not a per-entry spawn"
            );
        }
        assert_eq!(
            crate::services::engine::history::replay_scans_for_tests(&main, "staging"),
            1,
            "history shares the same tip-keyed scan"
        );
        let _ = src;
    }

    fn sandbox_audit_three() -> (PathBuf, PathBuf, Vec<String>, String, Registry) {
        let src = tmp("audit-src");
        init(&src, "master");
        write(&src, "a.txt", "a\n");
        let a = commit_all(&src, "Alpha feature");
        write(&src, "b.txt", "b\n");
        let b = commit_all(&src, "Beta feature");
        write(&src, "c.txt", "c\n");
        let c = commit_all(&src, "Gamma feature");

        let main = tmp("audit-main");
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
        (src, main, order, m, single_registry())
    }

    #[test]
    fn audit_included_commits_match_source_commit_trailers_in_order() {
        let (src, main, order, m, registry) = sandbox_audit_three();
        let audit = integration_event_audit_live(
            &main,
            Some(&src),
            &registry,
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
        }
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
    fn audit_unreachable_source_commit_is_unavailable_without_fabrication() {
        let src = tmp("audit-unreach-src");
        init(&src, "master");
        write(&src, "a.txt", "a\n");
        let real = commit_all(&src, "Real work");
        let bogus = "f".repeat(40);

        let main = tmp("audit-unreach-main");
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
    fn audit_changes_and_totals_match_real_numstat_for_multi_file_commit() {
        let main = tmp("audit-changes");
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

        let numstat = git(&main, &["show", "--numstat", "--format=", &m]);
        let mut expected: HashMap<String, (u32, u32)> = HashMap::new();
        for line in numstat.lines() {
            let mut cols = line.split('\t');
            if let (Some(a), Some(d), Some(p)) = (cols.next(), cols.next(), cols.next()) {
                expected.insert(
                    p.to_string(),
                    (a.parse().unwrap_or(0), d.parse().unwrap_or(0)),
                );
            }
        }

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
        let by_path: HashMap<&str, &str> = audit
            .changes
            .iter()
            .map(|c| {
                (
                    c.path.as_str(),
                    match c.change {
                        IntegrationEventChangeKind::Added => "added",
                        IntegrationEventChangeKind::Modified => "modified",
                        IntegrationEventChangeKind::Deleted => "deleted",
                        IntegrationEventChangeKind::Renamed => "renamed",
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
    fn audit_trailerless_commit_reports_unavailable_without_fabricated_commits() {
        let main = tmp("audit-plain");
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
    }

    #[test]
    fn audit_reverted_event_names_the_commit_it_undid() {
        let src = tmp("audit-revert-src");
        init(&src, "master");
        write(&src, "f.txt", "feature\n");
        let s = commit_all(&src, "Feature work");

        let main = tmp("audit-revert-main");
        init(&main, "master");
        write(&main, "base.txt", "base\n");
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);
        git(&main, &["checkout", "-q", "staging"]);
        write(&main, "f.txt", "feature\n");
        let integrate_msg = crate::services::engine::provenance::build_provenance_message(
            &crate::services::engine::provenance::ProvenanceInput {
                subsystem_id: "hr-management",
                source_repo: "ORG/hr-management",
                source_commits: std::slice::from_ref(&s),
                mode: "individual",
                target: "staging",
                subject: "Feature work",
            },
        );
        let i = commit_all(&main, &integrate_msg);
        write(&main, "f.txt", "undone\n");
        let revert_msg = crate::services::engine::provenance::build_revert_message(
            "hr-management",
            std::slice::from_ref(&s),
            &i,
            "Feature work",
        );
        let r = commit_all(&main, &revert_msg);

        let registry = single_registry();
        let undone = integration_event_audit_live(
            &main,
            Some(&src),
            &registry,
            "staging",
            "hr-management",
            &r,
        )
        .expect("audit");
        assert!(!undone.main_commit_unavailable);
        assert_eq!(undone.kind, IntegrationHistoryKind::Reverted);
        assert_eq!(
            undone.reverted_main_commit_sha.as_deref(),
            Some(i.as_str()),
            "a Reverted event names the commit it undid via the structured revert link"
        );
        let kept = integration_event_audit_live(
            &main,
            Some(&src),
            &registry,
            "staging",
            "hr-management",
            &i,
        )
        .expect("audit");
        assert_eq!(kept.kind, IntegrationHistoryKind::Merged);
        assert_eq!(kept.reverted_main_commit_sha, None);
    }

    /// One batched prefilter plus one batch `git log` for every included SHA,
    /// plus one `git show` for the commit's changes: a 3-commit event costs a
    /// small constant, proving no per-commit spawn loop (the defect this crate
    /// already paid ~12 spawns per row for).
    #[test]
    fn audit_three_commit_event_costs_one_batch_call_not_one_per_commit() {
        let (src, main, _order, m, registry) = sandbox_audit_three();
        let audit = integration_event_audit_live(
            &main,
            Some(&src),
            &registry,
            "staging",
            "hr-management",
            &m,
        )
        .expect("audit");
        assert_eq!(audit.included_commits.len(), 3);
        let src_spawns = crate::services::engine::git::git_spawns_for_tests(&src);
        assert_eq!(
            src_spawns, 2,
            "3 included commits must cost two batched calls (one `cat-file --batch-check` prefilter + one `git log --no-walk`), not one spawn per commit (got {src_spawns})"
        );
        let target_spawns = crate::services::engine::git::git_spawns_for_tests(&main);
        assert!(
            target_spawns <= 8,
            "one event audit cost {target_spawns} target spawns, expected O(1): branch resolution + one tip-keyed walk + one `git show`"
        );
        assert_eq!(
            crate::services::engine::history::replay_scans_for_tests(&main, "staging"),
            1,
            "the audit shares the tip-keyed replay instead of re-walking history"
        );
        let _ = integration_event_audit_live(
            &main,
            Some(&src),
            &registry,
            "staging",
            "hr-management",
            &m,
        )
        .expect("second audit");
        assert_eq!(
            crate::services::engine::history::replay_scans_for_tests(&main, "staging"),
            1,
            "a repeated click reuses the tip-keyed replay"
        );
    }

    // --- main commit diff (whole commit, one call) ------------------------

    #[test]
    fn main_commit_diff_returns_every_changed_file_in_one_call() {
        let (_src, main, _order, m, registry) = sandbox_audit_three();
        let before = crate::services::engine::git::git_spawns_for_tests(&main);
        let diff = main_commit_diff_live(&main, &registry, "staging", &m).expect("diff");
        let after = crate::services::engine::git::git_spawns_for_tests(&main);
        assert_eq!(diff.main_commit_sha, m);
        let paths: Vec<&str> = diff.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["x.txt", "y.txt"],
            "one entry per changed file, sorted by path"
        );
        for file in &diff.files {
            assert!(!file.binary, "{} is text", file.path);
            assert!(!file.hunks.is_empty(), "{} carries its hunks", file.path);
            assert_eq!(file.additions, 1, "{} adds one line", file.path);
            assert_eq!(file.deletions, 0);
            assert_eq!(file.language, fixtures::language_for(&file.path));
            assert!(
                file.source_url.contains("/commit/") && file.source_url.contains(&file.path),
                "source url names the main repo commit and path: {}",
                file.source_url
            );
        }
        assert_eq!(
            after - before,
            2,
            "one rev-parse + one whole-commit `git show`, never a spawn per file"
        );
    }

    #[test]
    fn main_commit_diff_resolves_rename_and_delete_paths_like_the_audit() {
        let main = tmp("main-rename");
        init(&main, "master");
        write(&main, "keep.txt", "keep\n");
        write(&main, "old.txt", "old\n");
        write(&main, "gone.txt", "gone\n");
        let _ = commit_all(&main, "base");
        git(&main, &["branch", "staging"]);
        git(&main, &["checkout", "-q", "staging"]);
        git(&main, &["mv", "old.txt", "new.txt"]);
        git(&main, &["rm", "-q", "gone.txt"]);
        write(&main, "keep.txt", "keep\nmore\n");
        let c = commit_all(&main, "rename delete edit");

        let diff = main_commit_diff_live(&main, &single_registry(), "staging", &c).expect("diff");
        let paths: Vec<&str> = diff.files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.contains(&"new.txt"),
            "a pure rename resolves to its new path: {paths:?}"
        );
        assert!(
            !paths.contains(&"old.txt"),
            "the pre-rename path is never listed: {paths:?}"
        );
        assert!(
            paths.contains(&"gone.txt"),
            "a deletion keeps its path: {paths:?}"
        );
        let keep = diff.files.iter().find(|f| f.path == "keep.txt").unwrap();
        assert!(keep.hunks.iter().any(|h| h.header.starts_with("@@")));
        assert_eq!((keep.additions, keep.deletions), (1, 0));
        let gone = diff.files.iter().find(|f| f.path == "gone.txt").unwrap();
        assert_eq!((gone.additions, gone.deletions), (0, 1));
    }

    #[test]
    fn main_commit_diff_uses_the_first_parent_delta_for_a_merge() {
        let main = tmp("main-merge");
        init(&main, "master");
        write(&main, "base.txt", "base\n");
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);
        git(&main, &["checkout", "-q", "staging"]);
        write(&main, "base.txt", "base\nstaging\n");
        let _ = commit_all(&main, "staging edit");
        git(&main, &["checkout", "-q", "master"]);
        write(&main, "other.txt", "other\n");
        let _ = commit_all(&main, "master work");
        git(
            &main,
            &["merge", "-q", "--no-ff", "-m", "merge staging", "staging"],
        );
        let merge = git(&main, &["rev-parse", "HEAD"]);

        // A plain `git show -p` on a merge is empty; the audit's own `changes`
        // (and the promotion engine) read the first-parent delta, so this must
        // too — otherwise a merged integration would show no diff at all.
        let diff =
            main_commit_diff_live(&main, &single_registry(), "staging", &merge).expect("diff");
        assert!(
            diff.files.iter().any(|f| f.path == "base.txt"),
            "the merge's first-parent delta lists base.txt: {:?}",
            diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn main_commit_diff_unknown_sha_is_a_truthful_not_found() {
        let (_src, main, _order, _m, registry) = sandbox_audit_three();
        let err = main_commit_diff_live(&main, &registry, "staging", &"f".repeat(40))
            .expect_err("an unknown sha is not found, never fabricated");
        assert_eq!(err.code, ErrorCode::NotFound);
    }

    fn status_page_for(src: &Path, main: &Path, registry: &Registry, sha: &str) -> CommitStatus {
        let github = StubGitHubOps::new();
        let query = CommitQuery {
            subsystem_id: "hr-management".to_string(),
            target_branch: "staging".to_string(),
            page: 1,
            page_size: 50,
            search: String::new(),
            status_filter: None,
            sort: crate::github::models::CommitSort::Newest,
            shas: None,
        };
        let page = commit_page_live(main, src, registry, &github, &query).expect("page");
        page.commits
            .iter()
            .find(|c| c.sha == sha)
            .unwrap_or_else(|| panic!("{sha} missing from page"))
            .status
            .clone()
    }

    #[test]
    fn lockfile_only_difference_reports_applied_but_keeps_delta() {
        let main = tmp("status-lock-main");
        init(&main, "master");
        write(&main, "src/FiltersBar.tsx", "v1\n");
        write(&main, "src/MultiSelectFilter.tsx", "v1\n");
        write(&main, "package-lock.json", "lock-old\n");
        write(&main, "package.json", "{\"name\":\"x\"}\n");
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);
        git(&main, &["checkout", "-q", "staging"]);
        write(&main, "src/FiltersBar.tsx", "v2\n");
        write(&main, "src/MultiSelectFilter.tsx", "v2\n");
        let _ = commit_all(&main, "target takes feature");

        let src = tmp("status-lock-src");
        init(&src, "master");
        write(&src, "src/FiltersBar.tsx", "v1\n");
        write(&src, "src/MultiSelectFilter.tsx", "v1\n");
        write(&src, "package-lock.json", "lock-old\n");
        write(&src, "package.json", "{\"name\":\"x\"}\n");
        let _ = commit_all(&src, "base");
        write(&src, "src/FiltersBar.tsx", "v2\n");
        write(&src, "src/MultiSelectFilter.tsx", "v2\n");
        write(&src, "package-lock.json", "lock-new\n");
        let feature = commit_all(&src, "feature plus lock bump");

        let registry = single_registry();
        let status = status_page_for(&src, &main, &registry, &feature);
        assert_eq!(status.kind, CommitStatusKind::Applied);
        assert!(
            status
                .detail
                .as_deref()
                .is_some_and(|d| d.contains("deferred")),
            "applied reason names the deferred exclusion: {:?}",
            status.detail
        );
        let commits = parse_source_log(&src, "master").expect("log");
        let logged = commits
            .iter()
            .find(|c| c.sha == feature)
            .expect("feature logged");
        assert!(
            logged
                .deltas
                .iter()
                .any(|(p, _, _)| p == "package-lock.json"),
            "lockfile stays in the delta: {:?}",
            logged.deltas
        );
    }

    #[test]
    fn main_owned_only_difference_reports_applied() {
        let main = tmp("status-owned-main");
        init(&main, "master");
        write(&main, "app.txt", "v1\n");
        write(&main, ".github/workflows/ci.yml", "old\n");
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);

        let src = tmp("status-owned-src");
        init(&src, "master");
        write(&src, "app.txt", "v1\n");
        write(&src, ".github/workflows/ci.yml", "old\n");
        let _ = commit_all(&src, "base");
        write(&src, ".github/workflows/ci.yml", "new\n");
        let feature = commit_all(&src, "workflow tweak");

        let status = status_page_for(&src, &main, &single_registry(), &feature);
        assert_eq!(status.kind, CommitStatusKind::Applied);
    }

    #[test]
    fn genuinely_out_of_sync_code_stays_eligible() {
        let main = tmp("status-code-main");
        init(&main, "master");
        write(&main, "app.txt", "v1\n");
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);

        let src = tmp("status-code-src");
        init(&src, "master");
        write(&src, "app.txt", "v1\n");
        let _ = commit_all(&src, "base");
        write(&src, "app.txt", "v2\n");
        let feature = commit_all(&src, "real work");

        let status = status_page_for(&src, &main, &single_registry(), &feature);
        assert_eq!(status.kind, CommitStatusKind::Eligible);
    }

    #[test]
    fn root_manifest_delta_stays_eligible() {
        let main = tmp("status-manifest-main");
        init(&main, "master");
        write(&main, "app.txt", "v1\n");
        write(
            &main,
            "package.json",
            "{\"name\":\"x\",\"dependencies\":{}}\n",
        );
        let _ = commit_all(&main, "main base");
        git(&main, &["branch", "staging"]);

        let src = tmp("status-manifest-src");
        init(&src, "master");
        write(&src, "app.txt", "v1\n");
        write(
            &src,
            "package.json",
            "{\"name\":\"x\",\"dependencies\":{}}\n",
        );
        let _ = commit_all(&src, "base");
        write(
            &src,
            "package.json",
            "{\"name\":\"x\",\"dependencies\":{\"left-pad\":\"^1.3.0\"}}\n",
        );
        let feature = commit_all(&src, "add dep");

        let status = status_page_for(&src, &main, &single_registry(), &feature);
        assert_eq!(status.kind, CommitStatusKind::Eligible);
    }
}

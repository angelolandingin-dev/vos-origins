//! Live read path: registry-backed subsystems, reachable-history status,
//! GitHub PRs/branches, and source-repo commits/details/diffs.
//!
//! Live is the default; fixtures only when `VOS_USE_FIXTURES=1`.
//! Every response is wholly live or wholly fixture and says which via
//! [`DataSource`]. All network/GitHub access goes through [`GitHubOps`]
//! plus a source-repo/local-git seam so tests run fully offline.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::{self, now_rfc3339};
use crate::github::models::{
    ChangedFile, CommitDetails, CommitDiff, CommitPage, CommitQuery, CommitStatus,
    CommitStatusKind, CommitSummary, DataSource, DependencyChange, DependencyChangeKind,
    DependencySection, DiffHunk, DiffLine, DiffLineKind, FileChangeKind, FileIntegrationPolicy,
    IntegrationEvidence, IntegrationHistory, IntegrationHistoryEntry, IntegrationHistoryKind,
    IntegrationPr, IntegrationStatus, PrState, PullRequestRef, ReviewState, SubsystemPolicy,
    SubsystemSummary, TargetBranch, TargetBranchList, TargetBranchPolicy, WorkflowRunStatus,
};
use crate::services::engine::github_ops::{BranchInfo, GitHubOps, PullRequestInfo};
use crate::services::engine::history::BranchStatusKind;
use crate::services::engine::{classify, deps, git, history};
use crate::services::registry::{Registry, SubsystemEntry};

pub const MAX_DIFF_LINES: usize = 1500;
const MAX_SOURCE_COMMITS: usize = 2000;
const WORKFLOW_FILE: &str = "promote-subsystem.yml";

/// Live is default; fixtures only on explicit opt-in.
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
        "Register the subsystem first; the registry override lives at %APPDATA%\\subsystem-integration-manager\\registry.json.",
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

// ---------------------------------------------------------------------------
// Registry -> summaries
// ---------------------------------------------------------------------------

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

/// Registry-backed subsystem summaries. No fixtures in live mode.
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

fn require_allowed_target(registry: &Registry, branch: &str) -> AppResult<String> {
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

// ---------------------------------------------------------------------------
// Checkout resolution (reusable on-disk cache)
// ---------------------------------------------------------------------------

/// Resolve the target checkout: `VOS_TARGET_REPO_DIR` when set, a local-dir
/// passthrough when the registry points at one, else the shared checkout
/// cache (clone once with `--no-tags`, `git fetch` on reuse). The cleanup
/// dir is always `None`: cached checkouts are owned by the cache and are
/// never deleted by readers, so repeat visits reuse the same clone.
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

/// Resolve a source checkout for `entry`: a local dir passes through, else
/// the shared checkout cache keyed by subsystem+repository. The cleanup dir
/// is always `None` (see above).
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

// ---------------------------------------------------------------------------
// Status: reachable trailers + open-PRs overlay (never fabricate Integrated)
// ---------------------------------------------------------------------------

fn commit_iso_subject(
    repo_dir: &Path,
    sha: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let iso = git::run(repo_dir, &["show", "-s", "--format=%cI", sha])
        .ok()
        .filter(|o| o.status == 0)
        .map(|o| o.stdout_trimmed())
        .filter(|s| !s.is_empty());
    let subject = git::run(repo_dir, &["show", "-s", "--format=%s", sha])
        .ok()
        .filter(|o| o.status == 0)
        .map(|o| o.stdout_trimmed());
    let message = git::run(repo_dir, &["show", "-s", "--format=%B", sha])
        .ok()
        .filter(|o| o.status == 0)
        .map(|o| o.stdout_str());
    (iso, subject, message)
}

fn evidence_for(
    repo_dir: &Path,
    integration_commit: &str,
    queried_branch: &str,
    promotion_target: Option<&str>,
) -> IntegrationEvidence {
    let (iso, subject, message) = commit_iso_subject(repo_dir, integration_commit);
    let target = promotion_target.unwrap_or(queried_branch).to_string();
    let inherited = if target == queried_branch {
        None
    } else {
        Some(target.clone())
    };
    IntegrationEvidence {
        main_commit_sha: integration_commit.to_string(),
        merged_at: iso.unwrap_or_else(now_rfc3339),
        message: message
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .or(subject)
            .unwrap_or_else(|| format!("Integration {integration_commit}")),
        pull_request_number: None,
        target_branch: queried_branch.to_string(),
        inherited_from_branch: inherited,
    }
}

fn branch_result_to_status(
    repo_dir: &Path,
    queried_branch: &str,
    result: &history::BranchStatusResult,
) -> CommitStatus {
    match result.status {
        BranchStatusKind::Integrated => {
            let evidence = result.integration_commit.as_deref().map(|commit| {
                evidence_for(repo_dir, commit, queried_branch, result.promotion_target.as_deref())
            });
            let detail = match &evidence {
                Some(ev) => format!(
                    "Integration commit {} recorded this source SHA on {}.",
                    ev.main_commit_sha.chars().take(7).collect::<String>(),
                    queried_branch
                ),
                None => format!("Recorded as integrated on {queried_branch}."),
            };
            CommitStatus::of(
                CommitStatusKind::Integrated,
                queried_branch,
                Some(detail),
                evidence,
            )
        }
        BranchStatusKind::Reverted => {
            let evidence = result.integration_commit.as_deref().map(|commit| {
                evidence_for(repo_dir, commit, queried_branch, result.promotion_target.as_deref())
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
                "Reachable from the protected source branch. No successful integration found for {queried_branch}."
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

/// Core status computation against an already-resolved target checkout.
/// `Integrated` only from reachable trailers; open PRs overlay as `PrOpen`.
pub fn integration_statuses_live(
    target_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    target_branch: &str,
    subsystem_id: Option<&str>,
    shas: &[String],
) -> AppResult<Vec<IntegrationStatus>> {
    let target_branch = require_allowed_target(registry, target_branch)?;
    if git::rev_parse(target_dir, &format!("refs/heads/{target_branch}")).is_none()
        && git::rev_parse(target_dir, &format!("refs/remotes/origin/{target_branch}")).is_none()
        && git::rev_parse(target_dir, &target_branch).is_none()
    {
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
    let open_prs = github
        .list_open_pull_requests(&registry.main_repository, &target_branch)
        .map_err(gh_error_to_app)?;
    let checked_at = now_rfc3339();
    let mut out = Vec::new();
    for id in ids {
        let results = history::compute_branch_status(target_dir, &target_branch, &id, shas);
        for result in results {
            let mut status = branch_result_to_status(target_dir, &target_branch, &result);
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

// ---------------------------------------------------------------------------
// Target branches: existing GitHub branches filtered by registry policy
// ---------------------------------------------------------------------------

fn target_policy(registry: &Registry) -> TargetBranchPolicy {
    TargetBranchPolicy {
        allowed_target_branches: registry.promotion.allowed_target_branches.clone(),
        allowed_target_branch_patterns: registry.promotion.allowed_target_branch_patterns.clone(),
        require_pull_request: registry.promotion.require_pull_request,
        allow_direct_push: false,
        workflow_file: WORKFLOW_FILE.to_string(),
    }
}

/// Existing branches from GitHub, filtered by registry policy. Every returned
/// branch exists; disallowed ones carry `allowed=false` + the policy reason.
pub fn target_branches_live(
    registry: &Registry,
    github: &dyn GitHubOps,
    selected: Option<&str>,
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
            allowed: decision.allowed,
            disallowed_reason: if decision.allowed {
                None
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
    branches.sort_by(|a, b| a.name.cmp(&b.name));
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

// ---------------------------------------------------------------------------
// Integration PRs from GitHubOps
// ---------------------------------------------------------------------------

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
    // Fallback: `integration/<id>-<short>-...` head naming.
    if let Some(rest) = head.strip_prefix("integration/") {
        let id = rest.split('-').next().unwrap_or("");
        // Heads embed `<id>-<12hex>`; require the id shape only.
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

/// Live integration PRs from [`GitHubOps`].
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

// ---------------------------------------------------------------------------
// Integration history from reachable history + open PRs
// ---------------------------------------------------------------------------

fn git_author(repo_dir: &Path, sha: &str) -> String {
    git::run(repo_dir, &["show", "-s", "--format=%an", sha])
        .ok()
        .filter(|o| o.status == 0)
        .map(|o| o.stdout_trimmed())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn resolve_history_branch(
    target_dir: &Path,
    registry: &Registry,
    requested: Option<&str>,
) -> AppResult<String> {
    match requested {
        Some(branch) => {
            let allowed = require_allowed_target(registry, branch)?;
            if git::rev_parse(target_dir, &format!("refs/heads/{allowed}")).is_none()
                && git::rev_parse(target_dir, &format!("refs/remotes/origin/{allowed}")).is_none()
                && git::rev_parse(target_dir, &allowed).is_none()
            {
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

/// Merged integrations, reverts and restores from reachable history, plus
/// pending open PRs. Not fixtures.
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
    let reference = git::rev_parse(target_dir, &format!("refs/heads/{branch}"))
        .or_else(|| git::rev_parse(target_dir, &format!("refs/remotes/origin/{branch}")))
        .or_else(|| git::rev_parse(target_dir, &branch))
        .ok_or_else(|| {
            branch_not_permitted(&branch, "branch does not exist in the target checkout")
        })?;
    let events = history::collect_events(target_dir, &reference);
    let replayed = history::replay_events(&events);
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
                actor: git_author(target_dir, &event.commit),
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
    // Pending open PRs overlay.
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

// ---------------------------------------------------------------------------
// Source commits / details / diff from a source clone (or gh api fallback)
// ---------------------------------------------------------------------------

struct SourceCommit {
    sha: String,
    subject: String,
    author_name: String,
    authored_at: String,
    committed_at: String,
    committed_unix: i64,
    parents: Vec<String>,
    files_changed: u32,
    additions: u32,
    deletions: u32,
    touches_manifest: bool,
    /// `(path, post-commit blob sha, status char)` per changed path — the blob
    /// ids let us compare content against the target branch without a new diff.
    deltas: Vec<(String, String, char)>,
}

fn parse_source_log(repo_dir: &Path, branch: &str) -> AppResult<Vec<SourceCommit>> {
    const REC: char = '\u{1e}';
    const FIELD: char = '\u{1f}';
    let fmt = format!(
        "--format=%H{FIELD}%P{FIELD}%an{FIELD}%ae{FIELD}%aI{FIELD}%cI{FIELD}%ct{FIELD}%s{REC}"
    );
    let out = git::run(
        repo_dir,
        &[
            "log",
            &format!("--max-count={MAX_SOURCE_COMMITS}"),
            &fmt,
            "--raw",
            branch,
        ],
    )
    .map_err(|err| {
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
            // Skip phantom rows that never parsed as a real header.
            if commit.sha.len() == 40 || commit.sha.len() == 64 {
                commits.push(commit);
            }
        }
    };
    for raw_line in text.lines() {
        // Strip a trailing REC marker and carriage returns; headers keep
        // their FIELD separators.
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
            // `--raw` row: `:<old_mode> <new_mode> <old_sha> <new_sha> <status>\t<path>`.
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
            "Promoting applies the first-parent delta of this exact SHA only. Newer and unselected commits are never imported."
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

/// Newest source commit that touches each path, as an index into the
/// newest-first commit list — used to spot commits a later one already covers.
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
/// dependency delta and lockfiles are regenerated, so recency must not treat
/// them as "the target is newer". The engine does not support `bun` lockfiles.
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

/// True when every one of these paths is also changed by a newer source commit,
/// so promoting the older commit would only re-apply stale content.
fn all_superseded(paths: &[&String], latest: &HashMap<String, usize>, index: usize) -> bool {
    !paths.is_empty()
        && paths
            .iter()
            .all(|path| latest.get(*path).is_some_and(|newer| *newer < index))
}

/// Epoch of the target branch's most recent change per path (bounded scan).
/// Compared against a source commit's date it answers "which side is newer?".
fn target_path_dates(target_dir: &Path, target_branch: &str) -> HashMap<String, i64> {
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
        return map;
    };
    if out.status != 0 {
        return map;
    }
    for chunk in out.stdout_str().split('\u{1e}') {
        let mut lines = chunk.lines();
        let Some(epoch) = lines
            .next()
            .map(str::trim)
            .and_then(|s| s.parse::<i64>().ok())
        else {
            continue;
        };
        for path in lines {
            let path = path.trim();
            if !path.is_empty() {
                map.entry(path.to_string()).or_insert(epoch);
            }
        }
    }
    map
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

/// Live commit page from the source repo, with per-commit status for the
/// selected target branch.
pub fn commit_page_live(
    target_dir: &Path,
    source_dir: &Path,
    registry: &Registry,
    github: &dyn GitHubOps,
    query: &CommitQuery,
) -> AppResult<CommitPage> {
    let entry = find_live_subsystem(registry, &query.subsystem_id)?;
    let target_branch = require_allowed_target(registry, &query.target_branch)?;
    if git::rev_parse(target_dir, &format!("refs/heads/{target_branch}")).is_none()
        && git::rev_parse(target_dir, &format!("refs/remotes/origin/{target_branch}")).is_none()
        && git::rev_parse(target_dir, &target_branch).is_none()
    {
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
    let target_dates = target_path_dates(target_dir, &target_branch);
    let shas: Vec<String> = commits.iter().map(|c| c.sha.clone()).collect();
    let results = history::compute_branch_status(target_dir, &target_branch, &entry.id, &shas);
    let mut status_by_sha: HashMap<String, history::BranchStatusResult> = HashMap::new();
    for result in results {
        status_by_sha.insert(result.sha.clone(), result);
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
        let branch_result = status_by_sha.get(&commit.sha);
        let mut status = match branch_result {
            Some(result) => branch_result_to_status(target_dir, &target_branch, result),
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
        // Content-level truth, computed by comparing the two trees rather than
        // from the commit's declared add/modify status: a trailer only records
        // what the tool integrated, and the subsystem's split history makes files
        // that exist on both sides look like adds (which is not a conflict).
        if matches!(
            status.kind,
            CommitStatusKind::Eligible | CommitStatusKind::Unknown
        ) {
            let out_of_sync: Vec<&String> = commit
                .deltas
                .iter()
                .map(|(path, _, _)| path)
                .filter(|path| source_tree.get(*path) != target_blobs.get(*path))
                .collect();
            // Manifest and lockfile paths are delta-merged or regenerated, never
            // written wholesale (spec §6.4/§7), so a newer copy on the target does
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
                        "Everything this commit changes already matches the target branch; there is nothing to promote."
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
                        "The target branch's copies of every source file this commit changes were themselves changed more recently; promoting would revert newer work. (Dependency manifest and lockfile paths are exempt — they are merged or regenerated.)"
                            .to_string(),
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
    })
}

// ---------------------------------------------------------------------------
// Commit details
// ---------------------------------------------------------------------------

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
        // Rename rows look like `old => new`; index both forms.
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
        classify::PathStatus::Regenerate => FileIntegrationPolicy::RegenerateInTarget,
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
    // No manifest delta → no dependency rows.
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

/// Live commit details from the source repo.
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
    // `%b` may itself contain the separator; recover the tail conservatively.
    let body = {
        let marker = format!("{subject}\x1f");
        show.stdout_str()
            .split_once(&marker)
            .map(|(_, tail)| tail.trim().to_string())
            .unwrap_or_default()
    };
    let parents: Vec<String> = parents_raw.split_whitespace().map(str::to_string).collect();
    let is_merge = parents.len() > 1;
    let (base_sha, _deltas) = git::name_status_delta(source_dir, &full_sha).map_err(|err| {
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
    // Re-derive deltas to keep rename metadata (the helper above only
    // validated readability).
    let deltas = git::name_status_delta(source_dir, &full_sha)
        .map(|(_, entries)| entries)
        .unwrap_or_default();
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
    // Status for this SHA on the selected target.
    let status = integration_statuses_live(
        target_dir,
        registry,
        github,
        &target_branch,
        Some(&entry.id),
        std::slice::from_ref(&full_sha),
    )
    .map(|mut v| {
        v.pop()
            .map(|s| s.status)
            .unwrap_or_else(|| CommitStatus::unknown(&target_branch, "Status unavailable."))
    })
    .unwrap_or_else(|_| CommitStatus::unknown(&target_branch, "Status unavailable for this SHA."));
    // Related open integration PRs.
    let mut related: Vec<PullRequestRef> = Vec::new();
    if let Ok(prs) = github.list_open_pull_requests(&registry.main_repository, &target_branch) {
        for pr in &prs {
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

// ---------------------------------------------------------------------------
// Commit diff (bounded)
// ---------------------------------------------------------------------------

fn parse_hunk_header(header: &str) -> (u32, u32) {
    // `@@ -old,count +new,count @@` — counts may be omitted when 1.
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
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_line = 1_u32;
    let mut new_line = 1_u32;
    let mut additions = 0_u32;
    let mut deletions = 0_u32;
    let flush = |current: &mut Option<DiffHunk>, hunks: &mut Vec<DiffHunk>| {
        if let Some(hunk) = current.take() {
            hunks.push(hunk);
        }
    };
    for raw in text.lines() {
        if raw.starts_with("diff --git")
            || raw.starts_with("index ")
            || raw.starts_with("--- ")
            || raw.starts_with("+++ ")
            || raw.starts_with("new file mode")
            || raw.starts_with("deleted file mode")
            || raw.starts_with("old mode")
            || raw.starts_with("new mode")
            || raw.starts_with("similarity index")
            || raw.starts_with("rename ")
        {
            continue;
        }
        if let Some(header) = raw.strip_prefix("@@") {
            flush(&mut current, &mut hunks);
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
            // `+++` already skipped above.
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
    flush(&mut current, &mut hunks);
    if hunks.is_empty() {
        return Err(AppError::not_found(format!(
            "No diff is available for '{chosen}' in commit {sha}."
        ))
        .with_remedy("Choose a file listed under 'Files Changed' in the commit details panel."));
    }
    let total_lines: u32 = hunks.iter().map(|h| h.lines.len() as u32).sum();
    // Bound the response; never embed a full editor.
    let mut budget = MAX_DIFF_LINES;
    let mut truncated = false;
    for hunk in &mut hunks {
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
            Some(format!(
                "Response bounded at {MAX_DIFF_LINES} lines to keep the window responsive; open the file on GitHub for the rest."
            ))
        } else {
            None
        },
    })
}

// ---------------------------------------------------------------------------
// Workflow runs stay fixture-labelled (no live source wired here)
// ---------------------------------------------------------------------------

/// Live mode has no workflow-run source: the engine runs in-tool and opens
/// PRs directly, so runs are empty rather than fabricated.
pub fn workflow_empty_live(total: u32) -> (String, DataSource) {
    let _ = total;
    (now_rfc3339(), DataSource::Live)
}

#[allow(dead_code)]
fn _keep_models_referenced() {
    let _ = WorkflowRunStatus::Unknown;
    let _ = HashSet::<String>::new();
    let _ = BranchInfo {
        name: String::new(),
        sha: String::new(),
        protected: false,
    };
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
        // Sorted by name.
        assert_eq!(summaries[0].name, "HR");
        assert_eq!(summaries[1].name, "Payroll");
        let hr = summaries.iter().find(|s| s.id == "hr-management").unwrap();
        assert_eq!(hr.repository, "ORG/hr-management");
        assert_eq!(hr.source_branch, "master");
        assert!(hr.enabled);
        // No fixture-only subsystems leak into live mode.
        assert!(!summaries.iter().any(|s| s.id == "inventory-management"));
        assert!(!summaries.iter().any(|s| s.id == "document-management"));
    }

    // --- local git sandbox ----------------------------------------------

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

    /// Source with A (base) then B (feature); main with staging + a real
    /// trailer integration commit for B only.
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
        // Real trailer integration for B on staging.
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
        // No PRs: B integrated via trailer, A eligible.
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
        // Open-PR-only SHA reports PrOpen, never Integrated.
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
        // An already-integrated SHA stays Integrated even with an open PR.
        assert_eq!(
            by_sha[&b.as_str()].status.kind,
            CommitStatusKind::Integrated
        );
        let _ = _src;
    }

    #[test]
    fn target_branch_policy_filtering_marks_allowed_and_reason() {
        let registry = single_registry();
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
        let list = target_branches_live(&registry, &github, Some("staging")).expect("branches");
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
    fn fixture_flag_and_target_dir_seam() {
        let _env = crate::test_support::env_lock();
        // Fixture flag defaults off; explicit 1 opts into fixtures.
        std::env::remove_var("VOS_USE_FIXTURES");
        assert!(!use_fixtures());
        std::env::set_var("VOS_USE_FIXTURES", "1");
        assert!(use_fixtures());
        // Fixture world stays reachable for UI dev.
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
        // History: the trailer integration for B is a Merged entry.
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
        // PRs: only trailers/heads count as integration PRs.
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
        // Commits: source log with live statuses.
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
        // Details: changed files with add flags + dependency rows when the
        // manifest moves; diff: bounded unified hunks for a path.
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
}

//! Promotion dispatch via the in-tool engine (§4/§9.2).
//! Fixture mode returns a labelled simulation naming the missing prerequisites.

use std::path::PathBuf;

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::now_rfc3339;
use crate::github::gh_cli;
use crate::github::models::*;
use crate::services::engine::github_ops::{authenticated_remote_url, GitHubOps, RealGitHubOps};
use crate::services::engine::lockfile::RealLockfileGenerator;
use crate::services::engine::{
    self, Blocker, EngineDeps, EngineOptions, EngineRequest, PromotionReport,
};
use crate::services::registry::{assert_full_sha, Registry, SubsystemEntry};
use crate::AppState;

#[tauri::command]
pub async fn dispatch_promotion(
    state: State<'_, AppState>,
    request: PromotionRequest,
) -> AppResult<PromotionDispatchResult> {
    if !request.confirmation_acknowledged {
        return Err(AppError::invalid_request(
            "The promotion was not confirmed: the confirmation line acknowledging an integration PR was not accepted.",
        )
        .with_remedy("Re-open the promotion dialog and confirm that this creates an integration PR against the selected target branch."));
    }

    if request.source_commit_shas.is_empty() {
        return Err(AppError::invalid_request("No source commit SHAs were selected.")
            .with_remedy("Tick at least one eligible commit in the Commit Browser or the Promote Commits list."));
    }

    // Same shape gate as the engine (`assert_full_sha`): 40/64 lowercase hex.
    // Uppercase is rejected and SHA-256 accepted, exactly like the engine.
    if let Some(invalid) = request
        .source_commit_shas
        .iter()
        .find(|sha| assert_full_sha(sha, "source_commit_shas[]").is_err())
    {
        return Err(AppError::invalid_request(format!(
            "'{invalid}' is not a full 40/64-character lowercase source commit SHA."
        ))
        .with_remedy(
            "Use full SHAs. Abbreviated SHAs are only for display and cannot be promoted.",
        ));
    }

    if request.mode == PromotionMode::Cumulative && request.base_sha.is_none() {
        return Err(AppError::invalid_request(
            "Cumulative mode requires an explicitly verified baseline SHA.",
        )
        .with_remedy("Provide the baseline SHA, or switch to individual mode to promote only the selected deltas."));
    }

    let registry = Registry::load(Registry::app_data_override_path().as_deref())?;

    let probe = tauri::async_runtime::spawn_blocking(gh_cli::probe)
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::Internal, format!("The environment probe did not complete: {error}"))
                .with_remedy("Retry; if it keeps failing, run `gh auth status` and `git --version` in a terminal.")
                .retryable()
        })?;

    let token = gh_cli::auth_token();
    let Some(token) = token else {
        return simulate(
            &registry,
            &request,
            &probe,
            Some(
                "No GitHub token is available (GH_TOKEN / GITHUB_TOKEN / `gh auth token`)."
                    .to_string(),
            ),
        );
    };

    let mut cloned_dir: Option<PathBuf> = None;
    let target_repo_dir = match std::env::var("VOS_TARGET_REPO_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
    {
        Some(dir) => dir,
        None => {
            let stamp: String = now_rfc3339()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect();
            let temp =
                std::env::temp_dir().join(format!("vos-target-{}-{stamp}", std::process::id()));
            let url = authenticated_remote_url(&registry.main_repository, &token);
            match crate::services::engine::git::clone_no_tags(&url, &temp) {
                Ok(()) => {
                    cloned_dir = Some(temp.clone());
                    temp
                }
                Err(err) => {
                    return simulate(
                        &registry,
                        &request,
                        &probe,
                        Some(format!(
                            "Cloning {} failed: {}",
                            registry.main_repository,
                            crate::services::engine::git::sanitize(&err.to_string())
                        )),
                    );
                }
            }
        }
    };

    gh_cli::require_prerequisites(&probe)?;

    state.cache.clear();

    let engine_registry = registry.clone();
    let engine_request = EngineRequest {
        subsystem_id: request.subsystem_id.clone(),
        commits: request.source_commit_shas.clone(),
        mode: request.mode,
        base_sha: request.base_sha.clone(),
        target_branch: request.target_branch.clone(),
    };

    let report = tauri::async_runtime::spawn_blocking(move || {
        let github = RealGitHubOps::new(token.clone());
        let lockfile = RealLockfileGenerator;
        let source_url = {
            let token = token.clone();
            move |subsystem: &SubsystemEntry| {
                Some(authenticated_remote_url(&subsystem.repository, &token))
            }
        };
        let temp_root = std::env::temp_dir().join("vos-integration");
        let _ = std::fs::create_dir_all(&temp_root);
        let options = EngineOptions {
            target_repo_dir,
            temp_root,
            push_remote_url: None,
            token: Some(token),
            allow_shared_config: false,
            allow_removals: false,
            nonce: format!("{}-{}", std::process::id(), now_rfc3339()),
            push: true,
            create_pr: true,
        };
        let deps = EngineDeps {
            github: &github as &dyn GitHubOps,
            lockfile: &lockfile,
            source_url: &source_url,
        };
        engine::run_promotion(&engine_registry, &engine_request, &options, &deps)
    })
    .await
    .map_err(|error| {
        AppError::new(
            ErrorCode::Internal,
            format!("The promotion engine did not complete: {error}"),
        )
        .with_remedy(
            "Retry; if it keeps failing, inspect the engine diagnostics in the parameter details.",
        )
        .retryable()
    })?;

    if !report.ok {
        cleanup_clone(&mut cloned_dir);
        return Err(map_blocker(&report));
    }

    let branch = report.integration_branch.clone().unwrap_or_default();
    let pr_url = report.pull_request.as_ref().and_then(|pr| pr.url.clone());
    let pr_number = report.pull_request.as_ref().and_then(|pr| pr.number);
    let commit_sha = report.commit_sha.clone();
    cleanup_clone(&mut cloned_dir);

    Ok(PromotionDispatchResult {
        accepted: true,
        pr_url: pr_url.clone(),
        pr_number,
        branch: branch.clone(),
        commit_sha: commit_sha.clone(),
        target_branch: request.target_branch.clone(),
        message: format!(
            "Integration branch '{branch}' was created from the {target} tip, the selected delta applied, and a pull request opened with base '{target}'{pr_note}.",
            target = request.target_branch,
            pr_note = pr_url.map(|url| format!(" ({url})")).unwrap_or_default()
        ),
        requested_at: now_rfc3339(),
        findings: engine_findings(&report),
    })
}

fn simulate(
    registry: &Registry,
    request: &PromotionRequest,
    probe: &gh_cli::GhProbe,
    reason: Option<String>,
) -> AppResult<PromotionDispatchResult> {
    let nonce = now_rfc3339();
    let branch = engine::integration_branch_name(
        &request.subsystem_id,
        &request.source_commit_shas,
        &request.target_branch,
        &nonce,
    );

    let mut findings = Vec::new();
    findings.push(ValidationFinding::warning(
        "engine_not_configured",
        "No in-tool promotion ran, so no branch or pull request was created.",
        "Set VOS_TARGET_REPO_DIR to a local clone of the main-system repository (and authenticate gh) to run the real in-tool promotion engine.",
    ));
    if let Some(reason) = reason {
        findings.push(ValidationFinding::warning(
            "engine_unavailable",
            reason,
            "Fix the prerequisite and retry, or set VOS_TARGET_REPO_DIR to a local clone of the main-system repository.",
        ));
    }
    findings.push(ValidationFinding::info(
        "no_direct_push",
        format!(
            "A live promotion would open an integration PR against {}/{}. Nothing is pushed directly to a protected branch.",
            registry.main_repository, request.target_branch
        ),
    ));
    findings.push(ValidationFinding::info(
        "selection_scope",
        format!(
            "Only the first-parent delta of the {} selected SHA(s) would be applied. Unselected earlier or later source commits are never imported.",
            request.source_commit_shas.len()
        ),
    ));
    findings.push(ValidationFinding::info(
        "prerequisites",
        format!(
            "Detected: gh {}, git {}, package manager {}. Token available: {}.",
            probe
                .version
                .clone()
                .unwrap_or_else(|| "missing".to_string()),
            probe
                .git_version
                .clone()
                .unwrap_or_else(|| "missing".to_string()),
            probe
                .package_manager
                .clone()
                .unwrap_or_else(|| "missing".to_string()),
            probe.token_available
        ),
    ));

    Ok(PromotionDispatchResult {
        accepted: true,
        pr_url: None,
        pr_number: None,
        branch: branch.clone(),
        commit_sha: None,
        target_branch: request.target_branch.clone(),
        message: format!(
            "Fixture mode: no engine ran and nothing was created. The integration branch would be '{branch}' targeting '{}'.",
            request.target_branch
        ),
        requested_at: now_rfc3339(),
        findings,
    })
}

fn engine_findings(report: &PromotionReport) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();
    findings.push(ValidationFinding::info(
        "no_direct_push",
        "The integration branch was pushed and a pull request opened; nothing was pushed directly to a protected branch.",
    ));
    findings.push(ValidationFinding::info(
        "selection_scope",
        format!(
            "Applied only the first-parent delta of {} selected SHA(s). Unselected commits were never imported.",
            report.verification.order.len()
        ),
    ));
    if !report.changes.main_owned_skipped.is_empty() {
        findings.push(ValidationFinding::warning(
            "main_owned_skipped",
            format!("Not imported (main-owned): {}", report.changes.main_owned_skipped.join(", ")),
            "Main-owned files (for example .github/workflows/**) are never overwritten by a promotion.",
        ));
    }
    if !report.changes.secret_excluded.is_empty() {
        findings.push(ValidationFinding::warning(
            "secret_excluded",
            format!(
                "Not imported (secret-looking): {}",
                report.changes.secret_excluded.join(", ")
            ),
            "Secret-looking files (.env*, credentials, keys) are never imported automatically.",
        ));
    }
    if !report.dependencies.added.is_empty() || !report.dependencies.updated.is_empty() {
        findings.push(ValidationFinding::info(
            "dependencies",
            format!(
                "Dependency delta applied: {} added, {} updated. The target lockfile was regenerated.",
                report.dependencies.added.len(),
                report.dependencies.updated.len()
            ),
        ));
    }
    for warning in &report.warnings {
        findings.push(ValidationFinding::info(
            &warning.code,
            warning.message.clone(),
        ));
    }
    findings
}

fn cleanup_clone(dir: &mut Option<PathBuf>) {
    if let Some(path) = dir.take() {
        let _ = std::fs::remove_dir_all(path);
    }
}

fn map_blocker(report: &PromotionReport) -> AppError {
    let blocker = report.blockers.first().cloned().unwrap_or_else(|| {
        Blocker::new("ENGINE_ERROR", "The promotion failed without a diagnostic.")
    });

    let (code, remedy) = match blocker.code.as_str() {
        "SUBSYSTEM_NOT_REGISTERED" | "SUBSYSTEM_DISABLED" | "SUBSYSTEM_INVALID" | "REGISTRY_INVALID" => (
            ErrorCode::InvalidRegistry,
            "Register or enable the subsystem in the tool registry (Settings → registry.json).",
        ),
        "TARGET_INVALID" | "TARGET_NOT_ALLOWED" | "TARGET_NOT_FOUND" | "TARGET_NOT_LOCAL" => (
            ErrorCode::BranchNotPermitted,
            "Choose a target branch that exists and is allowed by the registry policy (promotion.allowedTargetBranches / allowedTargetBranchPatterns).",
        ),
        "COMMIT_VERIFICATION_FAILED" => (
            ErrorCode::SourceCommitNotEligible,
            "Only commits reachable from the configured protected source branch can be promoted; use full 40-character SHAs in ancestry order.",
        ),
        "DUPLICATE_ACTIVE" | "PR_ALREADY_OPEN" => (
            ErrorCode::DuplicateIntegration,
            "Inspect the existing integration under Integration History, or promote a different SHA.",
        ),
        "PR_LOOKUP_FAILED" => (
            ErrorCode::Offline,
            "The tool could not list open pull requests; retry once the GitHub connection is healthy.",
        ),
        "PATCH_CONFLICT" | "UNSAFE_PATH" => (
            ErrorCode::PatchConflict,
            "The selected delta could not be applied cleanly. Resolve the divergence in the main system, then re-promote.",
        ),
        "DEPENDENCY_INCOMPATIBLE_RANGE" | "DEPENDENCY_RANGE_UNKNOWN" | "DEPENDENCY_REMOVAL_REQUIRES_REVIEW"
        | "MANIFEST_KEY_REQUIRES_REVIEW" | "MANIFEST_SHAPE_REQUIRES_REVIEW" => (
            ErrorCode::DependencyConflict,
            "Resolve the shared dependency range deliberately in the main-system manifest, then re-promote. The tool never force-installs.",
        ),
        "MISSING_DEPENDENCY_PREREQUISITE" => (
            ErrorCode::DependencyConflict,
            "Include the commit that introduced the dependency in the selection, then re-promote.",
        ),
        "LOCKFILE_REGENERATION_FAILED" | "LOCKFILE_MANAGER_UNSUPPORTED" => (
            ErrorCode::DependencyConflict,
            "Regenerate the lockfile with the project's package manager (npm/pnpm/yarn) and re-promote; bun lockfiles are unsupported.",
        ),
        "NOOP_PROMOTION" => (
            ErrorCode::InvalidRequest,
            "The selected delta produced no applicable changes. Select the commit that carries the actual change.",
        ),
        "SHARED_CONFIG_REQUIRES_REVIEW" => (
            ErrorCode::InvalidRequest,
            "Shared/global config changed. This requires explicit review before it can be promoted.",
        ),
        "PUSH_FAILED" => (
            ErrorCode::Unauthenticated,
            "The branch could not be pushed. Confirm gh is authenticated with the 'repo' scope and that VOS_TARGET_REPO_DIR is a clone you can push to.",
        ),
        "PR_CREATION_FAILED" => (
            ErrorCode::RepositoryInaccessible,
            "The branch was pushed but the pull request could not be created. Check repository permissions and retry.",
        ),
        "ENGINE_ERROR" | "INTEGRITY_BRANCH_ANCESTRY" => (
            ErrorCode::Internal,
            "The engine stopped unexpectedly. Review the repository state and retry.",
        ),
        _ => (ErrorCode::Internal, "The promotion failed. Review the diagnostics and retry."),
    };

    let mut error = AppError::new(code, blocker.message).with_remedy(remedy);
    if let Some(details) = blocker.details {
        error = error.with_details(details);
    }
    error
}

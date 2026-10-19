//! Promotion dispatch via the in-tool engine (§4/§9.2).
//! Fixture mode returns a labelled simulation naming the missing prerequisites.
//! Delivery is local-first: `local_direct` (default) commits the delta straight
//! onto the local target branch (no push, no PR — pushing stays manual);
//! `local` builds a branch + commit in the user's clone (no push, no PR);
//! `worktree` writes the delta into the user's own checkout of the target
//! branch and stages it (no branch, no commit, no push, no PR — the user
//! commits themselves); `push_pr` is the explicit opt-in that pushes and
//! opens a PR.

use std::path::{Path, PathBuf};

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures::now_rfc3339;
use crate::github::gh_cli;
use crate::github::models::*;
use crate::services::engine::github_ops::{authenticated_remote_url, GitHubOps, RealGitHubOps};
use crate::services::engine::lockfile::RealLockfileGenerator;
use crate::services::engine::preflight::PreflightRequest;
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
            "The promotion was not confirmed: the confirmation line acknowledging the delivery mode was not accepted.",
        )
        .with_remedy("Re-open the promotion dialog and confirm the selected delivery mode."));
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

    let delivery = request.delivery;

    // Target checkout resolution. `local*`/`worktree` prefers the user's own clone:
    // registry `localMainRepositoryPath` → `VOS_TARGET_REPO_DIR` → the
    // tool-managed checkout cache. `push_pr` keeps today's behaviour
    // (`VOS_TARGET_REPO_DIR` else a disposable temp clone).
    let (target_repo_dir, cloned_dir, used_managed_fallback) = match delivery {
        PromotionDelivery::Local | PromotionDelivery::LocalDirect | PromotionDelivery::Worktree => {
            let (dir, fallback) = resolve_local_target(&registry, &token)?;
            (dir, None, fallback)
        }
        PromotionDelivery::PushPr => {
            let (dir, cloned) = resolve_push_pr_target(&registry, &token).await?;
            (dir, cloned, false)
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
        delivery,
        on_conflict: request.on_conflict,
        message: request.message.clone(),
        excluded_paths: request.excluded_paths.clone(),
    };
    let is_push_pr = delivery == PromotionDelivery::PushPr;
    let local_dir_for_commands = target_repo_dir.clone();
    let subsystem_for_title = request.subsystem_id.clone();
    let commit_count = request.source_commit_shas.len();
    let target_for_message = request.target_branch.clone();

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
            push: is_push_pr,
            create_pr: is_push_pr,
        };
        // Cached in `gh_cli`, so this costs at most one `gh api user` per burst.
        let deps = EngineDeps {
            github: &github as &dyn GitHubOps,
            lockfile: &lockfile,
            source_url: &source_url,
            account: gh_cli::authenticated_account,
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
        let mut cloned = cloned_dir;
        cleanup_clone(&mut cloned);
        return Err(map_blocker(&report));
    }

    let branch = report.integration_branch.clone().unwrap_or_default();
    let commit_sha = report.commit_sha.clone();
    let mut cloned = cloned_dir;
    cleanup_clone(&mut cloned);

    // `local_direct`: the commit already landed on the local target branch —
    // report it there (never an integration branch), with the literal push
    // command as the only manual step. Never a push, never a PR.
    if delivery == PromotionDelivery::LocalDirect {
        let next_commands =
            build_direct_next_commands(&local_dir_for_commands, &target_for_message);
        let where_note = if used_managed_fallback {
            format!(
                " in the tool-managed checkout '{}' (your local clone was not configured, so the managed checkout under the checkouts cache was used — this is not your personal clone)",
                local_dir_for_commands.display()
            )
        } else {
            format!(
                " in your local clone '{}'",
                local_dir_for_commands.display()
            )
        };
        let short_sha = commit_sha.clone().unwrap_or_default();
        return Ok(PromotionDispatchResult {
            accepted: true,
            pr_url: None,
            pr_number: None,
            branch: target_for_message.clone(),
            commit_sha: commit_sha.clone(),
            target_branch: request.target_branch.clone(),
            message: format!(
                "The selected delta was committed onto your local '{target}'{where_note} (commit {short_sha}). No push, no PR — committed to {target} locally, push when ready with the listed push command.",
                target = target_for_message,
            ),
            requested_at: now_rfc3339(),
            findings: engine_findings_local_direct(&report, used_managed_fallback),
            delivery: PromotionDelivery::LocalDirect,
            next_commands,
            replaced: report.changes.replaced.clone(),
            deleted: report.changes.deleted.clone(),
            unchanged: report.changes.unchanged.clone(),
            flagged: report.changes.flagged.clone(),
            excluded: report.changes.excluded.clone(),
        });
    }

    if delivery == PromotionDelivery::Local {
        let next_commands = build_next_commands(
            &local_dir_for_commands,
            &branch,
            &target_for_message,
            &subsystem_for_title,
            commit_count,
        );
        let where_note = if used_managed_fallback {
            format!(
                " in the tool-managed checkout '{}' (your local clone was not configured, so the managed checkout under the checkouts cache was used — this is not your personal clone)",
                local_dir_for_commands.display()
            )
        } else {
            format!(
                " in your local clone '{}'",
                local_dir_for_commands.display()
            )
        };
        let short_sha = commit_sha.clone().unwrap_or_default();
        let message = format!(
            "Integration branch '{branch}' was created from the {target} tip{where_note} with the selected delta applied (commit {short_sha}). No push, no PR — you push it yourself with the listed next commands.",
            target = target_for_message,
        );
        return Ok(PromotionDispatchResult {
            accepted: true,
            pr_url: None,
            pr_number: None,
            branch: branch.clone(),
            commit_sha: commit_sha.clone(),
            target_branch: request.target_branch.clone(),
            message,
            requested_at: now_rfc3339(),
            findings: engine_findings_local(&report, used_managed_fallback),
            delivery: PromotionDelivery::Local,
            next_commands,
            replaced: report.changes.replaced.clone(),
            deleted: report.changes.deleted.clone(),
            unchanged: report.changes.unchanged.clone(),
            flagged: report.changes.flagged.clone(),
            excluded: report.changes.excluded.clone(),
        });
    }

    // `worktree`: the delta is staged in the user's own checkout of the
    // target branch — no branch, no commit (so no provenance trailers), no
    // push, no PR. Statuses are derived from reachable history, so a
    // worktree promotion leaves every commit status exactly as it was. The
    // message says so in plain words and never implies an integration.
    if delivery == PromotionDelivery::Worktree {
        let next_commands = build_worktree_next_commands(
            &local_dir_for_commands,
            &request.message,
            &report.staged_paths,
        );
        let where_note = if used_managed_fallback {
            format!(
                " in the tool-managed checkout '{}' (your local clone was not configured, so the managed checkout under the checkouts cache was used — this is not your personal clone)",
                local_dir_for_commands.display()
            )
        } else {
            format!(
                " in your local clone '{}'",
                local_dir_for_commands.display()
            )
        };
        let staged_count = report.staged_paths.len();
        return Ok(PromotionDispatchResult {
            accepted: true,
            pr_url: None,
            pr_number: None,
            branch: target_for_message.clone(),
            commit_sha: None,
            target_branch: request.target_branch.clone(),
            message: format!(
                "The selected delta was applied to your working tree on '{target}'{where_note} and staged ({staged_count} path(s)): review with `git diff --staged`, then commit yourself with the listed commit command. Nothing was committed, nothing was pushed, and no PR was opened — commit statuses are unchanged because status is derived from reachable history.",
                target = target_for_message,
            ),
            requested_at: now_rfc3339(),
            findings: engine_findings_worktree(&report, used_managed_fallback),
            delivery: PromotionDelivery::Worktree,
            next_commands,
            replaced: report.changes.replaced.clone(),
            deleted: report.changes.deleted.clone(),
            unchanged: report.changes.unchanged.clone(),
            flagged: report.changes.flagged.clone(),
            excluded: report.changes.excluded.clone(),
        });
    }

    let pr_url = report.pull_request.as_ref().and_then(|pr| pr.url.clone());
    let pr_number = report.pull_request.as_ref().and_then(|pr| pr.number);

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
        delivery: PromotionDelivery::PushPr,
        next_commands: Vec::new(),
        replaced: report.changes.replaced.clone(),
        deleted: report.changes.deleted.clone(),
        unchanged: report.changes.unchanged.clone(),
        flagged: report.changes.flagged.clone(),
        excluded: report.changes.excluded.clone(),
    })
}

/// Dry-run mergeability preflight for the promotion dialog: per-file
/// `clean` / `merged` / `conflict` / `skipped` for the selected SHAs against
/// the target branch tip, computed from the same blobs and blob-membership
/// rules the real apply uses. A diverged target copy reports `clean` (the
/// subsystem's copy overwrites it) and is listed in `replaced` — it is never
/// a conflict.
/// Read-only: creates no branch, no commit, and no worktree, and performs no
/// push or PR. The failure path is untouched — the real promotion still fails
/// closed exactly as it does today.
#[tauri::command]
pub async fn preflight_promotion(request: PromotionRequest) -> AppResult<PromotionPreflight> {
    if request.source_commit_shas.is_empty() {
        return Err(AppError::invalid_request("No source commit SHAs were selected.")
            .with_remedy("Tick at least one eligible commit in the Commit Browser or the Promote Commits list."));
    }

    // Same shape gate as the engine (`assert_full_sha`): 40/64 lowercase hex.
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
    let subsystem =
        crate::services::live_read::find_live_subsystem(&registry, &request.subsystem_id)?.clone();
    let decision = registry.is_target_allowed(&request.target_branch);
    if !decision.allowed {
        return Err(AppError::new(
            ErrorCode::BranchNotPermitted,
            format!(
                "Target branch '{}' is not permitted ({}).",
                request.target_branch, decision.reason
            ),
        )
        .with_remedy("Choose a target branch that exists and is allowed by promotion.allowedTargetBranches / allowedTargetBranchPatterns."));
    }

    // Cached checkouts only: no clone, no push, no PR; the TTL fetch costs no more than a read.
    let token = gh_cli::auth_token();
    let (target_repo_dir, _) = crate::services::live_read::resolve_target_checkout(
        &registry.main_repository,
        registry.local_main_repository_path.as_deref(),
        token.as_deref(),
    )?;
    let source_resolver = |entry: &SubsystemEntry| {
        Some(match token.as_deref() {
            Some(token) => authenticated_remote_url(&entry.repository, token),
            // No token: remote slugs fail below with a typed error, while
            // local-path repositories still pass straight through.
            None => entry.repository.clone(),
        })
    };
    let (source_repo_dir, _) = crate::services::live_read::resolve_source_checkout(
        &subsystem,
        &source_resolver,
        token.as_deref(),
    )?;

    let policy = subsystem.integration_policy.clone();
    let commits = request.source_commit_shas.clone();
    let mode = request.mode;
    let base_sha = request.base_sha.clone();
    let target_branch = request.target_branch.clone();
    let on_conflict = request.on_conflict;
    // Pin the same snapshot the apply would use (addendum C): globalFiles
    // and excludedPaths travel with this call, never re-read.
    let excluded_paths = request.excluded_paths.clone();
    let global_files = subsystem.global_files.clone().unwrap_or_default();
    // The source tip feeds the "reverts newer work" warning (information
    // only): `None` when the branch is unreadable skips the check, never
    // refuses.
    let source_tip =
        crate::services::engine::git::rev_parse(&source_repo_dir, &subsystem.source_branch);
    let report = tauri::async_runtime::spawn_blocking(move || {
        let scratch_root = std::env::temp_dir().join("vos-integration");
        let _ = std::fs::create_dir_all(&scratch_root);
        let preflight = PreflightRequest {
            policy: &policy,
            commits: &commits,
            mode,
            base_sha: base_sha.as_deref(),
            target_branch: &target_branch,
            on_conflict,
            excluded_paths: &excluded_paths,
            global_files: &global_files,
            source_tip: source_tip.as_deref(),
        };
        crate::services::engine::preflight::compute_preflight(
            &source_repo_dir,
            &target_repo_dir,
            &preflight,
            &scratch_root,
        )
    })
    .await
    .map_err(|error| {
        AppError::new(
            ErrorCode::Internal,
            format!("The preflight check did not complete: {error}"),
        )
        .with_remedy("Retry; if it keeps failing, inspect the commit selection and retry.")
        .retryable()
    })?
    .map_err(map_preflight_error)?;

    Ok(PromotionPreflight {
        files: report
            .files
            .into_iter()
            .map(|file| PreflightFileResult {
                path: file.path,
                outcome: match file.outcome {
                    crate::services::engine::preflight::PreflightOutcome::Clean => {
                        PreflightOutcome::Clean
                    }
                    crate::services::engine::preflight::PreflightOutcome::Merged => {
                        PreflightOutcome::Merged
                    }
                    crate::services::engine::preflight::PreflightOutcome::Conflict => {
                        PreflightOutcome::Conflict
                    }
                    crate::services::engine::preflight::PreflightOutcome::Skipped => {
                        PreflightOutcome::Skipped
                    }
                },
                reverts_newer: file.reverts_newer.map(|warn| {
                    crate::github::models::RevertsNewerWork {
                        sha: warn.sha,
                        subject: warn.subject,
                    }
                }),
            })
            .collect(),
        clean: report.clean,
        merged: report.merged,
        conflict: report.conflict,
        skipped: report.skipped,
        conflicts: report.conflicts,
        replaced: report.replaced,
        excluded: report.excluded,
    })
}

fn map_preflight_error(error: crate::services::engine::preflight::PreflightError) -> AppError {
    use crate::services::engine::preflight::PreflightError as EngineError;
    match error {
        EngineError::UnknownTargetBranch(_) => AppError::new(
            ErrorCode::BranchNotPermitted,
            error.message(),
        )
        .with_remedy("Choose a target branch that exists and is allowed by the registry policy (promotion.allowedTargetBranches / allowedTargetBranchPatterns)."),
        EngineError::UnknownCommit(_) | EngineError::InvalidRange(_) => {
            AppError::invalid_request(error.message()).with_remedy(
                "Refresh the commit list and re-select the commits; the source branch may have moved.",
            )
        }
        EngineError::Scratch(_) => AppError::new(ErrorCode::Internal, error.message())
            .with_remedy("Retry; if it keeps failing, check the temp directory is writable."),
    }
}

/// `local` checkout: registry path → `VOS_TARGET_REPO_DIR` → managed cache.
/// Returns the dir plus whether the managed-cache fallback was used.
/// An explicitly configured but invalid path fails closed (never silently
/// falls through to a different clone).
fn resolve_local_target(registry: &Registry, token: &str) -> AppResult<(PathBuf, bool)> {
    if let Some(configured) = registry
        .local_main_repository_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let path = PathBuf::from(configured);
        if !path.is_dir() {
            return Err(AppError::new(
                ErrorCode::RepositoryInaccessible,
                format!("localMainRepositoryPath '{configured}' is not a directory."),
            )
            .with_remedy("Open Settings → Main system registry and set Local main repository clone to the absolute path of your local main-system clone, or clear it to use VOS_TARGET_REPO_DIR / the managed checkout."));
        }
        if !crate::services::engine::git::is_git_repository(&path) {
            return Err(AppError::new(
                ErrorCode::RepositoryInaccessible,
                format!("localMainRepositoryPath '{configured}' is not a git repository."),
            )
            .with_remedy("Point Local main repository clone at the root of your local main-system git clone (the directory containing .git)."));
        }
        return Ok((path, false));
    }

    if let Ok(raw) = std::env::var("VOS_TARGET_REPO_DIR") {
        if raw.trim().is_empty() {
            // Empty env var behaves as unset → fall through to the cache.
        } else {
            let path = PathBuf::from(raw.trim());
            if !path.is_dir() {
                return Err(AppError::new(
                    ErrorCode::RepositoryInaccessible,
                    format!(
                        "VOS_TARGET_REPO_DIR '{}' is not a directory.",
                        path.display()
                    ),
                )
                .with_remedy("Point VOS_TARGET_REPO_DIR at a local clone of the main-system repository, or unset it to use the managed checkout."));
            }
            if !crate::services::engine::git::is_git_repository(&path) {
                return Err(AppError::new(
                    ErrorCode::RepositoryInaccessible,
                    format!(
                        "VOS_TARGET_REPO_DIR '{}' is not a git repository.",
                        path.display()
                    ),
                )
                .with_remedy("Point VOS_TARGET_REPO_DIR at the root of a git clone (the directory containing .git)."));
            }
            return Ok((path, false));
        }
    }

    let key = crate::services::checkouts::target_key(&registry.main_repository);
    let url = authenticated_remote_url(&registry.main_repository, token);
    let dir = crate::services::checkouts::ensure_remote_checkout(&key, &url, Some(token))?;
    Ok((dir, true))
}

/// `push_pr` checkout: today's behaviour (`VOS_TARGET_REPO_DIR` else a
/// disposable temp clone that the caller deletes afterwards).
async fn resolve_push_pr_target(
    registry: &Registry,
    token: &str,
) -> AppResult<(PathBuf, Option<PathBuf>)> {
    if let Ok(raw) = std::env::var("VOS_TARGET_REPO_DIR") {
        let path = PathBuf::from(raw.trim());
        if path.is_dir() {
            return Ok((path, None));
        }
    }
    let stamp: String = now_rfc3339()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    let temp = std::env::temp_dir().join(format!("vos-target-{}-{stamp}", std::process::id()));
    let url = authenticated_remote_url(&registry.main_repository, token);
    match crate::services::engine::git::clone_no_tags(&url, &temp) {
        Ok(()) => Ok((temp.clone(), Some(temp))),
        Err(err) => Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!(
                "Cloning {} failed: {}",
                registry.main_repository,
                crate::services::engine::git::sanitize(&err.to_string())
            ),
        )
        .with_remedy(
            "Check the network connection and that `gh` can read the repository, then retry.",
        )),
    }
}

/// Follow-up steps for a `worktree` promotion (copyable in the UI): the
/// delta is already staged, so the user reviews the index and commits with
/// the promotion message as the commit subject, keeping the provenance
/// wording if they want it.
pub fn build_worktree_next_commands(
    local_path: &Path,
    promotion_message: &str,
    staged_paths: &[String],
) -> Vec<String> {
    let subject = promotion_message
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let subject = subject.replace('\\', "\\\\").replace('"', "\\\"");
    let mut commands = vec![
        format!("cd {}", local_path.display()),
        "git diff --staged".to_string(),
    ];
    if staged_paths.is_empty() {
        commands.push(format!("git commit -m \"{subject}\""));
    } else {
        commands.push(format!(
            "git commit -m \"{subject}\" -- {}",
            staged_paths.join(" ")
        ));
    }
    commands
}

/// Literal push step for a `local_direct` promotion (copyable in the UI):
/// the commit is already on the local target branch, so only the push is
/// manual.
pub fn build_direct_next_commands(local_path: &Path, target: &str) -> Vec<String> {
    vec![
        format!("cd {}", local_path.display()),
        format!("git push origin {target}"),
    ]
}
pub fn build_next_commands(
    local_path: &Path,
    branch: &str,
    target: &str,
    subsystem_id: &str,
    commit_count: usize,
) -> Vec<String> {
    vec![
        format!("cd {}", local_path.display()),
        format!("git push -u origin {branch}"),
        format!(
            "gh pr create --base {target} --head {branch} --title \"{subsystem_id}: integrate {commit_count} commit(s)\""
        ),
    ]
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
        "Set the local clone in Settings (localMainRepositoryPath) or VOS_TARGET_REPO_DIR to a local clone of the main-system repository (and authenticate gh) to run the real in-tool promotion engine.",
    ));
    if let Some(reason) = reason {
        findings.push(ValidationFinding::warning(
            "engine_unavailable",
            reason,
            "Fix the prerequisite and retry, or set the local clone in Settings (localMainRepositoryPath) or VOS_TARGET_REPO_DIR to a local clone of the main-system repository.",
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
            "Only the first-parent delta of the {} selected SHA(s) would be applied. A skipped commit's edits to files no selected commit touches are not written.",
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
        delivery: request.delivery,
        next_commands: Vec::new(),
        replaced: Vec::new(),
        deleted: Vec::new(),
        unchanged: Vec::new(),
        flagged: Vec::new(),
        excluded: Vec::new(),
    })
}

fn engine_findings_local_direct(
    report: &PromotionReport,
    used_managed_fallback: bool,
) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();
    findings.push(ValidationFinding::info(
        "local_direct",
        format!(
            "Committed the first-parent delta of {} selected SHA(s) directly onto the local target branch; nothing was pushed and no PR was opened. Push when ready.",
            report.verification.order.len()
        ),
    ));
    if used_managed_fallback {
        findings.push(ValidationFinding::warning(
            "managed_checkout_fallback",
            "No local clone was configured, so the tool-managed checkout under the checkouts cache was used instead of your personal clone.",
            "Set the local clone in Settings (localMainRepositoryPath) or VOS_TARGET_REPO_DIR to promote inside your own clone.",
        ));
    }
    append_shared_findings(report, &mut findings);
    findings
}

fn engine_findings_worktree(
    report: &PromotionReport,
    used_managed_fallback: bool,
) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();
    findings.push(ValidationFinding::info(
        "worktree_only",
        format!(
            "Staged the first-parent delta of {} selected SHA(s) ({} path(s)) in the working tree; nothing was committed, nothing was pushed, and no PR was opened. Commit statuses are unchanged — review with `git diff --staged`, then commit yourself.",
            report.verification.order.len(),
            report.staged_paths.len()
        ),
    ));
    if used_managed_fallback {
        findings.push(ValidationFinding::warning(
            "managed_checkout_fallback",
            "No local clone was configured, so the tool-managed checkout under the checkouts cache was used instead of your personal clone.",
            "Set the local clone in Settings (localMainRepositoryPath) or VOS_TARGET_REPO_DIR to promote inside your own clone.",
        ));
    }
    findings.push(ValidationFinding::info(
        "selection_scope",
        format!(
            "Applied only the first-parent delta of {} selected SHA(s). Files copied whole may include a skipped commit's edits to those same files; a skipped commit's edits to files no selected commit touches are not written.",
            report.verification.order.len()
        ),
    ));
    append_shared_findings(report, &mut findings);
    findings
}

fn engine_findings(report: &PromotionReport) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();
    findings.push(ValidationFinding::info(
        "no_direct_push",
        format!(
            "Pushed the integration branch and opened a pull request from the first-parent delta of {} selected SHA(s); nothing was pushed directly to a protected branch.",
            report.verification.order.len()
        ),
    ));
    append_shared_findings(report, &mut findings);
    findings
}

fn engine_findings_local(
    report: &PromotionReport,
    used_managed_fallback: bool,
) -> Vec<ValidationFinding> {
    let mut findings = Vec::new();
    findings.push(ValidationFinding::info(
        "local_only",
        format!(
            "Created a local branch + commit in the target clone from the first-parent delta of {} selected SHA(s); nothing was pushed and no PR was opened. You push it yourself.",
            report.verification.order.len()
        ),
    ));
    if used_managed_fallback {
        findings.push(ValidationFinding::warning(
            "managed_checkout_fallback",
            "No local clone was configured, so the tool-managed checkout under the checkouts cache was used instead of your personal clone.",
            "Set the local clone in Settings (localMainRepositoryPath) or VOS_TARGET_REPO_DIR to promote inside your own clone.",
        ));
    }
    append_shared_findings(report, &mut findings);
    findings
}

fn append_shared_findings(report: &PromotionReport, findings: &mut Vec<ValidationFinding>) {
    // Per-path change lists (replaced / deleted / unchanged / excluded) travel
    // as structured fields on the result — the dialog renders them as one
    // condensed summary with expandable details — so they are NOT repeated as
    // raw path dumps here.
    if !report.changes.flagged.is_empty() {
        findings.push(ValidationFinding::warning(
            "flagged_overlap",
            format!(
                "Cross-subsystem overlap — {} replaced path(s) had independently diverged in the main system (listed under Replaced). Review before merging.",
                report.changes.flagged.len(),
            ),
            "These paths overwrote independent target work that happened to match source history. Review the diff before merging.",
        ));
    }
    if !report.changes.ignored_skipped.is_empty() {
        findings.push(ValidationFinding::warning(
            "worktree_ignored",
            format!(
                "{} path(s) are ignored by the target repository and were not staged: {}",
                report.changes.ignored_skipped.len(),
                report.changes.ignored_skipped.join(", ")
            ),
            "Remove the ignore rule or force-add manually if the file must be tracked; the tool never forces with `-f`.",
        ));
    }
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
            "Register or enable the subsystem in %APPDATA%\\vos-origins\\registry.json.",
        ),
        "TARGET_INVALID" | "TARGET_NOT_ALLOWED" | "TARGET_NOT_FOUND" | "TARGET_NOT_LOCAL" => (
            ErrorCode::BranchNotPermitted,
            "Choose a target branch that exists and is allowed by the registry policy (promotion.allowedTargetBranches / allowedTargetBranchPatterns).",
        ),
        "LOCAL_REPO_INVALID" => (
            ErrorCode::RepositoryInaccessible,
            "Set the local clone in Settings (localMainRepositoryPath) to the absolute path of your local main-system git clone, or set VOS_TARGET_REPO_DIR to such a clone.",
        ),
        "TARGET_MOVED" => (
            ErrorCode::BranchNotPermitted,
            "The target branch moved while the promotion was being built. Retry the promotion to rebuild against the new tip; nothing was committed.",
        ),
        "DIRTY_WORKTREE" => (
            ErrorCode::InvalidRequest,
            "Commit or stash your working-tree changes in the local clone first (`git status` to inspect them), then re-promote. The promotion never mixes with uncommitted work.",
        ),
        "TARGET_NOT_CHECKED_OUT" => (
            ErrorCode::InvalidRequest,
            "Check out the target branch in the local clone (`git checkout <branch>`), then re-promote. A worktree promotion writes into the working tree, so the target branch must be the checked-out one.",
        ),
        "BRANCH_ALREADY_EXISTS" => (
            ErrorCode::DuplicateIntegration,
            "The integration branch already exists in the local clone. Delete or rename it (`git branch -D <branch>`), then re-promote to create a fresh branch.",
        ),
        "COMMIT_VERIFICATION_FAILED" => (
            ErrorCode::SourceCommitNotEligible,
            "Only commits reachable from the configured protected source branch can be promoted; use full 40-character SHAs.",
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
        "TARGET_DIVERGED" => (
            ErrorCode::PatchConflict,
            "Exclude the diverged path via excludedPaths and re-promote, or adapt the file upstream in the subsystem and re-promote.",
        ),
        "GLOBAL_FILE_PROTECTED" => (
            ErrorCode::InvalidRequest,
            "Adapt the protected app-shell file in the subsystem first, then re-promote (or exclude it via excludedPaths if the shell copy is canonical).",
        ),
        "TARGET_CHECKED_OUT_ELSEWHERE" => (
            ErrorCode::InvalidRequest,
            "Finish or remove the linked worktree holding the target branch (`git worktree list`), or check out the target here, then re-promote.",
        ),
        "STAGED_MISMATCH" => (
            ErrorCode::Internal,
            "Something raced the promotion into the index. Retry on a clean tree; the working tree was rolled back.",
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

//! Trunk -> subsystem distribution commands: list the trunk branch's commits,
//! preview one commit against one subsystem (respecting excluded paths), then
//! push `distribute/<trunk>-<short-sha>` and open a pull request.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::gh_cli;
use crate::services::engine::distribute::{
    self, CommitSnapshot, DistributionOutcome, DistributionPlan, RunInput, TrunkCommit,
    DISTRIBUTION_PROGRESS_EVENT,
};
use crate::services::engine::git;
use crate::services::engine::github_ops::{authenticated_remote_url, RealGitHubOps};
use crate::services::engine::PromotionProgress;
use crate::services::live_read;
use crate::services::registry::{Registry, SubsystemEntry};
use crate::AppState;

use super::live_token;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    pub main_branch: String,
    pub subsystem_id: String,
    pub commit_sha: String,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub main_branch: String,
    pub subsystem_id: String,
    pub commit_sha: String,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    #[serde(default)]
    pub message: Option<String>,
}

struct MainContext {
    registry: Registry,
    token: String,
    main_dir: PathBuf,
}

fn prepare_main() -> AppResult<MainContext> {
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let (main_dir, _) = live_read::resolve_target_checkout(
        &registry.main_repository,
        registry.local_main_repository_path.as_deref(),
        Some(&token),
    )?;
    // Best effort: a stale trunk is still usable if offline.
    let _ = git::fetch_cached(&main_dir, Some(&token));
    Ok(MainContext {
        registry,
        token,
        main_dir,
    })
}

fn child_checkout(context: &MainContext, entry: &SubsystemEntry) -> AppResult<PathBuf> {
    let resolver = |candidate: &SubsystemEntry| {
        Some(authenticated_remote_url(
            &candidate.repository,
            &context.token,
        ))
    };
    let (dir, _) = live_read::resolve_source_checkout(entry, &resolver, Some(&context.token))?;
    let _ = git::fetch_cached(&dir, Some(&context.token));
    Ok(dir)
}

fn entry_for(context: &MainContext, subsystem_id: &str) -> AppResult<SubsystemEntry> {
    let entry = context
        .registry
        .find(subsystem_id)
        .cloned()
        .ok_or_else(|| {
            AppError::invalid_registry(format!(
                "'{subsystem_id}' is not present in the tool registry."
            ))
            .with_remedy("Register the subsystem in Settings first.")
        })?;
    if !entry.enabled {
        return Err(AppError::invalid_request(format!(
            "Subsystem '{}' is disabled in the registry.",
            entry.name
        ))
        .with_remedy("Enable the subsystem in Settings or choose another target subsystem."));
    }
    Ok(entry)
}

fn list_blocking(main_branch: String) -> AppResult<Vec<TrunkCommit>> {
    let context = prepare_main()?;
    distribute::trunk_commits(&context.main_dir, &main_branch)
}

#[tauri::command]
pub async fn list_trunk_commits(main_branch: String) -> AppResult<Vec<TrunkCommit>> {
    tauri::async_runtime::spawn_blocking(move || list_blocking(main_branch))
        .await
        .map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                format!("The trunk commit list did not load: {error}"),
            )
            .retryable()
        })?
}

fn preview_blocking(request: PreviewRequest) -> AppResult<DistributionPlan> {
    let context = prepare_main()?;
    let entry = entry_for(&context, &request.subsystem_id)?;
    let commit =
        CommitSnapshot::load(&context.main_dir, &request.main_branch, &request.commit_sha)?;
    let child_dir = child_checkout(&context, &entry)?;
    let github = RealGitHubOps::new(context.token.clone());
    let excluded: HashSet<String> = request
        .excluded_paths
        .into_iter()
        .map(|p| p.replace('\\', "/"))
        .collect();
    Ok(distribute::plan(
        &commit, &child_dir, &entry, &github, &excluded,
    ))
}

#[tauri::command]
pub async fn preview_distribution(request: PreviewRequest) -> AppResult<DistributionPlan> {
    tauri::async_runtime::spawn_blocking(move || preview_blocking(request))
        .await
        .map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                format!("The distribution preview did not complete: {error}"),
            )
            .retryable()
        })?
}

fn emit_step(app: &AppHandle, stage: &'static str, label: &str, index: usize, total: usize) {
    let _ = app.emit(
        DISTRIBUTION_PROGRESS_EVENT,
        &PromotionProgress {
            stage,
            label: label.to_string(),
            index: Some(index),
            total: Some(total),
            detail: None,
        },
    );
}

fn run_blocking(app: &AppHandle, request: RunRequest) -> AppResult<DistributionOutcome> {
    emit_step(app, "prepare", "Preparing checkouts", 1, 4);
    let context = prepare_main()?;
    let entry = entry_for(&context, &request.subsystem_id)?;
    let commit =
        CommitSnapshot::load(&context.main_dir, &request.main_branch, &request.commit_sha)?;
    let child_dir = child_checkout(&context, &entry)?;

    emit_step(app, "plan", "Validating distribution plan", 2, 4);
    let github = RealGitHubOps::new(context.token.clone());
    let excluded: HashSet<String> = request
        .excluded_paths
        .into_iter()
        .map(|p| p.replace('\\', "/"))
        .collect();
    let plan = distribute::plan(&commit, &child_dir, &entry, &github, &excluded);

    emit_step(
        app,
        "apply",
        "Applying files and opening pull request",
        3,
        4,
    );
    let temp_root = std::env::temp_dir().join("vos-integration");
    let remote_url = authenticated_remote_url(&entry.repository, &context.token);
    let attribute_to_account = context.registry.promotion.attribute_to_account;
    let account = if attribute_to_account {
        gh_cli::authenticated_account()
    } else {
        None
    };

    let outcome = distribute::run(RunInput {
        commit: &commit,
        child_dir: &child_dir,
        entry: &entry,
        main_repo: &context.registry.main_repository,
        plan: &plan,
        excluded_paths: &excluded,
        message: request.message.as_deref(),
        attribute_to_account,
        account,
        temp_root: &temp_root,
        remote_url: &remote_url,
        github: &github,
    });

    emit_step(app, "done", "Finished", 4, 4);
    Ok(outcome)
}

#[tauri::command]
pub async fn run_distribution(
    state: State<'_, AppState>,
    app: AppHandle,
    request: RunRequest,
) -> AppResult<DistributionOutcome> {
    let outcome = tauri::async_runtime::spawn_blocking(move || run_blocking(&app, request))
        .await
        .map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                format!("The distribution did not complete: {error}"),
            )
            .retryable()
        })??;
    if outcome.ok {
        state.cache.clear();
    }
    Ok(outcome)
}

//! Reverse migration commands: preview a main → subsystem plan, then run it as
//! a pull request. One run per selected subsystem, sequentially.

use std::path::PathBuf;

use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::services::engine::git;
use crate::services::engine::github_ops::{authenticated_remote_url, RealGitHubOps};
use crate::services::engine::migrate::{
    self, MainSnapshot, MigrateOutcome, MigratePreview, MigrateRunResult, RunInput, Selection,
    MIGRATION_PROGRESS_EVENT,
};
use crate::services::engine::PromotionProgress;
use crate::services::live_read;
use crate::services::registry::{Registry, SubsystemEntry};

use super::live_token;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    pub main_branch: String,
    pub subsystem_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub main_branch: String,
    #[serde(default)]
    pub message: Option<String>,
    pub subsystems: Vec<RunSelection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSelection {
    pub subsystem_id: String,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    #[serde(default)]
    pub include_paths: Vec<String>,
}

struct Context {
    registry: Registry,
    token: String,
    main: MainSnapshot,
}

fn prepare(main_branch: &str) -> AppResult<Context> {
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let (main_dir, _) = live_read::resolve_target_checkout(
        &registry.main_repository,
        registry.local_main_repository_path.as_deref(),
        Some(&token),
    )?;
    // Best effort: a stale main is still a usable snapshot, and the clone may
    // be offline. The reported main SHA tells the operator what was read.
    let _ = git::fetch_cached(&main_dir, Some(&token));
    let main = MainSnapshot::load(&main_dir, main_branch)?;
    Ok(Context {
        registry,
        token,
        main,
    })
}

fn child_checkout(context: &Context, entry: &SubsystemEntry) -> AppResult<PathBuf> {
    let resolver = |candidate: &SubsystemEntry| {
        Some(authenticated_remote_url(
            &candidate.repository,
            &context.token,
        ))
    };
    let (dir, _) = live_read::resolve_source_checkout(entry, &resolver, Some(&context.token))?;
    Ok(dir)
}

fn entry_for(context: &Context, subsystem_id: &str) -> AppResult<SubsystemEntry> {
    context.registry.find(subsystem_id).cloned().ok_or_else(|| {
        AppError::invalid_registry(format!(
            "'{subsystem_id}' is not present in the tool registry."
        ))
        .with_remedy("Register the subsystem in %APPDATA%\\vos-origins\\registry.json first.")
    })
}

fn preview_blocking(request: PreviewRequest) -> AppResult<MigratePreview> {
    if request.subsystem_ids.is_empty() {
        return Err(AppError::invalid_request(
            "Select at least one subsystem to preview.",
        ));
    }
    let context = prepare(&request.main_branch)?;
    let github = RealGitHubOps::new(context.token.clone());
    let mut subsystems = Vec::with_capacity(request.subsystem_ids.len());
    for id in &request.subsystem_ids {
        let entry = entry_for(&context, id)?;
        let child_dir = child_checkout(&context, &entry)?;
        subsystems.push(migrate::plan(&context.main, &child_dir, &entry, &github));
    }
    Ok(MigratePreview {
        main_branch: context.main.branch.clone(),
        main_sha: context.main.sha.clone(),
        subsystems,
    })
}

#[tauri::command]
pub async fn preview_migration(request: PreviewRequest) -> AppResult<MigratePreview> {
    tauri::async_runtime::spawn_blocking(move || preview_blocking(request))
        .await
        .map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                format!("The migration preview did not complete: {error}"),
            )
            .retryable()
        })?
}

fn blocked_outcome(subsystem_id: &str, main_branch: &str, err: &AppError) -> MigrateOutcome {
    MigrateOutcome {
        subsystem_id: subsystem_id.to_string(),
        ok: false,
        branch: migrate::migration_branch(main_branch),
        commit_sha: None,
        pull_request: None,
        applied: Vec::new(),
        dependencies: Vec::new(),
        blockers: vec![crate::services::engine::Blocker {
            code: format!("{:?}", err.code),
            message: err.message.clone(),
            details: err.remedy.clone().or_else(|| err.details.clone()),
        }],
        warnings: Vec::new(),
        message: None,
    }
}

fn run_blocking(app: &AppHandle, request: RunRequest) -> AppResult<MigrateRunResult> {
    if request.subsystems.is_empty() {
        return Err(AppError::invalid_request(
            "Select at least one subsystem to migrate into.",
        ));
    }
    let context = prepare(&request.main_branch)?;
    let github = RealGitHubOps::new(context.token.clone());
    let temp_root = std::env::temp_dir().join("vos-integration");
    let total = request.subsystems.len();
    let mut outcomes = Vec::with_capacity(total);

    for (index, selection) in request.subsystems.iter().enumerate() {
        let label = format!("Migrating into {}", selection.subsystem_id);
        let _ = app.emit(
            MIGRATION_PROGRESS_EVENT,
            &PromotionProgress {
                stage: "migrate.start",
                label: label.clone(),
                index: Some(index),
                total: Some(total),
                detail: None,
            },
        );
        let Some(entry) = context.registry.find(&selection.subsystem_id).cloned() else {
            outcomes.push(blocked_outcome(
                &selection.subsystem_id,
                &context.main.branch,
                &AppError::invalid_registry(format!(
                    "'{}' is not present in the tool registry.",
                    selection.subsystem_id
                )),
            ));
            continue;
        };
        let child_dir = match child_checkout(&context, &entry) {
            Ok(dir) => dir,
            Err(err) => {
                outcomes.push(blocked_outcome(&entry.id, &context.main.branch, &err));
                continue;
            }
        };
        let _ = git::fetch_cached(&child_dir, Some(&context.token));
        let plan = migrate::plan(&context.main, &child_dir, &entry, &github);
        let selection = Selection {
            excluded_paths: selection.excluded_paths.clone(),
            include_paths: selection.include_paths.clone(),
        };
        let remote_url = authenticated_remote_url(&entry.repository, &context.token);
        let outcome = migrate::run(RunInput {
            main: &context.main,
            child_dir: &child_dir,
            entry: &entry,
            main_repo: &context.registry.main_repository,
            plan: &plan,
            selection: &selection,
            message: request.message.as_deref(),
            temp_root: &temp_root,
            remote_url: &remote_url,
            github: &github,
        });
        let _ = app.emit(
            MIGRATION_PROGRESS_EVENT,
            &PromotionProgress {
                stage: "migrate.done",
                label,
                index: Some(index),
                total: Some(total),
                detail: outcome.pull_request.as_ref().and_then(|pr| pr.url.clone()),
            },
        );
        outcomes.push(outcome);
    }

    Ok(MigrateRunResult {
        main_branch: context.main.branch.clone(),
        main_sha: context.main.sha.clone(),
        outcomes,
    })
}

#[tauri::command]
pub async fn run_migration(app: AppHandle, request: RunRequest) -> AppResult<MigrateRunResult> {
    tauri::async_runtime::spawn_blocking(move || run_blocking(&app, request))
        .await
        .map_err(|error| {
            AppError::new(
                ErrorCode::Internal,
                format!("The migration did not complete: {error}"),
            )
            .retryable()
        })?
}

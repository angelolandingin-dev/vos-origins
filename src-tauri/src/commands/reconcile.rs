//! Tauri commands for folder-scoped bidirectional reconciliation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::gh_cli;
use crate::github::models::PromotionDelivery;
use crate::services::engine::git;
use crate::services::engine::github_ops::{authenticated_remote_url, RealGitHubOps};
use crate::services::engine::reconcile::{
    self, InspectTreeInput, PlanInput, ReconcileDirection, ReconcileFileAction, ReconcileFileDiff,
    ReconcileOutcome, ReconcilePlan, ReconcileTreeSnapshot, RunInput, RECONCILE_PROGRESS_EVENT,
};
use crate::services::engine::PromotionProgress;
use crate::services::live_read;
use crate::services::reconcile_scope;
use crate::services::registry::{Registry, SubsystemEntry};
use crate::AppState;

use super::live_token;
use super::promotion::{cleanup_clone, resolve_local_target, resolve_push_pr_target};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRequest {
    pub subsystem_id: String,
    pub main_branch: String,
    pub direction: ReconcileDirection,
    pub folders: Vec<String>,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    #[serde(default = "default_delivery")]
    pub delivery: PromotionDelivery,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRequest {
    pub subsystem_id: String,
    pub main_branch: String,
    pub direction: ReconcileDirection,
    pub folders: Vec<String>,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    #[serde(default = "default_delivery")]
    pub delivery: PromotionDelivery,
    #[serde(default)]
    pub message: Option<String>,
    pub confirmation_acknowledged: bool,
}

fn default_delivery() -> PromotionDelivery {
    PromotionDelivery::LocalDirect
}

fn scope_path() -> AppResult<PathBuf> {
    reconcile_scope::default_scope_path().ok_or_else(|| {
        AppError::new(
            ErrorCode::Internal,
            "Could not resolve APPDATA / XDG_CONFIG_HOME to store reconcile scopes.",
        )
    })
}

struct ReconcileContext {
    registry: Registry,
    token: String,
    entry: SubsystemEntry,
    main_branch: String,
    main_dir: PathBuf,
    sub_dir: PathBuf,
    used_managed_fallback: bool,
    cloned_dir: Option<PathBuf>,
}

impl ReconcileContext {
    fn source_and_dest(&self, direction: ReconcileDirection) -> (&Path, &str, &Path, &str) {
        match direction {
            ReconcileDirection::SubsystemToMain => (
                &self.sub_dir,
                &self.entry.source_branch,
                &self.main_dir,
                &self.main_branch,
            ),
            ReconcileDirection::MainToSubsystem => (
                &self.main_dir,
                &self.main_branch,
                &self.sub_dir,
                &self.entry.source_branch,
            ),
        }
    }
}

fn resolve_entry(registry: &Registry, subsystem_id: &str) -> AppResult<SubsystemEntry> {
    let entry = registry.find(subsystem_id).cloned().ok_or_else(|| {
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
        .with_remedy("Enable the subsystem in Settings or choose another subsystem."));
    }
    Ok(entry)
}

fn resolve_child_checkout(entry: &SubsystemEntry, token: &str) -> AppResult<PathBuf> {
    let owned_token = token.to_string();
    let resolver = move |candidate: &SubsystemEntry| {
        Some(authenticated_remote_url(
            &candidate.repository,
            &owned_token,
        ))
    };
    let (dir, _) = live_read::resolve_source_checkout(entry, &resolver, Some(token))?;
    let _ = git::fetch_cached(&dir, Some(token));
    Ok(dir)
}

fn prepare_for_read(subsystem_id: &str, main_branch: &str) -> AppResult<ReconcileContext> {
    let registry = live_read::load_live_registry()?;
    let entry = resolve_entry(&registry, subsystem_id)?;
    let main_branch = live_read::require_allowed_target(&registry, main_branch)?;
    let token = live_token()?;
    let (main_dir, managed) = live_read::resolve_target_checkout(
        &registry.main_repository,
        registry.local_main_repository_path.as_deref(),
        Some(&token),
    )?;
    let _ = git::fetch_cached(&main_dir, Some(&token));
    let sub_dir = resolve_child_checkout(&entry, &token)?;
    Ok(ReconcileContext {
        registry,
        token,
        entry,
        main_branch,
        main_dir,
        sub_dir,
        used_managed_fallback: managed.is_some(),
        cloned_dir: None,
    })
}

async fn prepare_for_run(
    subsystem_id: &str,
    main_branch: &str,
    direction: ReconcileDirection,
    delivery: PromotionDelivery,
) -> AppResult<ReconcileContext> {
    let registry = live_read::load_live_registry()?;
    let entry = resolve_entry(&registry, subsystem_id)?;
    let main_branch = live_read::require_allowed_target(&registry, main_branch)?;
    let token = live_token()?;

    let (main_dir, used_managed_fallback, cloned_dir) = if direction
        == ReconcileDirection::SubsystemToMain
        && delivery == PromotionDelivery::PushPr
    {
        let (dir, cloned) = resolve_push_pr_target(&registry, &token).await?;
        (dir, false, cloned)
    } else if direction == ReconcileDirection::SubsystemToMain {
        let (dir, fallback) = resolve_local_target(&registry, &token)?;
        let _ = git::fetch_cached(&dir, Some(&token));
        (dir, fallback, None)
    } else {
        let (dir, managed) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        let _ = git::fetch_cached(&dir, Some(&token));
        (dir, managed.is_some(), None)
    };

    let sub_dir = resolve_child_checkout(&entry, &token)?;
    Ok(ReconcileContext {
        registry,
        token,
        entry,
        main_branch,
        main_dir,
        sub_dir,
        used_managed_fallback,
        cloned_dir,
    })
}

fn resolve_branch_sha(
    dir: &Path,
    branch: &str,
    prefer_remote: bool,
    role_label: &str,
) -> AppResult<String> {
    let reference = if prefer_remote {
        git::remote_tracking_ref(dir, branch).or_else(|| git::resolve_branch_ref(dir, branch))
    } else {
        git::local_tip(dir, branch)
            .map(|_| format!("refs/heads/{branch}"))
            .or_else(|| git::resolve_branch_ref(dir, branch))
    }
    .ok_or_else(|| {
        AppError::new(
            ErrorCode::BranchNotPermitted,
            format!(
                "{role_label} branch '{branch}' does not exist in '{}'.",
                dir.display()
            ),
        )
        .with_remedy("Fetch the repository or select a branch that exists locally.")
    })?;
    git::rev_parse(dir, &reference).ok_or_else(|| {
        AppError::new(
            ErrorCode::RepositoryInaccessible,
            format!("Could not resolve {role_label} branch '{branch}'."),
        )
    })
}

#[tauri::command]
pub async fn get_reconcile_scope(subsystem_id: String) -> AppResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = scope_path()?;
        reconcile_scope::load_scope(&path, &subsystem_id)
    })
    .await
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not read saved reconcile folders: {err}"),
        )
    })?
}

#[tauri::command]
pub async fn save_reconcile_scope(
    subsystem_id: String,
    folders: Vec<String>,
) -> AppResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = scope_path()?;
        reconcile_scope::save_scope(&path, &subsystem_id, &folders)
    })
    .await
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not save reconcile folders: {err}"),
        )
    })?
}

#[tauri::command]
pub async fn inspect_reconcile_tree(
    subsystem_id: String,
    main_branch: String,
    direction: ReconcileDirection,
) -> AppResult<ReconcileTreeSnapshot> {
    tauri::async_runtime::spawn_blocking(move || {
        let context = prepare_for_read(&subsystem_id, &main_branch)?;
        let saved_folders = scope_path()
            .and_then(|p| reconcile_scope::load_scope(&p, &subsystem_id))
            .unwrap_or_default();
        let (src_dir, src_branch, dst_dir, dst_branch) = context.source_and_dest(direction);
        let src_sha = resolve_branch_sha(src_dir, src_branch, false, "Source")?;
        let dst_sha = resolve_branch_sha(dst_dir, dst_branch, false, "Destination")?;

        reconcile::inspect_tree(InspectTreeInput {
            direction,
            entry: &context.entry,
            main_repo: &context.registry.main_repository,
            main_branch: &context.main_branch,
            source_dir: src_dir,
            source_branch: src_branch,
            source_sha: &src_sha,
            dest_dir: dst_dir,
            dest_branch: dst_branch,
            dest_sha: &dst_sha,
            saved_folders,
        })
    })
    .await
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not inspect reconcile tree: {err}"),
        )
        .retryable()
    })?
}

#[tauri::command]
pub async fn preview_reconcile(request: PreviewRequest) -> AppResult<ReconcilePlan> {
    tauri::async_runtime::spawn_blocking(move || {
        let context = prepare_for_read(&request.subsystem_id, &request.main_branch)?;
        let (src_dir, src_branch, dst_dir, dst_branch) =
            context.source_and_dest(request.direction);
        let src_sha = resolve_branch_sha(src_dir, src_branch, false, "Source")?;
        let dst_sha = resolve_branch_sha(dst_dir, dst_branch, false, "Destination")?;
        let github = RealGitHubOps::new(context.token.clone());
        let excluded: HashSet<String> = request
            .excluded_paths
            .into_iter()
            .map(|p| p.replace('\\', "/"))
            .collect();

        let mut plan = reconcile::plan(PlanInput {
            direction: request.direction,
            entry: &context.entry,
            main_repo: &context.registry.main_repository,
            main_branch: &context.main_branch,
            source_dir: src_dir,
            source_branch: src_branch,
            source_sha: &src_sha,
            dest_dir: dst_dir,
            dest_branch: dst_branch,
            dest_sha: &dst_sha,
            delivery: request.delivery,
            folders: &request.folders,
            excluded_paths: &excluded,
            github: &github,
        });
        if request.direction == ReconcileDirection::SubsystemToMain
            && context.used_managed_fallback
            && plan.delivery != PromotionDelivery::PushPr
        {
            plan.warnings.push(
                "No local clone was configured, so the tool-managed checkout under the checkouts cache will be used instead of your personal clone."
                    .to_string(),
            );
        }
        Ok(plan)
    })
    .await
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Reconcile preview did not complete: {err}"),
        )
        .retryable()
    })?
}

#[tauri::command]
pub async fn get_reconcile_file_diff(
    subsystem_id: String,
    main_branch: String,
    direction: ReconcileDirection,
    path: String,
    action: ReconcileFileAction,
) -> AppResult<ReconcileFileDiff> {
    tauri::async_runtime::spawn_blocking(move || {
        let context = prepare_for_read(&subsystem_id, &main_branch)?;
        let (src_dir, src_branch, dst_dir, dst_branch) = context.source_and_dest(direction);
        let src_sha = resolve_branch_sha(src_dir, src_branch, false, "Source")?;
        let dst_sha = resolve_branch_sha(dst_dir, dst_branch, false, "Destination")?;
        reconcile::file_diff(src_dir, &src_sha, dst_dir, &dst_sha, &path, action)
    })
    .await
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not compute file diff: {err}"),
        )
    })?
}

fn emit_step(app: &AppHandle, stage: &'static str, label: &str, index: usize, total: usize) {
    let _ = app.emit(
        RECONCILE_PROGRESS_EVENT,
        &PromotionProgress {
            stage,
            label: label.to_string(),
            index: Some(index),
            total: Some(total),
            detail: None,
        },
    );
}

#[tauri::command]
pub async fn run_reconcile(
    state: State<'_, AppState>,
    app: AppHandle,
    request: RunRequest,
) -> AppResult<ReconcileOutcome> {
    if !request.confirmation_acknowledged {
        return Err(AppError::invalid_request(
            "Reconciliation was not confirmed: accept the confirmation summary before running.",
        )
        .with_remedy("Tick the confirmation checkbox, then click Reconcile."));
    }

    emit_step(&app, "prepare", "Preparing checkouts", 1, 5);
    let mut context = prepare_for_run(
        &request.subsystem_id,
        &request.main_branch,
        request.direction,
        request.delivery,
    )
    .await?;

    let mut cloned_dir = context.cloned_dir.take();
    let result = tauri::async_runtime::spawn_blocking(move || {
        emit_step(&app, "plan", "Validating reconcile plan", 2, 5);
        let (src_dir, src_branch, dst_dir, dst_branch) =
            context.source_and_dest(request.direction);
        let src_sha = resolve_branch_sha(src_dir, src_branch, false, "Source")?;
        let prefer_remote_dest = (request.direction == ReconcileDirection::SubsystemToMain
            && request.delivery == PromotionDelivery::PushPr)
            || request.direction == ReconcileDirection::MainToSubsystem;
        let dst_sha = resolve_branch_sha(dst_dir, dst_branch, prefer_remote_dest, "Destination")?;

        let github = RealGitHubOps::new(context.token.clone());
        let excluded: HashSet<String> = request
            .excluded_paths
            .into_iter()
            .map(|p| p.replace('\\', "/"))
            .collect();

        let mut plan = reconcile::plan(PlanInput {
            direction: request.direction,
            entry: &context.entry,
            main_repo: &context.registry.main_repository,
            main_branch: &context.main_branch,
            source_dir: src_dir,
            source_branch: src_branch,
            source_sha: &src_sha,
            dest_dir: dst_dir,
            dest_branch: dst_branch,
            dest_sha: &dst_sha,
            delivery: request.delivery,
            folders: &request.folders,
            excluded_paths: &excluded,
            github: &github,
        });
        if request.direction == ReconcileDirection::SubsystemToMain
            && context.used_managed_fallback
            && plan.delivery != PromotionDelivery::PushPr
        {
            plan.warnings.push(
                "No local clone was configured, so the tool-managed checkout under the checkouts cache was used instead of your personal clone."
                    .to_string(),
            );
        }

        emit_step(&app, "apply", "Applying folder contents", 3, 5);
        let temp_root = std::env::temp_dir().join("vos-integration");
        let attribute_to_account = context.registry.promotion.attribute_to_account;
        let account = if attribute_to_account {
            gh_cli::authenticated_account()
        } else {
            None
        };
        let remote_url = if plan.delivery == PromotionDelivery::PushPr {
            Some(authenticated_remote_url(&plan.dest_repo, &context.token))
        } else {
            None
        };

        emit_step(&app, "commit", "Recording reconciliation commit", 4, 5);
        emit_step(&app, "deliver", "Delivering reconciliation", 5, 5);
        Ok::<_, AppError>(reconcile::run(RunInput {
            plan: &plan,
            entry: &context.entry,
            source_dir: src_dir,
            dest_dir: dst_dir,
            excluded_paths: &excluded,
            message: request.message.as_deref(),
            attribute_to_account,
            account,
            temp_root: &temp_root,
            remote_url: remote_url.as_deref(),
            github: &github,
        }))
    })
    .await
    .map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Reconciliation failed to execute: {err}"),
        )
        .retryable()
    });

    cleanup_clone(&mut cloned_dir);
    let outcome = result??;
    if outcome.ok {
        state.cache.clear();
    }
    Ok(outcome)
}

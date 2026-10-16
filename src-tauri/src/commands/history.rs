//! Integration PRs and history: live by default, fixtures on opt-in.

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::models::{IntegrationHistory, IntegrationPr};
use crate::services::engine::github_ops::RealGitHubOps;
use crate::services::live_read;
use crate::AppState;

use super::read_world;

fn live_token() -> AppResult<String> {
    crate::github::gh_cli::auth_token().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
    })
}

#[tauri::command]
pub async fn list_integration_prs(
    state: State<'_, AppState>,
    target_branch: String,
    subsystem_id: Option<String>,
) -> AppResult<Vec<IntegrationPr>> {
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            state
                .adapter
                .integration_prs(world, &target_branch, subsystem_id.as_deref())
        });
    }
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let github = RealGitHubOps::new(token);
    live_read::integration_prs_live(&registry, &github, &target_branch, subsystem_id.as_deref())
}

#[tauri::command]
pub async fn list_integration_history(
    state: State<'_, AppState>,
    target_branch: Option<String>,
    subsystem_id: Option<String>,
    limit: Option<u32>,
) -> AppResult<IntegrationHistory> {
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            state.adapter.integration_history(
                world,
                target_branch.as_deref(),
                subsystem_id.as_deref(),
                limit.unwrap_or(50),
            )
        });
    }
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let (target_dir, mut target_cleanup) =
        live_read::resolve_target_checkout(&registry.main_repository, Some(&token))?;
    let github = RealGitHubOps::new(token);
    let result = live_read::integration_history_live(
        &target_dir,
        &registry,
        &github,
        target_branch.as_deref(),
        subsystem_id.as_deref(),
        limit.unwrap_or(50),
    );
    live_read::cleanup_dir(&mut target_cleanup);
    result
}

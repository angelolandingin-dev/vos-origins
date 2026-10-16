//! Integration PRs and history.

use tauri::State;

use crate::error::AppResult;
use crate::github::models::{IntegrationHistory, IntegrationPr};
use crate::AppState;

use super::read_world;

#[tauri::command]
pub async fn list_integration_prs(
    state: State<'_, AppState>,
    target_branch: String,
    subsystem_id: Option<String>,
) -> AppResult<Vec<IntegrationPr>> {
    read_world(&state, |world| {
        state
            .adapter
            .integration_prs(world, &target_branch, subsystem_id.as_deref())
    })
}

#[tauri::command]
pub async fn list_integration_history(
    state: State<'_, AppState>,
    target_branch: Option<String>,
    subsystem_id: Option<String>,
    limit: Option<u32>,
) -> AppResult<IntegrationHistory> {
    read_world(&state, |world| {
        state.adapter.integration_history(
            world,
            target_branch.as_deref(),
            subsystem_id.as_deref(),
            limit.unwrap_or(50),
        )
    })
}

//! Workflow runs (logs fetched on demand).

use tauri::State;

use crate::error::AppResult;
use crate::github::models::{WorkflowRunDetails, WorkflowRunList};
use crate::AppState;

use super::read_world;

#[tauri::command]
pub async fn list_workflow_runs(
    state: State<'_, AppState>,
    subsystem_id: Option<String>,
    target_branch: Option<String>,
    limit: Option<u32>,
) -> AppResult<WorkflowRunList> {
    read_world(&state, |world| {
        Ok(state.adapter.workflow_runs(
            world,
            subsystem_id.as_deref(),
            target_branch.as_deref(),
            limit.unwrap_or(25),
        ))
    })
}

#[tauri::command]
pub async fn get_workflow_run_details(
    state: State<'_, AppState>,
    run_id: u64,
) -> AppResult<WorkflowRunDetails> {
    read_world(&state, |world| {
        state.adapter.workflow_run_details(world, run_id)
    })
}

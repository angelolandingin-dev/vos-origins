//! Workflow runs (logs fetched on demand).
//! Live mode has no Actions-run source: the in-tool engine opens PRs
//! directly, so live returns an empty live-labelled list instead of fixtures.

use tauri::State;

use crate::error::{AppError, AppResult};
use crate::github::models::{DataSource, WorkflowRunDetails, WorkflowRunList};
use crate::services::live_read;
use crate::AppState;

use super::read_world;

#[tauri::command]
pub async fn list_workflow_runs(
    state: State<'_, AppState>,
    subsystem_id: Option<String>,
    target_branch: Option<String>,
    limit: Option<u32>,
) -> AppResult<WorkflowRunList> {
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            Ok(state.adapter.workflow_runs(
                world,
                subsystem_id.as_deref(),
                target_branch.as_deref(),
                limit.unwrap_or(25),
            ))
        });
    }
    let _ = (subsystem_id, target_branch);
    let capped = limit.unwrap_or(25).clamp(1, 100);
    let _ = capped;
    Ok(WorkflowRunList {
        runs: Vec::new(),
        total: 0,
        fetched_at: crate::fixtures::now_rfc3339(),
        data_source: DataSource::Live,
    })
}

#[tauri::command]
pub async fn get_workflow_run_details(
    state: State<'_, AppState>,
    run_id: u64,
) -> AppResult<WorkflowRunDetails> {
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            state.adapter.workflow_run_details(world, run_id)
        });
    }
    Err(AppError::not_found(format!("Workflow run {run_id} was not found.")).with_remedy(
        "Live mode has no workflow runs: promotions open integration PRs directly. See Integration History and PRs instead.",
    ))
}

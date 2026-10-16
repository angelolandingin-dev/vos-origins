//! GitHub identity / CLI status for the connection indicator.

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures;
use crate::github::{gh_cli, models::AuthStatus};
use crate::AppState;

#[tauri::command]
pub async fn get_auth_status(state: State<'_, AppState>) -> AppResult<AuthStatus> {
    let adapter = state.adapter.clone();
    let probe = tauri::async_runtime::spawn_blocking(gh_cli::probe)
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::Internal, format!("The GitHub CLI probe did not complete: {error}"))
                .with_remedy("Retry; if it keeps failing, run `gh auth status` in a terminal to confirm the CLI works.")
                .retryable()
        })?;

    Ok(gh_cli::auth_status(
        probe,
        adapter.data_source,
        &adapter.main_repository,
        &adapter.registry_branch,
        &adapter.workflow_file,
        fixtures::NAMESPACE,
        fixtures::now_rfc3339(),
    ))
}

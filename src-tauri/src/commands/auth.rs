//! GitHub identity / CLI status for the connection indicator.

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures;
use crate::github::{gh_cli, models::AuthStatus};
use crate::services::live_read;
use crate::AppState;

#[tauri::command]
pub async fn get_auth_status(state: State<'_, AppState>) -> AppResult<AuthStatus> {
    // Kick off the one-time, off-path warm so the first read is fresh.
    super::system::warm_checkouts_once();
    let adapter = state.adapter.clone();
    let probe = tauri::async_runtime::spawn_blocking(gh_cli::probe)
        .await
        .map_err(|error| {
            AppError::new(ErrorCode::Internal, format!("The GitHub CLI probe did not complete: {error}"))
                .with_remedy("Retry; if it keeps failing, run `gh auth status` in a terminal to confirm the CLI works.")
                .retryable()
        })?;
    let data_source = if live_read::use_fixtures() {
        adapter.data_source
    } else {
        crate::github::models::DataSource::Live
    };

    // Live mode must report the tool-local registry (the real source of truth), not the fixture adapter.
    let (main_repository, registry_branch, workflow_file) = if live_read::use_fixtures() {
        (
            adapter.main_repository.clone(),
            adapter.registry_branch.clone(),
            adapter.workflow_file.clone(),
        )
    } else {
        match live_read::load_live_registry() {
            Ok(registry) => (
                registry.main_repository.clone(),
                "(tool-local)".to_string(),
                "(none; the tool opens the PR)".to_string(),
            ),
            Err(_) => (
                adapter.main_repository.clone(),
                adapter.registry_branch.clone(),
                adapter.workflow_file.clone(),
            ),
        }
    };

    Ok(gh_cli::auth_status(
        probe,
        data_source,
        &main_repository,
        &registry_branch,
        &workflow_file,
        fixtures::NAMESPACE,
        fixtures::now_rfc3339(),
    ))
}

//! Commit browsing, details, diffs, and integration status.

use tauri::State;

use crate::error::AppResult;
use crate::github::models::{
    CommitDetails, CommitDiff, CommitPage, CommitQuery, IntegrationStatus,
};
use crate::AppState;

use super::read_world;

#[tauri::command]
pub async fn get_subsystem_commits(
    state: State<'_, AppState>,
    query: CommitQuery,
) -> AppResult<CommitPage> {
    let key = format!(
        "commits::{}::{}::{}::{}::{}::{:?}::{:?}::{}",
        query.subsystem_id,
        query.target_branch,
        query.page,
        query.page_size,
        query.search.to_ascii_lowercase(),
        query.status_filter,
        query.sort,
        query
            .shas
            .as_ref()
            .map(|shas| shas.join(","))
            .unwrap_or_default()
    );

    if let Some(cached) = state.cache.get(&key) {
        if let Ok(page) = serde_json::from_str::<CommitPage>(&cached) {
            return Ok(page);
        }
    }

    let page = read_world(&state, |world| state.adapter.commit_page(world, &query))?;
    if let Ok(encoded) = serde_json::to_string(&page) {
        state.cache.put(&key, encoded);
    }
    Ok(page)
}

#[tauri::command]
pub async fn get_commit_details(
    state: State<'_, AppState>,
    subsystem_id: String,
    sha: String,
    target_branch: String,
) -> AppResult<CommitDetails> {
    read_world(&state, |world| {
        state
            .adapter
            .commit_details(world, &subsystem_id, &sha, &target_branch)
    })
}

#[tauri::command]
pub async fn get_commit_diff(
    state: State<'_, AppState>,
    subsystem_id: String,
    sha: String,
    file_path: Option<String>,
    target_branch: Option<String>,
) -> AppResult<CommitDiff> {
    let key = format!(
        "diff::{}::{}::{}",
        subsystem_id,
        sha,
        file_path.clone().unwrap_or_else(|| "<first>".to_string())
    );
    if let Some(cached) = state.cache.get(&key) {
        if let Ok(diff) = serde_json::from_str::<CommitDiff>(&cached) {
            return Ok(diff);
        }
    }

    let diff = read_world(&state, |world| {
        if let Some(branch) = target_branch.as_deref() {
            state.adapter.require_target_branch(world, branch)?;
        }
        state
            .adapter
            .commit_diff(world, &subsystem_id, &sha, file_path.as_deref())
    })?;

    if let Ok(encoded) = serde_json::to_string(&diff) {
        state.cache.put(&key, encoded);
    }
    Ok(diff)
}

/// Never fabricates `Integrated`.
#[tauri::command]
pub async fn get_integration_statuses(
    state: State<'_, AppState>,
    target_branch: String,
    subsystem_id: Option<String>,
    shas: Vec<String>,
) -> AppResult<Vec<IntegrationStatus>> {
    read_world(&state, |world| {
        state
            .adapter
            .integration_statuses(world, &target_branch, subsystem_id.as_deref(), &shas)
    })
}

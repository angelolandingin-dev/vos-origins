//! Commit browsing, details, diffs, and integration status.
//! Live by default (reachable history + GitHub PRs); fixtures on opt-in.

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::models::{
    CommitDetails, CommitDiff, CommitPage, CommitQuery, IntegrationStatus,
};
use crate::services::engine::github_ops::{authenticated_remote_url, RealGitHubOps};
use crate::services::live_read;
use crate::services::registry::SubsystemEntry;
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

fn temp_root() -> std::path::PathBuf {
    let root = std::env::temp_dir().join("vos-live-read");
    let _ = std::fs::create_dir_all(&root);
    root
}

#[tauri::command]
pub async fn get_subsystem_commits(
    state: State<'_, AppState>,
    query: CommitQuery,
) -> AppResult<CommitPage> {
    // Keep the cache key mode-scoped so live and fixture never collide.
    let mode = if live_read::use_fixtures() {
        "fixture"
    } else {
        "live"
    };
    let key = format!(
        "{mode}::commits::{}::{}::{}::{}::{}::{:?}::{:?}::{}",
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

    if live_read::use_fixtures() {
        let page = read_world(&state, |world| state.adapter.commit_page(world, &query))?;
        if let Ok(encoded) = serde_json::to_string(&page) {
            state.cache.put(&key, encoded);
        }
        return Ok(page);
    }

    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let (target_dir, mut target_cleanup) =
        live_read::resolve_target_checkout(&registry.main_repository, Some(&token))?;
    let github = RealGitHubOps::new(token.clone());
    let source_resolver =
        |entry: &SubsystemEntry| Some(authenticated_remote_url(&entry.repository, &token));
    // Resolve the source checkout for this subsystem.
    let entry = registry.find(&query.subsystem_id).cloned().ok_or_else(|| {
        AppError::invalid_registry(format!(
            "'{}' is not present in the tool registry.",
            query.subsystem_id
        ))
        .with_remedy("Register the subsystem first; the registry lives in the tool.")
    })?;
    let (source_dir, mut source_cleanup) =
        live_read::resolve_source_checkout(&entry, &source_resolver, &temp_root())?;
    let result = live_read::commit_page_live(&target_dir, &source_dir, &registry, &github, &query);
    live_read::cleanup_dir(&mut source_cleanup);
    live_read::cleanup_dir(&mut target_cleanup);
    let page = result?;
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
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            state
                .adapter
                .commit_details(world, &subsystem_id, &sha, &target_branch)
        });
    }
    // Details are not cached: status + PR overlays change independently.
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let (target_dir, mut target_cleanup) =
        live_read::resolve_target_checkout(&registry.main_repository, Some(&token))?;
    let github = RealGitHubOps::new(token.clone());
    let source_resolver =
        |entry: &SubsystemEntry| Some(authenticated_remote_url(&entry.repository, &token));
    let entry = registry.find(&subsystem_id).cloned().ok_or_else(|| {
        AppError::invalid_registry(format!(
            "'{subsystem_id}' is not present in the tool registry."
        ))
        .with_remedy("Register the subsystem first; the registry lives in the tool.")
    })?;
    let (source_dir, mut source_cleanup) =
        live_read::resolve_source_checkout(&entry, &source_resolver, &temp_root())?;
    let result = live_read::commit_details_live(
        &target_dir,
        &source_dir,
        &registry,
        &github,
        &subsystem_id,
        &sha,
        &target_branch,
    );
    live_read::cleanup_dir(&mut source_cleanup);
    live_read::cleanup_dir(&mut target_cleanup);
    result
}

#[tauri::command]
pub async fn get_commit_diff(
    state: State<'_, AppState>,
    subsystem_id: String,
    sha: String,
    file_path: Option<String>,
    target_branch: Option<String>,
) -> AppResult<CommitDiff> {
    let mode = if live_read::use_fixtures() {
        "fixture"
    } else {
        "live"
    };
    let key = format!(
        "{mode}::diff::{}::{}::{}",
        subsystem_id,
        sha,
        file_path.clone().unwrap_or_else(|| "<first>".to_string())
    );
    if let Some(cached) = state.cache.get(&key) {
        if let Ok(diff) = serde_json::from_str::<CommitDiff>(&cached) {
            return Ok(diff);
        }
    }

    if live_read::use_fixtures() {
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
        return Ok(diff);
    }

    if let Some(branch) = target_branch.as_deref() {
        let registry = live_read::load_live_registry()?;
        let decision = registry.is_target_allowed(branch);
        if !decision.allowed {
            return Err(AppError::new(
                ErrorCode::BranchNotPermitted,
                format!(
                    "Target branch '{branch}' is not permitted ({}).",
                    decision.reason
                ),
            )
            .with_remedy("Choose a target branch allowed by the registry policy."));
        }
    }
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let source_resolver =
        |entry: &SubsystemEntry| Some(authenticated_remote_url(&entry.repository, &token));
    let entry = registry.find(&subsystem_id).cloned().ok_or_else(|| {
        AppError::invalid_registry(format!(
            "'{subsystem_id}' is not present in the tool registry."
        ))
        .with_remedy("Register the subsystem first; the registry lives in the tool.")
    })?;
    let (source_dir, mut source_cleanup) =
        live_read::resolve_source_checkout(&entry, &source_resolver, &temp_root())?;
    let result = live_read::commit_diff_live(
        &source_dir,
        &registry,
        &subsystem_id,
        &sha,
        file_path.as_deref(),
    );
    live_read::cleanup_dir(&mut source_cleanup);
    let diff = result?;
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
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            state.adapter.integration_statuses(
                world,
                &target_branch,
                subsystem_id.as_deref(),
                &shas,
            )
        });
    }
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let (target_dir, mut target_cleanup) =
        live_read::resolve_target_checkout(&registry.main_repository, Some(&token))?;
    let github = RealGitHubOps::new(token);
    let result = live_read::integration_statuses_live(
        &target_dir,
        &registry,
        &github,
        &target_branch,
        subsystem_id.as_deref(),
        &shas,
    );
    live_read::cleanup_dir(&mut target_cleanup);
    result
}

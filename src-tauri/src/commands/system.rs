//! Cache diagnostics + allow-listed external-URL opener (spec §9.3).

use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::gh_cli;
use crate::github::models::CacheStats;
use crate::AppState;

#[tauri::command]
pub async fn get_cache_stats(state: State<'_, AppState>) -> AppResult<CacheStats> {
    let mut stats = state.cache.stats();
    if !crate::services::live_read::use_fixtures() {
        stats.data_source = crate::github::models::DataSource::Live;
    }
    Ok(stats)
}

#[tauri::command]
pub async fn clear_cache(state: State<'_, AppState>) -> AppResult<CacheStats> {
    let removed = state.cache.clear();
    let mut stats = state.cache.stats();
    if !crate::services::live_read::use_fixtures() {
        stats.data_source = crate::github::models::DataSource::Live;
    }
    stats.note = format!(
        "Cleared {removed} cached response(s). Git history and GitHub remain the authority.",
    );
    Ok(stats)
}

#[tauri::command]
pub async fn open_external_url(app: tauri::AppHandle, url: String) -> AppResult<()> {
    if !gh_cli::url_is_allowlisted(&url) {
        return Err(AppError::invalid_request(format!("Refusing to open a non-allow-listed URL: {url}"))
            .with_remedy("Only https://github.com, https://docs.github.com and https://cli.github.com links can be opened."));
    }

    app.opener().open_url(&url, None::<&str>).map_err(|error| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not open {url}: {error}"),
        )
    })
}

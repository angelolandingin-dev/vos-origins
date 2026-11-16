//! Subsystem-versus-main diff: what a promotion left behind.
//! Live by default; fixture mode has no git checkouts to compare.

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::models::SubsystemDiff;
use crate::services::engine::github_ops::{authenticated_remote_url, RealGitHubOps};
use crate::services::registry::SubsystemEntry;
use crate::services::{live_read, subsystem_diff};
use crate::AppState;

use super::{cached_json, live_token};

pub(crate) fn diff_cache_key(mode: &str, subsystem_id: &str, target_branch: &str) -> String {
    format!("{mode}::subsystem-diff::{subsystem_id}::{target_branch}")
}

#[tauri::command]
pub async fn get_subsystem_diff(
    state: State<'_, AppState>,
    subsystem_id: String,
    target_branch: String,
) -> AppResult<SubsystemDiff> {
    if live_read::use_fixtures() {
        return Err(AppError::new(
            ErrorCode::RepositoryInaccessible,
            "The subsystem diff needs live git checkouts; fixture mode has none.",
        )
        .with_remedy(
            "Unset VOS_USE_FIXTURES (and configure the registry clones) to use the diff view.",
        ));
    }
    let key = diff_cache_key("live", &subsystem_id, &target_branch);
    cached_json(&state, &key, || {
        let registry = live_read::load_live_registry()?;
        let token = live_token()?;
        let (target_dir, _) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        let github = RealGitHubOps::new(token.clone());
        let source_resolver =
            |entry: &SubsystemEntry| Some(authenticated_remote_url(&entry.repository, &token));
        let entry = registry.find(&subsystem_id).cloned().ok_or_else(|| {
            AppError::invalid_registry(format!(
                "'{subsystem_id}' is not present in the tool registry."
            ))
            .with_remedy("Register the subsystem first in %APPDATA%\\vos-origins\\registry.json.")
        })?;
        let (source_dir, _) =
            live_read::resolve_source_checkout(&entry, &source_resolver, Some(&token))?;
        subsystem_diff::subsystem_diff_live(
            &target_dir,
            &source_dir,
            &registry,
            &github,
            &subsystem_id,
            &target_branch,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_carries_full_identity() {
        let key = diff_cache_key("live", "hr", "staging");
        assert!(key.contains("hr") && key.contains("staging"));
        assert_ne!(key, diff_cache_key("fixture", "hr", "staging"));
        assert_ne!(key, diff_cache_key("live", "hr", "development"));
        assert_ne!(key, diff_cache_key("live", "other", "staging"));
    }
}

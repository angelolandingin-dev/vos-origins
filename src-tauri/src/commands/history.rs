use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::github::models::{
    IntegrationEventAudit, IntegrationHistory, IntegrationPr, MainCommitDiff,
};
use crate::services::engine::github_ops::RealGitHubOps;
use crate::services::live_read;
use crate::AppState;

use super::{cached_json, read_world};

fn live_token() -> AppResult<String> {
    crate::github::gh_cli::auth_token().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
    })
}

pub(crate) fn history_cache_key(
    mode: &str,
    target_branch: Option<&str>,
    subsystem_id: Option<&str>,
    limit: u32,
) -> String {
    format!(
        "{mode}::history::{}::{}::{limit}",
        target_branch.unwrap_or("<default>"),
        subsystem_id.unwrap_or("<all>")
    )
}

pub(crate) fn event_audit_cache_key(
    mode: &str,
    target_branch: &str,
    subsystem_id: &str,
    main_commit_sha: &str,
) -> String {
    format!("{mode}::event-audit::{target_branch}::{subsystem_id}::{main_commit_sha}")
}

/// Mode-scoped like every other read key, so live and fixture never collide.
/// Keyed on the commit identity only: the diff is per-commit, not per-file.
pub(crate) fn main_commit_diff_cache_key(
    mode: &str,
    target_branch: &str,
    main_commit_sha: &str,
) -> String {
    format!("{mode}::main-commit-diff::{target_branch}::{main_commit_sha}")
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
    let capped = limit.unwrap_or(50);
    let mode = if live_read::use_fixtures() {
        "fixture"
    } else {
        "live"
    };
    let key = history_cache_key(
        mode,
        target_branch.as_deref(),
        subsystem_id.as_deref(),
        capped,
    );
    cached_json(&state, &key, || {
        if live_read::use_fixtures() {
            return read_world(&state, |world| {
                state.adapter.integration_history(
                    world,
                    target_branch.as_deref(),
                    subsystem_id.as_deref(),
                    capped,
                )
            });
        }
        let registry = live_read::load_live_registry()?;
        let token = live_token()?;
        let (target_dir, _) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        let github = RealGitHubOps::new(token);
        live_read::integration_history_live(
            &target_dir,
            &registry,
            &github,
            target_branch.as_deref(),
            subsystem_id.as_deref(),
            capped,
        )
    })
}

#[tauri::command]
pub async fn get_integration_event(
    state: State<'_, AppState>,
    target_branch: String,
    subsystem_id: String,
    main_commit_sha: String,
) -> AppResult<IntegrationEventAudit> {
    let key = event_audit_cache_key(
        if live_read::use_fixtures() {
            "fixture"
        } else {
            "live"
        },
        &target_branch,
        &subsystem_id,
        &main_commit_sha,
    );
    cached_json(&state, &key, || {
        if live_read::use_fixtures() {
            return read_world(&state, |world| {
                state.adapter.integration_event_audit(
                    world,
                    &target_branch,
                    &subsystem_id,
                    &main_commit_sha,
                )
            });
        }
        let registry = live_read::load_live_registry()?;
        let token = live_token()?;
        let (target_dir, _) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        let source_dir = live_read::cached_source_dir(&registry, &subsystem_id)?;
        live_read::integration_event_audit_live(
            &target_dir,
            source_dir.as_deref(),
            &registry,
            &target_branch,
            &subsystem_id,
            &main_commit_sha,
        )
    })
}

/// The whole main commit's per-file diff in ONE call, keyed by path. The main
/// commit lives in the MAIN repository, so this reads the target checkout
/// rather than a subsystem's; the audit page renders the files column + the
/// selected file's diff exactly like the commit browser. A repeated open hits
/// the response cache; a fresh commit costs one `rev-parse` + one `git show`.
#[tauri::command]
pub async fn get_main_commit_diff(
    state: State<'_, AppState>,
    target_branch: String,
    main_commit_sha: String,
) -> AppResult<MainCommitDiff> {
    let key = main_commit_diff_cache_key(
        if live_read::use_fixtures() {
            "fixture"
        } else {
            "live"
        },
        &target_branch,
        &main_commit_sha,
    );
    cached_json(&state, &key, || {
        if live_read::use_fixtures() {
            return read_world(&state, |world| {
                state
                    .adapter
                    .main_commit_diff(world, &target_branch, &main_commit_sha)
            });
        }
        let registry = live_read::load_live_registry()?;
        let token = live_token()?;
        let (target_dir, _) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        live_read::main_commit_diff_live(&target_dir, &registry, &target_branch, &main_commit_sha)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_cache_keys_are_mode_scoped_and_carry_full_identity() {
        let diff = main_commit_diff_cache_key("live", "staging", "abc123");
        assert!(diff.contains("staging") && diff.contains("abc123"));
        assert_ne!(
            diff,
            main_commit_diff_cache_key("fixture", "staging", "abc123")
        );
        assert_ne!(
            diff,
            main_commit_diff_cache_key("live", "development", "abc123")
        );
        assert_ne!(
            diff,
            main_commit_diff_cache_key("live", "staging", "def456")
        );

        let audit = event_audit_cache_key("live", "staging", "hr", "abc123");
        assert!(audit.contains("staging") && audit.contains("hr") && audit.contains("abc123"));
        assert_ne!(
            audit,
            event_audit_cache_key("fixture", "staging", "hr", "abc123")
        );
    }
}

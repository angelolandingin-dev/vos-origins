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

use super::{cached_json, read_world};

fn live_token() -> AppResult<String> {
    // `gh_cli::auth_token` caches the `gh auth token` spawn (~5 min), so a
    // burst of commands costs at most one `gh` process. `GH_TOKEN` /
    // `GITHUB_TOKEN` always win.
    crate::github::gh_cli::auth_token().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
    })
}

fn mode_prefix() -> &'static str {
    if live_read::use_fixtures() {
        "fixture"
    } else {
        "live"
    }
}

pub(crate) fn commits_cache_key(mode: &str, query: &CommitQuery) -> String {
    format!(
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
    )
}

/// Full request identity with sorted `shas`, so tab remounts and reordered
/// callers share one cache entry (and one clone+`gh` round).
pub(crate) fn statuses_cache_key(
    mode: &str,
    target_branch: &str,
    subsystem_id: Option<&str>,
    shas: &[String],
) -> String {
    let mut sorted = shas.to_vec();
    sorted.sort();
    format!(
        "{mode}::statuses::{}::{}::{}",
        target_branch,
        subsystem_id.unwrap_or("<all>"),
        sorted.join(",")
    )
}

pub(crate) fn details_cache_key(
    mode: &str,
    subsystem_id: &str,
    sha: &str,
    target_branch: &str,
) -> String {
    format!("{mode}::details::{subsystem_id}::{sha}::{target_branch}")
}

pub(crate) fn diff_cache_key(
    mode: &str,
    subsystem_id: &str,
    sha: &str,
    file_path: Option<&str>,
) -> String {
    format!(
        "{mode}::diff::{subsystem_id}::{sha}::{}",
        file_path.unwrap_or("<first>")
    )
}

fn missing_subsystem(subsystem_id: &str) -> AppError {
    AppError::invalid_registry(format!(
        "'{subsystem_id}' is not present in the tool registry."
    ))
    .with_remedy("Register the subsystem first in %APPDATA%\\vos-origin\\registry.json.")
}

#[tauri::command]
pub async fn get_subsystem_commits(
    state: State<'_, AppState>,
    query: CommitQuery,
) -> AppResult<CommitPage> {
    // Keep the cache key mode-scoped so live and fixture never collide.
    let key = commits_cache_key(mode_prefix(), &query);
    cached_json(&state, &key, || {
        if live_read::use_fixtures() {
            let page = read_world(&state, |world| state.adapter.commit_page(world, &query))?;
            return Ok(page);
        }

        let registry = live_read::load_live_registry()?;
        let token = live_token()?;
        // Reusable on-disk checkouts: repeat visits reuse the same clone.
        let (target_dir, _) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        let github = RealGitHubOps::new(token.clone());
        let source_resolver =
            |entry: &SubsystemEntry| Some(authenticated_remote_url(&entry.repository, &token));
        // Resolve the source checkout for this subsystem.
        let entry = registry
            .find(&query.subsystem_id)
            .cloned()
            .ok_or_else(|| missing_subsystem(&query.subsystem_id))?;
        let (source_dir, _) =
            live_read::resolve_source_checkout(&entry, &source_resolver, Some(&token))?;
        live_read::commit_page_live(&target_dir, &source_dir, &registry, &github, &query)
    })
}

#[tauri::command]
pub async fn get_commit_details(
    state: State<'_, AppState>,
    subsystem_id: String,
    sha: String,
    target_branch: String,
) -> AppResult<CommitDetails> {
    let key = details_cache_key(mode_prefix(), &subsystem_id, &sha, &target_branch);
    cached_json(&state, &key, || {
        if live_read::use_fixtures() {
            return read_world(&state, |world| {
                state
                    .adapter
                    .commit_details(world, &subsystem_id, &sha, &target_branch)
            });
        }
        // Cached like every other read: status + PR overlays refresh when the
        // 90s entry expires, and promotion clears the cache on dispatch.
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
        let entry = registry
            .find(&subsystem_id)
            .cloned()
            .ok_or_else(|| missing_subsystem(&subsystem_id))?;
        let (source_dir, _) =
            live_read::resolve_source_checkout(&entry, &source_resolver, Some(&token))?;
        live_read::commit_details_live(
            &target_dir,
            &source_dir,
            &registry,
            &github,
            &subsystem_id,
            &sha,
            &target_branch,
        )
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
    let key = diff_cache_key(mode_prefix(), &subsystem_id, &sha, file_path.as_deref());
    cached_json(&state, &key, || {
        if live_read::use_fixtures() {
            let diff = read_world(&state, |world| {
                if let Some(branch) = target_branch.as_deref() {
                    state.adapter.require_target_branch(world, branch)?;
                }
                state
                    .adapter
                    .commit_diff(world, &subsystem_id, &sha, file_path.as_deref())
            })?;
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
        let entry = registry
            .find(&subsystem_id)
            .cloned()
            .ok_or_else(|| missing_subsystem(&subsystem_id))?;
        let (source_dir, _) =
            live_read::resolve_source_checkout(&entry, &source_resolver, Some(&token))?;
        live_read::commit_diff_live(
            &source_dir,
            &registry,
            &subsystem_id,
            &sha,
            file_path.as_deref(),
        )
    })
}

/// Never fabricates `Integrated`.
#[tauri::command]
pub async fn get_integration_statuses(
    state: State<'_, AppState>,
    target_branch: String,
    subsystem_id: Option<String>,
    shas: Vec<String>,
) -> AppResult<Vec<IntegrationStatus>> {
    let key = statuses_cache_key(
        mode_prefix(),
        &target_branch,
        subsystem_id.as_deref(),
        &shas,
    );
    cached_json(&state, &key, || {
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
        let (target_dir, _) = live_read::resolve_target_checkout(
            &registry.main_repository,
            registry.local_main_repository_path.as_deref(),
            Some(&token),
        )?;
        let github = RealGitHubOps::new(token);
        live_read::integration_statuses_live(
            &target_dir,
            &registry,
            &github,
            &target_branch,
            subsystem_id.as_deref(),
            &shas,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::models::{CommitSort, CommitStatusKind};

    fn query(shas: Option<Vec<String>>) -> CommitQuery {
        CommitQuery {
            subsystem_id: "hr-management".to_string(),
            target_branch: "staging".to_string(),
            page: 1,
            page_size: 50,
            search: String::new(),
            status_filter: None,
            sort: CommitSort::Newest,
            shas,
        }
    }

    #[test]
    fn cache_keys_are_mode_scoped_and_carry_full_identity() {
        let live = statuses_cache_key(
            "live",
            "staging",
            Some("hr"),
            &["b".to_string(), "a".to_string()],
        );
        let reordered = statuses_cache_key(
            "live",
            "staging",
            Some("hr"),
            &["a".to_string(), "b".to_string()],
        );
        assert_eq!(live, reordered, "sha order must not split the cache");
        assert_ne!(
            live,
            statuses_cache_key("fixture", "staging", Some("hr"), &["a".to_string()])
        );
        assert_ne!(
            live,
            statuses_cache_key("live", "development", Some("hr"), &["a".to_string()])
        );
        assert_ne!(
            live,
            statuses_cache_key("live", "staging", None, &["a".to_string()])
        );

        let details = details_cache_key("live", "hr", "abc", "staging");
        assert!(details.contains("hr") && details.contains("abc") && details.contains("staging"));
        assert_ne!(
            details,
            details_cache_key("fixture", "hr", "abc", "staging")
        );

        let commits = commits_cache_key("live", &query(None));
        assert!(commits.starts_with("live::commits::hr-management::staging::"));
        let filtered = CommitQuery {
            status_filter: Some(CommitStatusKind::Integrated),
            ..query(None)
        };
        assert_ne!(commits, commits_cache_key("live", &filtered));

        let diff = diff_cache_key("live", "hr", "abc", Some("a.txt"));
        assert_ne!(diff, diff_cache_key("live", "hr", "abc", None));
    }
}

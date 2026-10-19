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

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutRefresh {
    pub refreshed: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
    pub message: String,
    pub data_source: crate::github::models::DataSource,
}

fn no_token_for_refresh() -> AppError {
    AppError::new(
        ErrorCode::Unauthenticated,
        "No GitHub token is available to refresh the checkouts.",
    )
    .with_remedy(
        "Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry. A configured local main clone (localMainRepositoryPath) can be refreshed without a token.",
    )
}

/// Token-less managed checkouts are skipped, not an error.
fn refresh_one(key: &str, repository: &str, token: Option<&str>, out: &mut CheckoutRefresh) {
    let Some(token) = token else {
        out.skipped.push(repository.to_string());
        return;
    };
    let url = crate::services::engine::github_ops::authenticated_remote_url(repository, token);
    match crate::services::checkouts::refresh_cached_checkout(key, &url, Some(token)) {
        Ok(true) => out.refreshed.push(repository.to_string()),
        Ok(false) => out.skipped.push(repository.to_string()),
        Err(err) => out.failed.push(format!("{repository}: {}", err.message)),
    }
}

fn configured_local_main_clone(
    registry: &crate::services::registry::Registry,
) -> Option<std::path::PathBuf> {
    let configured = registry
        .local_main_repository_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("VOS_TARGET_REPO_DIR")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        });
    if let Some(path) = configured {
        return path.is_dir().then_some(path);
    }
    crate::services::checkouts::local_dir_for(&registry.main_repository)
}

/// Manual "pull latest": force-fetch every **already-cached** checkout so the
/// next read is fresh. Never clones a repo that has not been opened yet, so the
/// action stays fast and predictable.
#[tauri::command]
pub async fn refresh_checkouts(
    state: State<'_, AppState>,
    subsystem_id: Option<String>,
) -> AppResult<CheckoutRefresh> {
    if crate::services::live_read::use_fixtures() {
        return Ok(CheckoutRefresh {
            refreshed: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            message: "Fixture mode: there is no checkout to refresh.".to_string(),
            data_source: crate::github::models::DataSource::Fixture,
        });
    }

    let registry = crate::services::live_read::load_live_registry()?;
    // The local clone can refresh without a token; only a managed-only setup is fatal.
    let local_clone = configured_local_main_clone(&registry);
    let token = gh_cli::auth_token();
    if token.is_none() && local_clone.is_none() {
        return Err(no_token_for_refresh());
    }
    let mut out = CheckoutRefresh {
        refreshed: Vec::new(),
        skipped: Vec::new(),
        failed: Vec::new(),
        message: String::new(),
        data_source: crate::github::models::DataSource::Live,
    };

    refresh_one(
        &crate::services::checkouts::target_key(&registry.main_repository),
        &registry.main_repository,
        token.as_deref(),
        &mut out,
    );
    if let Some(local) = local_clone {
        match crate::services::checkouts::refresh_local_clone(&local, token.as_deref()) {
            Ok(()) => out.refreshed.push(local.display().to_string()),
            Err(err) => out
                .failed
                .push(format!("{}: {}", local.display(), err.message)),
        }
    }
    for entry in registry.subsystems.iter().filter(|entry| {
        subsystem_id
            .as_deref()
            .map_or(entry.enabled, |id| id == entry.id)
    }) {
        refresh_one(
            &crate::services::checkouts::source_key(entry),
            &entry.repository,
            token.as_deref(),
            &mut out,
        );
    }

    // Cached responses would mask the refresh.
    state.cache.clear();
    out.message = format!(
        "Refreshed {} checkout(s); {} not cached yet; {} failed.",
        out.refreshed.len(),
        out.skipped.len(),
        out.failed.len()
    );
    Ok(out)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutClear {
    pub removed: usize,
    pub message: String,
}

/// Manual escape hatch: delete the cached clones entirely. Only needed when a
/// checkout is corrupt *and* the automatic self-heal could not fix it — every
/// clone is re-created on next use.
#[tauri::command]
pub async fn clear_checkouts(state: State<'_, AppState>) -> AppResult<CheckoutClear> {
    let root = crate::services::checkouts::checkouts_root();
    let removed = if root.is_dir() {
        std::fs::read_dir(&root)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
    } else {
        0
    };
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!("Could not delete {}: {err}", root.display()),
            )
            .with_remedy("Close anything using the folder (or a running git process) and retry.")
        })?;
    }
    state.cache.clear();
    Ok(CheckoutClear {
        removed,
        message: format!(
            "Deleted {removed} cached clone(s). Each one is re-cloned the next time it is used."
        ),
    })
}

/// Best-effort, once-per-process warm of the cached checkouts, run off the
/// request path so the window is never blocked by a network fetch.
pub fn warm_checkouts_once() {
    static WARMED: std::sync::Once = std::sync::Once::new();
    WARMED.call_once(|| {
        if crate::services::live_read::use_fixtures() {
            return;
        }
        tauri::async_runtime::spawn_blocking(|| {
            crate::services::checkouts::sweep_orphaned_checkouts();
            let Ok(registry) = crate::services::live_read::load_live_registry() else {
                return;
            };
            let Some(token) = gh_cli::auth_token() else {
                return;
            };
            let _ = crate::services::checkouts::refresh_cached_checkout(
                &crate::services::checkouts::target_key(&registry.main_repository),
                &crate::services::engine::github_ops::authenticated_remote_url(
                    &registry.main_repository,
                    &token,
                ),
                Some(&token),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vos-system-{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn registry_json(main_repository: &str, extra: &str) -> crate::services::registry::Registry {
        let json = format!(
            r#"{{
  "mainRepository": "{main_repository}",
  {extra}
  "promotion": {{ "allowedTargetBranches": ["staging"], "requirePullRequest": true }},
  "subsystems": []
}}"#
        );
        crate::services::registry::Registry::from_json(&json, "test").expect("registry")
    }

    #[test]
    fn configured_local_main_clone_finds_the_user_clone() {
        let _env = crate::test_support::env_lock();
        let dir = unique_dir("clone");
        let dir_json = dir.to_string_lossy().replace('\\', "\\\\");

        let via_path = registry_json(
            "ORG/main-system",
            &format!(r#""localMainRepositoryPath": "{dir_json}","#),
        );
        assert_eq!(configured_local_main_clone(&via_path), Some(dir.clone()));

        let missing_json = dir.join("nope").to_string_lossy().replace('\\', "\\\\");
        let missing = registry_json(
            "ORG/main-system",
            &format!(r#""localMainRepositoryPath": "{missing_json}","#),
        );
        assert_eq!(configured_local_main_clone(&missing), None);

        if std::env::var("VOS_TARGET_REPO_DIR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_none()
        {
            let remote_only = registry_json("ORG/main-system", "");
            assert_eq!(configured_local_main_clone(&remote_only), None);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}

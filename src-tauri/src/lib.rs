//! Control plane for subsystem integration (spec §9).
//! Never patches/builds itself; privileged work happens in GitHub Actions
//! which opens an integration PR against the target branch.

pub mod commands;
pub mod error;
pub mod fixtures;
pub mod github;
pub mod platform;
pub mod services;
#[cfg(test)]
pub mod test_support;

use std::sync::Mutex;

use fixtures::FixtureWorld;
use github::api::GitHubAdapter;
use services::cache::{Cache, Singleflight};

/// Disposable fixture world + response cache; never authoritative.
pub struct AppState {
    pub world: Mutex<FixtureWorld>,
    pub cache: Cache,
    pub singleflight: Singleflight,
    pub adapter: GitHubAdapter,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            world: Mutex::new(FixtureWorld::build()),
            cache: Cache::new(256),
            singleflight: Singleflight::new(),
            adapter: GitHubAdapter::default(),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::auth::get_auth_status,
            commands::registry::list_subsystems,
            commands::registry::get_subsystem_summaries,
            commands::registry::get_registry,
            commands::registry::save_registry,
            commands::registry::list_target_branches,
            commands::registry::submit_subsystem_registration,
            commands::commits::get_subsystem_commits,
            commands::commits::get_commit_details,
            commands::commits::get_commit_diff,
            commands::commits::get_integration_statuses,
            commands::diff::get_subsystem_diff,
            commands::promotion::dispatch_promotion,
            commands::promotion::preflight_promotion,
            commands::history::list_integration_prs,
            commands::history::list_integration_history,
            commands::history::get_integration_event,
            commands::history::get_main_commit_diff,
            commands::system::get_cache_stats,
            commands::system::clear_cache,
            commands::system::refresh_checkouts,
            commands::system::clear_checkouts,
            commands::system::open_external_url,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the Subsystem Integration Manager");
}

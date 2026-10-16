//! Registry commands: live (registry-backed) by default, fixtures on opt-in.

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures;
use crate::github::models::{
    RegistrationResult, SubsystemRegistrationRequest, SubsystemSummary, TargetBranchList,
};
use crate::services::live_read;
use crate::services::registration;
use crate::services::registry::Registry;
use crate::AppState;

use super::{read_world, write_world};

fn live_token() -> AppResult<String> {
    crate::github::gh_cli::auth_token().ok_or_else(|| {
        AppError::new(
            ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
    })
}

#[tauri::command]
pub async fn list_subsystems(state: State<'_, AppState>) -> AppResult<Vec<SubsystemSummary>> {
    if live_read::use_fixtures() {
        return read_world(&state, |world| Ok(state.adapter.list_subsystems(world)));
    }
    let registry = live_read::load_live_registry()?;
    Ok(live_read::subsystem_summaries(&registry))
}

#[tauri::command]
pub async fn list_target_branches(state: State<'_, AppState>) -> AppResult<TargetBranchList> {
    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            state
                .adapter
                .target_branches(world, fixtures::DEFAULT_TARGET_BRANCH)
        });
    }
    let registry = live_read::load_live_registry()?;
    let token = live_token()?;
    let github = crate::services::engine::github_ops::RealGitHubOps::new(token);
    // Selected defaults to the fixture default when it exists and is allowed.
    live_read::target_branches_live(&registry, &github, Some(fixtures::DEFAULT_TARGET_BRANCH))
}

/// Duplicates are `duplicate_registration`; other failures are `invalid_registry`.
#[tauri::command]
pub async fn submit_subsystem_registration(
    state: State<'_, AppState>,
    request: SubsystemRegistrationRequest,
) -> AppResult<RegistrationResult> {
    write_world(&state, |world| {
        let findings = registration::validate(&request, world);

        let duplicate = findings
            .iter()
            .any(|f| f.code == "duplicate_id" || f.code == "duplicate_repository");
        if duplicate {
            let messages = findings
                .iter()
                .filter(|f| f.level == crate::github::models::FindingLevel::Error)
                .map(|f| f.message.clone())
                .collect::<Vec<_>>()
                .join(" ");
            return Err(AppError::new(ErrorCode::DuplicateRegistration, messages)
                .with_remedy(
                    "Open the existing entry under Subsystems to inspect it, or register the subsystem under a different id/repository.",
                )
                .with_details(format_findings(&findings)));
        }

        if registration::has_errors(&findings) {
            let messages = findings
                .iter()
                .filter(|f| f.level == crate::github::models::FindingLevel::Error)
                .map(|f| f.message.clone())
                .collect::<Vec<_>>()
                .join(" ");
            return Err(AppError::invalid_registry(messages)
                .with_remedy("Correct the highlighted fields. The registration PR is only opened for a valid request.")
                .with_details(format_findings(&findings)));
        }

        let branch = registration::branch_name_for(&request.id);
        let pr_number = world.next_pr_number;
        world.next_pr_number += 1;

        Ok(RegistrationResult {
            accepted: true,
            branch: Some(branch.clone()),
            pull_request_number: Some(pr_number),
            pull_request_url: Some(format!(
                "https://github.com/{}/pull/{pr_number}",
                state.adapter.main_repository
            )),
            message: format!(
                "Registration PR #{pr_number} prepared on branch '{branch}' against {}/{}. Fixture mode: no real branch or PR was created.",
                state.adapter.main_repository, state.adapter.registry_branch
            ),
            findings,
        })
    })
}

fn format_findings(findings: &[crate::github::models::ValidationFinding]) -> String {
    findings
        .iter()
        .map(|finding| format!("[{:?}] {} {}", finding.level, finding.code, finding.message))
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistrySource {
    Bundled,
    Override,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySnapshot {
    pub config: Registry,
    pub source: RegistrySource,
    pub override_path: String,
    pub override_exists: bool,
}

fn registry_snapshot() -> AppResult<RegistrySnapshot> {
    let override_path = Registry::app_data_override_path();
    let override_exists = override_path
        .as_deref()
        .map(|path| path.exists())
        .unwrap_or(false);
    let config = Registry::load(override_path.as_deref())?;
    Ok(RegistrySnapshot {
        config,
        source: if override_exists {
            RegistrySource::Override
        } else {
            RegistrySource::Bundled
        },
        override_path: override_path
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "(no user configuration directory is available)".to_string()),
        override_exists,
    })
}

#[tauri::command]
pub async fn get_registry() -> AppResult<RegistrySnapshot> {
    registry_snapshot()
}

/// Validates fail-closed, then writes the tool-local override (`%APPDATA%`). Never touches a repo.
#[tauri::command]
pub async fn save_registry(config: Registry) -> AppResult<RegistrySnapshot> {
    let path = Registry::app_data_override_path().ok_or_else(|| {
        AppError::new(
            ErrorCode::Internal,
            "No user configuration directory is available to save the registry.",
        )
        .with_remedy("Set the APPDATA (or XDG_CONFIG_HOME) environment variable and retry.")
    })?;

    config.validate().map_err(|err| {
        AppError::invalid_registry(format!("Registry validation failed: {}", err.message))
            .with_remedy(err.remedy.unwrap_or_else(|| {
                "Correct the registry entry; the tool refuses to save an unsafe policy.".to_string()
            }))
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            AppError::new(
                ErrorCode::Internal,
                format!("Could not create {}: {err}", parent.display()),
            )
        })?;
    }

    let json = serde_json::to_string_pretty(&config).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not encode the registry: {err}"),
        )
    })?;

    std::fs::write(&path, json).map_err(|err| {
        AppError::new(
            ErrorCode::Internal,
            format!("Could not write {}: {err}", path.display()),
        )
        .with_remedy("Check permissions for the user configuration directory.")
    })?;

    registry_snapshot()
}

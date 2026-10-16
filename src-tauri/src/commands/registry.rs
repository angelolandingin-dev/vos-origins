//! Registry commands: live (registry-backed) by default, fixtures on opt-in.

use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures;
use crate::github::models::{
    RegistrationResult, SubsystemRegistrationRequest, SubsystemSummary, TargetBranchList,
};
use crate::services::live_read;
use crate::services::registration;
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

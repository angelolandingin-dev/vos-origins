//! Registry commands: live (registry-backed) by default, fixtures on opt-in.

use serde::Serialize;
use tauri::State;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::fixtures;
use crate::github::models::{
    RegistrationResult, SubsystemRegistrationRequest, SubsystemSummary, SubsystemSummaryCounts,
    TargetBranchList,
};
use crate::services::live_read;
use crate::services::registration;
use crate::services::registry::Registry;
use crate::AppState;

use super::{cached_json, read_world, write_world};

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

/// Per-subsystem counts for ALL registered subsystems in ONE call (§11, G7).
/// Wire shape is exact: `Vec<SubsystemSummaryCounts>` (camelCase
/// `subsystemId`, `indexed`, `total`, `tally` with existing status keys).
/// Bounded concurrency (max 4 parallel subsystems), reusing the existing
/// caches: the on-disk checkout cache (`ensure_remote_checkout` reuses clones
/// + TTL fetch) and the 90s response cache (`cached_json` with the same keys
/// as the commit/status reads, so a warm tab costs no extra clone/`gh` round).
#[tauri::command]
pub async fn get_subsystem_summaries(
    state: State<'_, AppState>,
) -> AppResult<Vec<SubsystemSummaryCounts>> {
    use std::collections::HashMap;

    if live_read::use_fixtures() {
        return read_world(&state, |world| {
            let subsystems = state.adapter.list_subsystems(world);
            let mut out: Vec<SubsystemSummaryCounts> = Vec::new();
            for sub in &subsystems {
                let commits = world.commits_of(&sub.id);
                let total = commits.len() as u32;
                // Fixture statuses via the adapter on the default target.
                let shas: Vec<String> = commits.iter().map(|c| c.sha.clone()).collect();
                let statuses = state.adapter.integration_statuses(
                    world,
                    fixtures::DEFAULT_TARGET_BRANCH,
                    Some(&sub.id),
                    &shas,
                )?;
                let mut tally: HashMap<String, u32> = HashMap::new();
                for st in &statuses {
                    let key = serde_json::to_string(&st.status.kind)
                        .map(|s| s.trim_matches('"').to_string())
                        .unwrap_or_else(|_| "unknown".to_string());
                    *tally.entry(key).or_insert(0) += 1;
                }
                out.push(SubsystemSummaryCounts {
                    subsystem_id: sub.id.clone(),
                    indexed: statuses.len() as u32,
                    total,
                    tally,
                });
            }
            out.sort_by(|a, b| a.subsystem_id.cmp(&b.subsystem_id));
            Ok(out)
        });
    }

    let registry = live_read::load_live_registry()?;
    // Tally target: first allowed branch (the UI's default target).
    let tally_branch = registry
        .promotion
        .allowed_target_branches
        .first()
        .cloned()
        .unwrap_or_else(|| fixtures::DEFAULT_TARGET_BRANCH.to_string());
    let ids: Vec<String> = registry.subsystems.iter().map(|s| s.id.clone()).collect();
    // Clone what threads need (State is not Sync across scope without Arc).
    let state_ref = &state;
    // Bounded parallelism: chunks of 4, sequential across chunks. Each worker
    // reuses the checkout cache and `cached_json` (same keys as the reads).
    const BOUND: usize = 4;
    let mut out: Vec<SubsystemSummaryCounts> = Vec::new();
    for chunk in ids.chunks(BOUND) {
        // `tauri::async_runtime::spawn_blocking` per subsystem would need
        // `'static`; instead run the (blocking git/`gh`) work in a scoped
        // thread pool — still bounded to BOUND concurrent subsystems.
        let results: Vec<AppResult<SubsystemSummaryCounts>> = std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for id in chunk {
                let id = id.clone();
                let registry = registry.clone();
                let tally_branch = tally_branch.clone();
                handles.push(scope.spawn(move || {
                    // Per-subsystem cache key so repeats hit the 90s cache.
                    let key = format!("live::summaries::{tally_branch}::{id}");
                    cached_json(state_ref, &key, || {
                        compute_one_summary(&registry, &tally_branch, &id)
                    })
                }));
            }
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(crate::error::AppError::new(
                            crate::error::ErrorCode::Internal,
                            "summary worker did not complete",
                        ))
                    })
                })
                .collect()
        });
        for r in results {
            out.push(r?);
        }
    }
    out.sort_by(|a, b| a.subsystem_id.cmp(&b.subsystem_id));
    Ok(out)
}

/// One subsystem's counts (blocking; runs inside the bounded pool above).
/// Reuses the checkout cache (clone-once + TTL fetch, no per-commit network
/// round-trip) and counts via the same `commit_page_live` the reads use, so
/// a warm cache costs no extra clone/`gh` round.
fn compute_one_summary(
    registry: &Registry,
    tally_branch: &str,
    subsystem_id: &str,
) -> crate::error::AppResult<SubsystemSummaryCounts> {
    use crate::github::models::{CommitQuery, CommitSort};
    use crate::services::engine::github_ops::RealGitHubOps;
    use std::collections::HashMap;

    let token = crate::github::gh_cli::auth_token().ok_or_else(|| {
        crate::error::AppError::new(
            crate::error::ErrorCode::Unauthenticated,
            "No GitHub token is available (checked GH_TOKEN, GITHUB_TOKEN and `gh auth token`).",
        )
        .with_remedy("Run `gh auth login --hostname github.com` or set GH_TOKEN, then retry.")
    })?;
    let (target_dir, _) = live_read::resolve_target_checkout(
        &registry.main_repository,
        registry.local_main_repository_path.as_deref(),
        Some(&token),
    )?;
    let entry = registry.find(subsystem_id).cloned().ok_or_else(|| {
        crate::error::AppError::invalid_registry(format!(
            "'{subsystem_id}' is not present in the tool registry."
        ))
    })?;
    let resolver = |e: &crate::services::registry::SubsystemEntry| {
        Some(crate::services::engine::github_ops::authenticated_remote_url(&e.repository, &token))
    };
    let (source_dir, _) = live_read::resolve_source_checkout(&entry, &resolver, Some(&token))?;
    let github = RealGitHubOps::new(token);
    // One page (50) via the same live path the Commit Browser uses; `total`
    // is the full source count, `indexed` is the page counted, `tally` keys
    // are the existing snake_case status keys.
    let query = CommitQuery {
        subsystem_id: subsystem_id.to_string(),
        target_branch: tally_branch.to_string(),
        page: 1,
        page_size: 50,
        search: String::new(),
        status_filter: None,
        sort: CommitSort::Newest,
        shas: None,
    };
    let page = live_read::commit_page_live(&target_dir, &source_dir, registry, &github, &query)?;
    let mut tally: HashMap<String, u32> = HashMap::new();
    for c in &page.commits {
        let key = serde_json::to_string(&c.status.kind)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        *tally.entry(key).or_insert(0) += 1;
    }
    Ok(SubsystemSummaryCounts {
        subsystem_id: subsystem_id.to_string(),
        indexed: page.commits.len() as u32,
        total: page.total,
        tally,
    })
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
pub async fn save_registry(
    state: State<'_, AppState>,
    config: Registry,
) -> AppResult<RegistrySnapshot> {
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

    // A changed main repository invalidates every cached read.
    state.cache.clear();

    registry_snapshot()
}

#[cfg(test)]
mod tests {
    use crate::github::models::SubsystemSummaryCounts;
    use std::collections::HashMap;

    /// G7: exact camelCase wire shape the React agent consumes.
    #[test]
    fn summaries_wire_shape_is_exact_camel_case() {
        let counts = SubsystemSummaryCounts {
            subsystem_id: "hr-management".to_string(),
            indexed: 50,
            total: 120,
            tally: HashMap::from([("eligible".to_string(), 40), ("integrated".to_string(), 10)]),
        };
        let v = serde_json::to_value(&counts).expect("serializes");
        assert_eq!(v["subsystemId"], "hr-management");
        assert_eq!(v["indexed"], 50);
        assert_eq!(v["total"], 120);
        assert_eq!(v["tally"]["eligible"], 40);
        assert_eq!(v["tally"]["integrated"], 10);
        // No snake_case leakage.
        assert!(v.get("subsystem_id").is_none());
        let back: SubsystemSummaryCounts = serde_json::from_value(v).expect("round-trips");
        assert_eq!(back.subsystem_id, "hr-management");
    }

    /// Wire field names for the new promotion inputs (exact).
    #[test]
    fn promotion_wire_fields_are_exact() {
        use crate::github::models::PromotionRequest;
        let req: PromotionRequest = serde_json::from_str(
            r#"{"subsystemId":"hr","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true,"delivery":"local_direct","onConflict":"fail","message":"m","allowGaps":true,"excludedPaths":["a.txt"]}"#,
        )
        .expect("parses");
        assert!(req.allow_gaps);
        assert_eq!(req.excluded_paths, vec!["a.txt".to_string()]);
        // Defaults when omitted.
        let req: PromotionRequest = serde_json::from_str(
            r#"{"subsystemId":"hr","sourceCommitShas":[],"mode":"individual","targetBranch":"staging","confirmationAcknowledged":true}"#,
        )
        .expect("parses without new fields");
        assert!(!req.allow_gaps);
        assert!(req.excluded_paths.is_empty());
    }
}

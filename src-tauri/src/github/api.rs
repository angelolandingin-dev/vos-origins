//! Fixture-backed adapter (Phase 2); signatures are the final contract.
//! Registry comes from the registry branch, status from reachable trailers
//! + open PRs/runs, diffs are bounded and fetched on demand.

use std::collections::HashMap;

use crate::error::{AppError, AppResult};
use crate::fixtures::{
    now_rfc3339, FixtureWorld, DEFAULT_TARGET_BRANCH, MAIN_REPOSITORY, REGISTRY_BRANCH,
    WORKFLOW_FILE,
};
use crate::github::models::*;
use crate::services::commit_index::{self, StatusSignals};

const MAX_DIFF_LINES: usize = 1500;

#[derive(Debug, Clone)]
pub struct GitHubAdapter {
    pub data_source: DataSource,
    pub main_repository: String,
    pub registry_branch: String,
    pub workflow_file: String,
}

impl Default for GitHubAdapter {
    fn default() -> Self {
        Self {
            data_source: DataSource::Fixture,
            main_repository: MAIN_REPOSITORY.to_string(),
            registry_branch: REGISTRY_BRANCH.to_string(),
            workflow_file: WORKFLOW_FILE.to_string(),
        }
    }
}

impl GitHubAdapter {
    fn status_map(
        world: &FixtureWorld,
        subsystem_id: &str,
        target_branch: &str,
    ) -> HashMap<String, CommitStatus> {
        let eligible: Vec<String> = world
            .commits_of(subsystem_id)
            .iter()
            .filter(|commit| commit.eligible)
            .map(|commit| commit.sha.clone())
            .collect();
        let ancestry = world.ancestry();
        commit_index::build_status_map(&StatusSignals {
            subsystem_id,
            target_branch,
            eligible_shas: &eligible,
            events: &world.events,
            prs: &world.prs,
            runs: &world.workflow_runs,
            failure_effects: &world.failure_effects,
            ancestry: &ancestry,
        })
    }

    pub fn subsystem<'a>(
        &self,
        world: &'a FixtureWorld,
        subsystem_id: &str,
    ) -> AppResult<&'a SubsystemSummary> {
        if subsystem_id.trim().is_empty() {
            return Err(
                AppError::invalid_registry("No subsystem was specified.").with_remedy(
                    "Pick a registered subsystem from the Subsystems or Commit Browser screen.",
                ),
            );
        }
        let subsystem = world.subsystem(subsystem_id).ok_or_else(|| {
            AppError::invalid_registry(format!(
                "'{subsystem_id}' is not present in the registry on {}/{REGISTRY_BRANCH}.",
                self.main_repository
            ))
            .with_remedy("Register the subsystem first; a registration PR adds .integration/subsystems/<id>.json.")
        })?;
        if !subsystem.enabled {
            return Err(AppError::invalid_registry(format!(
                "Subsystem '{}' is registered but disabled.",
                subsystem.id
            ))
            .with_remedy("Disabling is recorded through a registration PR; re-enable it the same way, then refresh."));
        }
        Ok(subsystem)
    }

    pub fn list_subsystems(&self, world: &FixtureWorld) -> Vec<SubsystemSummary> {
        let mut subsystems = world.subsystems.clone();
        subsystems.sort_by(|a, b| a.name.cmp(&b.name));
        subsystems
    }

    pub fn target_branches(
        &self,
        world: &FixtureWorld,
        selected: &str,
    ) -> AppResult<TargetBranchList> {
        let exists = world.branches.iter().any(|branch| branch.name == selected);
        if !exists {
            return Err(AppError::new(
                crate::error::ErrorCode::BranchNotPermitted,
                format!(
                    "Target branch '{selected}' does not exist on {}.",
                    self.main_repository
                ),
            )
            .with_remedy("Only existing branches that pass the registry policy can be selected."));
        }

        Ok(TargetBranchList {
            target_branch: selected.to_string(),
            main_repository: self.main_repository.clone(),
            registry_branch: self.registry_branch.clone(),
            branches: world.branches.clone(),
            policy: world.policy.clone(),
            fetched_at: now_rfc3339(),
            data_source: self.data_source,
        })
    }

    pub fn commit_page(&self, world: &FixtureWorld, query: &CommitQuery) -> AppResult<CommitPage> {
        let subsystem = self.subsystem(world, &query.subsystem_id)?;
        let target_branch = self.require_target_branch(world, &query.target_branch)?;
        let statuses = Self::status_map(world, &subsystem.id, &target_branch);

        let search = query.search.trim().to_ascii_lowercase();
        let mut commits: Vec<CommitSummary> = world
            .commits_of(&subsystem.id)
            .iter()
            .filter(|commit| {
                query
                    .shas
                    .as_ref()
                    .map(|shas| {
                        shas.iter()
                            .any(|sha| commit.sha.starts_with(sha) || sha == &commit.sha)
                    })
                    .unwrap_or(true)
            })
            .filter(|commit| {
                if search.is_empty() {
                    return true;
                }
                commit.subject.to_ascii_lowercase().contains(&search)
                    || commit.sha.to_ascii_lowercase().starts_with(&search)
                    || commit.author_name.to_ascii_lowercase().contains(&search)
                    || commit
                        .author_login
                        .as_deref()
                        .unwrap_or_default()
                        .to_ascii_lowercase()
                        .contains(&search)
                    || commit
                        .tags
                        .iter()
                        .any(|tag| tag.to_ascii_lowercase().contains(&search))
            })
            .cloned()
            .map(|mut commit| {
                commit.status = statuses.get(&commit.sha).cloned().unwrap_or_else(|| {
                    CommitStatus::unknown(&target_branch, "Status unavailable for this SHA.")
                });
                commit
            })
            .filter(|commit| match query.status_filter {
                Some(kind) => commit.status.kind == kind,
                None => true,
            })
            .collect();

        match query.sort {
            CommitSort::Newest => commits.sort_by(|a, b| b.committed_at.cmp(&a.committed_at)),
            CommitSort::Oldest => commits.sort_by(|a, b| a.committed_at.cmp(&b.committed_at)),
        }

        let total = commits.len() as u32;
        let page_size = query.page_size.clamp(5, 200);
        let page = query.page.max(1);
        let start = ((page - 1) * page_size) as usize;
        let paged: Vec<CommitSummary> = commits
            .into_iter()
            .skip(start)
            .take(page_size as usize)
            .collect();

        Ok(CommitPage {
            target_branch,
            source_branch: subsystem.source_branch.clone(),
            subsystem_id: subsystem.id.clone(),
            total,
            page,
            page_size,
            commits: paged,
            fetched_at: now_rfc3339(),
            data_source: self.data_source,
        })
    }

    pub fn commit_details(
        &self,
        world: &FixtureWorld,
        subsystem_id: &str,
        sha: &str,
        target_branch: &str,
    ) -> AppResult<CommitDetails> {
        let subsystem = self.subsystem(world, subsystem_id)?;
        let target_branch = self.require_target_branch(world, target_branch)?;
        let commit = world.find_commit(&subsystem.id, sha).ok_or_else(|| {
            AppError::not_found(format!(
                "Commit '{sha}' was not found on {}.",
                subsystem.repository
            ))
            .with_remedy(
                "Refresh the commit list; the branch may have moved or the SHA may be mistyped.",
            )
        })?;

        let statuses = Self::status_map(world, &subsystem.id, &target_branch);
        let status = statuses.get(&commit.sha).cloned().unwrap_or_else(|| {
            CommitStatus::unknown(&target_branch, "Status unavailable for this SHA.")
        });

        if let Some(existing) = world.details_for(&subsystem.id, &commit.sha) {
            let mut details = existing.clone();
            details.target_branch = target_branch.clone();
            details.status = status.clone();
            details.short_sha = commit.short_sha.clone();
            details.relative_time = commit.relative_time.clone();
            details.note = promotion_note(&status);
            return Ok(details);
        }

        Ok(CommitDetails {
            subsystem_id: subsystem.id.clone(),
            target_branch: target_branch.clone(),
            sha: commit.sha.clone(),
            short_sha: commit.short_sha.clone(),
            subject: commit.subject.clone(),
            body: String::new(),
            author_name: commit.author_name.clone(),
            author_login: commit.author_login.clone(),
            authored_at: commit.authored_at.clone(),
            committed_at: commit.committed_at.clone(),
            relative_time: commit.relative_time.clone(),
            status: status.clone(),
            tags: commit.tags.clone(),
            files: Vec::new(),
            files_changed: commit.files_changed,
            additions: commit.additions,
            deletions: commit.deletions,
            dependencies: Vec::new(),
            related_pull_requests: Vec::new(),
            parents: commit.parents.clone(),
            is_merge: commit.is_merge,
            source_url: format!("https://github.com/{}/commit/{}", subsystem.repository, commit.sha),
            note: Some(
                "No file detail is part of the fixture set for this commit. Live mode fetches the changed file list from GitHub."
                    .to_string(),
            ),
        })
    }

    pub fn commit_diff(
        &self,
        world: &FixtureWorld,
        subsystem_id: &str,
        sha: &str,
        file_path: Option<&str>,
    ) -> AppResult<CommitDiff> {
        let subsystem = self.subsystem(world, subsystem_id)?;
        if world.find_commit(&subsystem.id, sha).is_none() {
            return Err(AppError::not_found(format!(
                "Commit '{sha}' was not found on {}.",
                subsystem.repository
            ))
            .with_remedy("Refresh the commit list and try again."));
        }

        let mut diff = world
            .commit_diff(&subsystem.id, sha, file_path)
            .ok_or_else(|| {
                AppError::not_found(format!(
                    "No diff is available for '{}' in commit {sha}.",
                    file_path.unwrap_or("<first changed file>")
                ))
                .with_remedy(
                    "Choose a file listed under 'Files Changed' in the commit details panel.",
                )
            })?;

        let mut budget = MAX_DIFF_LINES;
        let mut truncated = false;
        for hunk in &mut diff.hunks {
            if budget == 0 {
                truncated = true;
                hunk.lines.clear();
                continue;
            }
            if hunk.lines.len() > budget {
                hunk.lines.truncate(budget);
                budget = 0;
                truncated = true;
            } else {
                budget -= hunk.lines.len();
            }
        }
        if truncated {
            diff.truncated = true;
            diff.note = Some(match diff.note.take() {
                Some(existing) => format!(
                    "{existing} Response bounded at {MAX_DIFF_LINES} lines; open the file on GitHub for the rest."
                ),
                None => format!(
                    "Response bounded at {MAX_DIFF_LINES} lines to keep the window responsive; open the file on GitHub for the rest."
                ),
            });
        }

        Ok(diff)
    }

    pub fn integration_statuses(
        &self,
        world: &FixtureWorld,
        target_branch: &str,
        subsystem_id: Option<&str>,
        shas: &[String],
    ) -> AppResult<Vec<IntegrationStatus>> {
        let target_branch = self.require_target_branch(world, target_branch)?;
        let subsystem_ids: Vec<String> = match subsystem_id {
            Some(id) => vec![self.subsystem(world, id)?.id.clone()],
            None => world.subsystems.iter().map(|s| s.id.clone()).collect(),
        };

        let checked_at = now_rfc3339();
        let mut out = Vec::new();
        for id in subsystem_ids {
            let statuses = Self::status_map(world, &id, &target_branch);
            for sha in shas {
                let direct = statuses.get(sha).cloned();
                let resolved = match direct {
                    Some(status) => status,
                    None => {
                        let full = world
                            .commits_of(&id)
                            .iter()
                            .find(|c| c.sha.starts_with(sha))
                            .map(|c| c.sha.clone());
                        match full.and_then(|full| statuses.get(&full).cloned()) {
                            Some(status) => status,
                            None => CommitStatus::unknown(
                                &target_branch,
                                "This SHA is not reachable from the configured protected source branch, or the target branch could not be read.",
                            ),
                        }
                    }
                };
                out.push(IntegrationStatus {
                    subsystem_id: id.clone(),
                    source_sha: sha.clone(),
                    target_branch: target_branch.clone(),
                    status: resolved,
                    checked_at: checked_at.clone(),
                });
            }
        }
        Ok(out)
    }

    pub fn workflow_runs(
        &self,
        world: &FixtureWorld,
        subsystem_id: Option<&str>,
        target_branch: Option<&str>,
        limit: u32,
    ) -> WorkflowRunList {
        let mut runs: Vec<WorkflowRun> = world
            .workflow_runs
            .iter()
            .filter(|run| {
                subsystem_id
                    .map(|id| run.subsystem_id == id)
                    .unwrap_or(true)
            })
            .filter(|run| {
                target_branch
                    .map(|branch| run.target_branch == branch)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        runs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let total = runs.len() as u32;
        runs.truncate(limit.clamp(1, 100) as usize);

        WorkflowRunList {
            runs,
            total,
            fetched_at: now_rfc3339(),
            data_source: self.data_source,
        }
    }

    pub fn workflow_run_details(
        &self,
        world: &FixtureWorld,
        run_id: u64,
    ) -> AppResult<WorkflowRunDetails> {
        if let Some(details) = world.run_details.get(&run_id) {
            return Ok(details.clone());
        }
        if let Some(run) = world.workflow_runs.iter().find(|run| run.id == run_id) {
            return Ok(WorkflowRunDetails {
                run: run.clone(),
                jobs: Vec::new(),
                logs: Vec::new(),
                logs_truncated: false,
                logs_url: Some(run.html_url.clone()),
                artifact_note: Some(
                    "Run detail is not part of the fixture set; logs are fetched on demand in live mode.".to_string(),
                ),
            });
        }
        Err(
            AppError::not_found(format!("Workflow run {run_id} was not found.")).with_remedy(
                "Refresh the Workflow Runs list; runs are limited to those started by this tool.",
            ),
        )
    }

    pub fn integration_prs(
        &self,
        world: &FixtureWorld,
        target_branch: &str,
        subsystem_id: Option<&str>,
    ) -> AppResult<Vec<IntegrationPr>> {
        let target_branch = self.require_target_branch(world, target_branch)?;
        let mut prs: Vec<IntegrationPr> = world
            .prs
            .iter()
            .filter(|pr| pr.base_branch == target_branch)
            .filter(|pr| subsystem_id.map(|id| pr.subsystem_id == id).unwrap_or(true))
            .cloned()
            .collect();
        prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(prs)
    }

    pub fn integration_history(
        &self,
        world: &FixtureWorld,
        target_branch: Option<&str>,
        subsystem_id: Option<&str>,
        limit: u32,
    ) -> AppResult<IntegrationHistory> {
        let target_branch = match target_branch {
            Some(branch) => self.require_target_branch(world, branch)?,
            None => DEFAULT_TARGET_BRANCH.to_string(),
        };

        let mut entries: Vec<IntegrationHistoryEntry> = world
            .history
            .iter()
            .filter(|entry| entry.target_branch == target_branch)
            .filter(|entry| {
                subsystem_id
                    .map(|id| entry.subsystem_id == id)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        entries.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
        let total = entries.len() as u32;
        entries.truncate(limit.clamp(1, 200) as usize);

        Ok(IntegrationHistory {
            target_branch,
            subsystem_id: subsystem_id.map(str::to_string),
            entries,
            total,
            fetched_at: now_rfc3339(),
            data_source: self.data_source,
        })
    }

    pub fn require_target_branch(
        &self,
        world: &FixtureWorld,
        target_branch: &str,
    ) -> AppResult<String> {
        let branch = world.branches.iter().find(|branch| branch.name == target_branch).ok_or_else(|| {
            AppError::new(
                crate::error::ErrorCode::BranchNotPermitted,
                format!("Target branch '{target_branch}' does not exist on {}.", self.main_repository),
            )
            .with_remedy("Refresh the target branch list; it only contains branches that exist and pass registry policy.")
        })?;

        if !branch.allowed {
            return Err(AppError::new(
                crate::error::ErrorCode::BranchNotPermitted,
                format!("Promoting into '{target_branch}' is not permitted by the registry policy."),
            )
            .with_remedy(format!(
                "Allowed targets: {} {}. Add a branch to .integration/config.json through a reviewed PR to extend the policy.",
                world.policy.allowed_target_branches.join(", "),
                if world.policy.allowed_target_branch_patterns.is_empty() {
                    String::new()
                } else {
                    format!("and patterns {}", world.policy.allowed_target_branch_patterns.join(", "))
                }
            )));
        }

        Ok(branch.name.clone())
    }
}

fn promotion_note(status: &CommitStatus) -> Option<String> {
    match status.kind {
        CommitStatusKind::Eligible => Some(
            "Promoting applies the first-parent delta of this exact SHA only. Newer and unselected commits are never imported."
                .to_string(),
        ),
        CommitStatusKind::Integrated => Some(
            "Already recorded as integrated on this target branch. Re-promoting the same SHA to the same branch is blocked; a different branch is still allowed."
                .to_string(),
        ),
        CommitStatusKind::Reverted => Some(
            "Previously integrated and then reverted on this target branch. A deliberate re-promotion opens a new integration PR."
                .to_string(),
        ),
        CommitStatusKind::PrOpen | CommitStatusKind::InReview => Some(
            "An integration PR for this SHA is already open. There is no need to dispatch another run.".to_string(),
        ),
        CommitStatusKind::Blocked | CommitStatusKind::Failed => Some(
            "The last promotion attempt did not produce a PR. Review the workflow logs and the conflict report before retrying."
                .to_string(),
        ),
        _ => None,
    }
}

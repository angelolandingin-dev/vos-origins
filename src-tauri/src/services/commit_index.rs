//! Branch-scoped status (spec §3.4/§5/§8). `Integrated` only from reachable
//! provenance; runs/PRs never count; unverifiable stays `Unknown`.

use std::collections::HashMap;

use crate::fixtures::{FailureEffect, IntegrationEvent, IntegrationEventKind};
use crate::github::models::*;

pub struct StatusSignals<'a> {
    pub subsystem_id: &'a str,
    pub target_branch: &'a str,
    pub eligible_shas: &'a [String],
    pub events: &'a [IntegrationEvent],
    pub prs: &'a [IntegrationPr],
    pub runs: &'a [WorkflowRun],
    pub failure_effects: &'a [FailureEffect],
    pub ancestry: &'a [(&'static str, Vec<&'static str>)],
}

pub fn branches_reachable_from(target: &str, ancestry: &[(&str, Vec<&str>)]) -> Vec<String> {
    let mut reachable: Vec<String> = vec![target.to_string()];
    let mut cursor = 0usize;
    while cursor < reachable.len() {
        let current = reachable[cursor].clone();
        cursor += 1;
        if let Some((_, parents)) = ancestry
            .iter()
            .find(|(branch, _)| *branch == current.as_str())
        {
            for parent in parents {
                if !reachable.iter().any(|b| b == parent) {
                    reachable.push((*parent).to_string());
                }
            }
        }
    }
    reachable
}

pub fn build_status_map(signals: &StatusSignals<'_>) -> HashMap<String, CommitStatus> {
    let reachable = branches_reachable_from(signals.target_branch, signals.ancestry);
    let mut map: HashMap<String, CommitStatus> = HashMap::new();

    let candidate =
        |sha: &str, status: CommitStatus, map: &mut HashMap<String, CommitStatus>| match map
            .get(sha)
        {
            Some(existing) if existing.kind.precedence() >= status.kind.precedence() => {}
            _ => {
                map.insert(sha.to_string(), status);
            }
        };

    let detail_target = signals.target_branch;
    for run in signals.runs {
        if run.subsystem_id != signals.subsystem_id || run.target_branch != *detail_target {
            continue;
        }
        let (kind, label_detail) = match run.status {
            WorkflowRunStatus::Queued
            | WorkflowRunStatus::Requested
            | WorkflowRunStatus::Waiting => (
                CommitStatusKind::PromotionQueued,
                format!(
                    "Workflow run #{} is queued. An integration PR does not exist yet.",
                    run.run_number
                ),
            ),
            WorkflowRunStatus::InProgress => (
                CommitStatusKind::PromotionRunning,
                format!(
                    "Workflow run #{} is running on the integration branch.",
                    run.run_number
                ),
            ),
            WorkflowRunStatus::Completed | WorkflowRunStatus::Unknown => continue,
        };
        for sha in &run.source_shas {
            candidate(
                sha,
                CommitStatus::of(kind, detail_target, Some(label_detail.clone()), None),
                &mut map,
            );
        }
    }

    for effect in signals.failure_effects {
        if effect.subsystem_id != signals.subsystem_id || effect.target_branch != *detail_target {
            continue;
        }
        for sha in &effect.source_shas {
            candidate(
                sha,
                CommitStatus::of(
                    effect.kind,
                    detail_target,
                    Some(format!(
                        "{} (workflow run {})",
                        effect.reason, effect.run_id
                    )),
                    None,
                ),
                &mut map,
            );
        }
    }

    // Open PRs never count as `Integrated`.
    for pr in signals.prs {
        if pr.subsystem_id != signals.subsystem_id || pr.base_branch != *detail_target {
            continue;
        }
        if pr.state != PrState::Open {
            continue;
        }
        let (kind, detail) = match pr.review_state {
            ReviewState::ChangesRequested | ReviewState::Approved => (
                CommitStatusKind::InReview,
                format!(
                    "Integration PR #{} is open on {} and awaiting review.",
                    pr.number, pr.base_branch
                ),
            ),
            _ => (
                CommitStatusKind::PrOpen,
                format!(
                    "Integration PR #{} targets {}. Not merged, so not integrated.",
                    pr.number, pr.base_branch
                ),
            ),
        };
        for sha in &pr.source_shas {
            candidate(
                sha,
                CommitStatus::of(kind, detail_target, Some(detail.clone()), None),
                &mut map,
            );
        }
    }

    let mut ordered: Vec<&IntegrationEvent> = signals
        .events
        .iter()
        .filter(|event| event.subsystem_id == signals.subsystem_id)
        .filter(|event| {
            reachable
                .iter()
                .any(|branch| branch == &event.target_branch)
        })
        .collect();
    ordered.sort_by(|a, b| a.merged_at.cmp(&b.merged_at));

    let mut applied: HashMap<String, IntegrationEvidence> = HashMap::new();
    let mut applied_by_event: HashMap<String, Vec<String>> = HashMap::new();
    let mut reverted: HashMap<String, (IntegrationEvidence, String)> = HashMap::new();

    for event in ordered {
        match event.kind {
            IntegrationEventKind::Integrate => {
                let evidence = evidence_from(event, signals.target_branch);
                for sha in &event.source_shas {
                    applied.insert(sha.clone(), evidence.clone());
                    reverted.remove(sha);
                }
                applied_by_event.insert(event.id.clone(), event.source_shas.clone());
            }
            IntegrationEventKind::Revert => {
                let affected: Vec<String> = event
                    .reverts_event_id
                    .as_ref()
                    .and_then(|id| applied_by_event.get(id).cloned())
                    .unwrap_or_else(|| event.source_shas.clone());

                for sha in affected {
                    if let Some(evidence) = applied.get(&sha).cloned() {
                        reverted.insert(
                            sha,
                            (
                                evidence,
                                format!(
                                    "Reverted by {} (revert PR #{}) onto {}.",
                                    short(&event.main_commit_sha),
                                    event
                                        .pull_request_number
                                        .map(|n| n.to_string())
                                        .unwrap_or_else(|| "-".into()),
                                    event.target_branch
                                ),
                            ),
                        );
                    }
                }
            }
            IntegrationEventKind::Restore => {
                let evidence = evidence_from(event, signals.target_branch);
                for sha in &event.source_shas {
                    applied.insert(sha.clone(), evidence.clone());
                    reverted.remove(sha);
                }
                applied_by_event.insert(event.id.clone(), event.source_shas.clone());
            }
        }
    }

    for (sha, (evidence, detail)) in &reverted {
        let full_detail = format!(
            "{} The original integration commit {} is still in history.",
            detail,
            short(&evidence.main_commit_sha)
        );
        candidate(
            sha,
            CommitStatus::of(
                CommitStatusKind::Reverted,
                detail_target,
                Some(full_detail),
                Some(evidence.clone()),
            ),
            &mut map,
        );
    }

    for (sha, evidence) in &applied {
        if reverted.contains_key(sha) {
            continue;
        }
        let detail = match &evidence.inherited_from_branch {
            Some(branch) => format!(
                "Reachable from {} by inheritance through a main-system branch merge (no separate promotion request on {}).",
                branch, detail_target
            ),
            None => format!(
                "Integration commit {} recorded this source SHA on {}.",
                short(&evidence.main_commit_sha),
                detail_target
            ),
        };
        candidate(
            sha,
            CommitStatus::of(
                CommitStatusKind::Integrated,
                detail_target,
                Some(detail),
                Some(evidence.clone()),
            ),
            &mut map,
        );
    }

    for sha in signals.eligible_shas {
        candidate(
            sha,
            CommitStatus::of(
                CommitStatusKind::Eligible,
                detail_target,
                Some(format!(
                    "Reachable from the protected source branch. No successful integration found for {}.",
                    detail_target
                )),
                None,
            ),
            &mut map,
        );
    }

    for sha in signals.eligible_shas {
        map.entry(sha.clone()).or_insert_with(|| {
            CommitStatus::unknown(
                detail_target,
                format!(
                    "Could not verify integration state for this SHA on {}.",
                    detail_target
                ),
            )
        });
    }

    map
}

fn evidence_from(event: &IntegrationEvent, queried_branch: &str) -> IntegrationEvidence {
    IntegrationEvidence {
        main_commit_sha: event.main_commit_sha.clone(),
        merged_at: event.merged_at.clone(),
        message: event.message.clone(),
        pull_request_number: event.pull_request_number,
        target_branch: event.target_branch.clone(),
        inherited_from_branch: if event.target_branch == queried_branch {
            None
        } else {
            Some(event.target_branch.clone())
        },
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

pub fn status_for(signals: &StatusSignals<'_>, sha: &str) -> CommitStatus {
    build_status_map(signals)
        .remove(sha)
        .unwrap_or_else(|| CommitStatus::unknown(signals.target_branch, "Unknown source commit."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::FixtureWorld;

    fn world() -> FixtureWorld {
        FixtureWorld::build()
    }

    fn signals<'a>(
        world: &'a FixtureWorld,
        subsystem: &'a str,
        branch: &'a str,
        shas: &'a [String],
        ancestry: &'a [(&'static str, Vec<&'static str>)],
    ) -> StatusSignals<'a> {
        StatusSignals {
            subsystem_id: subsystem,
            target_branch: branch,
            eligible_shas: shas,
            events: &world.events,
            prs: &world.prs,
            runs: &world.workflow_runs,
            failure_effects: &world.failure_effects,
            ancestry,
        }
    }

    #[test]
    fn branch_reachability_follows_main_merges() {
        let ancestry = vec![("staging", vec![]), ("master", vec!["staging"])];
        let reachable = branches_reachable_from("master", &ancestry);
        assert!(reachable.contains(&"master".to_string()));
        assert!(reachable.contains(&"staging".to_string()));
        assert!(!reachable.contains(&"development".to_string()));
    }

    #[test]
    fn status_is_branch_scoped_and_truthful() {
        let world = world();
        let shas: Vec<String> = world
            .commits_of("hr-management")
            .iter()
            .map(|c| c.sha.clone())
            .collect();
        let ancestry = world.ancestry();
        let staging = signals(&world, "hr-management", "staging", &shas, &ancestry);
        let development = signals(&world, "hr-management", "development", &shas, &ancestry);

        let map_staging = build_status_map(&staging);
        let map_dev = build_status_map(&development);

        // Merged promotion with provenance is integrated on staging ...
        let fix = "f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4";
        assert_eq!(map_staging[fix].kind, CommitStatusKind::Integrated);
        // ... and eligible on development, which never received it.
        assert_eq!(map_dev[fix].kind, CommitStatusKind::Eligible);

        // An open PR is never reported as integrated.
        let profile = "c71e392b48ac7f2e1d90b3a5c6e418d7f2a09b31";
        assert_eq!(map_staging[profile].kind, CommitStatusKind::PrOpen);

        // A failed patch application blocks the SHA.
        let cleanup = "5f0a2bc3d4e5f6071829304a5b6c7d8e9f001122";
        assert_eq!(map_staging[cleanup].kind, CommitStatusKind::Blocked);

        // A revert event wins over the original integration record.
        let stub = "b4e8a11f0c9d8e7f6a5b4c3d2e1f001122334455";
        assert_eq!(map_staging[stub].kind, CommitStatusKind::Reverted);

        // A running workflow shows as promotion running, not integrated.
        let filter = "3ac9d7e1b2c3d4e5f60718293a4b5c6d7e8f9012";
        assert_eq!(map_staging[filter].kind, CommitStatusKind::PromotionRunning);
    }

    #[test]
    fn master_inherits_records_through_branch_merges() {
        let world = world();
        let shas: Vec<String> = world
            .commits_of("hr-management")
            .iter()
            .map(|c| c.sha.clone())
            .collect();
        let ancestry = world.ancestry();
        let master = signals(&world, "hr-management", "master", &shas, &ancestry);
        let map = build_status_map(&master);

        let fix = "f12b301d7e5a4c9382b16f0d3c7e8459a1b2c3d4";
        assert_eq!(map[fix].kind, CommitStatusKind::Integrated);
        let evidence = map[fix].evidence.clone().expect("integration evidence");
        assert_eq!(evidence.inherited_from_branch.as_deref(), Some("staging"));
    }
}

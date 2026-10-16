//! Integration history from Git ancestry (spec §8): no ledger.
//! Status comes from reachable trailers + open PRs; reverts restore correctly.
//!
//! Posture notes (second addendum C — describing what the code does):
//! `compute_branch_status` returns `Eligible` when no trailer record exists
//! for a SHA. That behaviour is kept: the broader flip to `Unknown` would
//! break the normal squash flow, and `collect_events` already cannot emit
//! `Integrated` without both `Subsystem-ID` and `Source-Commit` trailers, so
//! a trailerless false-`Integrated` is unproducible. What the code DOES do:
//! a claimed `Integrated` whose commit lacks those trailers is reported as
//! `Unknown / needs verification` (see `branch_result_to_status` in
//! `live_read`), never `Eligible`.
//! Reverts may be invisible hand-reverts → divergence is decided by
//! blob/content comparison in the engine (`TARGET_DIVERGED`); message parsing
//! here is a hint, never a verdict. Dates determine display order
//! (deterministic timestamp-then-SHA sort in `build_plan`), never correctness.

use std::collections::HashMap;
use std::path::Path;

use super::git;
use super::github_ops::PullRequestInfo;
use super::provenance::parse_trailers;

const FIELD: char = '\u{1f}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    Integrate,
    Revert,
    Restore,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub kind: EventType,
    pub commit: String,
    pub committed_at: i64,
    pub committed_iso: String,
    pub subject: String,
    pub subsystem: Option<String>,
    pub source_repo: Option<String>,
    pub source_commits: Vec<String>,
    pub mode: Option<String>,
    pub target: Option<String>,
    pub reverts_integration: Option<String>,
    pub reverts_subject: Option<String>,
    pub restores_integration: Option<String>,
    pub ordinary: bool,
}

/// Ordinary GitHub revert: subject `Revert "<inner>"` + `This reverts commit <sha>`.
pub fn parse_ordinary_github_revert(
    subject: &str,
    message: &str,
) -> Option<(Option<String>, Option<String>)> {
    if !subject.starts_with("Revert \"") {
        return None;
    }
    let inner = subject
        .strip_prefix("Revert \"")
        .and_then(|rest| rest.strip_suffix('"'))
        .map(|s| s.to_string())
        .or_else(|| subject.strip_prefix("Revert \"").map(|s| s.to_string()));
    let marker = "This reverts commit ";
    let sha = message.find(marker).and_then(|idx| {
        let rest = &message[idx + marker.len()..];
        let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if hex.len() >= 7 && hex.len() <= 64 {
            Some(hex.to_ascii_lowercase())
        } else {
            None
        }
    });
    Some((sha, inner))
}

pub fn extract_source_commits(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split('\n') {
        let line = raw.trim_start_matches([' ', '\t', '>', '*', '-']);
        let Some(rest) = line.strip_prefix("Source-Commit:") else {
            continue;
        };
        let value = rest.trim();
        let hex_ok = (value.len() == 40 || value.len() == 64)
            && value.chars().all(|c| c.is_ascii_hexdigit());
        if hex_ok {
            out.push(value.to_ascii_lowercase());
        }
    }
    out
}

pub fn collect_events(repo_dir: &Path, rev: &str) -> Vec<Event> {
    let mut events = Vec::new();
    for record in git::log_records(repo_dir, rev, 0) {
        let parsed = parse_trailers(&record.message);
        let subsystem = parsed.first("Subsystem-ID").map(|s| s.to_string());
        // `Source-Commit` values arrive validated (40/64 hex, lowercased) and
        // phantom-free from `parse_trailers`; only the trailing block counts.
        let source_commits: Vec<String> = parsed.get("Source-Commit").to_vec();
        let base = |kind: EventType| Event {
            kind,
            commit: record.sha.clone(),
            committed_at: record.committed_at,
            committed_iso: record.committed_iso.clone(),
            subject: record.subject.clone(),
            subsystem: subsystem.clone(),
            source_repo: parsed.first("Source-Repo").map(|s| s.to_string()),
            source_commits: source_commits.clone(),
            mode: parsed.first("Promotion-Mode").map(|s| s.to_string()),
            target: parsed.first("Promotion-Target").map(|s| s.to_string()),
            reverts_integration: None,
            reverts_subject: None,
            restores_integration: None,
            ordinary: false,
        };

        if let Some(reverts) = parsed.first("Reverts-Integration") {
            let mut event = base(EventType::Revert);
            event.reverts_integration = Some(reverts.to_string());
            events.push(event);
        } else if let Some(restores) = parsed.first("Restores-Integration") {
            let mut event = base(EventType::Restore);
            event.restores_integration = Some(restores.to_string());
            events.push(event);
        } else if let Some((sha, subject)) =
            parse_ordinary_github_revert(&record.subject, &record.message)
        {
            if sha.is_some() || subject.is_some() {
                let mut event = base(EventType::Revert);
                event.reverts_integration = sha;
                event.reverts_subject = subject;
                event.ordinary = true;
                events.push(event);
            }
        } else if subsystem.is_some() && !source_commits.is_empty() {
            events.push(base(EventType::Integrate));
        }
    }
    events
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayStatus {
    Integrated,
    Reverted,
}

#[derive(Debug, Clone)]
pub struct StatusRecord {
    pub subsystem: String,
    pub sha: String,
    pub status: ReplayStatus,
    pub integration_commit: String,
    pub promotion_target: Option<String>,
    pub at: i64,
    pub reverted_by: Option<String>,
}

#[derive(Debug, Clone)]
struct IntegrationInfo {
    subsystem: String,
    source_commits: Vec<String>,
    target: Option<String>,
    subject: String,
}

#[derive(Debug, Clone, Default)]
pub struct ReplayResult {
    pub statuses: HashMap<String, StatusRecord>,
    pub history: Vec<Event>,
}

fn status_key(subsystem: &str, sha: &str) -> String {
    format!("{subsystem}{FIELD}{sha}")
}

pub fn replay_events(events: &[Event]) -> ReplayResult {
    let mut statuses: HashMap<String, StatusRecord> = HashMap::new();
    let mut integration_index: HashMap<String, IntegrationInfo> = HashMap::new();
    // Chronological (oldest-first) commit order: `HashMap` iteration is
    // nondeterministic, so revert resolution scans this instead.
    let mut integration_order: Vec<String> = Vec::new();
    let mut history: Vec<Event> = Vec::new();

    for event in events {
        match event.kind {
            EventType::Integrate => {
                integration_order.push(event.commit.clone());
                integration_index.insert(
                    event.commit.clone(),
                    IntegrationInfo {
                        subsystem: event.subsystem.clone().unwrap_or_default(),
                        source_commits: event.source_commits.clone(),
                        target: event.target.clone(),
                        subject: event.subject.clone(),
                    },
                );
                let subsystem = event.subsystem.clone().unwrap_or_default();
                for sha in &event.source_commits {
                    statuses.insert(
                        status_key(&subsystem, sha),
                        StatusRecord {
                            subsystem: subsystem.clone(),
                            sha: sha.clone(),
                            status: ReplayStatus::Integrated,
                            integration_commit: event.commit.clone(),
                            promotion_target: event.target.clone(),
                            at: event.committed_at,
                            reverted_by: None,
                        },
                    );
                }
                history.push(event.clone());
            }
            EventType::Revert => {
                let resolved = event.reverts_integration.clone();
                let mut target = resolved.as_ref().and_then(|sha| integration_index.get(sha));
                let mut resolved_commit = resolved.clone();
                if target.is_none() {
                    // Abbreviated SHA: resolve only on a unique prefix match,
                    // scanning integrations oldest-first so the winner is
                    // deterministic. Ambiguous prefixes fall through to the
                    // subject match instead of guessing.
                    if let Some(prefix) = resolved.clone() {
                        let mut candidates: Vec<String> = Vec::new();
                        for commit in &integration_order {
                            if commit.starts_with(&prefix) {
                                candidates.push(commit.clone());
                            }
                        }
                        if candidates.len() == 1 {
                            let commit = candidates.into_iter().next().unwrap_or_default();
                            target = integration_index.get(&commit);
                            resolved_commit = Some(commit);
                        }
                    }
                }
                if target.is_none() {
                    // Most recent subject match wins: scan newest-first.
                    if let Some(subject) = &event.reverts_subject {
                        for commit in integration_order.iter().rev() {
                            let matches = integration_index
                                .get(commit)
                                .map(|info| &info.subject == subject)
                                .unwrap_or(false);
                            if matches {
                                target = integration_index.get(commit);
                                resolved_commit = Some(commit.clone());
                                break;
                            }
                        }
                    }
                }
                let affected: Vec<String> = match target {
                    Some(info) => info.source_commits.clone(),
                    None => event.source_commits.clone(),
                };
                let subsystem = event
                    .subsystem
                    .clone()
                    .or_else(|| target.map(|i| i.subsystem.clone()));
                if target.is_none() && (subsystem.is_none() || affected.is_empty()) {
                    continue;
                }
                if let Some(subsystem) = subsystem {
                    for sha in &affected {
                        let key = status_key(&subsystem, sha);
                        let prev = statuses.get(&key).cloned();
                        // Only a currently-Integrated (subsystem, SHA) may
                        // transition to Reverted. Anything else (notably an
                        // Eligible SHA that was never integrated) is skipped
                        // rather than fabricating a Reverted record.
                        let Some(prev) = prev else {
                            continue;
                        };
                        if prev.status != ReplayStatus::Integrated {
                            continue;
                        }
                        statuses.insert(
                            key,
                            StatusRecord {
                                subsystem: subsystem.clone(),
                                sha: sha.clone(),
                                status: ReplayStatus::Reverted,
                                integration_commit: prev.integration_commit.clone(),
                                promotion_target: prev.promotion_target.clone(),
                                at: event.committed_at,
                                reverted_by: Some(event.commit.clone()),
                            },
                        );
                    }
                }
                let mut recorded = event.clone();
                recorded.reverts_integration = resolved_commit;
                history.push(recorded);
            }
            EventType::Restore => {
                let target = event
                    .restores_integration
                    .as_ref()
                    .and_then(|sha| integration_index.get(sha));
                let affected: Vec<String> = match target {
                    Some(info) => info.source_commits.clone(),
                    None => event.source_commits.clone(),
                };
                let subsystem = event
                    .subsystem
                    .clone()
                    .or_else(|| target.map(|i| i.subsystem.clone()));
                if let Some(subsystem) = subsystem {
                    for sha in &affected {
                        statuses.insert(
                            status_key(&subsystem, sha),
                            StatusRecord {
                                subsystem: subsystem.clone(),
                                sha: sha.clone(),
                                status: ReplayStatus::Integrated,
                                integration_commit: event.commit.clone(),
                                promotion_target: target.and_then(|i| i.target.clone()),
                                at: event.committed_at,
                                reverted_by: None,
                            },
                        );
                    }
                }
                history.push(event.clone());
            }
        }
    }

    ReplayResult { statuses, history }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchStatusKind {
    Integrated,
    Reverted,
    Eligible,
    Unknown,
}

impl BranchStatusKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BranchStatusKind::Integrated => "Integrated",
            BranchStatusKind::Reverted => "Reverted",
            BranchStatusKind::Eligible => "Eligible",
            BranchStatusKind::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BranchStatusResult {
    pub sha: String,
    pub status: BranchStatusKind,
    pub integration_commit: Option<String>,
    pub promotion_target: Option<String>,
    pub reverted_by: Option<String>,
}

fn resolve_ref(repo_dir: &Path, branch: &str) -> Option<String> {
    [
        format!("refs/heads/{branch}"),
        format!("refs/remotes/origin/{branch}"),
        branch.to_string(),
    ]
    .into_iter()
    .find(|candidate| git::rev_parse(repo_dir, candidate).is_some())
}

pub fn compute_branch_status(
    repo_dir: &Path,
    branch: &str,
    subsystem: &str,
    shas: &[String],
) -> Vec<BranchStatusResult> {
    let Some(reference) = resolve_ref(repo_dir, branch) else {
        return shas
            .iter()
            .map(|sha| BranchStatusResult {
                sha: sha.clone(),
                status: BranchStatusKind::Unknown,
                integration_commit: None,
                promotion_target: None,
                reverted_by: None,
            })
            .collect();
    };
    let events = collect_events(repo_dir, &reference);
    let replayed = replay_events(&events);
    shas.iter()
        .map(|sha| {
            let record = replayed.statuses.get(&status_key(subsystem, sha));
            match record {
                None => BranchStatusResult {
                    sha: sha.clone(),
                    status: BranchStatusKind::Eligible,
                    integration_commit: None,
                    promotion_target: None,
                    reverted_by: None,
                },
                Some(rec) => BranchStatusResult {
                    sha: sha.clone(),
                    status: match rec.status {
                        ReplayStatus::Integrated => BranchStatusKind::Integrated,
                        ReplayStatus::Reverted => BranchStatusKind::Reverted,
                    },
                    integration_commit: Some(rec.integration_commit.clone()),
                    promotion_target: rec.promotion_target.clone(),
                    reverted_by: rec.reverted_by.clone(),
                },
            }
        })
        .collect()
}

/// Per-SHA match (§8.3), so `[B, C]` blocks `[C]` and vice versa.
pub fn pr_references_requested_shas(
    pr: &PullRequestInfo,
    subsystem: &str,
    shas: &[String],
) -> bool {
    let body_shas: std::collections::HashSet<String> =
        extract_source_commits(&pr.body).into_iter().collect();
    if shas.iter().any(|s| body_shas.contains(s)) {
        return true;
    }
    let head = pr.head.clone();
    shas.iter().any(|s| {
        let needle = format!("{subsystem}-{}", s.chars().take(12).collect::<String>());
        head.contains(&needle)
    })
}

#[derive(Debug, Clone, Default)]
pub struct BranchDuplicates {
    pub active_shas: Vec<String>,
    pub reverted_shas: Vec<String>,
    pub open_prs: Vec<PullRequestInfo>,
    pub pr_lookup_error: Option<String>,
}

/// Duplicate / pending guard for a promotion request (§8.3).
pub fn detect_branch_duplicates(
    repo_dir: &Path,
    branch: &str,
    subsystem: &str,
    shas: &[String],
    repo_slug: &str,
    github: &dyn super::github_ops::GitHubOps,
) -> BranchDuplicates {
    let status = compute_branch_status(repo_dir, branch, subsystem, shas);
    let mut result = BranchDuplicates {
        active_shas: status
            .iter()
            .filter(|r| r.status == BranchStatusKind::Integrated)
            .map(|r| r.sha.clone())
            .collect(),
        reverted_shas: status
            .iter()
            .filter(|r| r.status == BranchStatusKind::Reverted)
            .map(|r| r.sha.clone())
            .collect(),
        ..Default::default()
    };
    match github.list_open_pull_requests(repo_slug, branch) {
        Ok(prs) => {
            result.open_prs = prs
                .into_iter()
                .filter(|pr| pr_references_requested_shas(pr, subsystem, shas))
                .collect();
        }
        Err(err) => result.pr_lookup_error = Some(err),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_revert_recognised() {
        let subject = "Revert \"integrate(hr-management): employee export\"";
        let message = format!("{subject}\n\nThis reverts commit {}.\n", "a".repeat(40));
        let (sha, subj) = parse_ordinary_github_revert(subject, &message).unwrap();
        assert_eq!(sha.unwrap(), "a".repeat(40));
        assert_eq!(subj.unwrap(), "integrate(hr-management): employee export");
    }

    #[test]
    fn extract_source_commits_from_pr_body() {
        let body = format!(
            "Some PR text\n- `{}`\n\nSource-Commit: {}\nSource-Commit: {}\n",
            "a".repeat(40),
            "b".repeat(40),
            "c".repeat(40)
        );
        assert_eq!(
            extract_source_commits(&body),
            vec!["b".repeat(40), "c".repeat(40)]
        );
    }

    fn integrate_event(commit: &str, subject: &str, sha: &str, at: i64) -> Event {
        Event {
            kind: EventType::Integrate,
            commit: commit.to_string(),
            committed_at: at,
            committed_iso: String::new(),
            subject: subject.to_string(),
            subsystem: Some("hr-management".to_string()),
            source_repo: None,
            source_commits: vec![sha.to_string()],
            mode: None,
            target: Some("staging".to_string()),
            reverts_integration: None,
            reverts_subject: None,
            restores_integration: None,
            ordinary: false,
        }
    }

    fn revert_event(
        commit: &str,
        at: i64,
        reverts: Option<String>,
        subject: Option<String>,
        shas: Vec<String>,
    ) -> Event {
        Event {
            kind: EventType::Revert,
            commit: commit.to_string(),
            committed_at: at,
            committed_iso: String::new(),
            subject: "revert".to_string(),
            subsystem: Some("hr-management".to_string()),
            source_repo: None,
            source_commits: shas,
            mode: None,
            target: None,
            reverts_integration: reverts,
            reverts_subject: subject,
            restores_integration: None,
            ordinary: false,
        }
    }

    #[test]
    fn revert_never_fabricates_reverted_for_eligible_sha() {
        let integrated_sha = "a".repeat(40);
        let eligible_sha = "b".repeat(40);
        let events = vec![
            integrate_event(&"c".repeat(40), "first", &integrated_sha, 1),
            // Revert names an unknown integration but carries an Eligible SHA.
            revert_event(
                &"d".repeat(40),
                2,
                Some("e".repeat(40)),
                None,
                vec![eligible_sha.clone()],
            ),
        ];
        let replayed = replay_events(&events);
        assert!(
            !replayed
                .statuses
                .contains_key(&status_key("hr-management", &eligible_sha)),
            "an Eligible SHA must never gain a Reverted record"
        );
        // The genuinely integrated SHA is untouched by the unrelated revert.
        let record = &replayed.statuses[&status_key("hr-management", &integrated_sha)];
        assert_eq!(record.status, ReplayStatus::Integrated);
        assert_eq!(record.reverted_by, None);
    }

    #[test]
    fn revert_resolution_prefers_exact_then_unique_prefix_then_newest_subject() {
        let old_commit = "1".repeat(40);
        let new_commit = "2".repeat(40);
        let old_sha = "a".repeat(40);
        let new_sha = "b".repeat(40);
        // Same subject on both integrations: subject fallback must pick newest.
        let events = vec![
            integrate_event(&old_commit, "same subject", &old_sha, 1),
            integrate_event(&new_commit, "same subject", &new_sha, 2),
            revert_event(
                &"3".repeat(40),
                3,
                None,
                Some("same subject".to_string()),
                Vec::new(),
            ),
        ];
        let replayed = replay_events(&events);
        assert_eq!(
            replayed.statuses[&status_key("hr-management", &new_sha)].status,
            ReplayStatus::Reverted,
            "subject fallback resolves to the most recent integration"
        );
        assert_eq!(
            replayed.statuses[&status_key("hr-management", &old_sha)].status,
            ReplayStatus::Integrated
        );

        // Exact SHA match wins even when the subject would match elsewhere.
        let events = vec![
            integrate_event(&old_commit, "same subject", &old_sha, 1),
            integrate_event(&new_commit, "same subject", &new_sha, 2),
            revert_event(
                &"4".repeat(40),
                3,
                Some(old_commit.clone()),
                Some("same subject".to_string()),
                Vec::new(),
            ),
        ];
        let replayed = replay_events(&events);
        assert_eq!(
            replayed.statuses[&status_key("hr-management", &old_sha)].status,
            ReplayStatus::Reverted
        );
        assert_eq!(
            replayed.statuses[&status_key("hr-management", &new_sha)].status,
            ReplayStatus::Integrated
        );

        // Unique abbreviated prefix resolves; ambiguous prefixes do not guess.
        let events = vec![
            integrate_event(&old_commit, "same subject", &old_sha, 1),
            integrate_event(&new_commit, "same subject", &new_sha, 2),
            revert_event(&"5".repeat(40), 3, Some("1".repeat(7)), None, Vec::new()),
        ];
        let replayed = replay_events(&events);
        assert_eq!(
            replayed.statuses[&status_key("hr-management", &old_sha)].status,
            ReplayStatus::Reverted,
            "unique prefix resolves"
        );
    }
}

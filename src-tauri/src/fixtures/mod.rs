//! Fixture world (Phase 2): fictitious data, always labelled `Fixture`.
//! Nothing here is authoritative; real reads come from the registry branch,
//! source ancestry, runs, and reachable trailers.

pub mod data;

use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::github::models::*;

pub use data::{
    DEFAULT_TARGET_BRANCH, MAIN_REPOSITORY, NAMESPACE, REGISTRY_BRANCH, STATUS_PENDING_NOTE,
    WORKFLOW_FILE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationEventKind {
    Integrate,
    Revert,
    Restore,
}

#[derive(Debug, Clone)]
pub struct IntegrationEvent {
    pub id: String,
    pub kind: IntegrationEventKind,
    pub subsystem_id: String,
    pub source_shas: Vec<String>,
    pub target_branch: String,
    pub main_commit_sha: String,
    pub merged_at: String,
    pub message: String,
    pub pull_request_number: Option<u64>,
    pub actor: String,
    pub reverts_event_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FailureEffect {
    pub subsystem_id: String,
    pub source_shas: Vec<String>,
    pub target_branch: String,
    pub kind: CommitStatusKind,
    pub reason: String,
}

pub struct FixtureWorld {
    pub built_at: DateTime<Utc>,
    pub subsystems: Vec<SubsystemSummary>,
    pub branches: Vec<TargetBranch>,
    pub policy: TargetBranchPolicy,
    /// `status` is recomputed per target.
    pub commits: HashMap<String, Vec<CommitSummary>>,
    pub details: HashMap<String, CommitDetails>,
    pub diffs: HashMap<String, CommitDiff>,
    pub events: Vec<IntegrationEvent>,
    pub failure_effects: Vec<FailureEffect>,
    pub prs: Vec<IntegrationPr>,
    pub history: Vec<IntegrationHistoryEntry>,
    pub next_pr_number: u64,
}

pub fn detail_key(subsystem_id: &str, sha: &str) -> String {
    format!("{subsystem_id}::{sha}")
}

pub fn diff_key(subsystem_id: &str, sha: &str, file_path: &str) -> String {
    format!("{subsystem_id}::{sha}::{file_path}")
}

impl FixtureWorld {
    pub fn build() -> Self {
        let now = Utc::now();
        let subsystems = data::subsystems(now);

        let mut commits: HashMap<String, Vec<CommitSummary>> = HashMap::new();
        let mut details: HashMap<String, CommitDetails> = HashMap::new();
        for subsystem in &subsystems {
            commits.insert(
                subsystem.id.clone(),
                data::commit_summaries(&subsystem.id, now),
            );
            for detail in data::commit_details(&subsystem.id, &subsystem.repository, now) {
                details.insert(detail_key(&subsystem.id, &detail.sha), detail);
            }
        }

        let mut diffs: HashMap<String, CommitDiff> = HashMap::new();
        for diff in data::hand_written_diffs() {
            diffs.insert(
                diff_key(&diff.subsystem_id, &diff.sha, &diff.file_path),
                diff,
            );
        }
        let large = data::large_generated_diff();
        diffs.insert(
            diff_key(&large.subsystem_id, &large.sha, &large.file_path),
            large,
        );

        Self {
            built_at: now,
            subsystems,
            branches: data::branches(),
            policy: data::target_policy(),
            commits,
            details,
            diffs,
            events: data::events(now),
            failure_effects: data::failure_effects(),
            prs: data::prs(now),
            history: data::history(now),
            next_pr_number: 414,
        }
    }

    pub fn subsystem(&self, subsystem_id: &str) -> Option<&SubsystemSummary> {
        self.subsystems.iter().find(|s| s.id == subsystem_id)
    }

    pub fn commits_of(&self, subsystem_id: &str) -> &[CommitSummary] {
        self.commits
            .get(subsystem_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn find_commit(&self, subsystem_id: &str, sha: &str) -> Option<&CommitSummary> {
        self.commits_of(subsystem_id)
            .iter()
            .find(|c| c.sha == sha || c.short_sha == sha)
    }

    pub fn details_for(&self, subsystem_id: &str, sha: &str) -> Option<&CommitDetails> {
        if let Some(detail) = self.details.get(&detail_key(subsystem_id, sha)) {
            return Some(detail);
        }
        self.commits_of(subsystem_id)
            .iter()
            .find(|c| c.short_sha == sha)
            .and_then(|c| self.details.get(&detail_key(subsystem_id, &c.sha)))
    }

    pub fn ancestry(&self) -> Vec<(&'static str, Vec<&'static str>)> {
        data::branch_ancestry()
    }

    pub fn commit_diff(
        &self,
        subsystem_id: &str,
        sha: &str,
        file_path: Option<&str>,
    ) -> Option<CommitDiff> {
        let (_resolved_sha, detail) = self.resolved_detail(subsystem_id, sha)?;
        let file_path = file_path.map(str::to_string).or_else(|| {
            detail
                .files
                .iter()
                .find(|f| f.additions + f.deletions > 0)
                .or_else(|| detail.files.first())
                .map(|f| f.path.clone())
        })?;
        let file = detail.files.iter().find(|f| f.path == file_path)?;
        let file = file.clone();

        if let Some(diff) = self
            .diffs
            .get(&diff_key(subsystem_id, &detail.sha, &file.path))
        {
            return Some(diff.clone());
        }

        Some(synthesize_diff(subsystem_id, detail, &file))
    }

    fn resolved_detail(&self, subsystem_id: &str, sha: &str) -> Option<(String, &CommitDetails)> {
        if let Some(detail) = self.details.get(&detail_key(subsystem_id, sha)) {
            return Some((detail.sha.clone(), detail));
        }
        let commit = self
            .commits_of(subsystem_id)
            .iter()
            .find(|c| c.short_sha == sha || c.sha == sha)?;
        let detail = self.details.get(&detail_key(subsystem_id, &commit.sha))?;
        Some((detail.sha.clone(), detail))
    }
}

pub fn relative_time(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = now.signed_duration_since(then).num_seconds().max(0);
    let count = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match secs {
        0..=59 => "just now".to_string(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => count(secs / 3600, "hour"),
        86_400..=604_799 => count(secs / 86_400, "day"),
        604_800..=2_591_999 => count(secs / 604_800, "week"),
        _ => count(secs / 2_592_000, "month"),
    }
}

pub fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn language_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "json" => "json",
        "css" => "css",
        "md" => "markdown",
        "yml" | "yaml" => "yaml",
        "sql" => "sql",
        _ => "text",
    }
}

fn synthesize_diff(subsystem_id: &str, detail: &CommitDetails, file: &ChangedFile) -> CommitDiff {
    const MAX_LINES: u32 = 120;
    let mut lines: Vec<DiffLine> = Vec::new();
    let mut old_line = 1_u32;
    let mut new_line = 1_u32;

    let stem = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .split('.')
        .next()
        .unwrap_or("module")
        .to_string();

    match file.change {
        FileChangeKind::Added => {
            let total = file.additions.min(MAX_LINES);
            for i in 0..total {
                lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_line: None,
                    new_line: Some(new_line),
                    text: format!("export const {stem}Line{i} = {i};"),
                });
                new_line += 1;
            }
        }
        FileChangeKind::Deleted => {
            let total = file.deletions.min(MAX_LINES);
            for i in 0..total {
                lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    old_line: Some(old_line),
                    new_line: None,
                    text: format!("export const {stem}Line{i} = {i};"),
                });
                old_line += 1;
            }
        }
        FileChangeKind::Modified | FileChangeKind::Renamed => {
            let pairs = file.additions.min(file.deletions).min(MAX_LINES / 2);
            for i in 0..pairs {
                lines.push(DiffLine {
                    kind: DiffLineKind::Removed,
                    old_line: Some(old_line),
                    new_line: None,
                    text: format!("const {stem}Value{i} = computeValue({i});"),
                });
                old_line += 1;
                lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_line: None,
                    new_line: Some(new_line),
                    text: format!("const {stem}Value{i} = computeValue({i}, options);"),
                });
                new_line += 1;
            }
            let extra = (file.additions.saturating_sub(pairs)).min(MAX_LINES);
            for i in 0..extra {
                lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    old_line: None,
                    new_line: Some(new_line),
                    text: format!("// additional change {i} in {stem}"),
                });
                new_line += 1;
            }
        }
    }

    let additions = lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Added)
        .count() as u32;
    let deletions = lines
        .iter()
        .filter(|l| l.kind == DiffLineKind::Removed)
        .count() as u32;
    let total_lines = lines.len() as u32;
    let truncated = total_lines >= MAX_LINES;

    CommitDiff {
        subsystem_id: subsystem_id.to_string(),
        sha: detail.sha.clone(),
        file_path: file.path.clone(),
        language: language_for(&file.path).to_string(),
        hunks: vec![DiffHunk {
            header: format!("@@ -1,{deletions} +1,{additions} @@"),
            lines,
        }],
        total_lines,
        additions,
        deletions,
        binary: file.binary,
        truncated,
        source_url: format!(
            "https://github.com/{NAMESPACE}/commit/{}#diff-{}",
            detail.sha, file.path
        ),
        note: Some(
            "Generated sample diff: the fixture set has no hand-written hunk for this file. Live mode fetches the real patch on demand."
                .to_string(),
        ),
    }
}

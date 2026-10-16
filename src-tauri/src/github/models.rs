//! Wire DTOs: `camelCase` to match `src/api/types.ts` exactly.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSource {
    Fixture,
    Live,
}

impl DataSource {
    pub fn as_str(self) -> &'static str {
        match self {
            DataSource::Fixture => "fixture",
            DataSource::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityState {
    Connected,
    Degraded,
    Unknown,
}

/// Status vocabulary (spec §3.4).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitStatusKind {
    Eligible,
    Selected,
    PromotionQueued,
    PromotionRunning,
    PrOpen,
    InReview,
    Integrated,
    Applied,
    Divergent,
    Superseded,
    Reverted,
    Blocked,
    Failed,
    Unknown,
}

impl CommitStatusKind {
    pub fn label(self) -> &'static str {
        match self {
            CommitStatusKind::Eligible => "Eligible",
            CommitStatusKind::Selected => "Selected",
            CommitStatusKind::PromotionQueued => "Promotion queued",
            CommitStatusKind::PromotionRunning => "Promotion running",
            CommitStatusKind::PrOpen => "PR open",
            CommitStatusKind::InReview => "In review",
            CommitStatusKind::Integrated => "Integrated",
            CommitStatusKind::Applied => "Already applied",
            CommitStatusKind::Divergent => "Divergent",
            CommitStatusKind::Superseded => "Superseded",
            CommitStatusKind::Reverted => "Reverted",
            CommitStatusKind::Blocked => "Blocked",
            CommitStatusKind::Failed => "Failed",
            CommitStatusKind::Unknown => "Unknown",
        }
    }

    pub fn precedence(self) -> u8 {
        match self {
            CommitStatusKind::Failed => 90,
            CommitStatusKind::Blocked => 80,
            CommitStatusKind::PromotionRunning => 70,
            CommitStatusKind::PromotionQueued => 60,
            CommitStatusKind::InReview => 55,
            CommitStatusKind::PrOpen => 50,
            CommitStatusKind::Reverted => 40,
            CommitStatusKind::Integrated => 30,
            CommitStatusKind::Applied => 25,
            CommitStatusKind::Divergent => 20,
            CommitStatusKind::Superseded => 15,
            CommitStatusKind::Selected => 20,
            CommitStatusKind::Eligible => 10,
            CommitStatusKind::Unknown => 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEvidence {
    pub main_commit_sha: String,
    pub merged_at: String,
    pub message: String,
    pub pull_request_number: Option<u64>,
    pub target_branch: String,
    pub inherited_from_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitStatus {
    pub kind: CommitStatusKind,
    pub label: String,
    pub target_branch: String,
    pub detail: Option<String>,
    pub evidence: Option<IntegrationEvidence>,
}

impl CommitStatus {
    pub fn of(
        kind: CommitStatusKind,
        target_branch: &str,
        detail: Option<String>,
        evidence: Option<IntegrationEvidence>,
    ) -> Self {
        Self {
            kind,
            label: kind.label().to_string(),
            target_branch: target_branch.to_string(),
            detail,
            evidence,
        }
    }

    /// No evidence → `Unknown`, never a fabricated `Integrated`.
    pub fn unknown(target_branch: &str, detail: impl Into<String>) -> Self {
        Self::of(
            CommitStatusKind::Unknown,
            target_branch,
            Some(detail.into()),
            None,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub gh_installed: bool,
    pub gh_version: Option<String>,
    pub authenticated: bool,
    pub hostname: String,
    pub account_login: Option<String>,
    pub account_name: Option<String>,
    pub organization: Option<String>,
    pub scopes: Vec<String>,
    pub token_source: Option<String>,
    pub token_available: bool,
    pub git_installed: bool,
    pub git_version: Option<String>,
    pub package_manager: Option<String>,
    pub package_manager_version: Option<String>,
    pub checked_at: String,
    pub message: String,
    pub remedy: Option<String>,
    pub data_source: DataSource,
    pub main_repository: String,
    pub registry_branch: String,
    pub workflow_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemPolicy {
    pub application_files: String,
    pub package_json: String,
    pub lockfiles: String,
    pub shared_configuration: String,
    pub github_workflows: String,
    pub secret_files: String,
}

impl Default for SubsystemPolicy {
    fn default() -> Self {
        Self {
            application_files: "selected-commit-delta".into(),
            package_json: "merge-dependency-delta".into(),
            lockfiles: "regenerate-in-target".into(),
            shared_configuration: "review-explicitly".into(),
            github_workflows: "main-owned".into(),
            secret_files: "never-import".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemSummary {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub source_branch: String,
    pub enabled: bool,
    pub description: String,
    pub connectivity: ConnectivityState,
    pub connectivity_message: Option<String>,
    pub policy: SubsystemPolicy,
    pub path_strategy: String,
    pub dependency_strategy: String,
    pub last_synced_at: String,
    pub registered_at: String,
    pub registration_pr_number: Option<u64>,
    pub registration_pr_url: Option<String>,
    pub allowed_target_branches: Vec<String>,
}

/// Per-subsystem counts for ALL registered subsystems in ONE call (§11).
/// Wire shape is exact (the React agent consumes it):
/// `SubsystemSummaryCounts { subsystemId, indexed, total, tally }`
/// where `tally` keys are the existing status keys (snake_case
/// `CommitStatusKind`: `eligible`, `integrated`, `pr_open`, …).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemSummaryCounts {
    pub subsystem_id: String,
    pub indexed: u32,
    pub total: u32,
    pub tally: std::collections::HashMap<String, u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetBranch {
    pub name: String,
    pub head_sha: String,
    pub protected: bool,
    pub is_default: bool,
    pub allowed: bool,
    pub disallowed_reason: Option<String>,
    pub open_pr_count: u32,
    pub active_promotions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetBranchPolicy {
    pub allowed_target_branches: Vec<String>,
    pub allowed_target_branch_patterns: Vec<String>,
    pub require_pull_request: bool,
    pub allow_direct_push: bool,
    pub workflow_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetBranchList {
    pub target_branch: String,
    pub main_repository: String,
    pub registry_branch: String,
    pub branches: Vec<TargetBranch>,
    pub policy: TargetBranchPolicy,
    pub fetched_at: String,
    pub data_source: DataSource,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemRegistrationRequest {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub source_branch: String,
    pub description: String,
    pub policy: SubsystemPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationFinding {
    pub level: FindingLevel,
    pub code: String,
    pub message: String,
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingLevel {
    Info,
    Warning,
    Error,
}

impl ValidationFinding {
    pub fn info(code: &str, message: impl Into<String>) -> Self {
        Self {
            level: FindingLevel::Info,
            code: code.into(),
            message: message.into(),
            remedy: None,
        }
    }
    pub fn info_with_remedy(
        code: &str,
        message: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Self {
        Self {
            level: FindingLevel::Info,
            code: code.into(),
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }
    pub fn warning(code: &str, message: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            level: FindingLevel::Warning,
            code: code.into(),
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }
    pub fn error(code: &str, message: impl Into<String>, remedy: impl Into<String>) -> Self {
        Self {
            level: FindingLevel::Error,
            code: code.into(),
            message: message.into(),
            remedy: Some(remedy.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationResult {
    pub accepted: bool,
    pub branch: Option<String>,
    pub pull_request_number: Option<u64>,
    pub pull_request_url: Option<String>,
    pub message: String,
    pub findings: Vec<ValidationFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommitSort {
    Newest,
    Oldest,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitQuery {
    pub subsystem_id: String,
    pub target_branch: String,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub status_filter: Option<CommitStatusKind>,
    #[serde(default = "default_sort")]
    pub sort: CommitSort,
    #[serde(default)]
    pub shas: Option<Vec<String>>,
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

fn default_sort() -> CommitSort {
    CommitSort::Newest
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author_name: String,
    pub author_login: Option<String>,
    pub authored_at: String,
    pub committed_at: String,
    pub relative_time: String,
    pub status: CommitStatus,
    pub parents: Vec<String>,
    pub is_merge: bool,
    pub eligible: bool,
    pub files_changed: u32,
    pub additions: u32,
    pub deletions: u32,
    pub tags: Vec<String>,
    pub touches_manifest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitPage {
    pub target_branch: String,
    pub source_branch: String,
    pub subsystem_id: String,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
    pub commits: Vec<CommitSummary>,
    pub fetched_at: String,
    pub data_source: DataSource,
    /// True when the 800-commit target path-date scan hit its limit (§10, E3).
    /// A truncated scan that finds no hit reports `Unknown` (never
    /// `Integrated`); dates determine display order only, never correctness.
    #[serde(default)]
    pub path_dates_truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIntegrationPolicy {
    Apply,
    RegenerateInTarget,
    ReviewRequired,
    NeverImport,
    MainOwned,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub change: FileChangeKind,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub policy: FileIntegrationPolicy,
    pub policy_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DependencySection {
    Dependencies,
    DevDependencies,
    PeerDependencies,
    OptionalDependencies,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyChangeKind {
    Added,
    Updated,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyChange {
    pub name: String,
    pub section: DependencySection,
    pub change: DependencyChangeKind,
    pub from_range: Option<String>,
    pub to_range: Option<String>,
    pub conflict: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestRef {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub base_branch: String,
    pub html_url: String,
    pub repository: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetails {
    pub subsystem_id: String,
    pub target_branch: String,
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub body: String,
    pub author_name: String,
    pub author_login: Option<String>,
    pub authored_at: String,
    pub committed_at: String,
    pub relative_time: String,
    pub status: CommitStatus,
    pub tags: Vec<String>,
    pub files: Vec<ChangedFile>,
    pub files_changed: u32,
    pub additions: u32,
    pub deletions: u32,
    pub dependencies: Vec<DependencyChange>,
    pub related_pull_requests: Vec<PullRequestRef>,
    pub parents: Vec<String>,
    pub is_merge: bool,
    pub source_url: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDiff {
    pub subsystem_id: String,
    pub sha: String,
    pub file_path: String,
    pub language: String,
    pub hunks: Vec<DiffHunk>,
    pub total_lines: u32,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
    pub truncated: bool,
    pub source_url: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationStatus {
    pub subsystem_id: String,
    pub source_sha: String,
    pub target_branch: String,
    pub status: CommitStatus,
    pub checked_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromotionMode {
    Individual,
    Cumulative,
}

impl PromotionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PromotionMode::Individual => "individual",
            PromotionMode::Cumulative => "cumulative",
        }
    }
}

/// Delivery mode for a promotion (wire `delivery`, camelCase field).
/// Omitted on the wire defaults to [`PromotionDelivery::LocalDirect`].
/// `worktree` applies the selected delta into the user's own target-clone
/// working tree on the target branch and stages it: no branch is created,
/// nothing is committed, nothing is pushed, and no PR is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromotionDelivery {
    Local,
    #[default]
    LocalDirect,
    PushPr,
    Worktree,
}

impl PromotionDelivery {
    pub fn as_str(self) -> &'static str {
        match self {
            PromotionDelivery::Local => "local",
            PromotionDelivery::LocalDirect => "local_direct",
            PromotionDelivery::PushPr => "push_pr",
            PromotionDelivery::Worktree => "worktree",
        }
    }
}

/// Conflict policy for a promotion (wire `onConflict`, camelCase field).
/// Omitted on the wire defaults to [`PromotionOnConflict::Fail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromotionOnConflict {
    #[default]
    Fail,
    TakeSubsystem,
}

impl PromotionOnConflict {
    pub fn as_str(self) -> &'static str {
        match self {
            PromotionOnConflict::Fail => "fail",
            PromotionOnConflict::TakeSubsystem => "take_subsystem",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionRequest {
    pub subsystem_id: String,
    pub source_commit_shas: Vec<String>,
    pub mode: PromotionMode,
    #[serde(default)]
    pub base_sha: Option<String>,
    pub target_branch: String,
    #[serde(default)]
    pub confirmation_acknowledged: bool,
    /// `"local" | "local_direct" | "push_pr" | "worktree"`; omitted ⇒ `local_direct`.
    #[serde(default)]
    pub delivery: PromotionDelivery,
    /// `"fail" | "take_subsystem"`; omitted ⇒ `fail`.
    #[serde(default)]
    pub on_conflict: PromotionOnConflict,
    /// The promotion commit's own message. Required: the engine refuses an empty
    /// one, so a multi-commit promotion is never labelled with only the last
    /// source commit's subject.
    #[serde(default)]
    pub message: String,
    /// Wire `allowGaps`: override the non-contiguous gap refusal. Recorded in
    /// the commit trailers as `Promotion-Gaps` + `Promotion-Override: true`
    /// (trailers survive squash-merge; the PR body does not) and mirrored in
    /// the PR body (addendum F). Nothing may depend on reading it back after
    /// a merge.
    #[serde(default)]
    pub allow_gaps: bool,
    /// Wire `excludedPaths`: paths skipped by the plan and reported as
    /// excluded.
    #[serde(default)]
    pub excluded_paths: Vec<String>,
}

/// Dry-run mergeability preflight (§9.2): per-file `clean` / `merged` /
/// `conflict` / `skipped` for the selected deltas against the target tip.
/// Read-only: computing it creates no branch, no commit, and no worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PreflightOutcome {
    Clean,
    Merged,
    Conflict,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightFileResult {
    pub path: String,
    pub outcome: PreflightOutcome,
}

/// In-tool promotion result (§9.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionDispatchResult {
    pub accepted: bool,
    pub pr_url: Option<String>,
    pub pr_number: Option<u64>,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub target_branch: String,
    pub message: String,
    pub requested_at: String,
    pub findings: Vec<ValidationFinding>,
    #[serde(default)]
    pub delivery: PromotionDelivery,
    #[serde(default)]
    pub next_commands: Vec<String>,
}

/// Dry-run mergeability summary for a promotion selection: per-file outcomes
/// plus the actionable `conflicts` path list. Counts always sum to
/// `files.len()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionPreflight {
    pub files: Vec<PreflightFileResult>,
    pub clean: usize,
    pub merged: usize,
    pub conflict: usize,
    pub skipped: usize,
    pub conflicts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Queued,
    InProgress,
    Completed,
    Waiting,
    Requested,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunConclusion {
    Success,
    Failure,
    Cancelled,
    Skipped,
    TimedOut,
    ActionRequired,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: u64,
    pub run_number: u64,
    pub name: String,
    pub display_title: String,
    pub event: String,
    pub status: WorkflowRunStatus,
    pub conclusion: Option<WorkflowRunConclusion>,
    /// Execution ref, never the PR base.
    pub head_branch: String,
    pub target_branch: String,
    pub subsystem_id: String,
    pub source_shas: Vec<String>,
    pub mode: PromotionMode,
    pub actor: String,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub number: u32,
    pub name: String,
    pub status: WorkflowRunStatus,
    pub conclusion: Option<WorkflowRunConclusion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowJob {
    pub id: u64,
    pub name: String,
    pub status: WorkflowRunStatus,
    pub conclusion: Option<WorkflowRunConclusion>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub steps: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowLogLine {
    pub ts: String,
    pub level: LogLevel,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunList {
    pub runs: Vec<WorkflowRun>,
    pub total: u32,
    pub fetched_at: String,
    pub data_source: DataSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunDetails {
    pub run: WorkflowRun,
    pub jobs: Vec<WorkflowJob>,
    pub logs: Vec<WorkflowLogLine>,
    pub logs_truncated: bool,
    pub logs_url: Option<String>,
    pub artifact_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Required,
    Approved,
    ChangesRequested,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationPr {
    pub number: u64,
    pub title: String,
    pub state: PrState,
    pub draft: bool,
    pub review_state: ReviewState,
    pub base_branch: String,
    pub head_branch: String,
    pub subsystem_id: String,
    pub source_shas: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub merged_at: Option<String>,
    pub merged_by: Option<String>,
    pub merge_commit_sha: Option<String>,
    pub author: String,
    pub html_url: String,
    /// False when squash-merge stripped the trailers (spec §8.2).
    pub provenance_ok: bool,
    pub provenance_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationHistoryKind {
    Merged,
    PendingPr,
    Reverted,
    Restored,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationHistoryEntry {
    pub id: String,
    pub kind: IntegrationHistoryKind,
    pub subsystem_id: String,
    pub source_shas: Vec<String>,
    pub target_branch: String,
    pub main_commit_sha: Option<String>,
    pub pull_request_number: Option<u64>,
    pub actor: String,
    pub occurred_at: String,
    pub detail: String,
    pub inherited_from_branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationHistory {
    pub target_branch: String,
    pub subsystem_id: Option<String>,
    pub entries: Vec<IntegrationHistoryEntry>,
    pub total: u32,
    pub fetched_at: String,
    pub data_source: DataSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub entries: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
    pub oldest_entry_at: Option<String>,
    pub rebuildable: bool,
    pub data_source: DataSource,
    pub note: String,
}

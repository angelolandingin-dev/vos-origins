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

/// One changed file's bounded diff inside a [`MainCommitDiff`] (wire,
/// camelCase). Mirrors the render-relevant subset of [`CommitDiff`]: the main
/// commit lives in the MAIN repository, so `subsystemId`/`sha` would name the
/// wrong repository and are deliberately absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainCommitFileDiff {
    pub path: String,
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

/// The whole main commit's per-file diff in ONE call (wire, camelCase). `files`
/// is keyed by `path` so a single `git show` serves every file — never a fetch
/// per file. The audit page renders the same files-column + selected-file diff
/// the commit browser uses; the main commit lives in the MAIN repository, so
/// the subsystem-scoped per-file `get_commit_diff` does not apply.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainCommitDiff {
    pub main_commit_sha: String,
    pub files: Vec<MainCommitFileDiff>,
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
/// Omitted on the wire defaults to [`PromotionOnConflict::Fail`]. Retained
/// for wire compatibility only: promotion is a copy and the subsystem's copy
/// always wins, so a diverged target copy is overwritten (and reported as
/// replaced) under either value.
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
    #[serde(default)]
    pub delivery: PromotionDelivery,
    /// `"fail" | "take_subsystem"`; omitted ⇒ `fail`. Retained for wire
    /// compatibility: the engine always installs the subsystem's copy over a
    /// diverged target copy, regardless of this value.
    #[serde(default)]
    pub on_conflict: PromotionOnConflict,
    /// The promotion commit's own message. Required: the engine refuses an empty
    /// one, so a multi-commit promotion is never labelled with only the last
    /// source commit's subject.
    #[serde(default)]
    pub message: String,
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
    #[serde(default)]
    pub reverts_newer: Option<RevertsNewerWork>,
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
    #[serde(default)]
    pub replaced: Vec<String>,
    #[serde(default)]
    pub deleted: Vec<String>,
    #[serde(default)]
    pub unchanged: Vec<String>,
    #[serde(default)]
    pub flagged: Vec<String>,
    #[serde(default)]
    pub excluded: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionPreflight {
    pub files: Vec<PreflightFileResult>,
    pub clean: usize,
    pub merged: usize,
    pub conflict: usize,
    pub skipped: usize,
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub replaced: Vec<String>,
    #[serde(default)]
    pub excluded: Vec<String>,
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

/// Bucket for one subsystem path against the target branch tip (wire
/// `bucket`, snake_case). Blob identity only: `missing` is absent in main,
/// `behind` is a main blob reachable from the source tip (safe to sync),
/// `main_side_changed` is a main blob from nowhere in subsystem history,
/// `identical` matches on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemDiffBucket {
    Missing,
    Behind,
    MainSideChanged,
    Identical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertsNewerWork {
    pub sha: String,
    pub subject: String,
}

/// One subsystem commit touching a diff path (wire, camelCase):
/// newest-first per path, each carrying its per-branch status from the
/// existing status machinery plus every path it touched (for the exclusions
/// prefill). A main-side changed path lists the same factual touchers — the
/// UI must say plainly that none of them produced main's copy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemDiffCommit {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author_name: String,
    pub committed_at: String,
    pub status: CommitStatus,
    pub touched_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemDiffEntry {
    pub path: String,
    pub bucket: SubsystemDiffBucket,
    pub change: FileChangeKind,
    pub subsystem_blob: Option<String>,
    pub main_blob: Option<String>,
    pub commits: Vec<SubsystemDiffCommit>,
    pub commits_truncated: bool,
    pub reverts_newer: Option<RevertsNewerWork>,
}

/// Subsystem-versus-main diff for one subsystem against one target branch
/// (wire, camelCase). The universe is the subsystem's tracked tree: main-only
/// paths are out of scope and never listed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubsystemDiff {
    pub subsystem_id: String,
    pub source_branch: String,
    pub source_tip: String,
    pub target_branch: String,
    pub target_tip: String,
    pub entries: Vec<SubsystemDiffEntry>,
    pub fetched_at: String,
    pub data_source: DataSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntegrationEventChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEventCommit {
    pub sha: String,
    pub short_sha: String,
    pub subject: String,
    pub author: Option<String>,
    pub authored_at: Option<String>,
    pub unavailable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEventChange {
    pub path: String,
    pub change: IntegrationEventChangeKind,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEventTotals {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEventProvenance {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationEventAudit {
    pub main_commit_sha: String,
    pub main_commit: Option<IntegrationEventCommit>,
    pub kind: IntegrationHistoryKind,
    pub subsystem_id: String,
    pub target_branch: String,
    pub actor: String,
    pub occurred_at: String,
    pub detail: String,
    pub pull_request_number: Option<u64>,
    pub inherited_from_branch: Option<String>,
    pub included_commits: Vec<IntegrationEventCommit>,
    pub changes: Vec<IntegrationEventChange>,
    pub totals: Option<IntegrationEventTotals>,
    pub provenance: Vec<IntegrationEventProvenance>,
    pub reverted_main_commit_sha: Option<String>,
    pub main_commit_unavailable: bool,
    pub data_source: DataSource,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .expect("value must be an object")
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    /// The frontend reads these keys verbatim: a renamed field or a dropped
    /// `rename_all = "camelCase"` renders `undefined` with both gates green.
    /// This pins the exact wire key sets to `src/api/types.ts`.
    #[test]
    fn audit_serialized_keys_match_typescript_contract() {
        let audit = IntegrationEventAudit {
            main_commit_sha: "a".repeat(40),
            main_commit: Some(IntegrationEventCommit {
                sha: "a".repeat(40),
                short_sha: "a".repeat(7),
                subject: "subject".to_string(),
                author: Some("T".to_string()),
                authored_at: Some("2026-01-01T00:00:00Z".to_string()),
                unavailable: false,
            }),
            kind: IntegrationHistoryKind::Merged,
            subsystem_id: "hr-management".to_string(),
            target_branch: "staging".to_string(),
            actor: "main.reviewer".to_string(),
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
            detail: "detail".to_string(),
            pull_request_number: None,
            inherited_from_branch: None,
            included_commits: vec![IntegrationEventCommit {
                sha: "b".repeat(40),
                short_sha: "b".repeat(7),
                subject: "subject".to_string(),
                author: Some("T".to_string()),
                authored_at: Some("2026-01-01T00:00:00Z".to_string()),
                unavailable: false,
            }],
            changes: vec![IntegrationEventChange {
                path: "a.txt".to_string(),
                change: IntegrationEventChangeKind::Modified,
                additions: 1,
                deletions: 1,
            }],
            totals: Some(IntegrationEventTotals {
                files: 1,
                additions: 1,
                deletions: 1,
            }),
            provenance: vec![IntegrationEventProvenance {
                key: "Subsystem-ID".to_string(),
                value: "hr-management".to_string(),
            }],
            reverted_main_commit_sha: None,
            main_commit_unavailable: false,
            data_source: DataSource::Live,
        };
        let value = serde_json::to_value(&audit).expect("audit serializes");
        assert_eq!(
            sorted_keys(&value),
            vec![
                "actor",
                "changes",
                "dataSource",
                "detail",
                "includedCommits",
                "inheritedFromBranch",
                "kind",
                "mainCommit",
                "mainCommitSha",
                "mainCommitUnavailable",
                "occurredAt",
                "provenance",
                "pullRequestNumber",
                "revertedMainCommitSha",
                "subsystemId",
                "targetBranch",
                "totals",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
            "dropping `rename_all = \"camelCase\"` on IntegrationEventAudit would emit main_commit_sha and silently blank the UI",
        );
        assert_eq!(
            sorted_keys(&value["includedCommits"][0]),
            vec![
                "author",
                "authoredAt",
                "sha",
                "shortSha",
                "subject",
                "unavailable",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
            "dropping `rename_all = \"camelCase\"` on IntegrationEventCommit would emit short_sha and silently blank the UI",
        );
        assert_eq!(
            sorted_keys(&value["mainCommit"]),
            vec![
                "author",
                "authoredAt",
                "sha",
                "shortSha",
                "subject",
                "unavailable",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
            "mainCommit must reuse the IntegrationEventCommit wire shape the audit page already renders",
        );
        assert_eq!(
            sorted_keys(&value["changes"][0]),
            vec!["additions", "change", "deletions", "path",]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&value["totals"]),
            vec!["additions", "deletions", "files",]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&value["provenance"][0]),
            vec!["key", "value",]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
    }

    /// Pins the whole-commit diff wire shape to `src/api/types.ts`. The main
    /// commit lives in the MAIN repository, so this carries no `subsystemId`:
    /// a dropped `rename_all = "camelCase"` here would render every diff field
    /// `undefined` with `cargo test` and `tsc` both green.
    #[test]
    fn main_commit_diff_serialized_keys_match_typescript_contract() {
        let diff = MainCommitDiff {
            main_commit_sha: "a".repeat(40),
            files: vec![MainCommitFileDiff {
                path: "src/modules/mod/a.ts".to_string(),
                language: "TypeScript".to_string(),
                hunks: vec![DiffHunk {
                    header: "@@ -1 +1 @@".to_string(),
                    lines: vec![DiffLine {
                        kind: DiffLineKind::Added,
                        old_line: None,
                        new_line: Some(1),
                        text: "line".to_string(),
                    }],
                }],
                total_lines: 1,
                additions: 1,
                deletions: 0,
                binary: false,
                truncated: false,
                source_url: "https://github.com/ORG/main-system/commit/abc#diff-a".to_string(),
                note: None,
            }],
        };
        let value = serde_json::to_value(&diff).expect("main commit diff serializes");
        assert_eq!(
            sorted_keys(&value),
            vec!["files", "mainCommitSha"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&value["files"][0]),
            vec![
                "additions",
                "binary",
                "deletions",
                "hunks",
                "language",
                "note",
                "path",
                "sourceUrl",
                "totalLines",
                "truncated",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&value["files"][0]["hunks"][0]),
            vec!["header", "lines"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            sorted_keys(&value["files"][0]["hunks"][0]["lines"][0]),
            vec!["kind", "newLine", "oldLine", "text"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            value["files"][0]["hunks"][0]["lines"][0]["kind"].as_str(),
            Some("added"),
            "diff line kinds stay snake_case"
        );
    }
}

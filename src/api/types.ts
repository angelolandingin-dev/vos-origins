/** Wire contract for the Rust command surface (spec §9.2); camelCase matches `rename_all`. */
/** Statuses are never invented in components; fixtures always carry `dataSource: 'fixture'`. */

export type DataSource = 'fixture' | 'live';

export type ConnectivityState = 'connected' | 'degraded' | 'unknown';

/** Truthful status vocabulary (spec section 3.4). Never fabricate `Ready`. */
export type CommitStatusKind =
  | 'eligible'
  | 'selected'
  | 'pr_open'
  | 'in_review'
  | 'integrated'
  | 'applied'
  | 'divergent'
  | 'superseded'
  | 'reverted'
  | 'blocked'
  | 'failed'
  | 'unknown';

export type FileChangeKind = 'added' | 'modified' | 'deleted' | 'renamed';

/** Per-path integration semantics (spec §6.4): apply in place; manifests merge; lockfiles regenerate; main-owned/secret files never import. */
export type FileIntegrationPolicy =
  'apply' | 'regenerate_in_target' | 'review_required' | 'never_import' | 'main_owned';

export type PromotionMode = 'individual' | 'cumulative';

/** Local-first delivery: `local_direct` (default) commits straight onto the local target branch (no push, no PR — pushing stays manual); `worktree` stages the files into your working tree on the checked-out target branch (no commit, no push, no PR); `local` builds a branch + commit in the user's clone; `push_pr` is the explicit opt-in. */
export type PromotionDelivery = 'local' | 'local_direct' | 'push_pr' | 'worktree';

/** Conflict policy (wire `onConflict`). Retained for wire compatibility only: promotion is a copy and the subsystem's copy always wins, so a diverged target copy is overwritten (and reported as replaced) under either value. */
export type PromotionOnConflict = 'fail' | 'take_subsystem';

export interface ValidationFinding {
  level: 'info' | 'warning' | 'error';
  code: string;
  message: string;
  remedy: string | null;
}

export interface AuthStatus {
  ghInstalled: boolean;
  ghVersion: string | null;
  authenticated: boolean;
  hostname: string;
  accountLogin: string | null;
  accountName: string | null;
  organization: string | null;
  scopes: string[];
  tokenSource: string | null;
  /** True when a token is obtainable (GH_TOKEN / GITHUB_TOKEN / `gh auth token`). */
  tokenAvailable: boolean;
  gitInstalled: boolean;
  gitVersion: string | null;
  packageManager: string | null;
  packageManagerVersion: string | null;
  checkedAt: string;
  message: string;
  remedy: string | null;
  dataSource: DataSource;
  mainRepository: string;
  registryBranch: string;
  workflowFile: string;
}

export interface SubsystemPolicy {
  applicationFiles: string;
  packageJson: string;
  lockfiles: string;
  sharedConfiguration: string;
  githubWorkflows: string;
  secretFiles: string;
}

export interface SubsystemSummary {
  id: string;
  name: string;
  repository: string;
  sourceBranch: string;
  enabled: boolean;
  description: string;
  connectivity: ConnectivityState;
  connectivityMessage: string | null;
  policy: SubsystemPolicy;
  pathStrategy: string;
  dependencyStrategy: string;
  lastSyncedAt: string;
  registeredAt: string;
  registrationPrNumber: number | null;
  registrationPrUrl: string | null;
  allowedTargetBranches: string[];
}

export interface TargetBranch {
  name: string;
  headSha: string;
  protected: boolean;
  isDefault: boolean;
  allowed: boolean;
  disallowedReason: string | null;
  openPrCount: number;
  activePromotions: number;
}

export interface TargetBranchPolicy {
  allowedTargetBranches: string[];
  allowedTargetBranchPatterns: string[];
  requirePullRequest: boolean;
  allowDirectPush: boolean;
  workflowFile: string;
}

export interface TargetBranchList {
  targetBranch: string;
  mainRepository: string;
  registryBranch: string;
  branches: TargetBranch[];
  policy: TargetBranchPolicy;
  fetchedAt: string;
  dataSource: DataSource;
}

export interface SubsystemRegistrationRequest {
  id: string;
  name: string;
  repository: string;
  sourceBranch: string;
  description: string;
  policy: SubsystemPolicy;
}

export interface RegistrationResult {
  accepted: boolean;
  branch: string | null;
  pullRequestNumber: number | null;
  pullRequestUrl: string | null;
  message: string;
  findings: ValidationFinding[];
}

export interface IntegrationEvidence {
  mainCommitSha: string;
  mergedAt: string;
  message: string;
  pullRequestNumber: number | null;
  targetBranch: string;
  inheritedFromBranch: string | null;
}

export interface CommitStatus {
  kind: CommitStatusKind;
  label: string;
  targetBranch: string;
  detail: string | null;
  evidence: IntegrationEvidence | null;
}

export interface CommitSummary {
  sha: string;
  shortSha: string;
  subject: string;
  authorName: string;
  authorLogin: string | null;
  authoredAt: string;
  committedAt: string;
  relativeTime: string;
  status: CommitStatus;
  parents: string[];
  isMerge: boolean;
  eligible: boolean;
  filesChanged: number;
  additions: number;
  deletions: number;
  tags: string[];
  touchesManifest: boolean;
}

export interface CommitPage {
  targetBranch: string;
  sourceBranch: string;
  subsystemId: string;
  total: number;
  page: number;
  pageSize: number;
  commits: CommitSummary[];
  fetchedAt: string;
  dataSource: DataSource;
  /**
   * True when the path-date scan hit its limit, so ordering may be incomplete.
   * The backend turns this into an `Unknown` status plus a note; the UI reads the
   * note rather than the flag, but the type must match the wire.
   */
  pathDatesTruncated: boolean;
}

export interface CommitQuery {
  subsystemId: string;
  targetBranch: string;
  page: number;
  pageSize: number;
  search: string;
  /** `null` means "all statuses". */
  statusFilter: CommitStatusKind | null;
  sort: 'newest' | 'oldest';
  shas?: string[];
}

export interface DependencyChange {
  name: string;
  section: 'dependencies' | 'devDependencies' | 'peerDependencies' | 'optionalDependencies';
  change: 'added' | 'updated' | 'removed';
  fromRange: string | null;
  toRange: string | null;
  conflict: boolean;
  note: string | null;
}

export interface ChangedFile {
  path: string;
  previousPath: string | null;
  change: FileChangeKind;
  additions: number;
  deletions: number;
  binary: boolean;
  policy: FileIntegrationPolicy;
  policyNote: string | null;
}

export interface PullRequestRef {
  number: number;
  title: string;
  state: 'open' | 'closed' | 'merged';
  baseBranch: string;
  htmlUrl: string;
  repository: string;
}

export interface CommitDetails {
  subsystemId: string;
  targetBranch: string;
  sha: string;
  shortSha: string;
  subject: string;
  body: string;
  authorName: string;
  authorLogin: string | null;
  authoredAt: string;
  committedAt: string;
  relativeTime: string;
  status: CommitStatus;
  tags: string[];
  files: ChangedFile[];
  filesChanged: number;
  additions: number;
  deletions: number;
  dependencies: DependencyChange[];
  relatedPullRequests: PullRequestRef[];
  parents: string[];
  isMerge: boolean;
  sourceUrl: string;
  note: string | null;
}

export type DiffLineKind = 'context' | 'added' | 'removed';

export interface DiffLine {
  kind: DiffLineKind;
  oldLine: number | null;
  newLine: number | null;
  text: string;
}

export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

export interface CommitDiff {
  subsystemId: string;
  sha: string;
  filePath: string;
  language: string;
  hunks: DiffHunk[];
  totalLines: number;
  additions: number;
  deletions: number;
  binary: boolean;
  truncated: boolean;
  sourceUrl: string;
  note: string | null;
}

export interface IntegrationStatus {
  subsystemId: string;
  sourceSha: string;
  targetBranch: string;
  status: CommitStatus;
  checkedAt: string;
}

export interface PromotionRequest {
  subsystemId: string;
  sourceCommitShas: string[];
  mode: PromotionMode;
  /** Required in cumulative mode: the explicitly verified baseline SHA. */
  baseSha: string | null;
  targetBranch: string;
  /** The user must have seen the delivery-mode confirmation. */
  confirmationAcknowledged: boolean;
  /** Omitted ⇒ `local_direct`. */
  delivery?: PromotionDelivery | null;
  /** Omitted ⇒ `fail`. */
  onConflict?: PromotionOnConflict | null;
  /**
   * The promotion commit's own message. Required: the engine refuses an empty
   * one, so a multi-commit promotion is never labelled with only the last
   * source commit's subject.
   */
  message: string;
  /**
   * Override the non-contiguous-selection refusal: a skipped commit's changes
   * are included in the promotion. Omitted ⇒ false.
   */
  allowGaps?: boolean | null;
  /** Paths to leave out of the promotion (exact paths or directory prefixes). */
  excludedPaths?: string[] | null;
}

/** Per-subsystem counts for all subsystems in one call (Dashboard scale fix). */
export interface SubsystemSummaryCounts {
  subsystemId: string;
  indexed: number;
  total: number;
  tally: Record<string, number>;
}

/** Per-file mergeability from the `preflight_promotion` dry run (mirrors the Rust `PreflightOutcome`). */
export type PreflightOutcome = 'clean' | 'merged' | 'conflict' | 'skipped';

export interface PreflightFileResult {
  path: string;
  outcome: PreflightOutcome;
}

/** Dry-run mergeability summary: per-file outcomes plus `conflicts` (true refusals only — a diverged target copy is overwritten, never a conflict) and `replaced` (diverged paths the promotion will overwrite). Counts sum to `files.length`. */
export interface PromotionPreflight {
  files: PreflightFileResult[];
  clean: number;
  merged: number;
  conflict: number;
  skipped: number;
  conflicts: string[];
  /** Paths overwritten with the subsystem version (a diverged target copy never refuses). */
  replaced?: string[] | null;
  /** Paths removed by the promotion. */
  deleted?: string[] | null;
  /** Paths already identical in the target. */
  unchanged?: string[] | null;
  /** Paths also changed in the main system by something else (warning, not an error). */
  flagged?: string[] | null;
  /** Paths left out via `excludedPaths`. */
  excluded?: string[] | null;
}

/** In-tool promotion result (§9.2): `local_direct` commits onto the local target branch; `worktree` stages into the working tree on the checked-out target branch (no commit, no push, no PR — status unchanged); `local` stops at a branch + commit in the user's clone; `push_pr` pushes the branch + opens a PR. */
export interface PromotionDispatchResult {
  accepted: boolean;
  prUrl: string | null;
  prNumber: number | null;
  branch: string;
  /** The provenance commit that carries the §8.1 trailers. */
  commitSha: string | null;
  targetBranch: string;
  message: string;
  requestedAt: string;
  findings: ValidationFinding[];
  delivery: PromotionDelivery;
  /** Literal next steps for `local_direct` / `local` / `worktree` delivery (empty for `push_pr`). */
  nextCommands: string[];
  /** Paths overwritten with the subsystem version. */
  replaced?: string[] | null;
  /** Paths removed by the promotion. */
  deleted?: string[] | null;
  /** Paths already identical in the target. */
  unchanged?: string[] | null;
  /** Paths also changed in the main system by something else (warning, not an error). */
  flagged?: string[] | null;
  /** Paths left out via `excludedPaths`. */
  excluded?: string[] | null;
}

export interface IntegrationPr {
  number: number;
  title: string;
  state: 'open' | 'closed' | 'merged';
  draft: boolean;
  reviewState: 'required' | 'approved' | 'changes_requested' | 'unknown';
  baseBranch: string;
  headBranch: string;
  subsystemId: string;
  sourceShas: string[];
  createdAt: string;
  updatedAt: string;
  mergedAt: string | null;
  mergedBy: string | null;
  mergeCommitSha: string | null;
  author: string;
  htmlUrl: string;
  provenanceOk: boolean;
  provenanceNote: string | null;
}

export type IntegrationHistoryKind = 'merged' | 'pending_pr' | 'reverted' | 'restored' | 'failed';

export interface IntegrationHistoryEntry {
  id: string;
  kind: IntegrationHistoryKind;
  subsystemId: string;
  sourceShas: string[];
  targetBranch: string;
  mainCommitSha: string | null;
  pullRequestNumber: number | null;
  actor: string;
  occurredAt: string;
  detail: string;
  inheritedFromBranch: string | null;
}

export interface IntegrationHistory {
  targetBranch: string;
  subsystemId: string | null;
  entries: IntegrationHistoryEntry[];
  total: number;
  fetchedAt: string;
  dataSource: DataSource;
}

export interface IntegrationEventCommit {
  sha: string;
  shortSha: string;
  subject: string;
  author: string | null;
  authoredAt: string | null;
  unavailable: boolean;
}

export interface IntegrationEventChange {
  path: string;
  change: 'added' | 'modified' | 'deleted' | 'renamed';
  additions: number;
  deletions: number;
}

export interface IntegrationEventAudit {
  mainCommitSha: string;
  /** Full detail of the main commit itself; null when unresolvable — never fabricated. */
  mainCommit: IntegrationEventCommit | null;
  kind: IntegrationHistoryKind;
  subsystemId: string;
  targetBranch: string;
  actor: string;
  occurredAt: string;
  detail: string;
  pullRequestNumber: number | null;
  inheritedFromBranch: string | null;
  includedCommits: IntegrationEventCommit[];
  changes: IntegrationEventChange[];
  totals: { files: number; additions: number; deletions: number } | null;
  provenance: { key: string; value: string }[];
  revertedMainCommitSha: string | null;
  mainCommitUnavailable: boolean;
  dataSource: DataSource;
}

export interface CacheStats {
  entries: number;
  capacity: number;
  hits: number;
  misses: number;
  oldestEntryAt: string | null;
  /** Always true: the cache can be dropped and rebuilt from Git/GitHub history. */
  rebuildable: boolean;
  dataSource: DataSource;
  note: string;
}

export type ApiErrorCode =
  | 'backend_unavailable'
  | 'unauthenticated'
  | 'missing_prerequisite'
  | 'insufficient_scope'
  | 'repository_inaccessible'
  | 'invalid_registry'
  | 'branch_not_permitted'
  | 'source_commit_not_eligible'
  | 'duplicate_integration'
  | 'duplicate_registration'
  | 'rate_limited'
  | 'offline'
  | 'patch_conflict'
  | 'dependency_conflict'
  | 'ci_failure'
  | 'not_found'
  | 'internal';

export interface ApiErrorPayload {
  code: ApiErrorCode;
  message: string;
  remedy: string | null;
  details: string | null;
  retryable: boolean;
}

/** Integration semantics per subsystem (mirrors the Rust `SubsystemEntry.integrationPolicy`). */
export interface RegistryIntegrationPolicy {
  applicationFiles: string;
  packageJson: string;
  lockfiles: string;
  sharedConfiguration: string;
  githubWorkflows: string;
  secretFiles: string;
}

export interface RegistrySubsystem {
  id: string;
  name: string;
  repository: string;
  sourceBranch: string;
  enabled: boolean;
  pathStrategy: string;
  dependencyStrategy: string;
  integrationPolicy: RegistryIntegrationPolicy;
}

export interface RegistryPromotionPolicy {
  allowedTargetBranches: string[];
  allowedTargetBranchPatterns: string[];
  /** True only for `push_pr`; a local delivery never opens a pull request. */
  requirePullRequest: boolean;
}

/** The tool-local registry (spec §4, relocated into the tool post-pivot). */
export interface RegistryConfig {
  mainRepository: string;
  promotion: RegistryPromotionPolicy;
  subsystems: RegistrySubsystem[];
  /** Absolute path to the user's local clone of the main repository (local delivery). Omitted/null ⇒ resolve via env / managed checkout. */
  localMainRepositoryPath?: string | null;
  /** Exact paths or directory prefixes owned by the main system (never imported). */
  globalFiles?: string[] | null;
}

export interface RegistrySnapshot {
  config: RegistryConfig;
  /** `bundled` = the shipped defaults; `override` = the %APPDATA% file is in use. */
  source: 'bundled' | 'override';
  overridePath: string;
  overrideExists: boolean;
}

/** Result of a manual "pull latest" over the already-cached checkouts. */
export interface CheckoutRefresh {
  refreshed: string[];
  skipped: string[];
  failed: string[];
  message: string;
  dataSource: DataSource;
}

/** Result of deleting the cached clones outright (escape hatch for corruption). */
export interface CheckoutClear {
  removed: number;
  message: string;
}

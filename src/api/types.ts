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

/** Per-path integration semantics (spec §6.4): apply in place; manifests merge; lockfiles defer to the deployment; main-owned/secret files never import. */
export type FileIntegrationPolicy =
  'apply' | 'regenerate_in_target' | 'review_required' | 'never_import' | 'main_owned';

export type PromotionMode = 'individual' | 'cumulative';

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
  pathDatesTruncated: boolean;
}

export interface CommitQuery {
  subsystemId: string;
  targetBranch: string;
  page: number;
  pageSize: number;
  search: string;
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

/**
 * One changed file's bounded diff inside a `MainCommitDiff` (wire, camelCase).
 * Mirrors the render-relevant subset of `CommitDiff`: the main commit lives in
 * the MAIN repository, so there is deliberately no `subsystemId`/`sha` here.
 */
export interface MainCommitFileDiff {
  path: string;
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

/**
 * The whole main commit's per-file diff in ONE call (wire, camelCase). `files`
 * is keyed by `path` so a single backend read serves every file — never a
 * fetch per file. The audit page renders the same files column + selected-file
 * diff the commit browser uses.
 */
export interface MainCommitDiff {
  mainCommitSha: string;
  files: MainCommitFileDiff[];
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
  confirmationAcknowledged: boolean;
  delivery?: PromotionDelivery | null;
  onConflict?: PromotionOnConflict | null;
  /**
   * The promotion commit's own message. Required: the engine refuses an empty
   * one, so a multi-commit promotion is never labelled with only the last
   * source commit's subject.
   */
  message: string;
  excludedPaths?: string[] | null;
}

export type PreflightOutcome = 'clean' | 'merged' | 'conflict' | 'skipped';

export interface PreflightFileResult {
  path: string;
  outcome: PreflightOutcome;
  revertsNewer: RevertsNewerWork | null;
}

export interface PromotionPreflight {
  files: PreflightFileResult[];
  clean: number;
  merged: number;
  conflict: number;
  skipped: number;
  conflicts: string[];
  replaced?: string[] | null;
  deleted?: string[] | null;
  unchanged?: string[] | null;
  /** Paths also changed in the main system by something else (warning, not an error). */
  flagged?: string[] | null;
  excluded?: string[] | null;
}

/** One engine progress event for a running promotion (`promotion-progress`). */
export interface PromotionProgress {
  stage: string;
  label: string;
  index?: number | null;
  total?: number | null;
  detail?: string | null;
}

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
  nextCommands: string[];
  replaced?: string[] | null;
  deleted?: string[] | null;
  unchanged?: string[] | null;
  /** Paths also changed in the main system by something else (warning, not an error). */
  flagged?: string[] | null;
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
  requirePullRequest: boolean;
  attributeToAccount: boolean;
}

/** The tool-local registry (spec §4, relocated into the tool post-pivot). */
export interface RegistryConfig {
  mainRepository: string;
  promotion: RegistryPromotionPolicy;
  subsystems: RegistrySubsystem[];
  localMainRepositoryPath?: string | null;
  globalFiles?: string[] | null;
}

export interface RegistrySnapshot {
  config: RegistryConfig;
  source: 'bundled' | 'override';
  overridePath: string;
  overrideExists: boolean;
}

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

export type SubsystemDiffBucket = 'missing' | 'behind' | 'main_side_changed' | 'identical';

export interface RevertsNewerWork {
  sha: string;
  subject: string;
}

export interface SubsystemDiffCommit {
  sha: string;
  shortSha: string;
  subject: string;
  authorName: string;
  committedAt: string;
  status: CommitStatus;
  touchedPaths: string[];
}

export interface SubsystemDiffEntry {
  path: string;
  bucket: SubsystemDiffBucket;
  change: FileChangeKind;
  subsystemBlob: string | null;
  mainBlob: string | null;
  commits: SubsystemDiffCommit[];
  commitsTruncated: boolean;
  revertsNewer: RevertsNewerWork | null;
}

export interface SubsystemDiff {
  subsystemId: string;
  sourceBranch: string;
  sourceTip: string;
  targetBranch: string;
  targetTip: string;
  entries: SubsystemDiffEntry[];
  fetchedAt: string;
  dataSource: DataSource;
}

/** Distribution: single trunk commit -> single subsystem branch, delivered as a pull request. */
export interface TrunkCommit {
  sha: string;
  shortSha: string;
  subject: string;
  authorName: string;
  committedAt: string;
  committedUnix: number;
  isMerge: boolean;
  filesChanged: number;
  paths: TrunkPath[];
}

export interface TrunkPath {
  path: string;
  change: FileChangeKind;
}

export type DistributionPathAction = 'add' | 'update' | 'delete';

export interface DistributionFile {
  path: string;
  action: DistributionPathAction;
  trunkChange: FileChangeKind;
}

export interface DistributionSkip {
  path: string;
  reason: string;
}

export type DistributionDepAction = 'add' | 'update' | 'move';

export interface DistributionDependency {
  name: string;
  action: DistributionDepAction;
  fromSection?: string;
  fromSpec?: string;
  section: string;
  spec: string;
}

export interface DistributionBlocker {
  code: string;
  message: string;
  details: string | null;
}

export interface DistributionPlan {
  subsystemId: string;
  name: string;
  mainBranch: string;
  commitSha: string;
  commitSubject: string;
  sourceBranch: string;
  sourceSha: string;
  branch: string;
  files: DistributionFile[];
  skipped: DistributionSkip[];
  dependencies: DistributionDependency[];
  blockers: DistributionBlocker[];
  warnings: string[];
}

export interface DistributionPullRequest {
  number: number | null;
  url: string | null;
  state: string;
  base: string;
  head: string;
}

export interface DistributionOutcome {
  subsystemId: string;
  ok: boolean;
  mainBranch: string;
  sourceCommitSha: string;
  branch: string;
  commitSha: string | null;
  pullRequest: DistributionPullRequest | null;
  applied: string[];
  deleted: string[];
  dependencies: DistributionDependency[];
  blockers: DistributionBlocker[];
  warnings: string[];
  message: string | null;
}

export interface DistributionPreviewRequest {
  mainBranch: string;
  subsystemId: string;
  commitSha: string;
  excludedPaths?: string[];
}

export interface DistributionRunRequest {
  mainBranch: string;
  subsystemId: string;
  commitSha: string;
  excludedPaths?: string[];
  message?: string | null;
}

/** Bidirectional folder reconciliation (`RECONCILE-DESIGN.md`). */
export type ReconcileDirection = 'subsystem_to_main' | 'main_to_subsystem';

export type ReconcileNodeKind = 'directory' | 'file';

export interface ReconcileDirectoryNode {
  kind: ReconcileNodeKind;
  path: string;
  name: string;
  parent: string | null;
  depth: number;
  hasChildren: boolean;
  added: number;
  modified: number;
  deleted: number;
  unchanged: number;
  totalSourceFiles: number;
  existsInDest: boolean;
}

export interface ReconcileTreeSnapshot {
  direction: ReconcileDirection;
  subsystemId: string;
  sourceRepo: string;
  sourceBranch: string;
  sourceSha: string;
  destRepo: string;
  destBranch: string;
  destSha: string;
  directories: ReconcileDirectoryNode[];
  savedFolders: string[];
}

export type ReconcileFileAction = 'add' | 'overwrite' | 'delete';

export interface ReconcileFile {
  path: string;
  folder: string;
  action: ReconcileFileAction;
}

export interface ReconcileSkip {
  path: string;
  reason: string;
}

export interface ReconcilePlan {
  direction: ReconcileDirection;
  subsystemId: string;
  subsystemName: string;
  sourceRepo: string;
  sourceBranch: string;
  sourceSha: string;
  destRepo: string;
  destBranch: string;
  destSha: string;
  branch: string;
  delivery: PromotionDelivery;
  folders: string[];
  files: ReconcileFile[];
  unchangedCount: number;
  skipped: ReconcileSkip[];
  dependencies: DistributionDependency[];
  blockers: DistributionBlocker[];
  warnings: string[];
}

export interface ReconcilePreviewRequest {
  subsystemId: string;
  mainBranch: string;
  direction: ReconcileDirection;
  folders: string[];
  excludedPaths?: string[];
  delivery?: PromotionDelivery;
}

export interface ReconcileFileDiff {
  path: string;
  action: ReconcileFileAction;
  language: string;
  hunks: DiffHunk[];
  binary: boolean;
  truncated: boolean;
  sourceUrl: string | null;
  note: string | null;
}

export interface ReconcileRunRequest {
  subsystemId: string;
  mainBranch: string;
  direction: ReconcileDirection;
  folders: string[];
  excludedPaths?: string[];
  delivery?: PromotionDelivery;
  message?: string | null;
  confirmationAcknowledged: boolean;
}

export interface ReconcileOutcome {
  ok: boolean;
  direction: ReconcileDirection;
  subsystemId: string;
  sourceBranch: string;
  sourceSha: string;
  destBranch: string;
  branch: string;
  delivery: PromotionDelivery;
  commitSha: string | null;
  pullRequest: DistributionPullRequest | null;
  applied: string[];
  overwritten: string[];
  deleted: string[];
  dependencies: DistributionDependency[];
  blockers: DistributionBlocker[];
  warnings: string[];
  localCommands: string[];
  message: string | null;
}

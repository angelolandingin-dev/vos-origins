import { useEffect, useRef, useState } from 'react';
import {
  Check,
  CheckCircle2,
  ChevronRight,
  Copy,
  ExternalLink,
  GitBranch,
  GitPullRequestArrow,
  Package,
  TriangleAlert,
  X,
} from 'lucide-react';

import type { ApiError } from '../api/invoke';
import type {
  CommitSummary,
  DependencyChange,
  PromotionDelivery,
  PromotionDispatchResult,
  PromotionMode,
  PromotionPreflight,
  PromotionProgress,
  SubsystemSummary,
  TargetBranchList,
  ValidationFinding,
} from '../api/types';
import { PathLabel } from './PathLabel';
import { RevertsNewerLine } from './RevertsNewer';
import { CommitStatusBadge } from './StatusBadge';
import { CommitFileList } from './CommitFileList';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import {
  Button,
  Checkbox,
  Field,
  IconButton,
  InlineAlert,
  Modal,
  Select,
  Spinner,
} from './ui/primitives';

/**
 * Promotion dialog (spec §5.1). Local-first: `local_direct` (default) commits
 * onto the local target branch; `worktree` stages into the working tree (no
 * commit, no push); `local` creates a branch + commit in the user's clone (no
 * push, no PR); `push_pr` is the explicit opt-in that opens an integration PR.
 */

export interface PromotionDialogProps {
  open: boolean;
  onClose: () => void;
  subsystem: SubsystemSummary | null;
  sourceShas: string[];
  onRemoveSha: (sha: string) => void;
  commits: CommitSummary[];
  commitsLoading: boolean;
  targetBranches: TargetBranchList | null;
  targetBranch: string;
  onTargetBranchChange: (branch: string) => void;
  mode: PromotionMode;
  onModeChange: (mode: PromotionMode) => void;
  baseSha: string;
  onBaseShaChange: (sha: string) => void;
  delivery: PromotionDelivery;
  onDeliveryChange: (delivery: PromotionDelivery) => void;
  message: string;
  onMessageChange: (message: string) => void;
  excludedPaths: string[];
  onAddExcludedPath: (path: string) => void;
  onSetExcludedPaths: (paths: string[]) => void;
  dependencies: DependencyChange[];
  dependenciesLoading: boolean;
  preflight: PromotionPreflight | null;
  preflightLoading: boolean;
  dispatching: boolean;
  progress: PromotionProgress[];
  error: ApiError | null;
  result: PromotionDispatchResult | null;
  onDispatch: () => void;
  onOpenExternal: (url: string) => void;
}

export function PromotionDialog(props: PromotionDialogProps) {
  if (!props.open) return null;
  return (
    <PromotionDialogContent
      key={JSON.stringify([props.sourceShas, props.targetBranch])}
      {...props}
    />
  );
}

function preflightSummary(preflight: PromotionPreflight): string {
  const parts = [`${preflight.clean} clean`, `${preflight.conflict} need attention`];
  if (preflight.skipped > 0) parts.push(`${preflight.skipped} skipped`);
  return `${preflight.files.length} files: ${parts.join(' · ')}`;
}

/**
 * One condensed "what changed" summary: a single scannable counts line plus an
 * expandable reveal per non-empty category. Replaces the old stack of one box
 * per category plus duplicate finding boxes repeating the same paths.
 */
function ResultChanges({
  replaced,
  flagged,
  deleted,
  unchanged,
  excluded,
}: {
  replaced: string[];
  flagged: string[];
  deleted: string[];
  unchanged: string[];
  excluded: string[];
}) {
  const total = replaced.length + deleted.length + unchanged.length + excluded.length;
  if (total === 0) return null;

  const stats = [
    { label: 'Overwritten', value: replaced.length, tone: 'text-accent-ink' },
    { label: 'Deleted', value: deleted.length, tone: 'text-status-blocked-ink' },
    { label: 'Unchanged', value: unchanged.length, tone: 'text-muted' },
    { label: 'Excluded', value: excluded.length, tone: 'text-muted' },
  ];

  return (
    <section className="space-y-3">
      <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">What changed</h3>
      <div className="grid grid-cols-4 divide-x divide-line overflow-hidden rounded-md border border-line bg-surface">
        {stats.map((stat) => (
          <div key={stat.label} className="px-3 py-2">
            <p className="text-[10px] font-medium text-muted">{stat.label}</p>
            <p
              className={cx(
                'mono mt-0.5 text-sm font-semibold tabular-nums',
                stat.value > 0 ? stat.tone : 'text-disabled',
              )}
            >
              {stat.value}
            </p>
          </div>
        ))}
      </div>
      {replaced.length > 0 && flagged.length > 0 ? (
        <p className="text-[11px] leading-relaxed text-muted">
          {flagged.length} of the {replaced.length} overwritten path
          {replaced.length === 1 ? '' : 's'} had diverged in the main system — review the diff
          before pushing.
        </p>
      ) : null}
      <div className="space-y-1.5">
        <ExpandablePathList title="Overwritten" paths={replaced} />
        <ExpandablePathList title="Deleted" paths={deleted} />
        <ExpandablePathList title="Unchanged" paths={unchanged} />
        <ExpandablePathList title="Excluded" paths={excluded} />
      </div>
    </section>
  );
}

function ResultNotes({ findings }: { findings: ValidationFinding[] }) {
  const alerts = findings.filter((finding) => finding.level !== 'info');
  const alertCodes = new Set(alerts.map((finding) => finding.code));
  const infos = findings.filter(
    (finding) => finding.level === 'info' && !alertCodes.has(finding.code),
  );
  if (alerts.length === 0 && infos.length === 0) return null;

  return (
    <div className="space-y-2.5">
      {alerts.map((finding) => (
        <InlineAlert
          key={finding.code}
          tone={finding.level === 'error' ? 'danger' : 'warning'}
          title={finding.message.replace(
            /\(listed under Replaced\)/g,
            '(listed under Overwritten)',
          )}
        >
          {finding.remedy ? <p>{finding.remedy}</p> : null}
        </InlineAlert>
      ))}
      {infos.length > 0 ? (
        <div className="rounded-md border border-line bg-raised/40 px-3 py-2.5">
          <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">Notes</p>
          <ul className="mt-1.5 space-y-1 text-[11px] leading-relaxed text-muted">
            {infos.map((finding) => (
              <li key={finding.code}>
                {finding.message.replace(
                  /\(listed under Replaced\)/g,
                  '(listed under Overwritten)',
                )}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

const PROMOTION_STAGES: ReadonlyArray<{ id: string; label: string }> = [
  { id: 'resolve', label: 'Resolving subsystem and target' },
  { id: 'fetch', label: 'Fetching selected commits' },
  { id: 'classify', label: 'Classifying paths' },
  { id: 'copy', label: 'Copying selected paths' },
  { id: 'manifest', label: 'Merging package.json delta' },
  { id: 'prereqs', label: 'Checking dependency prerequisites' },
  { id: 'commit', label: 'Committing with provenance trailers' },
  { id: 'deliver', label: 'Delivering' },
  { id: 'cleanup', label: 'Cleaning up' },
];

function PromotionProgressView({
  steps,
  outcome,
}: {
  steps: PromotionProgress[];
  outcome: 'running' | 'success' | 'failed';
}) {
  const byStage = new Map(steps.map((step) => [step.stage, step]));
  // The engine emits `cleanup` unconditionally after the run (success or
  // failure), so on failure the failed stage is the last event before a
  // trailing `cleanup` — never the cleanup itself.
  let failedId: string | null = null;
  if (outcome === 'failed' && steps.length > 0) {
    const last = steps[steps.length - 1]!;
    failedId =
      last.stage === 'cleanup' && steps.length > 1 ? steps[steps.length - 2]!.stage : last.stage;
  }
  let currentIndex = -1;
  PROMOTION_STAGES.forEach((stage, index) => {
    if (byStage.has(stage.id)) currentIndex = index;
  });
  if (failedId !== null) {
    const failedIndex = PROMOTION_STAGES.findIndex((stage) => stage.id === failedId);
    if (failedIndex >= 0) currentIndex = failedIndex;
  }
  // A successful run has finished, so its last stage is done rather than
  // current: the engine emits nothing after `cleanup` to say it completed.
  if (outcome === 'success') currentIndex = PROMOTION_STAGES.length;
  const current =
    currentIndex >= 0 && currentIndex < PROMOTION_STAGES.length
      ? byStage.get(PROMOTION_STAGES[currentIndex]!.id)
      : undefined;
  const currentFraction =
    current?.index != null && current?.total != null && current.total > 0
      ? Math.min(current.index / current.total, 1)
      : 0;
  const done = currentIndex < 0 ? 0 : currentIndex + currentFraction;
  const detail =
    outcome === 'success'
      ? 'Promotion complete.'
      : (current?.detail ?? current?.label ?? 'Starting…');
  const counts =
    current?.index != null && current?.total != null ? `${current.index} / ${current.total}` : null;

  return (
    <section
      aria-label="Promotion progress"
      className={`space-y-2 rounded-md border border-line p-3 ${
        outcome === 'success'
          ? 'bg-status-applied-soft animate-tint-done'
          : outcome === 'failed'
            ? 'bg-status-blocked-soft animate-tint-failed'
            : 'bg-surface'
      }`}
    >
      <div className="flex h-[29px] items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
          Promotion progress
        </h3>
        {counts ? <span className="mono text-right text-muted tabular-nums">{counts}</span> : null}
      </div>
      <div className="h-[3px] overflow-hidden rounded-full bg-line" aria-hidden="true">
        <div
          className={`h-full rounded-full transition-[width] ${
            outcome === 'failed' ? 'bg-status-blocked' : 'bg-accent'
          }`}
          style={{ width: `${Math.round((done / PROMOTION_STAGES.length) * 100)}%` }}
        />
      </div>
      <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
        {PROMOTION_STAGES.map((stage, index) => {
          const seen = byStage.get(stage.id);
          const isFailed = stage.id === failedId;
          const state =
            isFailed || index === currentIndex ? 'current' : index < currentIndex ? 'done' : 'todo';
          return (
            <li
              key={stage.id}
              className={`flex h-[21px] items-center gap-2 px-2.5 ${
                isFailed
                  ? 'bg-status-blocked-soft'
                  : state === 'done'
                    ? 'bg-status-applied-soft'
                    : ''
              }`}
            >
              <span
                aria-hidden="true"
                className={`h-1.5 w-1.5 shrink-0 rounded-full ${
                  isFailed
                    ? 'bg-status-blocked'
                    : state === 'done'
                      ? 'bg-status-applied'
                      : state === 'current'
                        ? 'animate-pulse bg-accent'
                        : 'bg-line'
                }`}
              />
              <span
                className={`min-w-0 flex-1 truncate text-[11px] ${
                  isFailed ? 'text-status-blocked-ink' : state === 'todo' ? 'text-muted' : 'text-fg'
                }`}
              >
                {seen?.label ?? stage.label}
              </span>
              {seen?.index != null && seen?.total != null ? (
                <span className="mono shrink-0 text-[11px] text-muted tabular-nums">
                  {seen.index} / {seen.total}
                </span>
              ) : null}
            </li>
          );
        })}
      </ul>
      <p role="status" className="mono truncate text-[11px] text-muted">
        {detail}
      </p>
    </section>
  );
}

function ExpandablePathList({ title, paths }: { title: string; paths: string[] }) {
  if (paths.length === 0) return null;
  return (
    <details className="overflow-hidden rounded-md border border-line bg-surface">
      <summary className="flex h-[29px] cursor-pointer items-center justify-between gap-2 px-2.5 text-[11px]">
        <span className="font-semibold tracking-wide text-muted uppercase">{title}</span>
        <span className="mono text-muted tabular-nums">
          {paths.length} path{paths.length === 1 ? '' : 's'}
        </span>
      </summary>
      <ul className="divide-y divide-line border-t border-line max-h-32 overflow-y-auto">
        {paths.map((path) => (
          <li key={path} className="flex h-[29px] items-center px-2.5">
            <PathLabel path={path} className="flex-1 text-[11px]" muted />
          </li>
        ))}
      </ul>
    </details>
  );
}

function PromotionDialogContent({
  open,
  onClose,
  subsystem,
  sourceShas,
  onRemoveSha,
  commits,
  commitsLoading,
  targetBranches,
  targetBranch,
  onTargetBranchChange,
  mode,
  onModeChange,
  baseSha,
  onBaseShaChange,
  delivery,
  onDeliveryChange,
  message,
  onMessageChange,
  excludedPaths,
  onAddExcludedPath,
  onSetExcludedPaths,
  dependencies,
  dependenciesLoading,
  preflight,
  preflightLoading,
  dispatching,
  progress,
  error,
  result,
  onDispatch,
  onOpenExternal,
}: PromotionDialogProps) {
  const [acknowledged, setAcknowledged] = useState(false);
  const [copied, setCopied] = useState(false);
  const outcomeRef = useRef<HTMLDivElement>(null);
  const settled = result?.accepted === true || error !== null;
  const progressOutcome =
    result?.accepted === true ? 'success' : error !== null ? 'failed' : 'running';

  useEffect(() => {
    if (settled) outcomeRef.current?.scrollIntoView({ block: 'nearest' });
  }, [settled]);

  const branches = targetBranches?.branches ?? [];
  const selectedBranch = branches.find((branch) => branch.name === targetBranch) ?? null;
  const conflicting = dependencies.filter((dependency) => dependency.conflict);
  const cumulative = mode === 'cumulative';
  const multiManifest = commits.filter((commit) => commit.touchesManifest).length > 1;
  const replaced = preflight?.replaced ?? [];
  const preflightExcluded = preflight?.excluded ?? [];
  const refusedPaths = Array.from(new Set([...(preflight?.conflicts ?? [])]));
  const revertsNewerFiles = (preflight?.files ?? []).filter((file) => file.revertsNewer);
  const isExcluded = (path: string) =>
    excludedPaths.includes(path) || preflightExcluded.includes(path);
  const pendingReverts = revertsNewerFiles.filter((file) => !isExcluded(file.path));
  const excludeAllRevertsNewer = () => {
    onSetExcludedPaths(
      Array.from(new Set([...excludedPaths, ...pendingReverts.map((file) => file.path)])),
    );
  };
  const pendingConflicts = refusedPaths.filter((path) => !isExcluded(path));
  const excludeAllConflicts = () => {
    onSetExcludedPaths(Array.from(new Set([...excludedPaths, ...pendingConflicts])));
  };
  const isDirect = delivery === 'local_direct';
  const isWorktree = delivery === 'worktree';
  const isLocal = delivery === 'local';
  const isPushPr = delivery === 'push_pr';
  const repoLabel = `${targetBranches?.mainRepository ?? 'main-system'}/${targetBranch}`;
  const deliveryHint = isDirect
    ? `Commits directly to local ${targetBranch}; push manually.`
    : isWorktree
      ? `Stages files into your working tree on ${targetBranch} without committing.`
      : isLocal
        ? 'Creates an integration branch and commit in your local clone.'
        : 'Pushes an integration branch and opens a pull request.';

  const handleDeliveryChange = (next: PromotionDelivery) => {
    setAcknowledged(false);
    onDeliveryChange(next);
  };

  const canDispatch =
    !dispatching &&
    acknowledged &&
    sourceShas.length > 0 &&
    message.trim().length > 0 &&
    Boolean(subsystem) &&
    Boolean(selectedBranch?.allowed) &&
    (!cumulative || baseSha.trim().length > 0);

  // A disabled primary button with no stated reason reads as a broken click.
  const blockedReason = dispatching
    ? null
    : !acknowledged
      ? 'Acknowledge the delivery checkbox to continue.'
      : sourceShas.length === 0
        ? 'Select at least one commit to promote.'
        : message.trim().length === 0
          ? 'Enter a commit message to continue.'
          : !subsystem
            ? 'Select a subsystem first.'
            : !selectedBranch?.allowed
              ? `Target branch ${targetBranch} is not permitted by registry policy.`
              : cumulative && baseSha.trim().length === 0
                ? 'Enter the cumulative baseline SHA.'
                : null;

  const copyNextCommands = () => {
    const text = (result?.nextCommands ?? []).join('\n');
    if (!text) return;
    void navigator.clipboard?.writeText(text).then(
      () => setCopied(true),
      () => setCopied(false),
    );
  };

  return (
    <Modal
      open={open}
      onClose={onClose}
      size="xl"
      title="Promote commits to main system"
      description={subsystem ? subsystem.name : 'Select a subsystem first.'}
      footer={
        result?.accepted ? (
          <Button variant="secondary" onClick={onClose}>
            Close
          </Button>
        ) : (
          <>
            {blockedReason ? (
              <span className="mr-auto text-[11px] text-status-unknown-ink">{blockedReason}</span>
            ) : null}
            <Button variant="secondary" onClick={onClose} disabled={dispatching}>
              Cancel
            </Button>
            <Button
              variant="primary"
              icon={isPushPr ? GitPullRequestArrow : GitBranch}
              loading={dispatching}
              disabled={!canDispatch}
              onClick={onDispatch}
            >
              {dispatching
                ? 'Dispatching…'
                : isDirect
                  ? `Commit to ${targetBranch} locally`
                  : isWorktree
                    ? 'Stage into working tree'
                    : isLocal
                      ? 'Create local branch + commit'
                      : 'Push & open PR'}
            </Button>
          </>
        )
      }
    >
      {result?.accepted ? (
        <div className="space-y-5">
          {(() => {
            const warningCount = result.findings.filter((f) => f.level === 'warning').length;
            const errorCount = result.findings.filter((f) => f.level === 'error').length;
            const statusLabel =
              result.delivery === 'local_direct'
                ? 'Committed locally'
                : result.delivery === 'worktree'
                  ? 'Staged in working tree'
                  : result.delivery === 'local'
                    ? 'Branch created'
                    : result.prUrl
                      ? `PR #${result.prNumber ?? '?'} opened`
                      : 'Succeeded';
            const headline =
              result.delivery === 'local_direct'
                ? `Committed to ${result.targetBranch} locally`
                : result.delivery === 'worktree'
                  ? `Staged into working tree on ${result.targetBranch}`
                  : result.delivery === 'local'
                    ? 'Local integration branch created'
                    : result.prUrl
                      ? `Integration PR #${result.prNumber ?? '?'} opened`
                      : 'Promotion succeeded';

            return (
              <div
                ref={outcomeRef}
                className="flex flex-wrap items-start justify-between gap-4 rounded-md border border-status-applied/40 border-l-[3px] border-l-status-applied bg-status-applied-soft px-4 py-3.5"
              >
                <div className="flex min-w-0 flex-1 items-start gap-3">
                  <CheckCircle2
                    className="mt-0.5 h-5 w-5 shrink-0 text-status-applied-ink"
                    aria-hidden="true"
                  />
                  <div className="min-w-0">
                    <div className="flex flex-wrap items-center gap-2">
                      <h3 className="text-[13px] font-semibold text-status-applied-ink">
                        {headline}
                      </h3>
                      <span className="mono rounded border border-status-applied/40 bg-status-applied/15 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide text-status-applied-ink uppercase">
                        {statusLabel}
                      </span>
                      {warningCount > 0 ? (
                        <span className="mono rounded border border-status-unknown/40 bg-status-unknown-soft px-1.5 py-0.5 text-[10px] font-semibold text-status-unknown-ink">
                          {warningCount} warning{warningCount === 1 ? '' : 's'}
                        </span>
                      ) : null}
                      {errorCount > 0 ? (
                        <span className="mono rounded border border-status-blocked/40 bg-status-blocked-soft px-1.5 py-0.5 text-[10px] font-semibold text-status-blocked-ink">
                          {errorCount} error{errorCount === 1 ? '' : 's'}
                        </span>
                      ) : null}
                    </div>
                    <p className="mt-1 text-[11px] leading-relaxed text-fg/85">{result.message}</p>
                  </div>
                </div>

                <div className="flex shrink-0 flex-col items-end gap-1 text-[11px]">
                  <span className="mono rounded border border-status-applied/30 bg-surface/80 px-2 py-0.5 font-medium text-fg">
                    {result.branch}
                  </span>
                  <span className="text-muted">{formatDateTime(result.requestedAt)}</span>
                </div>
              </div>
            );
          })()}

          <div className="grid gap-5 lg:grid-cols-[minmax(0,1.05fr)_minmax(0,1fr)]">
            {/* Left column: changed file counts & expandable path lists */}
            <div className="min-w-0 space-y-4">
              <ResultChanges
                replaced={result.replaced ?? []}
                flagged={result.flagged ?? []}
                deleted={result.deleted ?? []}
                unchanged={result.unchanged ?? []}
                excluded={result.excluded ?? []}
              />

              {progress.length > 0 ? (
                <details className="group overflow-hidden rounded-md border border-line bg-surface">
                  <summary className="flex h-[32px] cursor-pointer items-center justify-between gap-2 px-3 text-[11px] text-muted select-none hover:bg-hover hover:text-fg">
                    <span className="flex items-center gap-1.5 font-medium">
                      <ChevronRight
                        className="h-3.5 w-3.5 text-disabled transition-transform group-open:rotate-90"
                        aria-hidden="true"
                      />
                      Execution log
                    </span>
                    <span className="mono flex items-center gap-1.5 text-status-applied-ink">
                      <span
                        className="h-1.5 w-1.5 rounded-full bg-status-applied"
                        aria-hidden="true"
                      />
                      {PROMOTION_STAGES.length} / {PROMOTION_STAGES.length} steps
                    </span>
                  </summary>
                  <div className="border-t border-line p-2.5">
                    <PromotionProgressView steps={progress} outcome={progressOutcome} />
                  </div>
                </details>
              ) : null}
            </div>

            {/* Right column: next commands, PR link & review callouts */}
            <div className="min-w-0 space-y-4">
              {(result.delivery === 'local' ||
                result.delivery === 'local_direct' ||
                result.delivery === 'worktree') &&
              result.nextCommands.length > 0 ? (
                <section className="space-y-2">
                  <div className="flex items-center justify-between gap-2">
                    <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                      Next commands
                    </h3>
                    <Button
                      variant="secondary"
                      size="sm"
                      icon={copied ? Check : Copy}
                      onClick={copyNextCommands}
                    >
                      {copied ? 'Copied' : 'Copy'}
                    </Button>
                  </div>
                  <pre className="mono overflow-x-auto rounded-md border border-line bg-ground p-3 text-[11px] leading-relaxed whitespace-pre-wrap text-fg">
                    {result.nextCommands.join('\n')}
                  </pre>
                </section>
              ) : null}

              {result.delivery === 'worktree' ? (
                <p className="text-[11px] leading-relaxed text-muted">
                  Nothing was committed or pushed, and integration status is unchanged.
                </p>
              ) : null}

              {result.prUrl ? (
                <Button
                  variant="primary"
                  size="sm"
                  icon={ExternalLink}
                  onClick={() => onOpenExternal(result.prUrl ?? '')}
                >
                  Open integration PR on GitHub
                </Button>
              ) : null}

              <ResultNotes findings={result.findings} />
            </div>
          </div>
        </div>
      ) : (
        <div className="grid gap-5 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,1fr)]">
          {/* Left column: source commits, files, dependency impact & mergeability preflight */}
          <div className="min-w-0 space-y-4">
            <section className="space-y-2">
              <div className="flex h-[25px] items-center justify-between gap-2">
                <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Source commits
                </h3>
                <span className="mono text-right text-muted tabular-nums">{sourceShas.length}</span>
              </div>
              {sourceShas.length === 0 ? (
                <InlineAlert tone="warning" title="No commits selected">
                  Tick one or more eligible commits in the commit list first.
                </InlineAlert>
              ) : (
                <ul className="max-h-44 divide-y divide-line overflow-y-auto rounded-md border border-line bg-surface">
                  {sourceShas.map((sha) => {
                    const commit = commits.find((candidate) => candidate.sha === sha);
                    return (
                      <li key={sha} className="flex h-[29px] items-center gap-2 px-2.5">
                        <span className="mono shrink-0 text-[11px] text-accent-ink" title={sha}>
                          {sha.slice(0, 7)}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-xs text-fg">
                          {commitsLoading && !commit
                            ? 'Loading…'
                            : (commit?.subject ?? 'Unknown commit')}
                        </span>
                        {commit ? <CommitStatusBadge status={commit.status} /> : null}
                        <IconButton
                          icon={X}
                          label={`Remove ${sha.slice(0, 7)} from this promotion`}
                          onClick={() => onRemoveSha(sha)}
                          className="h-[21px] w-[21px]"
                        />
                      </li>
                    );
                  })}
                </ul>
              )}
            </section>

            <section className="rounded-md border border-line bg-surface p-3">
              <CommitFileList
                preflight={preflight}
                preflightLoading={preflightLoading}
                excludedPaths={excludedPaths}
                onSetExcludedPaths={onSetExcludedPaths}
              />
            </section>

            <section className="space-y-2">
              <h3 className="flex h-[25px] items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
                <Package className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
                Dependency impact
              </h3>
              {dependenciesLoading ? (
                <p className="flex items-center gap-1.5 text-[11px] text-muted">
                  <Spinner className="h-3 w-3" />
                  Loading dependency changes…
                </p>
              ) : dependencies.length === 0 ? (
                <p className="text-[11px] text-muted">No manifest changes.</p>
              ) : (
                <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                  {dependencies.map((dependency) => (
                    <li
                      key={`${dependency.section}-${dependency.name}`}
                      className="flex h-[29px] items-center gap-2 px-2.5 text-[11px]"
                    >
                      {dependency.conflict ? (
                        <TriangleAlert
                          className="h-3 w-3 shrink-0 text-status-blocked"
                          aria-hidden="true"
                        />
                      ) : null}
                      <span className="mono truncate text-fg">{dependency.name}</span>
                      <span className="meta shrink-0">{dependency.section}</span>
                      <span className="mono truncate text-muted">
                        {dependency.fromRange ?? '(absent)'} → {dependency.toRange ?? '(removed)'}
                      </span>
                      <span className="ml-auto shrink-0 tracking-wide text-muted uppercase">
                        {dependency.conflict ? (
                          <span className="text-status-blocked-ink">conflict</span>
                        ) : (
                          dependency.change
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              )}

              {multiManifest ? (
                <InlineAlert
                  tone="warning"
                  title="More than one selected commit changes package.json"
                >
                  Review the merged manifest in the promotion commit before pushing.
                </InlineAlert>
              ) : null}

              {conflicting.length > 0 ? (
                <InlineAlert tone="danger" title="Incompatible dependency range in this selection">
                  The promotion fails closed. Resolve the range or drop the commit.
                </InlineAlert>
              ) : null}
            </section>

            {preflightLoading ? (
              <div className="flex items-center gap-2 text-[11px] text-muted">
                <Spinner className="h-3.5 w-3.5" />
                Checking mergeability…
              </div>
            ) : preflight && preflight.files.length > 0 ? (
              <section className="space-y-2">
                <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Mergeability preflight
                </h3>
                <InlineAlert
                  tone={preflight.conflict > 0 ? 'warning' : 'info'}
                  title={preflightSummary(preflight)}
                >
                  {replaced.length > 0 ? (
                    <p>
                      {replaced.length} path{replaced.length === 1 ? '' : 's'} changed in main will
                      be overwritten with the subsystem&apos;s version.
                    </p>
                  ) : preflight.conflict > 0 ? (
                    <p>Resolve, exclude, or drop the conflicting commit, then retry.</p>
                  ) : (
                    <p>Ready to promote.</p>
                  )}
                </InlineAlert>
                <ExpandablePathList title="Will be overwritten" paths={replaced} />
                {revertsNewerFiles.length > 0 ? (
                  <InlineAlert
                    tone="warning"
                    title={`Reverts newer work — ${revertsNewerFiles.length} path${revertsNewerFiles.length === 1 ? '' : 's'}`}
                    actions={
                      pendingReverts.length > 0 ? (
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={excludeAllRevertsNewer}
                          title="Exclude these paths from this promotion — they will not be promoted"
                          aria-label={`Exclude all ${pendingReverts.length} reverts-newer path${pendingReverts.length === 1 ? '' : 's'} from this promotion`}
                        >
                          Exclude all {pendingReverts.length}
                        </Button>
                      ) : undefined
                    }
                  >
                    <p>
                      Main already holds newer subsystem content here. Proceed to overwrite
                      deliberately, or exclude the path.
                    </p>
                  </InlineAlert>
                ) : null}
                {revertsNewerFiles.length > 0 ? (
                  <ul className="max-h-40 divide-y divide-line overflow-hidden overflow-y-auto rounded-md border border-line bg-surface">
                    {revertsNewerFiles.map((file) => (
                      <li key={file.path} className="flex items-start gap-2 px-2.5 py-1.5">
                        <TriangleAlert
                          className="mt-0.5 h-3 w-3 shrink-0 text-status-unknown"
                          aria-hidden="true"
                        />
                        <div className="min-w-0 flex-1">
                          <PathLabel path={file.path} className="text-[11px]" />
                          {file.revertsNewer ? <RevertsNewerLine warn={file.revertsNewer} /> : null}
                        </div>
                        {isExcluded(file.path) ? (
                          <span className="shrink-0 text-[11px] text-muted">excluded</span>
                        ) : (
                          <Button
                            size="sm"
                            variant="secondary"
                            onClick={() => onAddExcludedPath(file.path)}
                          >
                            Exclude
                          </Button>
                        )}
                      </li>
                    ))}
                  </ul>
                ) : null}
                {refusedPaths.length > 0 ? (
                  <div className="space-y-1.5">
                    <div className="flex h-[29px] items-center justify-between gap-2">
                      <h4 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                        Conflicting paths
                      </h4>
                      {pendingConflicts.length > 0 ? (
                        <Button
                          size="sm"
                          variant="secondary"
                          onClick={excludeAllConflicts}
                          title="Exclude these paths from this promotion — they will not be promoted"
                          aria-label={`Exclude all ${pendingConflicts.length} conflicting path${pendingConflicts.length === 1 ? '' : 's'} from this promotion`}
                        >
                          Exclude all {pendingConflicts.length}
                        </Button>
                      ) : null}
                    </div>
                    <ul className="max-h-40 divide-y divide-line overflow-hidden overflow-y-auto rounded-md border border-line bg-surface">
                      {refusedPaths.map((path) => (
                        <li key={path} className="flex h-[29px] items-center gap-2 px-2.5">
                          <TriangleAlert
                            className="h-3 w-3 shrink-0 text-status-blocked"
                            aria-hidden="true"
                          />
                          <PathLabel path={path} className="flex-1 text-[11px]" />
                          {isExcluded(path) ? (
                            <span className="shrink-0 text-[11px] text-muted">excluded</span>
                          ) : (
                            <Button
                              size="sm"
                              variant="secondary"
                              onClick={() => onAddExcludedPath(path)}
                            >
                              Exclude
                            </Button>
                          )}
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
                <ExpandablePathList title="Excluded" paths={preflightExcluded} />
              </section>
            ) : null}
          </div>

          {/* Right column: delivery configuration, commit message, acknowledgement & live progress */}
          <div className="min-w-0 space-y-4">
            <Field label="Delivery" hint={deliveryHint}>
              <Select
                ariaLabel="Promotion delivery"
                value={delivery}
                onValueChange={(next) => handleDeliveryChange(next as PromotionDelivery)}
                options={[
                  { value: 'local_direct', label: `Commit to local ${targetBranch} (recommended)` },
                  { value: 'worktree', label: 'Add to working tree (no commit)' },
                  { value: 'local', label: 'Local integration branch' },
                  { value: 'push_pr', label: 'Push + PR' },
                ]}
              />
            </Field>

            <Field label="Target branch" required hint="Only policy-allowed branches.">
              <Select
                ariaLabel="Target main-system branch"
                value={targetBranch}
                onValueChange={(next) => onTargetBranchChange(next)}
                options={branches.map((branch) => ({
                  value: branch.name,
                  label: `${branch.name}${branch.isDefault ? ' (default)' : ''}${branch.allowed ? '' : ' — not permitted'}`,
                  disabled: !branch.allowed,
                  title: branch.allowed
                    ? `tip ${branch.headSha.slice(0, 7)}`
                    : (branch.disallowedReason ?? 'Not permitted by registry policy'),
                }))}
              />
            </Field>

            <div className="space-y-1.5">
              <span className="block text-[11px] font-semibold tracking-wide text-muted uppercase">
                Mode
              </span>
              <div
                role="group"
                aria-label="Promotion mode"
                className="grid h-[25px] grid-cols-2 overflow-hidden rounded-md border border-line bg-surface"
              >
                {(
                  [
                    { id: 'individual', label: 'Individual' },
                    { id: 'cumulative', label: 'Cumulative' },
                  ] as const
                ).map((item, idx) => (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => onModeChange(item.id)}
                    className={cx(
                      'px-2.5 text-[11px] transition-colors',
                      idx > 0 && 'border-l border-line',
                      mode === item.id
                        ? 'bg-raised font-semibold text-fg'
                        : 'text-muted hover:bg-hover hover:text-fg',
                    )}
                  >
                    {item.label}
                  </button>
                ))}
              </div>
            </div>

            {cumulative ? (
              <Field label="Cumulative baseline SHA" required>
                <input
                  className="input mono"
                  value={baseSha}
                  onChange={(event) => onBaseShaChange(event.target.value)}
                  placeholder="40-character full SHA"
                  aria-label="Cumulative baseline SHA"
                />
              </Field>
            ) : null}

            <Field label="Commit message" required>
              <textarea
                className="input min-h-[76px] py-2"
                aria-label="Commit message"
                rows={3}
                required
                value={message}
                onChange={(event) => onMessageChange(event.target.value)}
                placeholder="Promote subsystem changes"
              />
            </Field>

            <section className="space-y-2 rounded-md border border-line bg-surface p-3">
              <label className="flex items-start gap-2 text-xs text-fg">
                <Checkbox
                  checked={acknowledged}
                  onChange={(event) => setAcknowledged(event.target.checked)}
                  label={
                    isDirect
                      ? `Acknowledge local commit onto ${targetBranch}`
                      : isWorktree
                        ? `Acknowledge staging into working tree on ${targetBranch}`
                        : isLocal
                          ? `Acknowledge creating a local integration branch for ${targetBranch}`
                          : `Acknowledge pushing and opening a pull request against ${targetBranch}`
                  }
                  className="mt-0.5"
                />
                {isDirect ? (
                  <span>
                    Commits selected files locally onto <span className="mono">{repoLabel}</span>{' '}
                    with no push and no PR — push manually when ready.
                  </span>
                ) : isWorktree ? (
                  <span>
                    Copies selected files into your working tree on{' '}
                    <span className="mono">{repoLabel}</span> and stages them with no commit or push
                    — <span className="mono">{targetBranch}</span> must be checked out and clean.
                  </span>
                ) : isLocal ? (
                  <span>
                    Creates a local integration branch and commit in your clone for{' '}
                    <span className="mono">{repoLabel}</span> with no push and no PR.
                  </span>
                ) : (
                  <span>
                    Opens a pull request against <span className="mono">{repoLabel}</span> — does
                    not merge automatically.
                  </span>
                )}
              </label>
              {replaced.length > 0 ? (
                <p className="text-[11px] leading-relaxed text-muted">
                  Overwrites {replaced.length} path{replaced.length === 1 ? '' : 's'} changed in
                  main with the subsystem&apos;s version.
                </p>
              ) : null}
            </section>

            {progress.length > 0 ? (
              <PromotionProgressView steps={progress} outcome={progressOutcome} />
            ) : null}

            {error ? (
              <div ref={outcomeRef}>
                <InlineAlert tone="danger" title={error.message}>
                  {error.remedy ? <p>{error.remedy}</p> : null}
                  {error.details ? (
                    <p className="mono mt-1 text-[11px] whitespace-pre-wrap text-status-blocked-ink">
                      {error.details}
                    </p>
                  ) : null}
                </InlineAlert>
              </div>
            ) : null}
          </div>
        </div>
      )}
    </Modal>
  );
}

import { useMemo, useState } from 'react';
import {
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  GitPullRequestArrow,
  RefreshCw,
  Search,
  TriangleAlert,
} from 'lucide-react';

import type { ApiError } from '../../api/invoke';
import type {
  DistributionBlocker,
  DistributionDependency,
  FileChangeKind,
  PromotionDelivery,
  PromotionProgress,
  ReconcileDirection,
  ReconcileFile,
  ReconcileFileAction,
  ReconcileFileDiff,
  ReconcileOutcome,
  ReconcilePlan,
  ReconcileSkip,
} from '../../api/types';
import { ChangeKindIcon } from '../ChangeKindIcon';
import { DiffViewer } from '../DiffViewer';
import { ExecutionLogDisclosure, ExecutionProgressView } from '../ExecutionProgressView';
import type { ExecutionStage } from '../executionProgress';
import { PathLabel } from '../PathLabel';
import { cx } from '../ui/cx';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  Field,
  IconButton,
  InlineAlert,
  KeyValue,
  LoadingState,
  Panel,
  Select,
  Spinner,
} from '../ui/primitives';

export interface ReconcileReviewProps {
  direction: ReconcileDirection;
  subsystemName: string;
  folders: string[];
  delivery: PromotionDelivery;
  onDeliveryChange: (delivery: PromotionDelivery) => void;
  plan: ReconcilePlan | null;
  planLoading: boolean;
  planError: ApiError | null;
  onReloadPlan: () => void;
  excludedPaths: string[];
  onTogglePath: (path: string, included: boolean) => void;
  onSelectAll: () => void;
  onSelectNone: () => void;
  inspectedFile: ReconcileFile | null;
  onInspectFile: (file: ReconcileFile | null) => void;
  fileDiff: ReconcileFileDiff | null;
  diffLoading: boolean;
  diffError: ApiError | null;
  onReloadDiff: () => void;
  message: string;
  onMessageChange: (message: string) => void;
  confirmed: boolean;
  onConfirmedChange: (confirmed: boolean) => void;
  running: boolean;
  progressSteps: PromotionProgress[];
  runError: ApiError | null;
  outcome: ReconcileOutcome | null;
  onBackToFolders: () => void;
  onRun: () => void;
  onOpenExternal: (url: string) => void;
  onDone: () => void;
}

const ACTION_META: Record<
  ReconcileFileAction,
  { label: string; dot: string; changeKind: FileChangeKind }
> = {
  add: { label: 'Add', dot: 'bg-status-applied', changeKind: 'added' },
  overwrite: { label: 'Overwrite', dot: 'bg-accent', changeKind: 'modified' },
  delete: { label: 'Delete', dot: 'bg-status-blocked', changeKind: 'deleted' },
};

const RECONCILE_STAGES: ReadonlyArray<ExecutionStage> = [
  { id: 'prepare', label: 'Preparing checkouts' },
  { id: 'plan', label: 'Validating reconcile plan' },
  { id: 'apply', label: 'Applying folder contents' },
  { id: 'commit', label: 'Recording reconciliation commit' },
  { id: 'deliver', label: 'Delivering reconciliation' },
];

const DELIVERY_OPTIONS: Array<{ value: PromotionDelivery; label: string }> = [
  { value: 'local_direct', label: 'Local commit on target branch (default)' },
  { value: 'local', label: 'Local reconcile branch (no push)' },
  { value: 'worktree', label: 'Stage in working tree (no commit)' },
  { value: 'push_pr', label: 'Push branch and open pull request' },
];

export function ReconcileReview({
  direction,
  subsystemName,
  folders,
  delivery,
  onDeliveryChange,
  plan,
  planLoading,
  planError,
  onReloadPlan,
  excludedPaths,
  onTogglePath,
  onSelectAll,
  onSelectNone,
  inspectedFile,
  onInspectFile,
  fileDiff,
  diffLoading,
  diffError,
  onReloadDiff,
  message,
  onMessageChange,
  confirmed,
  onConfirmedChange,
  running,
  progressSteps,
  runError,
  outcome,
  onBackToFolders,
  onRun,
  onOpenExternal,
  onDone,
}: ReconcileReviewProps) {
  const [actionFilter, setActionFilter] = useState<'all' | ReconcileFileAction>('all');
  const [pathFilter, setPathFilter] = useState('');
  const [diffMode, setDiffMode] = useState<'unified' | 'side'>('unified');
  const [copiedCommands, setCopiedCommands] = useState(false);

  const excludedSet = useMemo(() => new Set(excludedPaths), [excludedPaths]);
  const files = useMemo(() => plan?.files ?? [], [plan?.files]);

  const counts = useMemo(() => {
    const tally = { add: 0, overwrite: 0, delete: 0 };
    for (const file of files) {
      if (!excludedSet.has(file.path)) tally[file.action] += 1;
    }
    return tally;
  }, [files, excludedSet]);

  const visibleFiles = useMemo(() => {
    const needle = pathFilter.trim().toLowerCase();
    return files.filter((file) => {
      if (actionFilter !== 'all' && file.action !== actionFilter) return false;
      if (needle && !file.path.toLowerCase().includes(needle)) return false;
      return true;
    });
  }, [files, actionFilter, pathFilter]);

  const includedCount = counts.add + counts.overwrite + counts.delete;
  const hasChanges = plan !== null && (includedCount > 0 || plan.dependencies.length > 0);
  const isBlocked = plan !== null && plan.blockers.length > 0;
  const canRun =
    plan !== null &&
    !planLoading &&
    !isBlocked &&
    hasChanges &&
    confirmed &&
    !running &&
    outcome === null;

  const defaultSubject =
    folders.length === 1 ? `sync ${folders[0]}` : `sync ${folders.length} folders`;

  const copyAllCommands = (commands: string[]) => {
    const text = commands.join('\n');
    if (!text) return;
    void navigator.clipboard.writeText(text).then(() => {
      setCopiedCommands(true);
      window.setTimeout(() => setCopiedCommands(false), 1600);
    });
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* Scope summary strip */}
      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-4 py-2">
        <div className="flex min-w-0 flex-1 items-center gap-2.5">
          <Button
            variant="secondary"
            size="sm"
            icon={ArrowLeft}
            disabled={running}
            onClick={onBackToFolders}
          >
            Edit folders
          </Button>
          <span className="mono min-w-0 truncate text-[11px] text-muted" title={folders.join(', ')}>
            {folders.join(' · ')}
          </span>
        </div>
        {plan ? (
          <span className="mono shrink-0 text-[11px] text-fg">
            {plan.sourceRepo} ({plan.sourceBranch}) → {plan.destRepo} ({plan.destBranch})
          </span>
        ) : null}
      </div>

      {outcome ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="scroll-area min-h-0 flex-1 p-5">
            <div className="mx-auto max-w-5xl space-y-5">
              {(() => {
                const statusBadge = !outcome.ok
                  ? 'Blocked'
                  : outcome.pullRequest?.number != null
                    ? `PR #${outcome.pullRequest.number} ${outcome.pullRequest.state}`
                    : outcome.delivery === 'local_direct'
                      ? 'Committed locally'
                      : outcome.delivery === 'worktree'
                        ? 'Staged in working tree'
                        : outcome.delivery === 'local'
                          ? 'Branch created'
                          : 'Completed';
                const headline = !outcome.ok
                  ? `${subsystemName} reconciliation blocked`
                  : outcome.pullRequest?.number != null
                    ? `Pull request #${outcome.pullRequest.number} opened for ${subsystemName}`
                    : outcome.delivery === 'local_direct'
                      ? `Reconciled ${subsystemName} into ${outcome.destBranch} locally`
                      : outcome.delivery === 'worktree'
                        ? `Staged ${subsystemName} reconciliation on ${outcome.destBranch}`
                        : `Reconciled ${subsystemName} on ${outcome.branch}`;

                return (
                  <div
                    className={cx(
                      'flex flex-wrap items-start justify-between gap-4 rounded-md border border-l-[3px] px-4 py-3.5',
                      outcome.ok
                        ? 'border-status-applied/40 border-l-status-applied bg-status-applied-soft'
                        : 'border-status-blocked/40 border-l-status-blocked bg-status-blocked-soft',
                    )}
                  >
                    <div className="flex min-w-0 flex-1 items-start gap-3">
                      {outcome.ok ? (
                        <CheckCircle2
                          className="mt-0.5 h-5 w-5 shrink-0 text-status-applied-ink"
                          aria-hidden="true"
                        />
                      ) : (
                        <TriangleAlert
                          className="mt-0.5 h-5 w-5 shrink-0 text-status-blocked-ink"
                          aria-hidden="true"
                        />
                      )}
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <h2
                            className={cx(
                              'text-[13px] font-semibold',
                              outcome.ok ? 'text-status-applied-ink' : 'text-status-blocked-ink',
                            )}
                          >
                            {headline}
                          </h2>
                          <span
                            className={cx(
                              'mono rounded border px-1.5 py-0.5 text-[10px] font-semibold tracking-wide uppercase',
                              outcome.ok
                                ? 'border-status-applied/40 bg-status-applied/15 text-status-applied-ink'
                                : 'border-status-blocked/40 bg-status-blocked/15 text-status-blocked-ink',
                            )}
                          >
                            {statusBadge}
                          </span>
                          {outcome.warnings.length > 0 ? (
                            <span className="mono rounded border border-status-unknown/40 bg-status-unknown-soft px-1.5 py-0.5 text-[10px] font-semibold text-status-unknown-ink">
                              {outcome.warnings.length} warning
                              {outcome.warnings.length === 1 ? '' : 's'}
                            </span>
                          ) : null}
                        </div>
                        {outcome.message ? (
                          <p className="mt-1 text-[11px] leading-relaxed text-fg/85">
                            {outcome.message}
                          </p>
                        ) : null}
                      </div>
                    </div>

                    <div className="flex shrink-0 flex-col items-end gap-1 text-[11px]">
                      <span className="mono rounded border border-line bg-surface/80 px-2 py-0.5 font-medium text-fg">
                        {outcome.destBranch}
                      </span>
                      <span className="mono text-muted">
                        {outcome.commitSha ? outcome.commitSha.slice(0, 7) : 'staged only'}
                      </span>
                    </div>
                  </div>
                );
              })()}

              <div className="grid gap-5 lg:grid-cols-[minmax(0,1.05fr)_minmax(0,1fr)]">
                {/* Left: change counts, expandable paths & execution log */}
                <div className="min-w-0 space-y-4">
                  <section className="space-y-3">
                    <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                      What changed
                    </h3>
                    <div className="grid grid-cols-4 divide-x divide-line overflow-hidden rounded-md border border-line bg-surface">
                      {[
                        {
                          label: 'Added',
                          value: outcome.applied.length,
                          tone: 'text-status-applied-ink',
                        },
                        {
                          label: 'Overwritten',
                          value: outcome.overwritten.length,
                          tone: 'text-accent-ink',
                        },
                        {
                          label: 'Deleted',
                          value: outcome.deleted.length,
                          tone: 'text-status-blocked-ink',
                        },
                        {
                          label: 'Deps',
                          value: outcome.dependencies.length,
                          tone: 'text-fg',
                        },
                      ].map((stat) => (
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

                    <div className="space-y-1.5">
                      <OutcomePathList title="Added" paths={outcome.applied} />
                      <OutcomePathList title="Overwritten" paths={outcome.overwritten} />
                      <OutcomePathList title="Deleted" paths={outcome.deleted} />
                    </div>
                  </section>

                  {outcome.dependencies.length > 0 ? (
                    <section className="space-y-2">
                      <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                        Dependencies updated ({outcome.dependencies.length})
                      </h3>
                      <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                        {outcome.dependencies.map((dep) => (
                          <DependencyRow key={`${dep.section}:${dep.name}`} dependency={dep} />
                        ))}
                      </ul>
                    </section>
                  ) : null}

                  <ExecutionLogDisclosure
                    stages={RECONCILE_STAGES}
                    steps={progressSteps}
                    outcome={outcome.ok ? 'success' : 'failed'}
                  />
                </div>

                {/* Right: next commands, metadata, blockers & warnings */}
                <div className="min-w-0 space-y-4">
                  {outcome.localCommands.length > 0 ? (
                    <section className="space-y-2">
                      <div className="flex items-center justify-between gap-2">
                        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                          Next commands
                        </h3>
                        <Button
                          variant="secondary"
                          size="sm"
                          icon={copiedCommands ? Check : Copy}
                          onClick={() => copyAllCommands(outcome.localCommands)}
                        >
                          {copiedCommands ? 'Copied' : 'Copy'}
                        </Button>
                      </div>
                      <pre className="mono overflow-x-auto rounded-md border border-line bg-ground p-3 text-[11px] leading-relaxed whitespace-pre-wrap text-fg">
                        {outcome.localCommands.join('\n')}
                      </pre>
                    </section>
                  ) : null}

                  <section className="space-y-2">
                    <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                      Details
                    </h3>
                    <dl className="divide-y divide-line rounded-md border border-line bg-surface px-3">
                      <KeyValue label="Source">
                        <span className="mono">
                          {outcome.sourceBranch} @ {outcome.sourceSha.slice(0, 7)}
                        </span>
                      </KeyValue>
                      <KeyValue label="Destination">
                        <span className="mono">{outcome.destBranch}</span>
                      </KeyValue>
                      <KeyValue label="Branch">
                        <span className="mono">{outcome.branch}</span>
                      </KeyValue>
                      <KeyValue label="Commit">
                        {outcome.commitSha ? (
                          <span className="mono">{outcome.commitSha.slice(0, 7)}</span>
                        ) : (
                          '— (staged only)'
                        )}
                      </KeyValue>
                    </dl>
                  </section>

                  <BlockerList blockers={outcome.blockers} />
                  <WarningsList warnings={outcome.warnings} />
                </div>
              </div>
            </div>
          </div>

          <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
            <span className="mono text-[11px] text-muted tabular-nums">
              {outcome.ok
                ? `+${outcome.applied.length} added · ~${outcome.overwritten.length} overwritten · -${outcome.deleted.length} deleted`
                : 'Reconciliation blocked'}
            </span>
            <div className="flex shrink-0 items-center gap-2">
              {outcome.pullRequest?.url ? (
                <Button
                  variant="secondary"
                  icon={ExternalLink}
                  onClick={() => {
                    if (outcome.pullRequest?.url) onOpenExternal(outcome.pullRequest.url);
                  }}
                >
                  Open Pull Request
                </Button>
              ) : null}
              <Button variant="primary" onClick={onDone}>
                Done
              </Button>
            </div>
          </footer>
        </div>
      ) : (
        <div className="grid min-h-0 flex-1 gap-3 p-3 xl:grid-cols-[minmax(0,1fr)_minmax(380px,440px)]">
          {/* Left column: file actions + inline diff viewer */}
          <div className="flex min-h-0 flex-col gap-3">
            <Panel
              title="Files to mirror"
              subtitle={
                plan
                  ? `${includedCount} of ${plan.files.length} selected · ${plan.unchangedCount} unchanged`
                  : undefined
              }
              actions={
                <div className="flex items-center gap-1">
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={running || !plan || excludedPaths.length === 0}
                    onClick={onSelectAll}
                  >
                    All
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    disabled={running || !plan || excludedPaths.length === files.length}
                    onClick={onSelectNone}
                  >
                    None
                  </Button>
                  <IconButton
                    icon={RefreshCw}
                    label="Refresh reconcile preview"
                    disabled={planLoading || running}
                    onClick={onReloadPlan}
                  />
                </div>
              }
              className="flex min-h-0 flex-1 flex-col"
              bodyClassName="flex min-h-0 flex-1 flex-col"
            >
              {planLoading && !plan ? (
                <LoadingState label="Computing folder mirror plan" />
              ) : planError ? (
                <ErrorState compact error={planError} onRetry={onReloadPlan} />
              ) : plan ? (
                <>
                  {planLoading ? (
                    <div
                      role="progressbar"
                      aria-label="Updating reconcile plan"
                      className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
                    >
                      <div className="h-full w-1/2 animate-pulse bg-accent" />
                    </div>
                  ) : null}
                  <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
                    <div
                      role="group"
                      aria-label="Filter by action"
                      className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
                    >
                      {(
                        [
                          { id: 'all', label: `All (${files.length})` },
                          { id: 'add', label: 'Add' },
                          { id: 'overwrite', label: 'Overwrite' },
                          { id: 'delete', label: 'Delete' },
                        ] as const
                      ).map((tab, index) => (
                        <button
                          key={tab.id}
                          type="button"
                          onClick={() => setActionFilter(tab.id)}
                          className={cx(
                            'px-2 text-[11px] transition-colors',
                            index > 0 && 'border-l border-line',
                            actionFilter === tab.id
                              ? 'bg-raised font-semibold text-fg'
                              : 'text-muted hover:bg-hover hover:text-fg',
                          )}
                        >
                          {tab.label}
                        </button>
                      ))}
                    </div>

                    <div className="relative min-w-[160px] flex-1">
                      <Search
                        className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                        aria-hidden="true"
                      />
                      <input
                        type="search"
                        value={pathFilter}
                        onChange={(event) => setPathFilter(event.target.value)}
                        placeholder="Filter paths…"
                        aria-label="Filter changed paths"
                        className="input pl-8"
                      />
                    </div>
                  </div>

                  <div
                    className={cx(
                      'scroll-area min-h-0 flex-1 transition-opacity',
                      planLoading && 'pointer-events-none opacity-55',
                    )}
                  >
                    {visibleFiles.length === 0 ? (
                      <EmptyState
                        title="No matching file changes"
                        message={
                          files.length === 0
                            ? 'All files inside the selected folders already match the source.'
                            : 'Adjust the action or path filter to see other files.'
                        }
                      />
                    ) : (
                      <ul className="divide-y divide-line">
                        {visibleFiles.map((file) => {
                          const included = !excludedSet.has(file.path);
                          const inspected = inspectedFile?.path === file.path;
                          const meta = ACTION_META[file.action];
                          return (
                            <li
                              key={file.path}
                              className={cx(
                                'row-hover flex h-[29px] items-center gap-2 px-2.5',
                                inspected && 'bg-selection',
                              )}
                            >
                              <Checkbox
                                checked={included}
                                disabled={running || isBlocked}
                                onChange={(event) => onTogglePath(file.path, event.target.checked)}
                                label={`Include ${file.path}`}
                              />
                              <button
                                type="button"
                                onClick={() => onInspectFile(inspected ? null : file)}
                                className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
                                title="Click to inspect diff against destination"
                              >
                                <ChangeKindIcon change={meta.changeKind} label={meta.label} />
                                <PathLabel
                                  path={file.path}
                                  muted={!included}
                                  className="min-w-0 flex-1 text-xs"
                                />
                              </button>
                              <span className="flex shrink-0 items-center gap-1.5 text-[11px] text-muted">
                                <span
                                  className={cx('h-1.5 w-1.5 rounded-full', meta.dot)}
                                  aria-hidden="true"
                                />
                                {meta.label}
                              </span>
                            </li>
                          );
                        })}
                      </ul>
                    )}
                  </div>

                  {inspectedFile ? (
                    <div className="flex h-[260px] shrink-0 flex-col border-t border-line">
                      <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-3 py-1.5">
                        <div className="flex min-w-0 items-center gap-2">
                          <span className="text-[11px] font-semibold text-fg">Diff</span>
                          <PathLabel path={inspectedFile.path} className="min-w-0 text-[11px]" />
                        </div>
                        <div className="flex shrink-0 items-center gap-2">
                          <div
                            role="group"
                            aria-label="Diff view mode"
                            className="flex h-[21px] overflow-hidden rounded-sm border border-line bg-surface"
                          >
                            {(['unified', 'side'] as const).map((mode, idx) => (
                              <button
                                key={mode}
                                type="button"
                                onClick={() => setDiffMode(mode)}
                                className={cx(
                                  'px-1.5 text-[10px]',
                                  idx > 0 && 'border-l border-line',
                                  diffMode === mode
                                    ? 'bg-raised font-semibold text-fg'
                                    : 'text-muted hover:text-fg',
                                )}
                              >
                                {mode === 'unified' ? 'Unified' : 'Split'}
                              </button>
                            ))}
                          </div>
                          <button
                            type="button"
                            onClick={() => onInspectFile(null)}
                            className="text-[11px] text-muted hover:text-fg"
                          >
                            Close
                          </button>
                        </div>
                      </div>
                      <div className="min-h-0 flex-1 overflow-auto bg-surface">
                        <DiffViewer
                          diff={
                            fileDiff ? { ...fileDiff, sourceUrl: fileDiff.sourceUrl ?? '' } : null
                          }
                          loading={diffLoading}
                          error={diffError}
                          onRetry={onReloadDiff}
                          mode={diffMode}
                          onOpenExternal={onOpenExternal}
                        />
                      </div>
                    </div>
                  ) : null}
                </>
              ) : null}
            </Panel>
          </div>

          {/* Right column: blockers, dependencies, delivery, confirmation & run */}
          <div className="scroll-area min-h-0 space-y-3">
            <Panel
              title={
                direction === 'subsystem_to_main'
                  ? `${subsystemName} → Main`
                  : `Main → ${subsystemName}`
              }
              subtitle={
                plan
                  ? `${plan.destBranch} ← ${plan.delivery === 'push_pr' ? plan.branch : plan.sourceBranch}`
                  : undefined
              }
              bodyClassName="space-y-3 p-3"
            >
              {planLoading ? (
                <div className="flex items-center gap-1.5 text-[11px] text-muted">
                  <Spinner className="h-3 w-3" />
                  Updating preview…
                </div>
              ) : null}

              {plan ? (
                <>
                  <BlockerList blockers={plan.blockers} />
                  <WarningsList warnings={plan.warnings} />

                  <dl>
                    <KeyValue label="Changes">
                      <span className="mono tabular-nums">
                        +{counts.add} added · ~{counts.overwrite} overwritten · -{counts.delete}{' '}
                        deleted
                      </span>
                    </KeyValue>
                    <KeyValue label="Unchanged">
                      <span className="mono tabular-nums">{plan.unchangedCount}</span>
                    </KeyValue>
                  </dl>

                  {plan.dependencies.length > 0 ? (
                    <div className="space-y-1">
                      <span className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                        Dependencies ({plan.dependencies.length})
                      </span>
                      <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                        {plan.dependencies.map((dep) => (
                          <DependencyRow key={`${dep.section}:${dep.name}`} dependency={dep} />
                        ))}
                      </ul>
                    </div>
                  ) : null}

                  {plan.skipped.length > 0 ? <SkippedList skipped={plan.skipped} /> : null}

                  {direction === 'subsystem_to_main' ? (
                    <Field label="Delivery" htmlFor="reconcile-delivery">
                      <Select
                        ariaLabel="Delivery mode"
                        value={delivery}
                        onValueChange={(value) => onDeliveryChange(value as PromotionDelivery)}
                        options={DELIVERY_OPTIONS}
                        disabled={running}
                      />
                    </Field>
                  ) : (
                    <dl>
                      <KeyValue label="Delivery">
                        <span className="mono">
                          PR → {plan.destBranch} ({plan.branch})
                        </span>
                      </KeyValue>
                    </dl>
                  )}

                  {plan.delivery === 'worktree' ? (
                    <p className="text-[11px] text-muted">
                      Worktree delivery stages files in your checked-out working tree without
                      creating a commit.
                    </p>
                  ) : (
                    <Field
                      label="Commit message"
                      htmlFor="reconcile-message"
                      hint={`Blank uses "${defaultSubject}".`}
                    >
                      <input
                        id="reconcile-message"
                        type="text"
                        className="input"
                        placeholder={defaultSubject}
                        value={message}
                        disabled={running}
                        onChange={(event) => onMessageChange(event.target.value)}
                      />
                    </Field>
                  )}

                  <div className="rounded-md border border-line bg-raised p-2.5">
                    <Checkbox
                      checked={confirmed}
                      disabled={running || isBlocked || !hasChanges}
                      onChange={(event) => onConfirmedChange(event.target.checked)}
                      label={
                        counts.delete > 0
                          ? `Confirm full overwrite of ${includedCount} file(s) (including ${counts.delete} deletion(s)) into ${plan.destBranch} via ${plan.delivery}.`
                          : `Confirm mirror of ${includedCount} file(s) into ${plan.destBranch} via ${plan.delivery}.`
                      }
                    />
                  </div>

                  <Button
                    variant="primary"
                    icon={GitPullRequestArrow}
                    disabled={!canRun}
                    loading={running}
                    onClick={onRun}
                    className="w-full"
                  >
                    {direction === 'main_to_subsystem' || plan.delivery === 'push_pr'
                      ? `Open Reconcile PR targeting ${plan.destBranch}`
                      : plan.delivery === 'worktree'
                        ? `Stage Reconcile on ${plan.destBranch}`
                        : `Reconcile into ${plan.destBranch}`}
                  </Button>

                  {running || (runError && progressSteps.length > 0) ? (
                    <ExecutionProgressView
                      title="Reconciliation progress"
                      stages={RECONCILE_STAGES}
                      steps={progressSteps}
                      outcome={running ? 'running' : runError ? 'failed' : 'success'}
                    />
                  ) : null}

                  {runError ? <ErrorState compact error={runError} /> : null}
                </>
              ) : null}
            </Panel>
          </div>
        </div>
      )}
    </div>
  );
}

function OutcomePathList({ title, paths }: { title: string; paths: string[] }) {
  if (paths.length === 0) return null;
  return (
    <details className="group overflow-hidden rounded-md border border-line bg-surface">
      <summary className="flex h-[31px] cursor-pointer items-center justify-between gap-2 px-3 text-[11px] select-none hover:bg-hover">
        <span className="flex items-center gap-1.5 font-semibold tracking-wide text-muted uppercase">
          <ChevronRight
            className="h-3.5 w-3.5 text-disabled transition-transform group-open:rotate-90"
            aria-hidden="true"
          />
          {title}
        </span>
        <span className="mono text-muted tabular-nums">
          {paths.length} path{paths.length === 1 ? '' : 's'}
        </span>
      </summary>
      <ul className="max-h-48 divide-y divide-line overflow-auto border-t border-line">
        {paths.map((path) => (
          <li key={path} className="flex h-[27px] items-center px-3">
            <PathLabel path={path} className="min-w-0 flex-1 text-[11px]" />
          </li>
        ))}
      </ul>
    </details>
  );
}

function BlockerList({ blockers }: { blockers: DistributionBlocker[] }) {
  if (blockers.length === 0) return null;
  return (
    <div className="space-y-1.5">
      {blockers.map((blocker, index) => (
        <InlineAlert key={`${blocker.code}-${index}`} tone="danger" title={blocker.message}>
          {blocker.details ? (
            <pre className="mono max-h-36 overflow-auto whitespace-pre-wrap break-words text-[11px] text-muted">
              {blocker.details}
            </pre>
          ) : null}
        </InlineAlert>
      ))}
    </div>
  );
}

function WarningsList({ warnings }: { warnings: string[] }) {
  if (warnings.length === 0) return null;
  return (
    <ul className="space-y-1">
      {warnings.map((warning, index) => (
        <li key={`${warning}-${index}`} className="flex gap-1.5 text-[11px] text-muted">
          <TriangleAlert
            className="mt-px h-3 w-3 shrink-0 text-status-unknown"
            aria-hidden="true"
          />
          <span className="min-w-0 break-words">{warning}</span>
        </li>
      ))}
    </ul>
  );
}

function DependencyRow({ dependency }: { dependency: DistributionDependency }) {
  const from = dependency.fromSection
    ? `${dependency.fromSection}${dependency.fromSpec ? ` ${dependency.fromSpec}` : ''}`
    : 'not present';
  const to = `${dependency.section}${dependency.spec ? ` ${dependency.spec}` : ''}`;
  return (
    <li className="flex h-[27px] items-center gap-2 px-2.5 text-[11px]">
      <span className="mono w-[46px] shrink-0 uppercase text-muted">{dependency.action}</span>
      <span className="mono min-w-0 flex-1 truncate text-fg" title={dependency.name}>
        {dependency.name}
      </span>
      <span className="mono shrink-0 truncate text-muted" title={`${from} → ${to}`}>
        {from} → {to}
      </span>
    </li>
  );
}

function SkippedList({ skipped }: { skipped: ReconcileSkip[] }) {
  return (
    <details className="group">
      <summary className="flex h-[25px] cursor-pointer list-none items-center gap-1.5 text-[11px] text-muted">
        <ChevronDown
          className="h-3 w-3 transition-transform group-open:rotate-180"
          aria-hidden="true"
        />
        {skipped.length} path{skipped.length === 1 ? '' : 's'} skipped
      </summary>
      <ul className="mt-1 max-h-48 divide-y divide-line overflow-auto rounded-md border border-line bg-surface">
        {skipped.map((entry, index) => (
          <li
            key={`${entry.path}-${index}`}
            className="flex min-h-[25px] items-center gap-2 px-2.5 py-1"
          >
            <span className="min-w-0 flex-1">
              <PathLabel path={entry.path} muted className="text-[11px]" />
            </span>
            <span
              className="max-w-[55%] shrink-0 truncate text-[11px] text-muted"
              title={entry.reason}
            >
              {entry.reason}
            </span>
          </li>
        ))}
      </ul>
    </details>
  );
}

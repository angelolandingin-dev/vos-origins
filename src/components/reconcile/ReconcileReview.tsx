import { useMemo, useState } from 'react';
import {
  ArrowLeft,
  ChevronDown,
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
  progress: PromotionProgress | null;
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
  progress,
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
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);

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

  const copyCommand = (cmd: string) => {
    void navigator.clipboard.writeText(cmd).then(() => {
      setCopiedCommand(cmd);
      window.setTimeout(() => setCopiedCommand(null), 1500);
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
        <div className="scroll-area min-h-0 flex-1 p-4">
          <div className="mx-auto max-w-2xl">
            <Panel
              title={`${subsystemName} reconciliation`}
              subtitle={
                outcome.ok
                  ? outcome.pullRequest?.number != null
                    ? `Pull request #${outcome.pullRequest.number} ${outcome.pullRequest.state}`
                    : 'Completed'
                  : 'Reconciliation blocked'
              }
              actions={
                <Button variant="primary" size="sm" onClick={onDone}>
                  Done
                </Button>
              }
              bodyClassName="space-y-3 p-3"
            >
              <BlockerList blockers={outcome.blockers} />
              <WarningsList warnings={outcome.warnings} />
              {outcome.message ? <p className="text-xs text-fg">{outcome.message}</p> : null}

              <dl>
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
                <KeyValue label="Changes">
                  <span className="mono tabular-nums">
                    +{outcome.applied.length} added · ~{outcome.overwritten.length} overwritten · -
                    {outcome.deleted.length} deleted
                  </span>
                </KeyValue>
              </dl>

              {outcome.pullRequest?.url ? (
                <Button
                  variant="primary"
                  icon={ExternalLink}
                  onClick={() => {
                    if (outcome.pullRequest?.url) onOpenExternal(outcome.pullRequest.url);
                  }}
                >
                  Open Pull Request
                </Button>
              ) : null}

              {outcome.localCommands.length > 0 ? (
                <div className="space-y-1">
                  <span className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                    Next commands
                  </span>
                  <ul className="divide-y divide-line rounded-md border border-line bg-raised">
                    {outcome.localCommands.map((cmd) => (
                      <li
                        key={cmd}
                        className="flex items-center justify-between gap-2 px-2.5 py-1.5"
                      >
                        <code className="mono min-w-0 flex-1 truncate text-[11px] text-fg">
                          {cmd}
                        </code>
                        <Button
                          variant="ghost"
                          size="sm"
                          icon={Copy}
                          onClick={() => copyCommand(cmd)}
                        >
                          {copiedCommand === cmd ? 'Copied' : 'Copy'}
                        </Button>
                      </li>
                    ))}
                  </ul>
                </div>
              ) : null}
            </Panel>
          </div>
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

                  {running ? (
                    <div className="space-y-1" role="status" aria-live="polite">
                      <div className="flex items-center justify-between gap-2 text-[11px]">
                        <span className="flex min-w-0 items-center gap-1.5 text-fg">
                          <Spinner className="h-3 w-3" />
                          <span className="truncate">{progress?.label ?? 'Reconciling…'}</span>
                        </span>
                        {progress?.index != null && progress.total != null ? (
                          <span className="mono shrink-0 text-muted">
                            {progress.index}/{progress.total}
                          </span>
                        ) : null}
                      </div>
                      <div className="h-[3px] overflow-hidden rounded-full bg-line">
                        <div
                          className="h-full bg-accent transition-[width]"
                          style={{
                            width: `${
                              progress?.index && progress.total
                                ? Math.max(10, Math.round((progress.index / progress.total) * 100))
                                : 15
                            }%`,
                          }}
                        />
                      </div>
                    </div>
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

import { useEffect, useRef, useState } from 'react';
import {
  Check,
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
import {
  Button,
  Checkbox,
  Field,
  IconButton,
  InlineAlert,
  KeyValue,
  Modal,
  Select,
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
  onRemoveExcludedPath: (path: string) => void;
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
      key={JSON.stringify([props.sourceShas, props.targetBranch, props.delivery])}
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
  const counts = [
    `${replaced.length} overwritten`,
    `${deleted.length} deleted`,
    `${unchanged.length} unchanged`,
    `${excluded.length} excluded`,
  ].join(' · ');
  return (
    <section className="space-y-2">
      <div className="flex h-[29px] items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
          What changed
        </h3>
        <span className="mono text-right text-muted tabular-nums">{counts}</span>
      </div>
      {replaced.length > 0 ? (
        <p className="text-[11px] leading-relaxed text-muted">
          {replaced.length} path{replaced.length === 1 ? '' : 's'} overwritten with the
          subsystem&apos;s copy
          {flagged.length > 0
            ? ` (${flagged.length} had diverged in the main system — review before merging)`
            : ''}
          . The overwrite is recorded in the commit trailers.
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
  const infos = findings.filter((finding) => finding.level === 'info');
  if (findings.length === 0) return null;
  return (
    <div className="space-y-2">
      <ul className="space-y-2">
        {alerts.map((finding) => (
          <li key={finding.code}>
            <InlineAlert
              tone={finding.level === 'error' ? 'danger' : 'warning'}
              title={finding.message}
            >
              {finding.remedy ? <p>{finding.remedy}</p> : null}
            </InlineAlert>
          </li>
        ))}
      </ul>
      {infos.length > 0 ? (
        <InlineAlert tone="info" title={`${infos.length} note${infos.length === 1 ? '' : 's'}`}>
          <ul className="list-disc space-y-1 pl-4">
            {infos.map((finding) => (
              <li key={finding.code}>
                <p>{finding.message}</p>
                {finding.remedy ? <p className="text-muted">{finding.remedy}</p> : null}
              </li>
            ))}
          </ul>
        </InlineAlert>
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
  onRemoveExcludedPath,
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
  const isDirect = delivery === 'local_direct';
  const isWorktree = delivery === 'worktree';
  const isLocal = delivery === 'local';
  const isPushPr = delivery === 'push_pr';
  const deliveryHint = isDirect
    ? `The commit lands on your local ${targetBranch}; pushing is yours to do.`
    : isWorktree
      ? `Stages the files into your working tree on ${targetBranch} — no commit, no push.`
      : isLocal
        ? 'Creates a branch + commit in your clone (you push it yourself).'
        : 'Pushes the integration branch and opens a PR (explicit opt-in).';

  const canDispatch =
    !dispatching &&
    acknowledged &&
    sourceShas.length > 0 &&
    message.trim().length > 0 &&
    Boolean(subsystem) &&
    Boolean(selectedBranch?.allowed) &&
    (!cumulative || baseSha.trim().length > 0);

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
      size="lg"
      title="Promote commits to a main-system branch"
      description={subsystem ? subsystem.name : 'Select a subsystem first.'}
      footer={
        result?.accepted ? (
          <Button variant="secondary" onClick={onClose}>
            Close
          </Button>
        ) : (
          <>
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
                      : 'Create integration PR request'}
            </Button>
          </>
        )
      }
    >
      {result?.accepted ? (
        <div className="space-y-4">
          <div ref={outcomeRef}>
            <InlineAlert
              tone="success"
              title={
                result.delivery === 'local_direct'
                  ? `Committed to ${result.targetBranch} locally — push when ready`
                  : result.delivery === 'worktree'
                    ? `Staged into working tree on ${result.targetBranch} — no commit, no push`
                    : result.delivery === 'local'
                      ? 'Local branch + commit created'
                      : result.prUrl
                        ? `Integration PR #${result.prNumber ?? '?'} opened`
                        : 'Promotion request accepted'
              }
            >
              <p>{result.message}</p>
            </InlineAlert>
          </div>

          <dl className="divide-y divide-line border-y border-line px-1">
            <KeyValue
              label={
                result.delivery === 'local_direct' || result.delivery === 'worktree'
                  ? 'Target branch'
                  : 'Integration branch'
              }
            >
              <span className="mono text-right">{result.branch}</span>
            </KeyValue>
            <KeyValue label="Requested at">
              <span className="mono text-right">{result.requestedAt}</span>
            </KeyValue>
          </dl>

          {result.delivery === 'worktree' ? (
            <p className="text-[11px] leading-relaxed text-muted">
              Nothing was committed or pushed, and integration status is unchanged.
            </p>
          ) : null}

          {(result.delivery === 'local' ||
            result.delivery === 'local_direct' ||
            result.delivery === 'worktree') &&
          result.nextCommands.length > 0 ? (
            <section className="space-y-2">
              <div className="flex h-[29px] items-center justify-between gap-2">
                <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Next commands (run in your clone)
                </h3>
                <Button variant="secondary" size="sm" onClick={copyNextCommands}>
                  {copied ? 'Copied' : 'Copy'}
                </Button>
              </div>
              <pre className="mono overflow-x-auto rounded-md border border-line bg-raised p-2 text-[11px] leading-relaxed whitespace-pre-wrap">
                {result.nextCommands.join('\n')}
              </pre>
            </section>
          ) : null}

          {result.prUrl ? (
            <Button
              variant="secondary"
              size="sm"
              icon={ExternalLink}
              onClick={() => onOpenExternal(result.prUrl ?? '')}
            >
              Open the integration PR on GitHub
            </Button>
          ) : null}

          <ResultChanges
            replaced={result.replaced ?? []}
            flagged={result.flagged ?? []}
            deleted={result.deleted ?? []}
            unchanged={result.unchanged ?? []}
            excluded={result.excluded ?? []}
          />

          <ResultNotes findings={result.findings} />
        </div>
      ) : (
        <div className="space-y-5">
          <section className="space-y-2">
            <div className="flex h-[29px] items-center justify-between gap-2">
              <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                Source commits
              </h3>
              <span className="mono text-right text-muted">{sourceShas.length}</span>
            </div>
            {sourceShas.length === 0 ? (
              <InlineAlert tone="warning" title="No commits selected">
                Tick one or more eligible commits in the commit list first.
              </InlineAlert>
            ) : (
              <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
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

          <section className="space-y-2 rounded-md border border-line bg-surface p-3">
            <p className="text-[11px] leading-relaxed text-muted">
              Promotion is a copy: the subsystem&apos;s version of every selected file always wins.
              Unsafe paths, secrets, main-owned files, and dependency failures still block the
              promotion.
            </p>
            <CommitFileList
              preflight={preflight}
              preflightLoading={preflightLoading}
              excludedPaths={excludedPaths}
              onAddExcludedPath={onAddExcludedPath}
              onRemoveExcludedPath={onRemoveExcludedPath}
            />
          </section>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Delivery" hint={deliveryHint}>
              <Select
                ariaLabel="Promotion delivery"
                value={delivery}
                onValueChange={(next) => onDeliveryChange(next as PromotionDelivery)}
                options={[
                  { value: 'local_direct', label: `Commit to local ${targetBranch} (recommended)` },
                  { value: 'worktree', label: 'Add to working tree (no commit)' },
                  { value: 'local', label: 'Local integration branch' },
                  { value: 'push_pr', label: 'Push + PR' },
                ]}
              />
            </Field>

            <Field label="Mode">
              <Select
                ariaLabel="Promotion mode"
                value={mode}
                onValueChange={(next) => onModeChange(next as PromotionMode)}
                options={[
                  { value: 'individual', label: 'Individual (recommended)' },
                  { value: 'cumulative', label: 'Cumulative (requires baseline)' },
                ]}
              />
            </Field>

            <div className="sm:col-span-2">
              <Field label="Commit message" required>
                <textarea
                  className="input min-h-[76px] py-2"
                  aria-label="Commit message"
                  rows={3}
                  required
                  value={message}
                  onChange={(event) => onMessageChange(event.target.value)}
                  placeholder="Promote hr-management a81f2c3..f12b301"
                />
              </Field>
            </div>

            <div className={cumulative ? undefined : 'sm:col-span-2'}>
              <Field
                label="Target main-system branch"
                required
                hint="Only policy-allowed branches."
              >
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
          </div>

          <section className="space-y-2">
            <h3 className="flex h-[29px] items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
              <Package className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
              Dependency impact
            </h3>
            {dependenciesLoading ? (
              <p className="text-[11px] text-muted">Loading dependency changes…</p>
            ) : dependencies.length === 0 ? (
              <p className="text-[11px] text-muted">No manifest changes in this selection.</p>
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
            <p className="text-[11px] text-muted">Checking mergeability…</p>
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
                    {replaced.length} path{replaced.length === 1 ? '' : 's'} the main system changed
                    will be overwritten with the subsystem&apos;s version.
                  </p>
                ) : preflight.conflict > 0 ? (
                  <p>Resolve, exclude, or drop the conflicting commit, then re-promote.</p>
                ) : (
                  <p>Ready to promote.</p>
                )}
              </InlineAlert>
              <ExpandablePathList title="Will be overwritten" paths={replaced} />
              {revertsNewerFiles.length > 0 ? (
                <InlineAlert
                  tone="warning"
                  title={`Reverts newer work — ${revertsNewerFiles.length} path${revertsNewerFiles.length === 1 ? '' : 's'}`}
                >
                  <p>
                    Main already holds newer subsystem content here. Promoting moves it back —
                    proceed to overwrite deliberately, or exclude the path.
                  </p>
                </InlineAlert>
              ) : null}
              {revertsNewerFiles.length > 0 ? (
                <ul className="divide-y divide-line overflow-hidden overflow-y-auto rounded-md border border-line bg-surface max-h-40">
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
                <ul className="divide-y divide-line overflow-hidden overflow-y-auto rounded-md border border-line bg-surface max-h-40">
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
              ) : null}
              <ExpandablePathList title="Excluded" paths={preflightExcluded} />
            </section>
          ) : null}

          <section className="space-y-2 rounded-md border border-line bg-surface p-3">
            <p className="truncate text-xs text-fg">
              {isDirect ? (
                <span>Commits onto {targetBranch} locally — no push, no PR.</span>
              ) : isWorktree ? (
                <span>
                  Stages files into your working tree on {targetBranch} — no commit, no push.
                </span>
              ) : isLocal ? (
                <span>Creates a local branch + commit — no push, no PR.</span>
              ) : (
                <span>Pushes the integration branch and opens a PR against {targetBranch}.</span>
              )}
            </p>
            <label className="flex items-start gap-2 text-xs text-fg">
              <Checkbox
                checked={acknowledged}
                onChange={(event) => setAcknowledged(event.target.checked)}
                label={
                  isDirect
                    ? 'Acknowledge that this commits directly onto my local target branch (no push, no PR)'
                    : isWorktree
                      ? 'Acknowledge that this stages files into my working tree (no commit, no push)'
                      : isLocal
                        ? 'Acknowledge that this creates a local branch + commit (no push, no PR)'
                        : 'Acknowledge that this creates an integration PR against the selected main-system branch'
                }
                className="mt-0.5"
              />
              {isDirect ? (
                <span>
                  I understand this commits the selected files onto my local{' '}
                  <span className="mono">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  , with no push and no PR — I push it myself.
                </span>
              ) : isWorktree ? (
                <span>
                  I understand this copies the selected files into my working tree on{' '}
                  <span className="mono">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  and stages them, with no commit and no push — {targetBranch} must be checked out
                  and my tree clean.
                </span>
              ) : isLocal ? (
                <span>
                  I understand this creates a local branch + commit in my clone for{' '}
                  <span className="mono">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  , with no push and no PR — I push it myself.
                </span>
              ) : (
                <span>
                  I understand this opens a reviewed integration PR against{' '}
                  <span className="mono">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  , not a push or deploy.
                </span>
              )}
            </label>
            {replaced.length > 0 ? (
              <p className="text-[11px] leading-relaxed text-muted">
                This overwrites {replaced.length} path
                {replaced.length === 1 ? '' : 's'} the main system changed with the subsystem&apos;s
                version.
              </p>
            ) : null}
            {acknowledged ? (
              <p className="flex items-center gap-1.5 text-[11px] text-status-applied-ink">
                <Check className="h-3.5 w-3.5" aria-hidden="true" />
                Confirmation recorded for this request.
              </p>
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
          </section>
        </div>
      )}
      {progress.length > 0 ? (
        <div className="mt-4">
          <PromotionProgressView steps={progress} outcome={progressOutcome} />
        </div>
      ) : null}
    </Modal>
  );
}

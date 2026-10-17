import { useState } from 'react';
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
  SubsystemSummary,
  TargetBranchList,
} from '../api/types';
import { CommitStatusBadge } from './StatusBadge';
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
  allowGaps: boolean;
  onAllowGapsChange: (allowGaps: boolean) => void;
  excludedPaths: string[];
  onAddExcludedPath: (path: string) => void;
  onRemoveExcludedPath: (path: string) => void;
  dependencies: DependencyChange[];
  dependenciesLoading: boolean;
  preflight: PromotionPreflight | null;
  preflightLoading: boolean;
  dispatching: boolean;
  error: ApiError | null;
  result: PromotionDispatchResult | null;
  onDispatch: () => void;
  onOpenExternal: (url: string) => void;
  onViewRuns: () => void;
}

export function PromotionDialog(props: PromotionDialogProps) {
  // Remount while closed so the acknowledgement resets without an effect.
  if (!props.open) return null;
  return (
    <PromotionDialogContent
      key={JSON.stringify([props.sourceShas, props.targetBranch, props.delivery])}
      {...props}
    />
  );
}

function preflightSummary(preflight: PromotionPreflight): string {
  const parts = [
    `${preflight.clean} clean`,
    `${preflight.merged} merged`,
    `${preflight.conflict} conflict`,
  ];
  if (preflight.skipped > 0) parts.push(`${preflight.skipped} skipped`);
  return `${preflight.files.length} files: ${parts.join(' · ')}`;
}

function PathList({ title, paths }: { title: string; paths: string[] }) {
  if (paths.length === 0) return null;
  return (
    <section className="space-y-1">
      <div className="flex h-[29px] items-center justify-between gap-2">
        <h4 className="text-[11px] font-semibold tracking-wide text-muted uppercase">{title}</h4>
        <span className="mono text-right text-muted tabular-nums">{paths.length}</span>
      </div>
      <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
        {paths.map((path) => (
          <li
            key={path}
            className="mono flex h-[29px] items-center truncate px-2.5 text-[11px] text-muted tabular-nums"
            title={path}
          >
            <span className="truncate">{path}</span>
          </li>
        ))}
      </ul>
    </section>
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
  allowGaps,
  onAllowGapsChange,
  excludedPaths,
  onAddExcludedPath,
  onRemoveExcludedPath,
  dependencies,
  dependenciesLoading,
  preflight,
  preflightLoading,
  dispatching,
  error,
  result,
  onDispatch,
  onOpenExternal,
  onViewRuns,
}: PromotionDialogProps) {
  const [acknowledged, setAcknowledged] = useState(false);
  const [copied, setCopied] = useState(false);
  const [newExcludedPath, setNewExcludedPath] = useState('');

  const branches = targetBranches?.branches ?? [];
  const selectedBranch = branches.find((branch) => branch.name === targetBranch) ?? null;
  const conflicting = dependencies.filter((dependency) => dependency.conflict);
  const cumulative = mode === 'cumulative';
  const multiManifest = commits.filter((commit) => commit.touchesManifest).length > 1;
  const replaced = preflight?.replaced ?? [];
  const deleted = preflight?.deleted ?? [];
  const unchanged = preflight?.unchanged ?? [];
  const flagged = preflight?.flagged ?? [];
  const preflightExcluded = preflight?.excluded ?? [];
  const skippedFiles = (preflight?.files ?? [])
    .filter((file) => file.outcome === 'skipped')
    .map((file) => file.path);
  const refusedPaths = Array.from(new Set([...(preflight?.conflicts ?? []), ...skippedFiles]));
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
          <>
            <Button variant="secondary" onClick={onClose}>
              Close
            </Button>
            {result.delivery === 'push_pr' ? (
              <Button variant="primary" icon={GitPullRequestArrow} onClick={onViewRuns}>
                View workflow runs
              </Button>
            ) : null}
          </>
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
              <p className="text-[11px] leading-relaxed text-muted">
                {result.delivery === 'local_direct'
                  ? 'Not pushed — the commit is on your local branch.'
                  : result.delivery === 'worktree'
                    ? 'Staged only — the commit is yours to make.'
                    : 'Not pushed — you push the branch yourself.'}
              </p>
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

          {(result.flagged ?? []).length > 0 ? (
            <InlineAlert
              tone="warning"
              title={`${(result.flagged ?? []).length} path${(result.flagged ?? []).length === 1 ? '' : 's'} also changed in the main system by something else`}
            >
              <ul className="mono space-y-0.5 text-[11px] break-all">
                {(result.flagged ?? []).map((path) => (
                  <li key={path}>{path}</li>
                ))}
              </ul>
            </InlineAlert>
          ) : null}
          <PathList title="Replaced" paths={result.replaced ?? []} />
          <PathList title="Deleted" paths={result.deleted ?? []} />
          <PathList title="Unchanged" paths={result.unchanged ?? []} />
          <PathList title="Excluded" paths={result.excluded ?? []} />

          <ul className="space-y-2">
            {result.findings.map((finding) => (
              <li key={finding.code}>
                <InlineAlert
                  tone={
                    finding.level === 'error'
                      ? 'danger'
                      : finding.level === 'warning'
                        ? 'warning'
                        : 'info'
                  }
                  title={finding.message}
                >
                  {finding.remedy ? <p>{finding.remedy}</p> : null}
                </InlineAlert>
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div className="space-y-5">
          {error ? (
            <InlineAlert tone="danger" title={error.message}>
              {error.remedy ? <p>{error.remedy}</p> : null}
              {error.details ? (
                <p className="mono mt-1 text-[11px] whitespace-pre-wrap text-status-blocked-ink">
                  {error.details}
                </p>
              ) : null}
            </InlineAlert>
          ) : null}

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
            <label className="flex items-start gap-2 text-xs text-fg">
              <Checkbox
                checked={allowGaps}
                onChange={(event) => onAllowGapsChange(event.target.checked)}
                label="Allow gaps — a skipped commit's changes are included"
                className="mt-0.5"
              />
              <span>Allow gaps — a skipped commit&apos;s changes are included</span>
            </label>
            <div className="space-y-1.5">
              <div className="flex h-[29px] items-center justify-between gap-2">
                <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Excluded paths
                </h3>
                <span className="mono text-right text-muted tabular-nums">
                  {excludedPaths.length}
                </span>
              </div>
              {excludedPaths.length === 0 ? (
                <p className="text-[11px] text-muted">No paths excluded.</p>
              ) : (
                <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                  {excludedPaths.map((path) => (
                    <li key={path} className="flex h-[29px] items-center gap-2 px-2.5">
                      <span
                        className="mono min-w-0 flex-1 truncate text-[11px] text-muted"
                        title={path}
                      >
                        {path}
                      </span>
                      <IconButton
                        icon={X}
                        label={`Stop excluding ${path}`}
                        onClick={() => onRemoveExcludedPath(path)}
                        className="h-[21px] w-[21px]"
                      />
                    </li>
                  ))}
                </ul>
              )}
              <div className="flex gap-1.5">
                <input
                  className="input mono h-[25px] text-[11px]"
                  value={newExcludedPath}
                  onChange={(event) => setNewExcludedPath(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      onAddExcludedPath(newExcludedPath);
                      setNewExcludedPath('');
                    }
                  }}
                  placeholder="path/to/file or directory/prefix"
                  aria-label="Add an excluded path"
                  spellCheck={false}
                  autoComplete="off"
                />
                <Button
                  size="sm"
                  variant="secondary"
                  disabled={!newExcludedPath.trim()}
                  onClick={() => {
                    onAddExcludedPath(newExcludedPath);
                    setNewExcludedPath('');
                  }}
                >
                  Exclude
                </Button>
              </div>
            </div>
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

            <Field label="Commit message" required>
              <textarea
                className="input h-auto min-h-[50px] py-1.5"
                aria-label="Commit message"
                rows={2}
                required
                value={message}
                onChange={(event) => onMessageChange(event.target.value)}
                placeholder="Promote hr-management a81f2c3..f12b301"
              />
            </Field>

            <Field label="Target main-system branch" required hint="Only policy-allowed branches.">
              <Select
                ariaLabel="Target main-system branch"
                value={targetBranch}
                onValueChange={(next) => onTargetBranchChange(next)}
                options={branches.map((branch) => ({
                  value: branch.name,
                  label: `${branch.name}${branch.isDefault ? ' (default)' : ''}${branch.allowed ? '' : ' — not permitted'}`,
                  disabled: !branch.allowed,
                  title: branch.allowed
                    ? `tip ${branch.headSha.slice(0, 7)} · ${branch.openPrCount} open integration PR(s)`
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

          {selectedBranch ? (
            <dl className="divide-y divide-line border-y border-line px-1">
              <KeyValue label="Branch tip">
                <span className="mono text-right">{selectedBranch.headSha.slice(0, 7)}</span>
              </KeyValue>
              <KeyValue label="Protection">
                {selectedBranch.protected ? 'Protected — PR required' : 'Unprotected'}
              </KeyValue>
              <KeyValue label="Open integration PRs">
                <span className="mono text-right">{selectedBranch.openPrCount}</span>
              </KeyValue>
            </dl>
          ) : null}

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
                {preflight.conflict > 0 ? (
                  <p>Resolve, exclude, or drop the conflicting commit, then re-promote.</p>
                ) : (
                  <p>Ready to promote.</p>
                )}
              </InlineAlert>
              {refusedPaths.length > 0 ? (
                <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                  {refusedPaths.map((path) => (
                    <li key={path} className="flex h-[29px] items-center gap-2 px-2.5">
                      <TriangleAlert
                        className="h-3 w-3 shrink-0 text-status-blocked"
                        aria-hidden="true"
                      />
                      <span
                        className="mono min-w-0 flex-1 truncate text-[11px] text-fg"
                        title={path}
                      >
                        {path}
                      </span>
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
              {flagged.length > 0 ? (
                <InlineAlert
                  tone="warning"
                  title={`${flagged.length} path${flagged.length === 1 ? '' : 's'} also changed in the main system by something else`}
                >
                  <ul className="mono space-y-0.5 text-[11px] break-all">
                    {flagged.map((path) => (
                      <li key={path}>{path}</li>
                    ))}
                  </ul>
                </InlineAlert>
              ) : null}
              <PathList title="Replaced" paths={replaced} />
              <PathList title="Deleted" paths={deleted} />
              <PathList title="Unchanged" paths={unchanged} />
              <PathList title="Excluded" paths={preflightExcluded} />
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
            {acknowledged ? (
              <p className="flex items-center gap-1.5 text-[11px] text-status-applied-ink">
                <Check className="h-3.5 w-3.5" aria-hidden="true" />
                Confirmation recorded for this request.
              </p>
            ) : null}
          </section>
        </div>
      )}
    </Modal>
  );
}

import { useState } from 'react';
import {
  Check,
  ExternalLink,
  GitBranch,
  GitPullRequestArrow,
  Package,
  ShieldAlert,
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
  PromotionOnConflict,
  PromotionPreflight,
  SubsystemSummary,
  TargetBranchList,
} from '../api/types';
import { CommitStatusBadge, StatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import { Button, Checkbox, Field, InlineAlert, Modal, Select } from './ui/primitives';

/**
 * Promotion dialog (spec §5.1). Local-first: `local` creates a branch + commit
 * in the user's clone (no push, no PR); `push_pr` is the explicit opt-in that
 * opens an integration PR against main-system/<branch>.
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
  onConflict: PromotionOnConflict;
  onOnConflictChange: (onConflict: PromotionOnConflict) => void;
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
      key={JSON.stringify([props.sourceShas, props.targetBranch, props.delivery, props.onConflict])}
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
  onConflict,
  onOnConflictChange,
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

  const branches = targetBranches?.branches ?? [];
  const selectedBranch = branches.find((branch) => branch.name === targetBranch) ?? null;
  const conflicting = dependencies.filter((dependency) => dependency.conflict);
  const cumulative = mode === 'cumulative';
  const multiManifest = commits.filter((commit) => commit.touchesManifest).length > 1;
  const isDirect = delivery === 'local_direct';
  const isLocal = delivery === 'local';
  const isPushPr = delivery === 'push_pr';
  const deliveryHint = isDirect
    ? `The commit lands on your local ${targetBranch}; pushing is yours to do.`
    : isLocal
      ? 'Creates a branch + commit in your clone (you push it yourself).'
      : 'Pushes the integration branch and opens a PR (explicit opt-in).';

  const canDispatch =
    !dispatching &&
    acknowledged &&
    sourceShas.length > 0 &&
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
      description={
        subsystem
          ? `Source: ${subsystem.name} (${subsystem.repository}) · source branch ${subsystem.sourceBranch}`
          : 'Select a subsystem first.'
      }
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
                : result.delivery === 'local'
                  ? 'Local branch + commit created'
                  : result.prUrl
                    ? `Integration PR #${result.prNumber ?? '?'} opened`
                    : 'Promotion request accepted'
            }
          >
            <p>{result.message}</p>
          </InlineAlert>

          <dl className="grid gap-2 sm:grid-cols-2">
            <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-3">
              <dt className="text-2xs uppercase tracking-wide text-slate-500">
                {result.delivery === 'local_direct' ? 'Target branch' : 'Integration branch'}
              </dt>
              <dd className="mono mt-1 text-xs text-slate-200">{result.branch}</dd>
            </div>
            <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-3">
              <dt className="text-2xs uppercase tracking-wide text-slate-500">Requested at</dt>
              <dd className="mono mt-1 text-xs text-slate-200">{result.requestedAt}</dd>
            </div>
          </dl>

          {(result.delivery === 'local' || result.delivery === 'local_direct') &&
          result.nextCommands.length > 0 ? (
            <section className="space-y-2 rounded-lg border border-ink-700/60 bg-ink-850/50 p-3">
              <div className="flex items-center justify-between gap-2">
                <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-400">
                  Next commands (run in your clone)
                </h3>
                <Button variant="secondary" size="sm" onClick={copyNextCommands}>
                  {copied ? 'Copied' : 'Copy'}
                </Button>
              </div>
              <pre className="mono overflow-x-auto whitespace-pre-wrap rounded-md bg-ink-900/70 p-2 text-2xs leading-relaxed text-slate-200">
                {result.nextCommands.join('\n')}
              </pre>
              <p className="text-2xs text-slate-500">
                {result.delivery === 'local_direct'
                  ? 'Nothing was pushed and no PR was opened — the commit is on your local branch; push when ready.'
                  : 'Nothing was pushed and no PR was opened — you push the branch yourself.'}
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
                <p className="mono mt-1 whitespace-pre-wrap text-2xs text-rose-200/80">
                  {error.details}
                </p>
              ) : null}
            </InlineAlert>
          ) : null}

          <section className="space-y-2">
            <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-400">
              Source commits ({sourceShas.length})
            </h3>
            {sourceShas.length === 0 ? (
              <InlineAlert tone="warning" title="No commits selected">
                Tick one or more eligible commits in the commit list first. Only these exact SHAs
                are promoted.
              </InlineAlert>
            ) : (
              <ul className="divide-y divide-ink-700/60 overflow-hidden rounded-lg border border-ink-700/60">
                {sourceShas.map((sha) => {
                  const commit = commits.find((candidate) => candidate.sha === sha);
                  return (
                    <li key={sha} className="flex items-center gap-3 bg-ink-850/50 px-3 py-2">
                      <span className="mono shrink-0 text-xs text-brand-300" title={sha}>
                        {sha.slice(0, 7)}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-xs text-slate-200">
                        {commitsLoading && !commit
                          ? 'Loading…'
                          : (commit?.subject ?? 'Unknown commit')}
                      </span>
                      {commit ? <CommitStatusBadge status={commit.status} /> : null}
                      <button
                        type="button"
                        onClick={() => onRemoveSha(sha)}
                        aria-label={`Remove ${sha.slice(0, 7)} from this promotion`}
                        title="Remove from this promotion"
                        className="rounded-sm p-1 text-slate-400 hover:bg-ink-700 hover:text-white"
                      >
                        <X className="h-3.5 w-3.5" aria-hidden="true" />
                      </button>
                    </li>
                  );
                })}
              </ul>
            )}
            <p className="text-2xs leading-relaxed text-slate-500">
              Full 40-character SHAs are sent to the engine. Commits are applied in the order
              listed; unselected earlier or later source commits are never imported.
            </p>
          </section>

          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="Delivery" hint={deliveryHint}>
              <Select
                aria-label="Promotion delivery"
                value={delivery}
                onChange={(event) => onDeliveryChange(event.target.value as PromotionDelivery)}
                options={[
                  { value: 'local_direct', label: `Commit to local ${targetBranch} (recommended)` },
                  { value: 'local', label: 'Local integration branch' },
                  { value: 'push_pr', label: 'Push + PR' },
                ]}
              />
            </Field>

            <Field
              label="Mode"
              hint="Individual applies each selected commit's first-parent delta. Cumulative is not part of the MVP and requires an explicit baseline."
            >
              <Select
                aria-label="Promotion mode"
                value={mode}
                onChange={(event) => onModeChange(event.target.value as PromotionMode)}
                options={[
                  { value: 'individual', label: 'Individual (recommended)' },
                  { value: 'cumulative', label: 'Cumulative (requires baseline)' },
                ]}
              />
            </Field>

            <Field
              label="On conflict"
              hint="Fail stops the promotion. Take the subsystem's version applies it instead — the replaced paths are reported in the result."
            >
              <Select
                aria-label="On conflict"
                value={onConflict}
                onChange={(event) => onOnConflictChange(event.target.value as PromotionOnConflict)}
                options={[
                  { value: 'fail', label: 'Fail (recommended)' },
                  { value: 'take_subsystem', label: "Take the subsystem's version" },
                ]}
              />
            </Field>

            <Field
              label="Target main-system branch"
              required
              hint="Only existing branches allowed by .integration/config.json are selectable."
            >
              <Select
                aria-label="Target main-system branch"
                value={targetBranch}
                onChange={(event) => onTargetBranchChange(event.target.value)}
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
            <Field
              label="Cumulative baseline SHA"
              required
              hint="The exactly verified baseline. The workflow diffs baseline..endpoint, which intentionally includes the intervening source commits."
            >
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
            <div className="grid gap-2 sm:grid-cols-3">
              <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-3">
                <p className="text-2xs uppercase tracking-wide text-slate-500">Branch tip</p>
                <p className="mono mt-1 text-xs text-slate-200">
                  {selectedBranch.headSha.slice(0, 7)}
                </p>
              </div>
              <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-3">
                <p className="text-2xs uppercase tracking-wide text-slate-500">Protection</p>
                <p className="mt-1 text-xs text-slate-200">
                  {selectedBranch.protected ? 'Protected — PR required' : 'Unprotected'}
                </p>
              </div>
              <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-3">
                <p className="text-2xs uppercase tracking-wide text-slate-500">
                  Open integration PRs
                </p>
                <p className="mt-1 text-xs text-slate-200">{selectedBranch.openPrCount}</p>
              </div>
            </div>
          ) : null}

          <section className="space-y-2">
            <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-slate-400">
              <Package className="h-3.5 w-3.5" aria-hidden="true" />
              Dependency impact
            </h3>
            {dependenciesLoading ? (
              <p className="text-2xs text-slate-500">Loading dependency deltas…</p>
            ) : dependencies.length === 0 ? (
              <p className="text-2xs text-slate-500">
                No manifest changes are part of the selected delta. Nothing is added to the target
                lockfile.
              </p>
            ) : (
              <ul className="space-y-1.5">
                {dependencies.map((dependency) => (
                  <li
                    key={`${dependency.section}-${dependency.name}`}
                    className={cx(
                      'flex items-center gap-2 rounded-lg border px-3 py-2 text-2xs',
                      dependency.conflict
                        ? 'border-rose-900/70 bg-rose-950/30 text-rose-200'
                        : 'border-ink-700/60 bg-ink-850/60 text-slate-300',
                    )}
                  >
                    <span className="mono text-slate-100">{dependency.name}</span>
                    <span className="chip bg-ink-800 text-slate-300">{dependency.section}</span>
                    <span className="text-slate-500">{dependency.fromRange ?? '(absent)'}</span>
                    <span aria-hidden="true">→</span>
                    <span>{dependency.toRange ?? '(removed)'}</span>
                    <span className="ml-auto uppercase tracking-wide text-slate-500">
                      {dependency.change}
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
                Manifests are merged as an explicit delta and the target lockfile is regenerated.
                Review the combined result in the integration PR before merging.
              </InlineAlert>
            ) : null}

            {conflicting.length > 0 ? (
              <InlineAlert tone="danger" title="Incompatible dependency range in this selection">
                The promotion fails closed instead of force-installing. Resolve the shared range in
                the main-system manifest, or drop the conflicting commit from the selection.
              </InlineAlert>
            ) : null}
          </section>

          {preflightLoading ? (
            <p className="text-2xs text-slate-500">Checking mergeability…</p>
          ) : preflight && preflight.files.length > 0 ? (
            <section className="space-y-2">
              <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-400">
                Mergeability preflight
              </h3>
              <InlineAlert
                tone={preflight.conflict > 0 ? 'warning' : 'info'}
                title={preflightSummary(preflight)}
              >
                {preflight.conflict > 0 ? (
                  <>
                    <p>Promoting now will fail on these files until they are resolved.</p>
                    <ul className="mono space-y-0.5 break-all text-2xs">
                      {preflight.conflicts.map((path) => (
                        <li key={path}>{path}</li>
                      ))}
                    </ul>
                    <p>
                      Resolve the divergence in the main system, or drop the conflicting commit from
                      the selection, then re-promote. Dependency manifests merge and lockfiles
                      regenerate instead of conflicting.
                    </p>
                  </>
                ) : (
                  <p>
                    Every selected delta applies without a merge conflict. Dependency manifests
                    merge and lockfiles regenerate.
                  </p>
                )}
              </InlineAlert>
            </section>
          ) : null}

          <InlineAlert tone="info" title="Only the ticked commits move">
            A commit can depend on code or packages added by an unselected commit. Such dependencies
            stay behind unless you add them explicitly. Main-system tests alone are not proof of
            correctness.
          </InlineAlert>

          <section className="space-y-2 rounded-lg border border-brand-700/50 bg-brand-950/30 p-3">
            <p className="flex items-start gap-2 text-xs leading-relaxed text-brand-100">
              <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-brand-300" aria-hidden="true" />
              {isDirect ? (
                <span>
                  Confirming this commits the delta{' '}
                  <strong>straight onto your local {targetBranch}</strong>; pushing is yours to do (
                  <strong>no push, no PR</strong>). The tool builds the commit from the target tip
                  in a temporary worktree and fast-forwards your local branch — uncommitted work is
                  never touched (a dirty checkout refuses instead, and a moved target tip fails
                  closed).
                </span>
              ) : isLocal ? (
                <span>
                  Confirming this creates a <strong>local branch + commit in your clone</strong>;
                  you push it yourself (<strong>no push, no PR</strong>). The tool builds the
                  integration branch from the target tip in a temporary worktree (your working tree,
                  index and current branch are left alone) and keeps the branch for you to push.
                </span>
              ) : (
                <span>
                  Confirming this creates an{' '}
                  <strong>integration PR against main-system/{targetBranch}</strong>. It is
                  <strong> not</strong> a direct push to a protected branch and <strong>not</strong>{' '}
                  a production deploy. The tool builds the integration branch from the target tip
                  and re-validates the registry, ancestry and branch policy before it pushes.
                </span>
              )}
            </p>
            <p className="text-2xs leading-relaxed text-brand-100/80">
              Files the subsystem adds over an existing main-system file are applied as updates when
              the subsystem's copy is newer; a newer main-system copy blocks the promotion.
            </p>
            <label className="flex items-start gap-2 text-xs text-slate-200">
              <Checkbox
                checked={acknowledged}
                onChange={(event) => setAcknowledged(event.target.checked)}
                label={
                  isDirect
                    ? 'Acknowledge that this commits directly onto my local target branch (no push, no PR)'
                    : isLocal
                      ? 'Acknowledge that this creates a local branch + commit (no push, no PR)'
                      : 'Acknowledge that this creates an integration PR against the selected main-system branch'
                }
                className="mt-0.5"
              />
              {isDirect ? (
                <span>
                  I understand this commits the delta onto my local{' '}
                  <span className="mono text-slate-100">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  , with no push and no PR — I push it myself.
                </span>
              ) : isLocal ? (
                <span>
                  I understand this creates a local branch + commit in my clone for{' '}
                  <span className="mono text-slate-100">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  , with no push and no PR — I push it myself.
                </span>
              ) : (
                <span>
                  I understand this opens a reviewed integration PR against{' '}
                  <span className="mono text-slate-100">
                    {targetBranches?.mainRepository ?? 'main-system'}/{targetBranch}
                  </span>
                  , not a push or deploy.
                </span>
              )}
            </label>
            {!acknowledged ? (
              <p className="flex items-center gap-1.5 text-2xs text-amber-200">
                <TriangleAlert className="h-3.5 w-3.5" aria-hidden="true" />
                The confirmation must be accepted before the request can be sent.
              </p>
            ) : (
              <p className="flex items-center gap-1.5 text-2xs text-emerald-200">
                <Check className="h-3.5 w-3.5" aria-hidden="true" />
                Confirmation recorded for this request.
              </p>
            )}
          </section>

          <p className="flex items-center gap-1.5 text-2xs text-slate-500">
            <GitBranch className="h-3.5 w-3.5" aria-hidden="true" />
            {isDirect
              ? `The commit lands on your local ${targetBranch}; pushing is yours to do.`
              : isLocal
                ? `The integration branch is created from the target branch tip (${targetBranch}) in your local clone; you push it yourself.`
                : `The integration branch is created from the target branch tip and the PR is opened with base (${targetBranch}).`}
          </p>

          {commits.some((commit) => commit.status.kind === 'selected') ? (
            <div className="flex items-center gap-2">
              <StatusBadge kind="selected" />
              <span className="text-2xs text-slate-400">
                A ticked commit is only a local selection — statuses come from Git history and
                GitHub.
              </span>
            </div>
          ) : null}
        </div>
      )}
    </Modal>
  );
}

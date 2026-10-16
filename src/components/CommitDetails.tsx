import { useState } from 'react';
import {
  ArrowRight,
  Check,
  Copy,
  ExternalLink,
  FileText,
  GitCommitHorizontal,
  GitFork,
  GitPullRequestArrow,
  Package,
  ShieldAlert,
  Tag,
  TriangleAlert,
} from 'lucide-react';

import type { ApiError } from '../api/invoke';
import type { ChangedFile, CommitDetails, FileIntegrationPolicy } from '../api/types';
import { CommitStatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import {
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  InlineAlert,
  KeyValue,
  LoadingState,
} from './ui/primitives';

/** Right-hand commit details panel (spec §3.2). */

const POLICY_LABEL: Record<
  FileIntegrationPolicy,
  { label: string; className: string; note: string }
> = {
  apply: {
    label: 'copy to target',
    className: 'text-slate-400',
    note: 'Applied at the same relative path in the main-system tree.',
  },
  regenerate_in_target: {
    label: 'regenerate in target',
    className: 'text-amber-300',
    note: 'Lockfiles are never copied; the target branch lockfile is regenerated.',
  },
  review_required: {
    label: 'review required',
    className: 'text-amber-300',
    note: 'Shared configuration/manifest change that needs an explicit reviewer decision.',
  },
  never_import: {
    label: 'never imported',
    className: 'text-rose-300',
    note: 'Secret-looking files are rejected by policy and reported instead of transferred.',
  },
  main_owned: {
    label: 'main-owned',
    className: 'text-rose-300',
    note: 'Owned by the main repository; a subsystem approval does not authorize overwriting it.',
  },
};

function fileGlyph(file: ChangedFile): { glyph: string; className: string } {
  switch (file.change) {
    case 'added':
      return { glyph: '+', className: 'text-emerald-400' };
    case 'deleted':
      return { glyph: '-', className: 'text-rose-400' };
    case 'renamed':
      return { glyph: '→', className: 'text-sky-400' };
    default:
      return { glyph: '~', className: 'text-amber-300' };
  }
}

export interface CommitDetailsProps {
  details: CommitDetails | null;
  loading: boolean;
  error: ApiError | null;
  onRetry: () => void;
  targetBranch: string;
  selectedShas: string[];
  onPromote: (sha: string) => void;
  onPromoteSelection: () => void;
  onShowDiff: (sha: string, filePath: string) => void;
  activeDiffPath: string | null;
  onOpenExternal: (url: string) => void;
  onClose?: () => void;
  mainRepository: string | null;
}

export function CommitDetailsPanel({
  details,
  loading,
  error,
  onRetry,
  targetBranch,
  selectedShas,
  onPromote,
  onPromoteSelection,
  onShowDiff,
  activeDiffPath,
  onOpenExternal,
  onClose,
  mainRepository,
}: CommitDetailsProps) {
  const [copied, setCopied] = useState(false);

  if (loading && !details) return <LoadingState label="Loading commit details…" />;
  if (error) return <ErrorState error={error} onRetry={onRetry} />;
  if (!details) {
    return (
      <EmptyState
        icon={GitCommitHorizontal}
        title="No commit selected"
        message="Pick a commit in the list to inspect its SHA, status, changed files and dependency impact."
      />
    );
  }

  const copySha = async () => {
    try {
      await navigator.clipboard?.writeText(details.sha);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  const isSelected = selectedShas.includes(details.sha);
  const alreadyIntegrated = details.status.kind === 'integrated';
  const hasOpenPr = details.status.kind === 'pr_open' || details.status.kind === 'in_review';
  const isPromotable = details.status.kind === 'eligible' || details.status.kind === 'reverted';

  return (
    <div className="flex min-h-0 flex-col">
      <header className="flex items-center justify-between gap-2 border-b border-ink-700/70 px-4 py-3">
        <div className="flex items-center gap-2">
          <h2 className="panel-title">Commit Details</h2>
          {onClose ? (
            <IconButton icon={ArrowRight} label="Hide commit details" onClick={onClose} />
          ) : null}
        </div>
        <CommitStatusBadge status={details.status} size="md" />
      </header>

      <div className="scroll-area min-h-0 flex-1 space-y-4 p-4">
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void copySha()}
              title={`${details.sha}\nClick to copy the full SHA (abbreviated values are display-only)`}
              className="mono rounded-sm text-sm font-semibold text-brand-300 hover:text-brand-200 hover:underline"
            >
              {details.shortSha}
            </button>
            <IconButton
              icon={copied ? Check : Copy}
              label={copied ? 'Full SHA copied' : 'Copy full SHA'}
              onClick={() => void copySha()}
            />
            <IconButton
              icon={ExternalLink}
              label="Open this commit on GitHub"
              onClick={() => onOpenExternal(details.sourceUrl)}
            />
          </div>
          <p className="text-sm font-medium leading-snug text-slate-100">{details.subject}</p>
        </div>

        <div className="flex items-start gap-3 rounded-lg border border-ink-700/60 bg-ink-850/60 p-2.5">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-ink-700 text-2xs font-semibold uppercase text-slate-200">
            {(details.authorLogin ?? details.authorName).slice(0, 2)}
          </span>
          <div className="min-w-0 text-xs">
            <p className="truncate font-medium text-slate-200">
              {details.authorLogin ?? details.authorName}
            </p>
            <p className="mt-0.5 text-slate-400">{formatDateTime(details.committedAt)}</p>
            <p className="text-2xs text-slate-500">
              {details.relativeTime} · authored {formatDateTime(details.authoredAt)}
            </p>
          </div>
        </div>

        {details.body ? (
          <p className="whitespace-pre-wrap text-xs leading-relaxed text-slate-300">
            {details.body}
          </p>
        ) : null}

        {details.tags.length > 0 ? (
          <ul className="flex flex-wrap gap-1.5" aria-label="Commit tags">
            {details.tags.map((tag) => (
              <li
                key={tag}
                className="chip bg-brand-900/40 text-brand-200 ring-1 ring-inset ring-brand-700/60"
              >
                <Tag className="h-3 w-3" aria-hidden="true" />
                {tag}
              </li>
            ))}
          </ul>
        ) : null}

        {details.status.evidence ? (
          <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-2.5 text-2xs text-slate-300">
            <p className="flex items-center gap-1.5 font-semibold text-slate-200">
              <GitFork className="h-3.5 w-3.5 text-emerald-400" aria-hidden="true" />
              Integration evidence on {details.status.targetBranch}
            </p>
            <p className="mono mt-1">{details.status.evidence.mainCommitSha}</p>
            <p className="mt-0.5 text-slate-400">
              {formatDateTime(details.status.evidence.mergedAt)}
            </p>
            {details.status.evidence.inheritedFromBranch ? (
              <p className="mt-1 text-amber-200">
                Inherited from {details.status.evidence.inheritedFromBranch} through a main-system
                branch merge.
              </p>
            ) : null}
            {details.status.evidence.pullRequestNumber && mainRepository ? (
              <button
                type="button"
                onClick={() =>
                  onOpenExternal(
                    `https://github.com/${mainRepository}/pull/${details.status.evidence?.pullRequestNumber}`,
                  )
                }
                className="mt-1 inline-flex items-center gap-1 text-brand-300 hover:underline"
              >
                Integration PR #{details.status.evidence.pullRequestNumber}
                <ExternalLink className="h-3 w-3" aria-hidden="true" />
              </button>
            ) : null}
          </div>
        ) : null}

        {details.dependencies.length > 0 ? (
          <div className="space-y-1.5">
            <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-slate-400">
              <Package className="h-3.5 w-3.5" aria-hidden="true" />
              Dependency changes
            </h3>
            <ul className="space-y-1.5">
              {details.dependencies.map((dependency) => (
                <li
                  key={`${dependency.section}-${dependency.name}`}
                  className={cx(
                    'rounded-lg border p-2 text-2xs',
                    dependency.conflict
                      ? 'border-rose-900/70 bg-rose-950/30 text-rose-200'
                      : 'border-ink-700/60 bg-ink-850/60 text-slate-300',
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="mono text-slate-100">{dependency.name}</span>
                    <span className="chip bg-ink-800 text-slate-300">{dependency.section}</span>
                  </div>
                  <p className="mt-1 flex items-center gap-1.5">
                    <span className="text-slate-500">{dependency.fromRange ?? '(absent)'}</span>
                    <ArrowRight className="h-3 w-3" aria-hidden="true" />
                    <span>{dependency.toRange ?? '(removed)'}</span>
                    <span className="ml-auto uppercase tracking-wide text-slate-500">
                      {dependency.change}
                    </span>
                  </p>
                  {dependency.note ? (
                    <p className="mt-1 text-slate-400">{dependency.note}</p>
                  ) : null}
                  {dependency.conflict ? (
                    <p className="mt-1 flex items-center gap-1 font-semibold">
                      <TriangleAlert className="h-3 w-3" aria-hidden="true" />
                      Incompatible range — the promotion fails closed until this is resolved.
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <h3 className="flex items-center gap-1.5 text-xs font-semibold uppercase tracking-wide text-slate-400">
              <FileText className="h-3.5 w-3.5" aria-hidden="true" />
              Files Changed
            </h3>
            <span className="chip bg-ink-800 text-slate-300">{details.filesChanged}</span>
          </div>

          {details.files.length === 0 ? (
            <p className="text-2xs text-slate-500">
              No file list is available for this commit in the current data source.
            </p>
          ) : (
            <ul className="space-y-0.5">
              {details.files.map((file) => {
                const glyph = fileGlyph(file);
                const policy = POLICY_LABEL[file.policy];
                const active = activeDiffPath === file.path;
                return (
                  <li key={file.path}>
                    <button
                      type="button"
                      onClick={() => onShowDiff(details.sha, file.path)}
                      title={`${file.path}\n${policy.note}`}
                      className={cx(
                        'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left transition-colors',
                        active
                          ? 'bg-ink-800 ring-1 ring-inset ring-brand-600/60'
                          : 'hover:bg-ink-800/70',
                      )}
                    >
                      <span
                        className={cx(
                          'w-3 shrink-0 text-center text-sm font-bold',
                          glyph.className,
                        )}
                        aria-hidden="true"
                      >
                        {glyph.glyph}
                      </span>
                      <span className="mono min-w-0 flex-1 truncate text-slate-200">
                        {file.path}
                      </span>
                      <span className="shrink-0 text-2xs text-emerald-400">+{file.additions}</span>
                      <span className="shrink-0 text-2xs text-rose-400">-{file.deletions}</span>
                      {file.policy !== 'apply' ? (
                        <span className={cx('chip shrink-0 bg-ink-800', policy.className)}>
                          {policy.label}
                        </span>
                      ) : null}
                    </button>
                    {file.previousPath ? (
                      <p className="mono px-2 pl-7 text-2xs text-slate-500">
                        renamed from {file.previousPath}
                      </p>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        {details.note ? (
          <InlineAlert tone="info" title="Promotion semantics">
            {details.note}
          </InlineAlert>
        ) : null}

        {hasOpenPr || alreadyIntegrated ? (
          <InlineAlert tone="warning" title="This SHA is not promotable to this branch">
            {alreadyIntegrated
              ? 'It is already recorded as integrated on this target branch. Promoting again would duplicate the change; choose a different branch or revert first.'
              : 'An integration PR for this SHA is already open on this target branch. Review or merge it instead of dispatching another run.'}
          </InlineAlert>
        ) : null}

        <div className="space-y-2 border-t border-ink-700/70 pt-3">
          <div className="flex items-stretch gap-1">
            <Button
              variant="primary"
              size="md"
              icon={GitPullRequestArrow}
              className="flex-1"
              disabled={!isPromotable}
              onClick={() => onPromote(details.sha)}
              title={
                isPromotable
                  ? 'Choose the target main-system branch and confirm the source SHA'
                  : 'Only eligible (or previously reverted) commits can be promoted'
              }
            >
              Promote This Commit
            </Button>
            {selectedShas.length > 1 && isSelected ? (
              <Button
                variant="secondary"
                size="md"
                onClick={onPromoteSelection}
                title="Promote every ticked commit in one ordered integration PR"
              >
                Promote {selectedShas.length} selected
              </Button>
            ) : null}
          </div>

          <Button
            variant="secondary"
            size="md"
            icon={ArrowRight}
            className="w-full"
            onClick={() => onShowDiff(details.sha, details.files[0]?.path ?? '')}
            disabled={details.files.length === 0}
          >
            View Full Diff
          </Button>

          <div className="rounded-lg border border-ink-700/60 bg-ink-850/60 p-2.5">
            <p className="flex items-start gap-2 text-2xs leading-relaxed text-slate-400">
              <ShieldAlert
                className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-300"
                aria-hidden="true"
              />
              A promotion opens an integration PR against the chosen main-system branch. It is never
              a direct push and never a production deploy.
            </p>
          </div>

          <dl className="rounded-lg border border-ink-700/60 bg-ink-850/60 px-3 py-1">
            <KeyValue label="Target branch">{targetBranch}</KeyValue>
            <KeyValue label="Parents">
              <span className="mono">
                {details.parents.map((parent) => parent.slice(0, 7)).join(', ') || '—'}
              </span>
            </KeyValue>
            <KeyValue label="Delta mode">
              {details.isMerge ? 'first parent (merge commit)' : 'commit delta'}
            </KeyValue>
            <KeyValue label="Totals">
              <span className="text-emerald-400">+{details.additions}</span> /{' '}
              <span className="text-rose-400">-{details.deletions}</span>
            </KeyValue>
          </dl>
        </div>
      </div>
    </div>
  );
}

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
  Spinner,
} from './ui/primitives';

/** Right-hand commit details panel (spec §3.2). */

const POLICY_LABEL: Record<
  FileIntegrationPolicy,
  { label: string; className: string; note: string }
> = {
  apply: {
    label: 'copy to target',
    className: 'text-muted',
    note: 'Applied at the same relative path in the main-system tree.',
  },
  regenerate_in_target: {
    label: 'regenerate in target',
    className: 'text-status-unknown-ink',
    note: 'Lockfiles are never copied; the target branch lockfile is regenerated.',
  },
  review_required: {
    label: 'review required',
    className: 'text-status-unknown-ink',
    note: 'Shared configuration/manifest change that needs an explicit reviewer decision.',
  },
  never_import: {
    label: 'never imported',
    className: 'text-status-blocked-ink',
    note: 'Secret-looking files are rejected by policy and reported instead of transferred.',
  },
  main_owned: {
    label: 'main-owned',
    className: 'text-status-blocked-ink',
    note: 'Owned by the main repository; a subsystem approval does not authorize overwriting it.',
  },
};

function fileGlyph(file: ChangedFile): { glyph: string; className: string } {
  switch (file.change) {
    case 'added':
      return { glyph: '+', className: 'text-status-applied' };
    case 'deleted':
      return { glyph: '-', className: 'text-status-blocked' };
    case 'renamed':
      return { glyph: '→', className: 'text-status-eligible' };
    default:
      return { glyph: '~', className: 'text-status-unknown-ink' };
  }
}

export interface CommitDetailsProps {
  details: CommitDetails | null;
  loading: boolean;
  error: ApiError | null;
  onRetry: () => void;
  targetBranch: string;
  /**
   * Read-only handoff: selects this commit in the shared selection and opens
   * the Promote tab, which owns the whole promotion flow. The browser never
   * opens the promotion dialog itself.
   */
  onPromoteHandoff: (sha: string) => void;
  onShowDiff: (sha: string, filePath: string) => void;
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
  onPromoteHandoff,
  onShowDiff,
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
        message="Pick a commit in the list to inspect it."
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

  const alreadyIntegrated = details.status.kind === 'integrated';
  const hasOpenPr = details.status.kind === 'pr_open' || details.status.kind === 'in_review';
  const isPromotable = details.status.kind === 'eligible' || details.status.kind === 'reverted';

  return (
    <div className="flex min-h-0 flex-col" aria-busy={loading}>
      <header className="flex items-center justify-between gap-2 border-b border-line px-2.5 py-1.5">
        <div className="flex items-center gap-2">
          <h2 className="panel-title">Commit Details</h2>
          {loading ? (
            <span
              role="status"
              aria-label="Loading the newly selected commit"
              title="Loading the newly selected commit — the panel still shows the previous commit until it arrives."
            >
              <Spinner className="h-3.5 w-3.5" />
            </span>
          ) : null}
          {onClose ? (
            <IconButton icon={ArrowRight} label="Hide commit details" onClick={onClose} />
          ) : null}
        </div>
        <CommitStatusBadge status={details.status} size="md" />
      </header>

      <div className="scroll-area min-h-0 flex-1 space-y-4 p-2.5">
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => void copySha()}
              title={`${details.sha}\nClick to copy the full SHA (abbreviated values are display-only)`}
              className="mono rounded-[3px] text-sm font-semibold text-accent-ink hover:underline"
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
          <p className="truncate text-xs font-medium text-fg" title={details.subject}>
            {details.subject}
          </p>
        </div>

        <div className="flex items-start gap-2.5 rounded-md border border-line bg-raised p-2.5">
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[3px] border border-line bg-hover text-[11px] font-semibold text-muted uppercase">
            {(details.authorLogin ?? details.authorName).slice(0, 2)}
          </span>
          <div className="min-w-0 text-xs">
            <p className="truncate font-medium text-fg">
              {details.authorLogin ?? details.authorName}
            </p>
            <p className="mt-0.5 text-muted">{formatDateTime(details.committedAt)}</p>
            <p className="mono text-right text-[11px] text-disabled tabular-nums">
              {details.relativeTime} · authored {formatDateTime(details.authoredAt)}
            </p>
          </div>
        </div>

        {details.body ? (
          <p className="truncate text-xs leading-relaxed text-muted" title={details.body}>
            {details.body}
          </p>
        ) : null}

        {details.tags.length > 0 ? (
          <p className="text-[11px] text-muted" aria-label="Commit tags">
            {details.tags.join(' · ')}
          </p>
        ) : null}

        {details.status.evidence ? (
          <div className="rounded-md border border-line bg-raised p-2.5 text-[11px] text-muted">
            <p className="flex items-center gap-1.5 font-semibold text-fg">
              <GitFork className="h-3.5 w-3.5 text-status-applied" aria-hidden="true" />
              Integration evidence on {details.status.targetBranch}
            </p>
            <p
              className="mono mt-1 truncate text-right tabular-nums"
              title={details.status.evidence.mainCommitSha}
            >
              {details.status.evidence.mainCommitSha}
            </p>
            <p className="mono mt-0.5 text-right text-disabled tabular-nums">
              {formatDateTime(details.status.evidence.mergedAt)}
            </p>
            {details.status.evidence.inheritedFromBranch ? (
              <p className="mt-1 text-status-unknown-ink">
                Inherited from {details.status.evidence.inheritedFromBranch}.
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
                className="mt-1 inline-flex items-center gap-1 text-accent-ink hover:underline"
              >
                Integration PR #{details.status.evidence.pullRequestNumber}
                <ExternalLink className="h-3 w-3" aria-hidden="true" />
              </button>
            ) : null}
          </div>
        ) : null}

        {details.dependencies.length > 0 ? (
          <div className="space-y-1.5">
            <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
              <Package className="h-3.5 w-3.5" aria-hidden="true" />
              Dependency changes
            </h3>
            <ul className="space-y-1.5">
              {details.dependencies.map((dependency) => (
                <li
                  key={`${dependency.section}-${dependency.name}`}
                  className={cx(
                    'rounded-md border border-line p-2 text-[11px]',
                    dependency.conflict
                      ? 'bg-status-blocked-soft text-status-blocked-ink'
                      : 'bg-raised text-muted',
                  )}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="mono truncate text-fg">{dependency.name}</span>
                    <span className="meta">{dependency.section}</span>
                  </div>
                  <p className="mono mt-1 flex items-center gap-1.5 truncate tabular-nums">
                    <span className="text-disabled">{dependency.fromRange ?? '(absent)'}</span>
                    <ArrowRight className="h-3 w-3 shrink-0" aria-hidden="true" />
                    <span>{dependency.toRange ?? '(removed)'}</span>
                    <span className="ml-auto tracking-wide text-disabled uppercase">
                      {dependency.change}
                    </span>
                  </p>
                  {dependency.note ? (
                    <p className="mt-1 truncate" title={dependency.note}>
                      {dependency.note}
                    </p>
                  ) : null}
                  {dependency.conflict ? (
                    <p className="mt-1 flex items-center gap-1 font-semibold">
                      <TriangleAlert className="h-3 w-3" aria-hidden="true" />
                      Incompatible range — promotion fails closed.
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        <div className="space-y-1.5">
          <div className="flex items-center justify-between">
            <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
              <FileText className="h-3.5 w-3.5" aria-hidden="true" />
              Files Changed
            </h3>
            <span className="meta tabular-nums">{details.filesChanged}</span>
          </div>

          {details.files.length === 0 ? (
            <p className="text-[11px] text-disabled">No file list is available for this commit.</p>
          ) : (
            <ul className="space-y-0.5">
              {details.files.map((file) => {
                const glyph = fileGlyph(file);
                const policy = POLICY_LABEL[file.policy];
                return (
                  <li key={file.path}>
                    <button
                      type="button"
                      onClick={() => onShowDiff(details.sha, file.path)}
                      title={`${file.path}\n${policy.note}`}
                      className={cx(
                        'flex min-h-[29px] w-full items-start gap-2 rounded-md px-2.5 py-1 text-left transition-colors hover:bg-hover',
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
                      <span className="mono min-w-0 flex-1 text-xs break-all text-fg tabular-nums">
                        {file.path}
                      </span>
                      <span className="mono shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                        +{file.additions}
                      </span>
                      <span className="mono shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                        -{file.deletions}
                      </span>
                      {file.policy !== 'apply' ? (
                        <span className={cx('meta shrink-0', policy.className)}>
                          {policy.label}
                        </span>
                      ) : null}
                    </button>
                    {file.previousPath ? (
                      <p className="mono px-2.5 pl-7 text-[11px] break-all text-disabled">
                        renamed from {file.previousPath}
                      </p>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        {details.relatedPullRequests.length > 0 ? (
          <div className="space-y-1.5">
            <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
              <GitPullRequestArrow className="h-3.5 w-3.5" aria-hidden="true" />
              Related Pull Requests
            </h3>
            <ul className="divide-y divide-line border-y border-line">
              {details.relatedPullRequests.map((pr) => (
                <li key={pr.htmlUrl}>
                  <button
                    type="button"
                    onClick={() => onOpenExternal(pr.htmlUrl)}
                    aria-label={`Open pull request #${pr.number} on GitHub`}
                    title={`${pr.title}\n${pr.htmlUrl}`}
                    className="flex w-full items-center gap-1.5 py-1 text-left text-[11px] transition-colors hover:bg-hover"
                  >
                    <span className="mono shrink-0 text-accent-ink">#{pr.number}</span>
                    <span className="min-w-0 flex-1 truncate text-muted">{pr.title}</span>
                    <ExternalLink className="h-3 w-3 shrink-0 text-disabled" aria-hidden="true" />
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ) : null}

        {hasOpenPr || alreadyIntegrated ? (
          <InlineAlert tone="warning" title="Not promotable on this branch">
            {alreadyIntegrated
              ? 'Already integrated on this target branch.'
              : 'An integration PR is already open for this SHA.'}
          </InlineAlert>
        ) : null}

        <div className="space-y-2 border-t border-line pt-3">
          <Button
            variant="primary"
            size="md"
            icon={GitPullRequestArrow}
            className="w-full"
            disabled={!isPromotable}
            onClick={() => onPromoteHandoff(details.sha)}
            title={
              isPromotable
                ? 'Select this commit and open the Promote tab to confirm it'
                : 'Only eligible (or previously reverted) commits can be promoted'
            }
          >
            Promote…
          </Button>

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

          <dl className="rounded-md border border-line bg-raised px-3 py-1">
            <KeyValue label="Target branch">{targetBranch}</KeyValue>
            <KeyValue label="Parents">
              <span className="mono">
                {details.parents.map((parent) => parent.slice(0, 7)).join(', ') || '—'}
              </span>
            </KeyValue>
            <KeyValue label="Content from">
              {details.isMerge ? 'first parent of the merge' : 'this commit'}
            </KeyValue>
            <KeyValue label="Totals">
              <span className="text-status-applied-ink">+{details.additions}</span> /{' '}
              <span className="text-status-blocked-ink">-{details.deletions}</span>
            </KeyValue>
          </dl>
        </div>
      </div>
    </div>
  );
}

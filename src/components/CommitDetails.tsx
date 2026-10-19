import {
  ArrowRight,
  ExternalLink,
  GitFork,
  GitPullRequestArrow,
  Package,
  TriangleAlert,
} from 'lucide-react';

import type { CommitDetails } from '../api/types';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import { InlineAlert, KeyValue } from './ui/primitives';

export interface CommitDetailsContextProps {
  details: CommitDetails;
  targetBranch: string;
  onOpenExternal: (url: string) => void;
  mainRepository: string | null;
}

export function CommitDetailsContext({
  details,
  targetBranch,
  onOpenExternal,
  mainRepository,
}: CommitDetailsContextProps) {
  const alreadyIntegrated = details.status.kind === 'integrated';
  const hasOpenPr = details.status.kind === 'pr_open' || details.status.kind === 'in_review';

  return (
    <div className="space-y-4 p-2.5">
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
            {(details.dependencies ?? []).map((dependency) => (
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

      {(details.relatedPullRequests ?? []).length > 0 ? (
        <div className="space-y-1.5">
          <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
            <GitPullRequestArrow className="h-3.5 w-3.5" aria-hidden="true" />
            Related Pull Requests
          </h3>
          <ul className="divide-y divide-line border-y border-line">
            {(details.relatedPullRequests ?? []).map((pr) => (
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

      <dl className="rounded-md border border-line bg-raised px-3 py-1">
        <KeyValue label="Target branch">{targetBranch}</KeyValue>
        <KeyValue label="Parents">
          <span className="mono">
            {(details.parents ?? []).map((parent) => parent.slice(0, 7)).join(', ') || '-'}
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
  );
}

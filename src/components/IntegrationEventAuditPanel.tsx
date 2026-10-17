import {
  ExternalLink,
  FileText,
  GitCommitHorizontal,
  History,
  ScrollText,
  Undo2,
} from 'lucide-react';

import type { ApiError } from '../api/invoke';
import type {
  IntegrationEventAudit,
  IntegrationEventChange,
  IntegrationHistoryEntry,
} from '../api/types';
import { INTEGRATION_KIND_META } from './integrationKindMeta';
import { StatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import { EmptyState, ErrorState, KeyValue, LoadingState } from './ui/primitives';

/** Audit rail for one integration event (spec §§3.3/8.3). Same rail idiom as the commit details panel. */

function changeGlyph(change: IntegrationEventChange['change']): {
  glyph: string;
  className: string;
} {
  switch (change) {
    case 'added':
      return { glyph: '+', className: 'text-status-applied' };
    case 'deleted':
      return { glyph: '−', className: 'text-status-blocked' };
    case 'renamed':
      return { glyph: '→', className: 'text-status-eligible' };
    default:
      return { glyph: '~', className: 'text-status-unknown-ink' };
  }
}

export interface IntegrationEventAuditPanelProps {
  /** The selected table entry. Null means nothing is selected yet. */
  entry: IntegrationHistoryEntry | null;
  /**
   * The fetched audit, already gated on request identity by the caller: null
   * while a fetch is pending so a mid-flight selection switch never flashes
   * the previous event's data.
   */
  audit: IntegrationEventAudit | null;
  loading: boolean;
  error: ApiError | null;
  onRetry: () => void;
  onOpenExternal: (url: string) => void;
  mainRepository: string | null;
}

export function IntegrationEventAuditPanel({
  entry,
  audit,
  loading,
  error,
  onRetry,
  onOpenExternal,
  mainRepository,
}: IntegrationEventAuditPanelProps) {
  if (!entry) {
    return (
      <EmptyState
        icon={History}
        title="No event selected"
        message="Select an event in the table to see which commits it included and what changed."
      />
    );
  }

  // Request identity: a pending fetch — or an audit that does not belong to
  // the current selection — renders as loading, never as stale data.
  if (loading || (audit && entry.mainCommitSha && audit.mainCommitSha !== entry.mainCommitSha)) {
    return <LoadingState label="Loading event audit…" />;
  }

  if (error) {
    return <ErrorState error={error} onRetry={onRetry} />;
  }

  // The loader only resolves null for an entry with no main commit, which has
  // no audit to fetch. Those rows are not clickable; this is a defensive path.
  if (!audit) {
    const meta = INTEGRATION_KIND_META[entry.kind];
    return (
      <div className="flex min-h-0 flex-col">
        <header className="flex items-center justify-between gap-2 border-b border-line px-2.5 py-1.5">
          <h2 className="panel-title">Event Audit</h2>
          <StatusBadge kind={meta.kind} label={meta.label} size="md" />
        </header>
        <div className="scroll-area min-h-0 flex-1 space-y-4 p-2.5">
          <p className="truncate text-xs text-fg" title={entry.detail}>
            {entry.detail}
          </p>
          <p className="text-[11px] leading-relaxed text-muted">
            This event has no main-system commit, so there is no audit to fetch.
          </p>
        </div>
      </div>
    );
  }

  const meta = INTEGRATION_KIND_META[audit.kind];
  const isArtifactKind =
    audit.kind === 'merged' || audit.kind === 'restored' || audit.kind === 'reverted';

  return (
    <div className="flex min-h-0 flex-col">
      <header className="flex items-center justify-between gap-2 border-b border-line px-2.5 py-1.5">
        <h2 className="panel-title">Event Audit</h2>
        <StatusBadge kind={meta.kind} label={meta.label} size="md" />
      </header>

      <div className="scroll-area min-h-0 flex-1 space-y-4 p-2.5">
        <div className="space-y-1">
          <p className="truncate text-xs font-medium text-fg" title={audit.subsystemId}>
            {audit.subsystemId}
          </p>
          <p className="mono truncate text-muted" title={audit.targetBranch}>
            {audit.targetBranch}
          </p>
          <p className="mono truncate text-muted tabular-nums" title={audit.mainCommitSha}>
            {audit.mainCommitSha.slice(0, 7)}
          </p>
          <p
            className="truncate text-[11px] text-muted"
            title={`${audit.actor} · ${audit.occurredAt}`}
          >
            {audit.actor} · {formatDateTime(audit.occurredAt)}
          </p>
          {audit.inheritedFromBranch ? (
            <p className="text-[11px] text-status-unknown-ink">
              Inherited from {audit.inheritedFromBranch}.
            </p>
          ) : null}
          {audit.revertedMainCommitSha ? (
            <p className="flex items-center gap-1.5 text-[11px] text-muted">
              <Undo2 className="h-3 w-3 shrink-0" aria-hidden="true" />
              Reverts{' '}
              <span className="mono tabular-nums" title={audit.revertedMainCommitSha}>
                {audit.revertedMainCommitSha.slice(0, 7)}
              </span>
            </p>
          ) : null}
          {audit.pullRequestNumber && mainRepository ? (
            <button
              type="button"
              onClick={() =>
                onOpenExternal(
                  `https://github.com/${mainRepository}/pull/${audit.pullRequestNumber}`,
                )
              }
              className="mt-0.5 inline-flex items-center gap-1 text-accent-ink hover:underline"
            >
              PR #{audit.pullRequestNumber}
              <ExternalLink className="h-3 w-3" aria-hidden="true" />
            </button>
          ) : null}
        </div>

        <p className="truncate text-xs leading-relaxed text-muted" title={audit.detail}>
          {audit.detail}
        </p>

        {audit.kind === 'failed' ? (
          <p className="text-[11px] leading-relaxed text-muted">
            This event recorded a failure — there is no artifact. The detail above is the audit
            record.
          </p>
        ) : null}

        {audit.kind === 'pending_pr' ? (
          <div className="space-y-1">
            <p className="text-[11px] leading-relaxed text-muted">
              No main-system commit yet — the diff summary does not exist until the PR merges.
            </p>
            {entry.sourceShas.length > 0 ? (
              <p
                className="mono truncate text-muted tabular-nums"
                title={entry.sourceShas.join('\n')}
              >
                source {entry.sourceShas.map((sha) => sha.slice(0, 7)).join(', ')}
              </p>
            ) : null}
          </div>
        ) : null}

        {isArtifactKind ? (
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
                <GitCommitHorizontal className="h-3.5 w-3.5" aria-hidden="true" />
                Included commits
              </h3>
              <span className="meta tabular-nums">{audit.includedCommits.length}</span>
            </div>
            {audit.includedCommits.length === 0 ? (
              <p className="text-[11px] text-disabled">
                No included commits were recorded for this event.
              </p>
            ) : (
              <ul className="divide-y divide-line border-y border-line">
                {audit.includedCommits.map((commit) => (
                  <li key={commit.sha} className="py-1">
                    <div className="flex items-baseline gap-2">
                      <span className="mono shrink-0 text-fg tabular-nums" title={commit.sha}>
                        {commit.shortSha}
                      </span>
                      {commit.unavailable ? (
                        <span className="min-w-0 flex-1 truncate text-[11px] text-disabled">
                          Not present in the local clone.
                        </span>
                      ) : (
                        <span
                          className="min-w-0 flex-1 truncate text-xs text-fg"
                          title={commit.subject || undefined}
                        >
                          {commit.subject || '—'}
                        </span>
                      )}
                    </div>
                    {!commit.unavailable && (commit.author || commit.authoredAt) ? (
                      <p className="mt-0.5 truncate text-[11px] text-muted">
                        {commit.author ?? 'unknown author'}
                        {commit.authoredAt ? ` · ${formatDateTime(commit.authoredAt)}` : ''}
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </div>
        ) : null}

        {isArtifactKind ? (
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
                <FileText className="h-3.5 w-3.5" aria-hidden="true" />
                Changes
              </h3>
              {audit.totals ? (
                <span className="meta tabular-nums">
                  {audit.totals.files} file(s) · +{audit.totals.additions} −{audit.totals.deletions}
                </span>
              ) : (
                <span className="meta tabular-nums">{audit.changes.length} file(s)</span>
              )}
            </div>
            {audit.mainCommitUnavailable ? (
              <p className="text-[11px] leading-relaxed text-muted">
                The change summary isn&apos;t available — the main commit isn&apos;t in the local
                clone.
              </p>
            ) : audit.changes.length === 0 ? (
              <p className="text-[11px] text-disabled">
                No file changes were recorded for this event.
              </p>
            ) : (
              <ul className="space-y-0.5">
                {audit.changes.map((change) => {
                  const glyph = changeGlyph(change.change);
                  return (
                    <li
                      key={change.path}
                      className="flex min-h-[29px] items-start gap-2 px-2.5 py-1"
                      title={change.path}
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
                      <span className="mono min-w-0 flex-1 truncate text-xs text-fg tabular-nums">
                        {change.path}
                      </span>
                      <span className="mono shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                        +{change.additions}
                      </span>
                      <span className="mono shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                        −{change.deletions}
                      </span>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        ) : null}

        {audit.provenance.length > 0 ? (
          <div className="space-y-1.5">
            <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
              <ScrollText className="h-3.5 w-3.5" aria-hidden="true" />
              Provenance
            </h3>
            <dl className="rounded-md border border-line bg-raised px-3 py-1">
              {audit.provenance.map((trailer) => (
                <KeyValue key={trailer.key} label={trailer.key}>
                  <span className="mono block truncate tabular-nums" title={trailer.value}>
                    {trailer.value}
                  </span>
                </KeyValue>
              ))}
            </dl>
          </div>
        ) : null}
      </div>
    </div>
  );
}

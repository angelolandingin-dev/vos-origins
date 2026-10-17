import { useEffect, useState } from 'react';
import {
  ArrowLeft,
  ExternalLink,
  FileText,
  GitCommitHorizontal,
  History,
  ScrollText,
  Search,
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
import { Button, EmptyState, ErrorState, KeyValue, LoadingState, Panel } from './ui/primitives';

/**
 * Full-page audit for one integration event (spec §§3.3/8.3). Replaces the
 * history list within the tab while open — the CommitDiffView precedent — so
 * the audit is read as a page, not glanced at in a side rail.
 *
 * Everything rendered comes from `getIntegrationEvent`. The Main commit
 * section carries the SHA, the commit's own subject/author/authored date via
 * `mainCommit` (null when unresolvable — nothing is invented), and the
 * commit's change summary.
 */

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

function SectionTitle({ icon: Icon, children }: { icon: typeof FileText; children: string }) {
  return (
    <h3 className="flex items-center gap-1.5 text-[11px] font-semibold tracking-wide text-muted uppercase">
      <Icon className="h-3.5 w-3.5" aria-hidden="true" />
      {children}
    </h3>
  );
}

export interface IntegrationEventAuditPageProps {
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
  onBack: () => void;
}

export function IntegrationEventAuditPage({
  entry,
  audit,
  loading,
  error,
  onRetry,
  onOpenExternal,
  mainRepository,
  onBack,
}: IntegrationEventAuditPageProps) {
  const [fileFilter, setFileFilter] = useState('');

  // Escape returns to the event list — the CommitDiffView idiom.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onBack();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onBack]);

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
      <Panel
        className="min-h-0 flex-1"
        bodyClassName="flex min-h-0 flex-1 flex-col"
        title={
          <span className="flex min-w-0 items-center gap-2">
            <Button
              size="sm"
              variant="secondary"
              icon={ArrowLeft}
              onClick={onBack}
              title="Back to events (Esc)"
            >
              Back
            </Button>
            <span className="min-w-0 flex-1 truncate text-xs font-medium" title={entry.detail}>
              {entry.detail}
            </span>
          </span>
        }
        actions={<StatusBadge kind={meta.kind} label={meta.label} />}
      >
        <p className="p-4 text-[11px] leading-relaxed text-muted">
          This event has no main-system commit, so there is no audit to fetch.
        </p>
      </Panel>
    );
  }

  const meta = INTEGRATION_KIND_META[audit.kind];
  const query = fileFilter.trim().toLowerCase();
  const listedChanges = query
    ? audit.changes.filter((change) => change.path.toLowerCase().includes(query))
    : audit.changes;

  return (
    <Panel
      className="min-h-0 flex-1"
      bodyClassName="scroll-area min-h-0 flex-1 overflow-y-auto"
      title={
        <span className="flex min-w-0 items-center gap-2">
          <Button
            size="sm"
            variant="secondary"
            icon={ArrowLeft}
            onClick={onBack}
            title="Back to events (Esc)"
          >
            Back
          </Button>
          <span className="mono shrink-0 text-accent-ink tabular-nums" title={audit.mainCommitSha}>
            {audit.mainCommitSha.slice(0, 7)}
          </span>
          <span className="min-w-0 flex-1 truncate text-xs font-medium" title={audit.detail}>
            {audit.detail}
          </span>
        </span>
      }
      actions={<StatusBadge kind={meta.kind} label={meta.label} />}
    >
      <div className="mx-auto w-full max-w-[880px] divide-y divide-line px-5">
        <section aria-label="Event" className="py-3">
          <dl>
            <KeyValue label="Subsystem">
              <span className="block truncate" title={audit.subsystemId}>
                {audit.subsystemId}
              </span>
            </KeyValue>
            <KeyValue label="Target branch">
              <span className="mono block truncate tabular-nums" title={audit.targetBranch}>
                {audit.targetBranch}
              </span>
            </KeyValue>
            <KeyValue label="Actor">
              <span className="block truncate" title={audit.actor}>
                {audit.actor}
              </span>
            </KeyValue>
            <KeyValue label="When">
              <span className="mono block truncate tabular-nums" title={audit.occurredAt}>
                {formatDateTime(audit.occurredAt)}
              </span>
            </KeyValue>
            {audit.inheritedFromBranch ? (
              <KeyValue label="Inherited from">
                <span className="block truncate" title={audit.inheritedFromBranch}>
                  {audit.inheritedFromBranch}
                </span>
              </KeyValue>
            ) : null}
            {audit.revertedMainCommitSha ? (
              <KeyValue label="Reverts">
                <span className="inline-flex items-center gap-1.5">
                  <Undo2 className="h-3 w-3 shrink-0 text-muted" aria-hidden="true" />
                  <span className="mono truncate tabular-nums" title={audit.revertedMainCommitSha}>
                    {audit.revertedMainCommitSha.slice(0, 7)}
                  </span>
                </span>
              </KeyValue>
            ) : null}
            {audit.pullRequestNumber && mainRepository ? (
              <KeyValue label="Pull request">
                <button
                  type="button"
                  onClick={() =>
                    onOpenExternal(
                      `https://github.com/${mainRepository}/pull/${audit.pullRequestNumber}`,
                    )
                  }
                  className="inline-flex items-center gap-1 text-accent-ink hover:underline"
                >
                  PR #{audit.pullRequestNumber}
                  <ExternalLink className="h-3 w-3" aria-hidden="true" />
                </button>
              </KeyValue>
            ) : null}
          </dl>
        </section>

        <section aria-label="Main commit" className="space-y-1.5 py-3">
          <div className="flex items-center justify-between gap-2">
            <SectionTitle icon={History}>Main commit</SectionTitle>
            {audit.totals ? (
              <span className="meta text-right tabular-nums">
                {audit.totals.files} file(s) · +{audit.totals.additions} −{audit.totals.deletions}
              </span>
            ) : null}
          </div>
          <p className="mono truncate text-xs text-fg tabular-nums" title={audit.mainCommitSha}>
            {audit.mainCommitSha}
          </p>
          {audit.mainCommit && !audit.mainCommit.unavailable ? (
            <>
              <p
                className="min-w-0 flex-1 truncate text-xs text-fg"
                title={audit.mainCommit.subject || undefined}
              >
                {audit.mainCommit.subject || '—'}
              </p>
              {(audit.mainCommit.author || audit.mainCommit.authoredAt) && (
                <p className="truncate text-[11px] text-muted">
                  {audit.mainCommit.author ?? 'unknown author'}
                  {audit.mainCommit.authoredAt
                    ? ` · ${formatDateTime(audit.mainCommit.authoredAt)}`
                    : ''}
                </p>
              )}
            </>
          ) : null}
          {audit.mainCommitUnavailable ? (
            <p className="text-[11px] leading-relaxed text-muted">
              Not present in the local clone — the change summary below is unavailable.
            </p>
          ) : null}
        </section>

        <section aria-label="Included commits" className="space-y-1.5 py-3">
          <div className="flex items-center justify-between gap-2">
            <SectionTitle icon={GitCommitHorizontal}>Included commits</SectionTitle>
            <span className="meta text-right tabular-nums">{audit.includedCommits.length}</span>
          </div>
          {audit.includedCommits.length === 0 ? (
            <p className="text-[11px] text-disabled">
              No included commits were recorded for this event.
            </p>
          ) : (
            <ol className="divide-y divide-line border-y border-line">
              {audit.includedCommits.map((commit, index) => (
                <li key={commit.sha} className="px-1 py-1.5">
                  <div className="flex min-h-[29px] items-baseline gap-2">
                    <span className="meta shrink-0 text-right tabular-nums">{index + 1}</span>
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
                    <p className="truncate pl-6 text-[11px] text-muted">
                      {commit.author ?? 'unknown author'}
                      {commit.authoredAt ? ` · ${formatDateTime(commit.authoredAt)}` : ''}
                    </p>
                  ) : null}
                </li>
              ))}
            </ol>
          )}
        </section>

        <section aria-label="Changes" className="space-y-1.5 py-3">
          <div className="flex items-center justify-between gap-2">
            <SectionTitle icon={FileText}>Changes</SectionTitle>
            {audit.totals ? (
              <span className="meta text-right tabular-nums">
                {audit.totals.files} file(s) · +{audit.totals.additions} −{audit.totals.deletions}
              </span>
            ) : (
              <span className="meta text-right tabular-nums">{audit.changes.length} file(s)</span>
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
            <>
              {audit.changes.length > 10 ? (
                <div className="relative">
                  <Search
                    className="pointer-events-none absolute top-1/2 left-2.5 h-4 w-4 -translate-y-1/2 text-disabled"
                    aria-hidden="true"
                  />
                  <input
                    type="search"
                    value={fileFilter}
                    onChange={(event) => setFileFilter(event.target.value)}
                    placeholder="Filter files…"
                    aria-label="Filter changed files by path"
                    className="input pl-9"
                  />
                </div>
              ) : null}
              {query && listedChanges.length === 0 ? (
                <p className="text-[11px] text-disabled">No files match this filter.</p>
              ) : (
                <ul className="divide-y divide-line border-y border-line">
                  {listedChanges.map((change) => {
                    const glyph = changeGlyph(change.change);
                    return (
                      <li
                        key={change.path}
                        className="flex min-h-[29px] items-center gap-2 px-1 py-1"
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
                        <span className="w-[62px] shrink-0 truncate text-[11px] text-muted">
                          {change.change}
                        </span>
                        <span className="mono w-14 shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                          +{change.additions}
                        </span>
                        <span className="mono w-14 shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                          −{change.deletions}
                        </span>
                      </li>
                    );
                  })}
                </ul>
              )}
              {query ? (
                <p className="meta tabular-nums">
                  {listedChanges.length} of {audit.changes.length} file(s)
                </p>
              ) : null}
            </>
          )}
        </section>

        {audit.provenance.length > 0 ? (
          <section aria-label="Provenance" className="space-y-1.5 py-3">
            <SectionTitle icon={ScrollText}>Provenance</SectionTitle>
            <dl className="divide-y divide-line border-y border-line">
              {audit.provenance.map((trailer) => (
                <KeyValue key={trailer.key} label={trailer.key}>
                  <span className="mono block truncate tabular-nums" title={trailer.value}>
                    {trailer.value}
                  </span>
                </KeyValue>
              ))}
            </dl>
          </section>
        ) : null}
      </div>
    </Panel>
  );
}

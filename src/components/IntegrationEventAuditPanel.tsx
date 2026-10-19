import { useEffect, useState } from 'react';
import {
  ArrowLeft,
  Columns2,
  ExternalLink,
  FileCode2,
  GitCommitHorizontal,
  History,
  Rows3,
  ScrollText,
  Search,
  Undo2,
  type LucideIcon,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { ApiError } from '../api/invoke';
import type {
  IntegrationEventAudit,
  IntegrationEventChange,
  IntegrationHistoryEntry,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { DiffViewer } from './DiffViewer';
import { INTEGRATION_KIND_META } from './integrationKindMeta';
import { PathLabel } from './PathLabel';
import { StatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import { Button, EmptyState, ErrorState, KeyValue, LoadingState, Panel } from './ui/primitives';
import { SPLIT_DEFAULT_WIDTH, splitWidthStyle } from './ui/split-pane';
import { SplitResizer } from './ui/split-resizer';

/**
 * Full-page audit for one integration event (spec §§3.3/8.3). Replaces the
 * history list within the tab while open — the CommitDiffView precedent — so
 * the audit is read as a page, not glanced at in a side rail.
 *
 * The layout follows the commit browser (GitHub Desktop History): the commit
 * is the panel header, a changed-files column sits at the left with the event's
 * own audit context below it, and beside it the selected file's diff — a file
 * header and its hunks — rendered by the shared `DiffViewer`. The whole main
 * commit's diff arrives in ONE call (`getMainCommitDiff`), keyed by path; the
 * main commit lives in the MAIN repository, so the subsystem-scoped per-file
 * diff command does not apply.
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

function SectionTitle({ icon: Icon, children }: { icon: LucideIcon; children: string }) {
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
  const app = useApp();
  const [fileFilter, setFileFilter] = useState('');
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [mode, setMode] = useState<'side' | 'unified'>('unified');
  const [filesWidth, setFilesWidth] = useState(SPLIT_DEFAULT_WIDTH);

  // Escape returns to the event list — the CommitDiffView idiom.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onBack();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onBack]);

  // The whole main commit's per-file diff in ONE call — never a fetch per file.
  // The loader resolves null while the audit is absent or the main commit is
  // not in the local clone, so no request is issued for an event with nothing
  // to show.
  const diff = useAsyncData(
    () =>
      audit && !audit.mainCommitUnavailable
        ? api.getMainCommitDiff({
            targetBranch: audit.targetBranch,
            mainCommitSha: audit.mainCommitSha,
          })
        : Promise.resolve(null),
    [audit?.targetBranch, audit?.mainCommitSha, audit?.mainCommitUnavailable, app.dataVersion],
  );

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
  // Keyed by path so the single whole-commit read serves every file; a path
  // the patch could not resolve (a mode-only change) has no entry and reads as
  // "no text diff" rather than an invented one.
  const diffByPath = new Map((diff.data?.files ?? []).map((file) => [file.path, file]));
  const activeChange =
    listedChanges.find((change) => change.path === selectedPath) ?? listedChanges[0] ?? null;
  const activeGlyph = activeChange ? changeGlyph(activeChange.change) : null;
  const activeDiff = activeChange ? (diffByPath.get(activeChange.path) ?? null) : null;

  return (
    <Panel
      className="min-h-0 min-w-0 flex-1"
      bodyClassName="flex min-h-0 flex-1 flex-col md:flex-row"
      bodyStyle={splitWidthStyle(filesWidth)}
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
      actions={
        <>
          {audit.totals ? (
            <span className="meta hidden tabular-nums xl:flex">
              {audit.totals.files} file(s) · +{audit.totals.additions} −{audit.totals.deletions}
            </span>
          ) : null}
          <StatusBadge kind={meta.kind} label={meta.label} />
          <Button
            size="sm"
            variant="secondary"
            icon={mode === 'side' ? Columns2 : Rows3}
            onClick={() => setMode(mode === 'side' ? 'unified' : 'side')}
            title={mode === 'side' ? 'Switch to unified diff' : 'Switch to side-by-side diff'}
            aria-label={mode === 'side' ? 'Switch to unified diff' : 'Switch to side-by-side diff'}
          >
            {mode === 'side' ? 'Side-by-side' : 'Unified'}
          </Button>
        </>
      }
    >
      {/* Changed-files column, with the event's own audit context beneath it. */}
      <div className="flex max-h-[45%] min-w-0 shrink-0 flex-col border-b border-line md:max-h-none md:w-[var(--split-width)] md:shrink md:border-b-0">
        <div className="flex h-[29px] shrink-0 items-center justify-between gap-2 px-2.5">
          <p className="text-[11px] font-semibold tracking-wide text-muted uppercase tabular-nums">
            {audit.changes.length} changed file{audit.changes.length === 1 ? '' : 's'}
          </p>
          {audit.totals ? (
            <span className="mono shrink-0 text-[11px] tabular-nums">
              <span className="text-status-applied-ink">+{audit.totals.additions}</span>{' '}
              <span className="text-status-blocked-ink">−{audit.totals.deletions}</span>
            </span>
          ) : null}
        </div>

        {audit.changes.length > 0 ? (
          <div className="relative shrink-0 px-2.5 pb-2">
            <Search
              className="pointer-events-none absolute top-1/2 left-5 h-4 w-4 -translate-y-[calc(50%+4px)] text-disabled"
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

        <div className="scroll-area min-h-0 flex-1 overflow-y-auto">
          {audit.mainCommitUnavailable ? (
            <p className="px-2.5 pb-2 text-[11px] leading-relaxed text-muted">
              The change summary isn&apos;t available — the main commit isn&apos;t in the local
              clone.
            </p>
          ) : audit.changes.length === 0 ? (
            <p className="px-2.5 pb-2 text-[11px] text-disabled">
              No file changes were recorded for this event.
            </p>
          ) : listedChanges.length === 0 ? (
            <p className="px-2.5 pb-2 text-[11px] text-disabled">No files match this filter.</p>
          ) : (
            <ul className="px-1 pb-2">
              {listedChanges.map((change) => {
                const glyph = changeGlyph(change.change);
                const active = activeChange?.path === change.path;
                return (
                  <li key={change.path}>
                    <button
                      type="button"
                      onClick={() => setSelectedPath(change.path)}
                      aria-current={active ? 'true' : undefined}
                      title={change.path}
                      className={cx(
                        'flex min-h-[29px] w-full items-center gap-2 rounded-md px-1.5 py-1 text-left transition-colors hover:bg-hover',
                        active && 'bg-selection',
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
                      <PathLabel path={change.path} className="flex-1" />
                      <span className="mono w-11 shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                        +{change.additions}
                      </span>
                      <span className="mono w-11 shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                        −{change.deletions}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
          {query && listedChanges.length > 0 ? (
            <p className="px-2.5 pb-2 text-[11px] text-muted tabular-nums">
              {listedChanges.length} of {audit.changes.length} file(s)
            </p>
          ) : null}

          <div className="divide-y divide-line border-t border-line">
            <section aria-label="Event" className="space-y-1 px-2.5 py-3">
              <SectionTitle icon={History}>Event</SectionTitle>
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
                      <span
                        className="mono truncate tabular-nums"
                        title={audit.revertedMainCommitSha}
                      >
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

            <section aria-label="Main commit" className="space-y-1 px-2.5 py-3">
              <SectionTitle icon={History}>Main commit</SectionTitle>
              <p className="mono truncate text-xs text-fg tabular-nums" title={audit.mainCommitSha}>
                {audit.mainCommitSha}
              </p>
              {audit.mainCommit && !audit.mainCommit.unavailable ? (
                <>
                  <p
                    className="min-w-0 truncate text-xs text-fg"
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
            </section>

            <section aria-label="Included commits" className="space-y-1 px-2.5 py-3">
              <div className="flex items-center justify-between gap-2">
                <SectionTitle icon={GitCommitHorizontal}>Included commits</SectionTitle>
                <span className="meta tabular-nums">{audit.includedCommits.length}</span>
              </div>
              {audit.includedCommits.length === 0 ? (
                <p className="text-[11px] text-disabled">
                  No included commits were recorded for this event.
                </p>
              ) : (
                <ol className="divide-y divide-line border-y border-line">
                  {audit.includedCommits.map((commit, index) => (
                    <li key={commit.sha} className="py-1.5">
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

            {audit.provenance.length > 0 ? (
              <section aria-label="Provenance" className="space-y-1 px-2.5 py-3">
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
        </div>
      </div>

      <SplitResizer
        width={filesWidth}
        onWidthChange={setFilesWidth}
        label="Resize the changed-files column"
      />

      {/* Selected file's diff — a file header and its hunks. */}
      <div className="scroll-area min-h-0 min-w-0 flex-1 overflow-y-auto bg-ground md:min-w-[320px]">
        {activeChange && activeGlyph ? (
          <>
            <div className="flex min-h-[29px] items-center gap-2 border-b border-line bg-raised px-2.5 py-1">
              <span
                className={cx('w-3 shrink-0 text-center text-sm font-bold', activeGlyph.className)}
                aria-hidden="true"
              >
                {activeGlyph.glyph}
              </span>
              <PathLabel path={activeChange.path} className="flex-1" />
              <span className="meta shrink-0">{activeChange.change}</span>
              <span className="mono shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                +{activeChange.additions}
              </span>
              <span className="mono shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                −{activeChange.deletions}
              </span>
            </div>
            {diff.loading && !diff.data ? (
              <LoadingState label="Loading the commit diff…" />
            ) : diff.error ? (
              <ErrorState error={diff.error} onRetry={diff.reload} compact />
            ) : activeDiff ? (
              <DiffViewer
                diff={activeDiff}
                loading={false}
                error={null}
                onRetry={diff.reload}
                mode={mode}
                onOpenExternal={onOpenExternal}
              />
            ) : (
              <EmptyState
                icon={FileCode2}
                title="No text diff for this file"
                message={activeChange.path}
                className="p-4"
              />
            )}
          </>
        ) : (
          <EmptyState
            icon={FileCode2}
            title="No file selected"
            message={
              audit.mainCommitUnavailable
                ? 'The main commit isn’t in the local clone, so there is no diff to show.'
                : 'No file changes were recorded for this event.'
            }
          />
        )}
      </div>
    </Panel>
  );
}

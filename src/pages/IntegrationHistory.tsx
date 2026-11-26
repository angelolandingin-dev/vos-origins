import { Fragment, useCallback, useMemo, useState } from 'react';
import {
  ArrowDownToLine,
  ExternalLink,
  History,
  RefreshCw,
  Search,
  ShieldCheck,
  TriangleAlert,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { IntegrationHistoryKind, IntegrationPr } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { IntegrationEventAuditPage } from '../components/IntegrationEventAuditPanel';
import { INTEGRATION_KIND_META } from '../components/integrationKindMeta';
import { StatusBadge } from '../components/StatusBadge';
import { cx } from '../components/ui/cx';
import { formatDateTime } from '../components/ui/format';
import {
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  InlineAlert,
  LoadingState,
  Select,
  Spinner,
} from '../components/ui/primitives';

/** Integration History (spec §§3.3/8.3). Pending PRs are never shown as integrated; provenance problems surface. */

const HISTORY_LIMITS = [50, 100, 200];
const HISTORY_CAP = 200;

function prStateBadge(pr: IntegrationPr) {
  if (pr.state === 'merged') return <StatusBadge kind="integrated" label="Merged" />;
  if (pr.state === 'open')
    return <StatusBadge kind={pr.reviewState === 'changes_requested' ? 'in_review' : 'pr_open'} />;
  return <StatusBadge kind="unknown" label="Closed" />;
}

export function IntegrationHistory() {
  const app = useApp();
  const [subsystemFilter, setSubsystemFilter] = useState<string>('all');
  const [limit, setLimit] = useState<number>(HISTORY_LIMITS[0] ?? 50);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [view, setView] = useState<'events' | 'prs'>('events');
  const [search, setSearch] = useState('');
  const [kindFilter, setKindFilter] = useState<'all' | IntegrationHistoryKind>('all');

  const history = useAsyncData(
    () =>
      api.listIntegrationHistory(
        app.targetBranch,
        subsystemFilter === 'all' ? null : subsystemFilter,
        limit,
      ),
    [app.targetBranch, subsystemFilter, limit, app.dataVersion],
  );

  const prs = useAsyncData(
    () =>
      api.listIntegrationPrs(app.targetBranch, subsystemFilter === 'all' ? null : subsystemFilter),
    [app.targetBranch, subsystemFilter, app.dataVersion],
  );

  const openInCommitBrowser = useCallback(
    (subsystemId: string, sha: string) => {
      if (app.selectedSubsystemId !== subsystemId) app.selectSubsystem(subsystemId);
      app.setFocusedSha(sha);
      app.navigate('promote');
    },
    [app],
  );

  const entries = useMemo(() => history.data?.entries ?? [], [history.data]);
  const prsList = useMemo(() => prs.data ?? [], [prs.data]);
  const total = history.data?.total ?? 0;
  const shown = entries.length;
  const selectedEntry =
    entries.find((entry) => entry.id === selectedId && entry.mainCommitSha) ?? null;

  const filteredEntries = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return entries.filter((entry) => {
      if (kindFilter !== 'all' && entry.kind !== kindFilter) return false;
      if (!needle) return true;
      return `${entry.subsystemId} ${entry.detail} ${entry.actor} ${entry.mainCommitSha ?? ''} ${entry.sourceShas.join(' ')} ${entry.pullRequestNumber ?? ''}`
        .toLowerCase()
        .includes(needle);
    });
  }, [entries, kindFilter, search]);

  const filteredPrs = useMemo(() => {
    const needle = search.trim().toLowerCase();
    if (!needle) return prsList;
    return prsList.filter((pr) =>
      `${pr.number} ${pr.title} ${pr.subsystemId} ${pr.headBranch} ${pr.author} ${pr.sourceShas.join(' ')}`
        .toLowerCase()
        .includes(needle),
    );
  }, [prsList, search]);

  // Fetched on selection only, keyed by the selected event.
  // The audit replaces the list within the tab while open: selecting a row
  // opens the full page, Back / Escape returns.
  const closeAudit = useCallback(() => setSelectedId(null), []);
  const audit = useAsyncData(
    () =>
      selectedEntry && selectedEntry.mainCommitSha
        ? api.getIntegrationEvent({
            targetBranch: selectedEntry.targetBranch,
            subsystemId: selectedEntry.subsystemId,
            mainCommitSha: selectedEntry.mainCommitSha,
          })
        : Promise.resolve(null),
    [
      selectedEntry?.targetBranch,
      selectedEntry?.subsystemId,
      selectedEntry?.mainCommitSha,
      app.dataVersion,
    ],
  );

  const visibleAudit = audit.loading ? null : audit.data;

  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);
  const selectedBranch = app.targetBranches.data?.branches.find(
    (branch) => branch.name === app.targetBranch,
  );

  const countLine = history.data
    ? shown === total
      ? `${total} event(s)`
      : `Showing ${shown} of ${total}`
    : undefined;
  const capped = limit >= HISTORY_CAP && total > shown;
  const isRefreshing =
    (view === 'events' && history.loading && shown > 0) ||
    (view === 'prs' && prs.loading && prsList.length > 0);
  const kindEntries = Object.entries(INTEGRATION_KIND_META) as Array<
    [IntegrationHistoryKind, (typeof INTEGRATION_KIND_META)[IntegrationHistoryKind]]
  >;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-2.5">
        <div className="flex min-w-0 items-center gap-2.5">
          <h1 className="text-[14px] font-semibold text-fg">Integration History</h1>
          {selectedBranch ? (
            <span className="mono hidden truncate text-[11px] text-muted sm:inline">
              tip {selectedBranch.headSha.slice(0, 7)}
            </span>
          ) : null}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted">Branch</span>
            <Select
              ariaLabel="Target main-system branch"
              className="w-[180px]"
              value={app.targetBranch}
              onValueChange={(next) => app.setTargetBranch(next)}
              options={branches.map((branch) => ({
                value: branch.name,
                label: branch.name,
              }))}
            />
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted">Subsystem</span>
            <Select
              ariaLabel="Filter history by subsystem"
              className="w-[210px]"
              value={subsystemFilter}
              onValueChange={(next) => setSubsystemFilter(next)}
              options={[
                { value: 'all', label: 'All subsystems' },
                ...(app.subsystems.data ?? []).map((subsystem) => ({
                  value: subsystem.id,
                  label: subsystem.name,
                })),
              ]}
            />
          </div>
          <Button
            variant="secondary"
            icon={ArrowDownToLine}
            loading={app.pullingCheckouts}
            disabled={app.pullingCheckouts}
            onClick={() => app.pullLatest(subsystemFilter === 'all' ? null : subsystemFilter)}
            title="Fetch the latest refs for the cached checkouts"
          >
            {app.pullingCheckouts ? 'Pulling…' : 'Pull latest'}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            icon={RefreshCw}
            loading={history.loading || prs.loading}
            onClick={() => {
              history.reload();
              prs.reload();
              audit.reload();
            }}
          >
            Refresh
          </Button>
        </div>
      </header>

      {selectedEntry ? (
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden p-3">
          <IntegrationEventAuditPage
            entry={selectedEntry}
            audit={visibleAudit}
            loading={audit.loading}
            error={audit.error}
            onRetry={audit.reload}
            onOpenExternal={app.openExternal}
            mainRepository={app.auth.data?.mainRepository ?? null}
            onBack={closeAudit}
          />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col bg-surface">
          <div className="flex flex-wrap items-center gap-2 border-b border-line bg-raised px-3 py-2">
            <div
              role="group"
              aria-label="History view"
              className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
            >
              {(
                [
                  { id: 'events', label: `Events (${shown})` },
                  { id: 'prs', label: `Pull requests (${prsList.length})` },
                ] as const
              ).map((tab, idx) => (
                <button
                  key={tab.id}
                  type="button"
                  onClick={() => setView(tab.id)}
                  className={cx(
                    'px-2.5 text-[11px] tabular-nums transition-colors',
                    idx > 0 && 'border-l border-line',
                    view === tab.id
                      ? 'bg-raised font-semibold text-fg'
                      : 'text-muted hover:bg-hover hover:text-fg',
                  )}
                >
                  {tab.label}
                </button>
              ))}
            </div>

            <div className="relative min-w-[200px] flex-1">
              <Search
                className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                aria-hidden="true"
              />
              <input
                type="search"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Filter by SHA, subsystem, detail, or actor…"
                aria-label="Filter history"
                className="input pl-8"
              />
            </div>

            {view === 'events' ? (
              <div
                role="group"
                aria-label="Filter events by kind"
                className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
              >
                <button
                  type="button"
                  onClick={() => setKindFilter('all')}
                  className={cx(
                    'px-2 text-[11px] transition-colors',
                    kindFilter === 'all'
                      ? 'bg-raised font-semibold text-fg'
                      : 'text-muted hover:bg-hover hover:text-fg',
                  )}
                >
                  All
                </button>
                {kindEntries.map(([id, meta]) => (
                  <button
                    key={id}
                    type="button"
                    onClick={() => setKindFilter(id)}
                    className={cx(
                      'border-l border-line px-2 text-[11px] transition-colors',
                      kindFilter === id
                        ? 'bg-raised font-semibold text-fg'
                        : 'text-muted hover:bg-hover hover:text-fg',
                    )}
                  >
                    {meta.label}
                  </button>
                ))}
              </div>
            ) : null}

            <div className="ml-auto flex shrink-0 items-center gap-2 text-[11px] text-muted">
              {isRefreshing ? (
                <span className="flex items-center gap-1">
                  <Spinner className="h-3 w-3" />
                  Updating…
                </span>
              ) : null}
              {countLine ? <span className="mono tabular-nums">{countLine}</span> : null}
            </div>
          </div>

          {isRefreshing ? (
            <div
              role="progressbar"
              aria-label="Refreshing integration history"
              className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
            >
              <div className="h-full w-1/2 animate-pulse bg-accent" />
            </div>
          ) : null}

          <div
            className={cx(
              'scroll-area min-h-0 flex-1 transition-opacity',
              isRefreshing && 'pointer-events-none opacity-55',
            )}
          >
            {view === 'events' ? (
              history.loading && shown === 0 ? (
                <LoadingState label="Loading integration history…" />
              ) : history.error ? (
                <ErrorState error={history.error} onRetry={history.reload} />
              ) : shown === 0 ? (
                <EmptyState
                  icon={History}
                  title="No integration events for this branch"
                  message="Promotions appear here after an integration PR is merged."
                />
              ) : filteredEntries.length === 0 ? (
                <EmptyState
                  icon={Search}
                  title="No matching events"
                  message="Clear the search or event-kind filter to see all loaded events."
                  action={
                    <Button
                      size="sm"
                      onClick={() => {
                        setSearch('');
                        setKindFilter('all');
                      }}
                    >
                      Reset filters
                    </Button>
                  }
                />
              ) : (
                <>
                  {capped ? (
                    <InlineAlert
                      tone="info"
                      title={`Showing ${shown} of ${total} events — the list is capped at ${HISTORY_CAP}`}
                      className="m-3 mb-0"
                    >
                      Narrow the branch or subsystem filter to reach older events.
                    </InlineAlert>
                  ) : null}
                  <table className="w-full border-separate border-spacing-0 text-xs">
                    <thead className="sticky top-0 z-10">
                      <tr className="table-head">
                        <th scope="col" className="border-b border-line py-2 pl-4 text-left">
                          Event
                        </th>
                        <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                          Subsystem
                        </th>
                        <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                          Source SHA(s)
                        </th>
                        <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                          Main commit / PR
                        </th>
                        <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                          Detail
                        </th>
                        <th scope="col" className="border-b border-line py-2 pr-4 text-right">
                          When
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {filteredEntries.map((entry) => {
                        const auditable = entry.mainCommitSha != null;
                        const selected = selectedId === entry.id;
                        const select = () => setSelectedId(entry.id);
                        return (
                          <tr
                            key={entry.id}
                            onClick={auditable ? select : undefined}
                            onKeyDown={
                              auditable
                                ? (event) => {
                                    if (event.key === 'Enter' || event.key === ' ') {
                                      event.preventDefault();
                                      select();
                                    }
                                  }
                                : undefined
                            }
                            tabIndex={auditable ? 0 : undefined}
                            aria-selected={auditable ? selected : undefined}
                            className={cx(
                              'row-hover border-b border-line align-top',
                              auditable &&
                                'cursor-pointer focus-visible:bg-hover focus-visible:outline-none',
                              selected && 'bg-selection',
                            )}
                          >
                            <td className="border-b border-line py-2.5 pl-4">
                              <StatusBadge
                                kind={INTEGRATION_KIND_META[entry.kind].kind}
                                label={INTEGRATION_KIND_META[entry.kind].label}
                              />
                              {entry.inheritedFromBranch ? (
                                <span className="mt-1 block text-[11px] text-status-unknown-ink">
                                  inherited from {entry.inheritedFromBranch}
                                </span>
                              ) : null}
                            </td>
                            <td className="truncate border-b border-line py-2.5 pr-3 text-fg">
                              {entry.subsystemId}
                            </td>
                            <td
                              className="mono truncate border-b border-line py-2.5 pr-3 text-muted tabular-nums"
                              title={entry.sourceShas.join('\n')}
                            >
                              {entry.sourceShas.length === 0
                                ? '—'
                                : entry.sourceShas.map((sha, index) => (
                                    <Fragment key={sha}>
                                      {index > 0 ? ', ' : null}
                                      <button
                                        type="button"
                                        onClick={(event) => {
                                          event.stopPropagation();
                                          openInCommitBrowser(entry.subsystemId, sha);
                                        }}
                                        title={`${sha} — open in the Commit Browser`}
                                        className="cursor-pointer hover:text-fg hover:underline"
                                      >
                                        {sha.slice(0, 7)}
                                      </button>
                                    </Fragment>
                                  ))}
                            </td>
                            <td className="border-b border-line py-2.5 pr-3">
                              {entry.mainCommitSha ? (
                                <span className="mono block text-muted tabular-nums">
                                  {entry.mainCommitSha.slice(0, 7)}
                                </span>
                              ) : null}
                              {entry.pullRequestNumber ? (
                                <button
                                  type="button"
                                  onClick={(event) => {
                                    // The PR link leaves for GitHub; it must not select the row.
                                    event.stopPropagation();
                                    app.openExternal(
                                      `https://github.com/${app.auth.data?.mainRepository ?? ''}/pull/${entry.pullRequestNumber}`,
                                    );
                                  }}
                                  className="mt-0.5 inline-flex items-center gap-1 text-accent-ink hover:underline"
                                >
                                  PR #{entry.pullRequestNumber}
                                  <ExternalLink className="h-3 w-3" aria-hidden="true" />
                                </button>
                              ) : (
                                <span className="text-disabled">—</span>
                              )}
                            </td>
                            <td
                              className="max-w-[38ch] truncate border-b border-line py-2.5 pr-3 text-muted"
                              title={entry.detail}
                            >
                              {entry.detail}
                            </td>
                            <td className="mono border-b border-line py-2.5 pr-4 text-right whitespace-nowrap text-muted tabular-nums">
                              {formatDateTime(entry.occurredAt)}
                              <span className="block truncate text-[11px] text-disabled">
                                {entry.actor}
                              </span>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                  {shown < total ? (
                    <div className="flex justify-center border-b border-line px-4 py-2">
                      <Button
                        size="sm"
                        variant="secondary"
                        disabled={history.loading}
                        onClick={() =>
                          setLimit((current) =>
                            Math.min(HISTORY_CAP, current >= 100 ? HISTORY_CAP : 100),
                          )
                        }
                        title={`Load more events (${shown} of ${total} shown)`}
                      >
                        Show more ({shown} of {total})
                      </Button>
                    </div>
                  ) : null}
                </>
              )
            ) : prs.loading && prsList.length === 0 ? (
              <LoadingState label="Loading pull requests…" />
            ) : prs.error ? (
              <ErrorState error={prs.error} onRetry={prs.reload} />
            ) : prsList.length === 0 ? (
              <EmptyState
                icon={ShieldCheck}
                title="No integration PRs for this branch"
                message="Integration PRs created by this tool appear here."
              />
            ) : filteredPrs.length === 0 ? (
              <EmptyState
                icon={Search}
                title="No matching pull requests"
                message="Clear the search filter to see all loaded integration pull requests."
                action={
                  <Button size="sm" onClick={() => setSearch('')}>
                    Clear search
                  </Button>
                }
              />
            ) : (
              <ul className="divide-y divide-line">
                {filteredPrs.map((pr) => (
                  <li key={pr.number} className="space-y-1.5 px-4 py-3">
                    <div className="flex flex-wrap items-center gap-2">
                      {prStateBadge(pr)}
                      <span className="mono text-right text-fg tabular-nums">#{pr.number}</span>
                      <span className="min-w-0 flex-1 truncate text-xs text-fg">{pr.title}</span>
                      <span className="meta">{pr.subsystemId}</span>
                      <span className="mono truncate text-disabled">base {pr.baseBranch}</span>
                      <IconButton
                        icon={ExternalLink}
                        label={`Open PR #${pr.number}`}
                        onClick={() => app.openExternal(pr.htmlUrl)}
                      />
                    </div>
                    <p className="mono truncate text-disabled tabular-nums">
                      {pr.headBranch} · source{' '}
                      {pr.sourceShas.map((sha) => sha.slice(0, 7)).join(', ')} · {pr.author} ·{' '}
                      {formatDateTime(pr.updatedAt)}
                      {pr.mergedAt
                        ? ` · merged ${formatDateTime(pr.mergedAt)} by ${pr.mergedBy ?? 'unknown'}`
                        : ''}
                    </p>
                    {!pr.provenanceOk ? (
                      <p
                        className={cx(
                          'flex items-start gap-2 rounded-md border border-status-unknown/40 bg-status-unknown-soft px-2.5 py-2 text-[11px] text-status-unknown-ink',
                        )}
                      >
                        <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                        <span className="truncate">
                          <span className="font-semibold">Provenance integrity:</span>{' '}
                          {pr.provenanceNote}
                        </span>
                      </p>
                    ) : null}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

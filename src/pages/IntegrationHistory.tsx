import { useState } from 'react';
import { ExternalLink, History, RefreshCw, ShieldCheck, TriangleAlert } from 'lucide-react';

import { api } from '../api/invoke';
import type { CommitStatusKind, IntegrationHistoryKind, IntegrationPr } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
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
  Panel,
  Select,
} from '../components/ui/primitives';

/** Integration History (spec §§3.3/8.3). Pending PRs are never shown as integrated; provenance problems surface. */

const KIND: Record<IntegrationHistoryKind, { label: string; kind: CommitStatusKind }> = {
  merged: { label: 'Merged', kind: 'integrated' },
  pending_pr: { label: 'Pending PR', kind: 'pr_open' },
  reverted: { label: 'Reverted', kind: 'reverted' },
  restored: { label: 'Restored', kind: 'integrated' },
  failed: { label: 'Failed', kind: 'failed' },
};

function prStateBadge(pr: IntegrationPr) {
  if (pr.state === 'merged') return <StatusBadge kind="integrated" label="Merged" />;
  if (pr.state === 'open')
    return <StatusBadge kind={pr.reviewState === 'changes_requested' ? 'in_review' : 'pr_open'} />;
  return <StatusBadge kind="unknown" label="Closed" />;
}

export function IntegrationHistory() {
  const app = useApp();
  const [subsystemFilter, setSubsystemFilter] = useState<string>('all');

  const history = useAsyncData(
    () =>
      api.listIntegrationHistory(
        app.targetBranch,
        subsystemFilter === 'all' ? null : subsystemFilter,
        100,
      ),
    [app.targetBranch, subsystemFilter, app.dataVersion],
  );

  const prs = useAsyncData(
    () =>
      api.listIntegrationPrs(app.targetBranch, subsystemFilter === 'all' ? null : subsystemFilter),
    [app.targetBranch, subsystemFilter, app.dataVersion],
  );

  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);
  const selectedBranch = app.targetBranches.data?.branches.find(
    (branch) => branch.name === app.targetBranch,
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[50px] flex-wrap items-center gap-3 border-b border-line px-5 py-2">
        <h1 className="text-[14px] font-semibold text-fg">Integration History</h1>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <Select
            aria-label="Target main-system branch"
            className="w-[200px]"
            value={app.targetBranch}
            onChange={(event) => app.setTargetBranch(event.target.value)}
            options={branches.map((branch) => ({
              value: branch.name,
              label: `branch: ${branch.name}`,
            }))}
          />
          <Select
            aria-label="Filter history by subsystem"
            className="w-[220px]"
            value={subsystemFilter}
            onChange={(event) => setSubsystemFilter(event.target.value)}
            options={[
              { value: 'all', label: 'All subsystems' },
              ...(app.subsystems.data ?? []).map((subsystem) => ({
                value: subsystem.id,
                label: subsystem.name,
              })),
            ]}
          />
          <Button
            variant="secondary"
            icon={RefreshCw}
            onClick={() => {
              history.reload();
              prs.reload();
            }}
          >
            Refresh
          </Button>
        </div>
      </header>

      <div className="scroll-area min-h-0 flex-1 space-y-3 p-3">
        <InlineAlert
          tone="info"
          title="Status comes from reachable Git history — no ledger, no shadow database"
        >
          <span className="mono">
            {app.targetBranch}
            {selectedBranch ? ` · tip ${selectedBranch.headSha.slice(0, 7)}` : ''}
          </span>
        </InlineAlert>

        <Panel
          title={`Promotion events on ${app.targetBranch}`}
          subtitle={history.data ? `${history.data.total} event(s)` : undefined}
          bodyClassName="scroll-area max-h-[420px] overflow-x-auto"
        >
          {history.loading && (history.data?.entries.length ?? 0) === 0 ? (
            <LoadingState label="Loading integration history…" />
          ) : history.error ? (
            <ErrorState error={history.error} onRetry={history.reload} />
          ) : (history.data?.entries.length ?? 0) === 0 ? (
            <EmptyState
              icon={History}
              title="No integration events for this branch"
              message="Promotions appear here after an integration PR is merged."
            />
          ) : (
            <table className="w-full border-separate border-spacing-0 text-xs">
              <thead>
                <tr className="table-head">
                  <th scope="col" className="py-2 pl-4 text-left">
                    Event
                  </th>
                  <th scope="col" className="py-2 pr-3 text-left">
                    Subsystem
                  </th>
                  <th scope="col" className="py-2 pr-3 text-left">
                    Source SHA(s)
                  </th>
                  <th scope="col" className="py-2 pr-3 text-left">
                    Main commit / PR
                  </th>
                  <th scope="col" className="py-2 pr-3 text-left">
                    Detail
                  </th>
                  <th scope="col" className="py-2 pr-4 text-right">
                    When
                  </th>
                </tr>
              </thead>
              <tbody>
                {(history.data?.entries ?? []).map((entry) => (
                  <tr key={entry.id} className="row-hover border-b border-line align-top">
                    <td className="py-2.5 pl-4">
                      <StatusBadge kind={KIND[entry.kind].kind} label={KIND[entry.kind].label} />
                      {entry.inheritedFromBranch ? (
                        <span className="mt-1 block text-[11px] text-status-unknown-ink">
                          inherited from {entry.inheritedFromBranch}
                        </span>
                      ) : null}
                    </td>
                    <td className="truncate py-2.5 pr-3 text-fg">{entry.subsystemId}</td>
                    <td
                      className="mono truncate py-2.5 pr-3 text-muted tabular-nums"
                      title={entry.sourceShas.join('\n')}
                    >
                      {entry.sourceShas.map((sha) => sha.slice(0, 7)).join(', ') || '—'}
                    </td>
                    <td className="py-2.5 pr-3">
                      {entry.mainCommitSha ? (
                        <span className="mono block text-muted tabular-nums">
                          {entry.mainCommitSha.slice(0, 7)}
                        </span>
                      ) : null}
                      {entry.pullRequestNumber ? (
                        <button
                          type="button"
                          onClick={() =>
                            app.openExternal(
                              `https://github.com/${app.auth.data?.mainRepository ?? ''}/pull/${entry.pullRequestNumber}`,
                            )
                          }
                          className="mt-0.5 inline-flex items-center gap-1 text-accent-ink hover:underline"
                        >
                          PR #{entry.pullRequestNumber}
                          <ExternalLink className="h-3 w-3" aria-hidden="true" />
                        </button>
                      ) : (
                        <span className="text-disabled">—</span>
                      )}
                    </td>
                    <td className="max-w-[38ch] truncate py-2.5 pr-3 text-muted">{entry.detail}</td>
                    <td className="mono py-2.5 pr-4 text-right whitespace-nowrap text-muted tabular-nums">
                      {formatDateTime(entry.occurredAt)}
                      <span className="block truncate text-[11px] text-disabled">
                        {entry.actor}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>

        <Panel
          title="Integration pull requests"
          subtitle="Open, merged and closed integration PRs whose base is the selected branch"
        >
          {prs.loading && (prs.data?.length ?? 0) === 0 ? (
            <LoadingState label="Loading pull requests…" />
          ) : prs.error ? (
            <ErrorState error={prs.error} onRetry={prs.reload} />
          ) : (prs.data?.length ?? 0) === 0 ? (
            <EmptyState
              icon={ShieldCheck}
              title="No integration PRs for this branch"
              message="Integration PRs created by this tool appear here."
            />
          ) : (
            <ul className="divide-y divide-line">
              {(prs.data ?? []).map((pr) => (
                <li key={pr.number} className="space-y-1.5 px-4 py-3">
                  <div className="flex flex-wrap items-center gap-2">
                    {prStateBadge(pr)}
                    <span className="mono text-right text-fg tabular-nums">#{pr.number}</span>
                    <span className="min-w-0 flex-1 truncate text-xs text-fg">{pr.title}</span>
                    <span className="chip">{pr.subsystemId}</span>
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
        </Panel>
      </div>
    </div>
  );
}

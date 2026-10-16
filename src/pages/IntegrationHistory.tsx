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
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pb-3 pt-4">
        <div>
          <h1 className="flex items-center gap-2 text-xl font-semibold tracking-tight text-white">
            <History className="h-5 w-5 text-brand-400" aria-hidden="true" />
            Integration History
          </h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Promotion events derived from commits reachable from the selected main-system branch
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
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

      <div className="scroll-area min-h-0 flex-1 space-y-3 px-3 pb-3">
        <InlineAlert tone="info" title="How status is derived">
          Integration state comes from the main-system Git history reachable from{' '}
          <span className="mono">{app.targetBranch}</span>
          {selectedBranch ? (
            <>
              {' '}
              (tip <span className="mono">{selectedBranch.headSha.slice(0, 7)}</span>)
            </>
          ) : null}
          . A branch can inherit records through a main-system branch merge; those rows are labelled
          with the branch they came from. There is no integrated-commits.json ledger and no shadow
          database.
        </InlineAlert>

        <Panel
          title={`Promotion events on ${app.targetBranch}`}
          subtitle={history.data ? `${history.data.total} event(s)` : undefined}
          bodyClassName="scroll-area max-h-[420px]"
        >
          {history.loading && (history.data?.entries.length ?? 0) === 0 ? (
            <LoadingState label="Loading integration history…" />
          ) : history.error ? (
            <ErrorState error={history.error} onRetry={history.reload} />
          ) : (history.data?.entries.length ?? 0) === 0 ? (
            <EmptyState
              icon={History}
              title="No integration events for this branch"
              message="Promotions appear here after an integration PR is merged on this target branch. Promoting to a different branch does not create a record here."
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
                  <tr key={entry.id} className="row-hover border-b border-ink-800/70 align-top">
                    <td className="py-2.5 pl-4">
                      <StatusBadge kind={KIND[entry.kind].kind} label={KIND[entry.kind].label} />
                      {entry.inheritedFromBranch ? (
                        <span className="mt-1 block text-2xs text-amber-200">
                          inherited from {entry.inheritedFromBranch}
                        </span>
                      ) : null}
                    </td>
                    <td className="py-2.5 pr-3 text-slate-200">{entry.subsystemId}</td>
                    <td
                      className="mono py-2.5 pr-3 text-slate-300"
                      title={entry.sourceShas.join('\n')}
                    >
                      {entry.sourceShas.map((sha) => sha.slice(0, 7)).join(', ') || '—'}
                    </td>
                    <td className="py-2.5 pr-3">
                      {entry.mainCommitSha ? (
                        <span className="mono block text-slate-300">
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
                          className="mt-0.5 inline-flex items-center gap-1 text-brand-300 hover:underline"
                        >
                          PR #{entry.pullRequestNumber}
                          <ExternalLink className="h-3 w-3" aria-hidden="true" />
                        </button>
                      ) : (
                        <span className="text-slate-500">—</span>
                      )}
                    </td>
                    <td className="max-w-[38ch] py-2.5 pr-3 text-slate-400">{entry.detail}</td>
                    <td className="whitespace-nowrap py-2.5 pr-4 text-right text-slate-400">
                      {formatDateTime(entry.occurredAt)}
                      <span className="block text-2xs text-slate-500">{entry.actor}</span>
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
              message="Integration PRs are created by the promotion workflow with base equal to the selected target branch."
            />
          ) : (
            <ul className="divide-y divide-ink-800/70">
              {(prs.data ?? []).map((pr) => (
                <li key={pr.number} className="space-y-1.5 px-4 py-3">
                  <div className="flex flex-wrap items-center gap-2">
                    {prStateBadge(pr)}
                    <span className="text-xs font-medium text-slate-100">#{pr.number}</span>
                    <span className="min-w-0 flex-1 truncate text-xs text-slate-200">
                      {pr.title}
                    </span>
                    <span className="chip bg-ink-800 text-slate-300">{pr.subsystemId}</span>
                    <span className="mono text-2xs text-slate-500">base {pr.baseBranch}</span>
                    <IconButton
                      icon={ExternalLink}
                      label={`Open PR #${pr.number}`}
                      onClick={() => app.openExternal(pr.htmlUrl)}
                    />
                  </div>
                  <p className="mono text-2xs text-slate-500">
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
                        'flex items-start gap-2 rounded-lg border border-amber-900/60 bg-amber-950/30 px-2.5 py-2 text-2xs text-amber-200',
                      )}
                    >
                      <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                      <span>
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

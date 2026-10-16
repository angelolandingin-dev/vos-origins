import { useMemo } from 'react';
import {
  ExternalLink,
  GitBranch,
  GitPullRequestArrow,
  History,
  Plus,
  RefreshCw,
  Workflow,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { CommitStatusKind, IntegrationHistoryKind } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { StatusBadge } from '../components/StatusBadge';
import { formatDateTime } from '../components/ui/format';
import {
  Button,
  EmptyState,
  ErrorState,
  LoadingState,
  Panel,
  Select,
} from '../components/ui/primitives';

/** Dashboard (spec §3.3). Counts use the same branch-scoped engine as the Commit Browser — never an invented status. */

const COUNTED: Array<{ key: CommitStatusKind; label: string; group: string }> = [
  { key: 'eligible', label: 'Eligible', group: 'awaiting' },
  { key: 'promotion_queued', label: 'Promotion queued', group: 'inflight' },
  { key: 'promotion_running', label: 'Promotion running', group: 'inflight' },
  { key: 'pr_open', label: 'PR open', group: 'inflight' },
  { key: 'in_review', label: 'In review', group: 'inflight' },
  { key: 'integrated', label: 'Integrated', group: 'done' },
  { key: 'reverted', label: 'Reverted', group: 'attention' },
  { key: 'blocked', label: 'Blocked', group: 'attention' },
  { key: 'failed', label: 'Failed', group: 'attention' },
  { key: 'unknown', label: 'Unknown', group: 'attention' },
];

const HISTORY_LABEL: Record<IntegrationHistoryKind, { label: string; kind: CommitStatusKind }> = {
  merged: { label: 'Merged', kind: 'integrated' },
  pending_pr: { label: 'Pending PR', kind: 'pr_open' },
  reverted: { label: 'Reverted', kind: 'reverted' },
  restored: { label: 'Restored', kind: 'integrated' },
  failed: { label: 'Failed', kind: 'failed' },
};

export function Dashboard() {
  const app = useApp();
  const subsystems = app.subsystems.data ?? [];

  const counts = useAsyncData(async () => {
    const summaries = await api.getSubsystemSummaries();
    const byId = new Map(summaries.map((summary) => [summary.subsystemId, summary]));
    return subsystems.map((subsystem) => {
      const summary = byId.get(subsystem.id);
      const tally: Partial<Record<CommitStatusKind, number>> = {};
      for (const [kind, value] of Object.entries(summary?.tally ?? {})) {
        tally[kind as CommitStatusKind] = value ?? 0;
      }
      return {
        subsystem,
        indexed: summary?.indexed ?? 0,
        total: summary?.total ?? 0,
        tally,
      };
    });
  }, [subsystems, app.targetBranch, app.dataVersion]);

  const runs = useAsyncData(() => api.listWorkflowRuns(null, null, 6), [app.dataVersion]);
  const history = useAsyncData(
    () => api.listIntegrationHistory(app.targetBranch, null, 6),
    [app.targetBranch, app.dataVersion],
  );

  const totals = useMemo(() => {
    const aggregate: Partial<Record<CommitStatusKind, number>> = {};
    for (const entry of counts.data ?? []) {
      for (const [kind, value] of Object.entries(entry.tally)) {
        const key = kind as CommitStatusKind;
        aggregate[key] = (aggregate[key] ?? 0) + (value ?? 0);
      }
    }
    return aggregate;
  }, [counts.data]);

  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[50px] flex-wrap items-center gap-3 border-b border-line px-5 py-2">
        <h1 className="text-[14px] font-semibold text-fg">Dashboard</h1>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <Select
            aria-label="Main-system target branch for dashboard counts"
            className="w-[190px]"
            value={app.targetBranch}
            onChange={(event) => app.setTargetBranch(event.target.value)}
            options={branches.map((branch) => ({
              value: branch.name,
              label: `target: ${branch.name}`,
            }))}
          />
          <Button variant="secondary" icon={RefreshCw} onClick={app.bumpDataVersion}>
            Refresh
          </Button>
          <Button
            variant="primary"
            icon={GitPullRequestArrow}
            onClick={() => app.navigate('promote-commits')}
          >
            Promote Commits
          </Button>
        </div>
      </header>

      <div className="flex h-[29px] shrink-0 items-center gap-2 border-b border-line px-5">
        <GitBranch className="h-3.5 w-3.5 shrink-0 text-accent" aria-hidden="true" />
        <span className="mono truncate text-muted">target: {app.targetBranch}</span>
        <span className="muted truncate">branch-scoped counts · indexed commits only</span>
      </div>

      <div className="scroll-area min-h-0 flex-1 space-y-4 p-5">
        <div className="grid gap-4 xl:grid-cols-3">
          <Panel title="Status overview" className="min-w-0" bodyClassName="overflow-x-auto">
            {counts.loading && (counts.data?.length ?? 0) === 0 ? (
              <LoadingState label="Counting statuses…" />
            ) : counts.error ? (
              <ErrorState error={counts.error} onRetry={counts.reload} compact />
            ) : (
              <table className="w-full border-separate border-spacing-0 text-xs">
                <thead>
                  <tr className="table-head">
                    <th scope="col" className="py-2 pl-4 text-left">
                      Status
                    </th>
                    <th scope="col" className="py-2 pr-4 text-right">
                      Commits
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {COUNTED.map((entry) => (
                    <tr key={entry.key} className="row-hover border-b border-line">
                      <td className="h-[29px] py-0 pl-4">
                        <StatusBadge kind={entry.key} label={entry.label} />
                      </td>
                      <td className="mono py-0 pr-4 text-right text-fg tabular-nums">
                        {counts.loading ? '—' : (totals[entry.key] ?? 0)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Panel>

          <Panel
            className="min-w-0 xl:col-span-2"
            title="Registered subsystems"
            actions={
              <Button
                size="sm"
                variant="secondary"
                icon={Plus}
                onClick={() => app.navigate('subsystems')}
              >
                Register
              </Button>
            }
            bodyClassName="scroll-area max-h-[340px] overflow-x-auto"
          >
            {counts.loading ? (
              <LoadingState label="Counting statuses…" />
            ) : counts.error ? (
              <ErrorState error={counts.error} onRetry={counts.reload} compact />
            ) : (
              <table className="w-full border-separate border-spacing-0 text-xs">
                <thead>
                  <tr className="table-head">
                    <th scope="col" className="py-2 pl-4 text-left">
                      Subsystem
                    </th>
                    <th scope="col" className="py-2 pr-3 text-left">
                      Source branch
                    </th>
                    <th scope="col" className="py-2 pr-3 text-right">
                      Indexed
                    </th>
                    <th scope="col" className="py-2 pr-3 text-right">
                      Integrated
                    </th>
                    <th scope="col" className="py-2 pr-3 text-right">
                      PR open
                    </th>
                    <th scope="col" className="py-2 pr-4 text-right">
                      Eligible
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {(counts.data ?? []).map((entry) => (
                    <tr key={entry.subsystem.id} className="row-hover border-b border-line">
                      <td className="h-[29px] py-0 pl-4">
                        <button
                          type="button"
                          onClick={() => {
                            app.selectSubsystem(entry.subsystem.id);
                            app.navigate('subsystems');
                          }}
                          className="block min-w-0 text-left text-fg hover:underline"
                        >
                          <span className="block truncate">{entry.subsystem.name}</span>
                          <span className="mono block truncate text-disabled">
                            {entry.subsystem.repository}
                          </span>
                        </button>
                      </td>
                      <td className="mono truncate py-0 pr-3 text-muted">
                        {entry.subsystem.sourceBranch}
                      </td>
                      <td className="mono py-0 pr-3 text-right text-fg tabular-nums">
                        {entry.indexed}
                      </td>
                      <td className="mono py-0 pr-3 text-right text-fg tabular-nums">
                        {entry.tally.integrated ?? 0}
                      </td>
                      <td className="mono py-0 pr-3 text-right text-fg tabular-nums">
                        {(entry.tally.pr_open ?? 0) + (entry.tally.in_review ?? 0)}
                      </td>
                      <td className="mono py-0 pr-4 text-right text-fg tabular-nums">
                        {entry.tally.eligible ?? 0}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Panel>
        </div>

        <div className="grid gap-4 xl:grid-cols-2">
          <Panel
            className="min-w-0"
            title="Recent promotion runs"
            subtitle="Runs started by this tool"
            actions={
              <Button size="sm" variant="secondary" onClick={() => app.navigate('workflow-runs')}>
                All runs
              </Button>
            }
          >
            {runs.loading ? (
              <LoadingState label="Loading runs…" />
            ) : (runs.data?.runs.length ?? 0) === 0 ? (
              <EmptyState
                icon={Workflow}
                title="No promotion runs yet"
                message="Dispatch a promotion to create the first run."
              />
            ) : (
              <ul className="divide-y divide-line">
                {(runs.data?.runs ?? []).map((run) => (
                  <li
                    key={run.id}
                    className="flex h-[50px] flex-wrap items-center gap-2 overflow-hidden px-4"
                  >
                    <StatusBadge
                      kind={
                        run.status === 'in_progress'
                          ? 'promotion_running'
                          : run.status === 'queued'
                            ? 'promotion_queued'
                            : run.conclusion === 'success'
                              ? 'pr_open'
                              : run.conclusion === 'failure'
                                ? 'failed'
                                : 'unknown'
                      }
                      label={`#${run.runNumber} ${run.status === 'completed' ? (run.conclusion ?? 'done') : run.status}`}
                    />
                    <span className="min-w-0 flex-1 truncate text-xs text-fg">
                      {run.displayTitle}
                    </span>
                    <span className="meta">{run.subsystemId}</span>
                    <span className="mono truncate text-disabled">{run.targetBranch}</span>
                    <span className="truncate text-[11px] text-muted">
                      {formatDateTime(run.updatedAt)}
                    </span>
                    <button
                      type="button"
                      onClick={() => app.openExternal(run.htmlUrl)}
                      className="rounded-md p-1 text-muted hover:bg-hover hover:text-fg"
                      aria-label={`Open run ${run.runNumber} on GitHub`}
                      title="Open the run on GitHub"
                    >
                      <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel
            className="min-w-0"
            title="Latest target activity"
            subtitle={`Integration events reachable from ${app.targetBranch}`}
            actions={
              <Button
                size="sm"
                variant="secondary"
                onClick={() => app.navigate('integration-history')}
              >
                History
              </Button>
            }
            bodyClassName="scroll-area max-h-[340px]"
          >
            {history.loading ? (
              <LoadingState label="Loading history…" />
            ) : (history.data?.entries.length ?? 0) === 0 ? (
              <EmptyState
                icon={History}
                title="No integration events"
                message="No promotion has been recorded for this target branch yet."
              />
            ) : (
              <ul className="divide-y divide-line">
                {(history.data?.entries ?? []).map((entry) => (
                  <li key={entry.id} className="space-y-1 px-4 py-2.5">
                    <div className="flex items-center gap-2">
                      <StatusBadge
                        kind={HISTORY_LABEL[entry.kind].kind}
                        label={HISTORY_LABEL[entry.kind].label}
                      />
                      <span className="meta">{entry.subsystemId}</span>
                      {entry.inheritedFromBranch ? (
                        <span className="meta text-status-unknown-ink">
                          inherited from {entry.inheritedFromBranch}
                        </span>
                      ) : null}
                    </div>
                    <p className="truncate text-[11px] text-muted">{entry.detail}</p>
                    <p className="mono truncate text-disabled">
                      {entry.sourceShas.map((sha) => sha.slice(0, 7)).join(', ')} ·{' '}
                      {formatDateTime(entry.occurredAt)} · {entry.actor}
                    </p>
                  </li>
                ))}
              </ul>
            )}
          </Panel>
        </div>
      </div>
    </div>
  );
}

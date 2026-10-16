import { useMemo } from 'react';
import {
  ArrowRight,
  Boxes,
  ExternalLink,
  GitBranch,
  GitPullRequestArrow,
  History,
  Plus,
  RefreshCw,
  Settings as SettingsIcon,
  Workflow,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { CommitStatusKind, IntegrationHistoryKind } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { StatusBadge } from '../components/StatusBadge';
import { cx } from '../components/ui/cx';
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
    const perSubsystem = await Promise.all(
      subsystems.map(async (subsystem) => {
        const page = await api.getSubsystemCommits({
          subsystemId: subsystem.id,
          targetBranch: app.targetBranch,
          page: 1,
          pageSize: 200,
          search: '',
          statusFilter: null,
          sort: 'newest',
        });
        const statuses = await api.getIntegrationStatuses(
          app.targetBranch,
          subsystem.id,
          page.commits.map((commit) => commit.sha),
        );
        const tally: Partial<Record<CommitStatusKind, number>> = {};
        for (const status of statuses) {
          tally[status.status.kind] = (tally[status.status.kind] ?? 0) + 1;
        }
        return { subsystem, indexed: page.commits.length, total: page.total, tally };
      }),
    );
    return perSubsystem;
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
    <div className="scroll-area h-full min-h-0 space-y-4 p-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-white">Dashboard</h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Integration state across registered subsystems for target branch{' '}
            <span className="mono text-slate-300">{app.targetBranch}</span>
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
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

      <section
        aria-label="Counts by status"
        className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5"
      >
        {COUNTED.filter((entry) => entry.group !== 'attention').map((entry) => (
          <div key={entry.key} className="rounded-xl border border-ink-700/70 bg-ink-870 p-3">
            <StatusBadge kind={entry.key} size="md" />
            <p className="mt-2 text-2xl font-semibold text-white">
              {counts.loading ? '—' : (totals[entry.key] ?? 0)}
            </p>
            <p className="text-2xs text-slate-500">indexed commits on {app.targetBranch}</p>
          </div>
        ))}
      </section>

      <section aria-label="Needs attention" className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {COUNTED.filter((entry) => entry.group === 'attention').map((entry) => (
          <div key={entry.key} className="rounded-xl border border-ink-700/70 bg-ink-870 p-3">
            <StatusBadge kind={entry.key} />
            <p className="mt-2 text-xl font-semibold text-white">
              {counts.loading ? '—' : (totals[entry.key] ?? 0)}
            </p>
          </div>
        ))}
      </section>

      <div className="grid gap-4 xl:grid-cols-3">
        <Panel
          className="xl:col-span-2"
          title="Registered subsystems"
          subtitle={`Status counts for ${app.targetBranch} · indexed commits only`}
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
          bodyClassName="scroll-area max-h-[340px]"
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
                  <tr key={entry.subsystem.id} className="row-hover border-b border-ink-800/70">
                    <td className="py-2 pl-4">
                      <button
                        type="button"
                        onClick={() => {
                          app.selectSubsystem(entry.subsystem.id);
                          app.navigate('subsystems');
                        }}
                        className="text-left text-slate-100 hover:text-white hover:underline"
                      >
                        {entry.subsystem.name}
                        <span className="mono block text-2xs text-slate-500">
                          {entry.subsystem.repository}
                        </span>
                      </button>
                    </td>
                    <td className="mono py-2 pr-3 text-slate-400">
                      {entry.subsystem.sourceBranch}
                    </td>
                    <td className="py-2 pr-3 text-right text-slate-300">{entry.indexed}</td>
                    <td className="py-2 pr-3 text-right">
                      <span
                        className={cx(
                          (entry.tally.integrated ?? 0) > 0 ? 'text-emerald-300' : 'text-slate-500',
                        )}
                      >
                        {entry.tally.integrated ?? 0}
                      </span>
                    </td>
                    <td className="py-2 pr-3 text-right">
                      <span
                        className={cx(
                          (entry.tally.pr_open ?? 0) > 0 ? 'text-violet-300' : 'text-slate-500',
                        )}
                      >
                        {(entry.tally.pr_open ?? 0) + (entry.tally.in_review ?? 0)}
                      </span>
                    </td>
                    <td className="py-2 pr-4 text-right text-slate-300">
                      {entry.tally.eligible ?? 0}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Panel>

        <Panel
          title="Shortcuts"
          subtitle="Every claim in this tool is backed by Git history or GitHub state"
        >
          <div className="space-y-2 p-3">
            <Button
              variant="secondary"
              className="w-full justify-start"
              icon={Boxes}
              onClick={() => app.navigate('subsystems')}
            >
              Subsystems and commit workspace
              <ArrowRight className="ml-auto h-4 w-4 opacity-70" aria-hidden="true" />
            </Button>
            <Button
              variant="secondary"
              className="w-full justify-start"
              icon={GitBranch}
              onClick={() => app.navigate('commit-browser')}
            >
              Browse commits and select SHAs
              <ArrowRight className="ml-auto h-4 w-4 opacity-70" aria-hidden="true" />
            </Button>
            <Button
              variant="secondary"
              className="w-full justify-start"
              icon={Workflow}
              onClick={() => app.navigate('workflow-runs')}
            >
              Workflow runs and logs
              <ArrowRight className="ml-auto h-4 w-4 opacity-70" aria-hidden="true" />
            </Button>
            <Button
              variant="secondary"
              className="w-full justify-start"
              icon={History}
              onClick={() => app.navigate('integration-history')}
            >
              Integration history per branch
              <ArrowRight className="ml-auto h-4 w-4 opacity-70" aria-hidden="true" />
            </Button>
            <Button
              variant="secondary"
              className="w-full justify-start"
              icon={SettingsIcon}
              onClick={() => app.navigate('settings')}
            >
              Settings and diagnostics
              <ArrowRight className="ml-auto h-4 w-4 opacity-70" aria-hidden="true" />
            </Button>
          </div>
        </Panel>

        <Panel
          className="xl:col-span-2"
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
            <ul className="divide-y divide-ink-800/70">
              {(runs.data?.runs ?? []).map((run) => (
                <li key={run.id} className="flex flex-wrap items-center gap-2 px-4 py-2.5">
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
                  <span className="min-w-0 flex-1 truncate text-xs text-slate-200">
                    {run.displayTitle}
                  </span>
                  <span className="chip bg-ink-800 text-slate-300">{run.subsystemId}</span>
                  <span className="mono text-2xs text-slate-500">{run.targetBranch}</span>
                  <span className="text-2xs text-slate-500">{formatDateTime(run.updatedAt)}</span>
                  <button
                    type="button"
                    onClick={() => app.openExternal(run.htmlUrl)}
                    className="rounded-sm p-1 text-slate-400 hover:bg-ink-700 hover:text-white"
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
            <ul className="divide-y divide-ink-800/70">
              {(history.data?.entries ?? []).map((entry) => (
                <li key={entry.id} className="space-y-1 px-4 py-2.5">
                  <div className="flex items-center gap-2">
                    <StatusBadge
                      kind={HISTORY_LABEL[entry.kind].kind}
                      label={HISTORY_LABEL[entry.kind].label}
                    />
                    <span className="chip bg-ink-800 text-slate-300">{entry.subsystemId}</span>
                    {entry.inheritedFromBranch ? (
                      <span className="chip bg-amber-950/60 text-amber-200">
                        inherited from {entry.inheritedFromBranch}
                      </span>
                    ) : null}
                  </div>
                  <p className="text-2xs leading-relaxed text-slate-400">{entry.detail}</p>
                  <p className="mono text-2xs text-slate-500">
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
  );
}

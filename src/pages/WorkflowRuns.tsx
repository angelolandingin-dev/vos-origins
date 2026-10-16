import { useState } from 'react';
import {
  CircleCheck,
  ExternalLink,
  LoaderCircle,
  RefreshCw,
  ScrollText,
  TriangleAlert,
  Workflow,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { WorkflowRun } from '../api/types';
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
  KeyValue,
  LoadingState,
  Panel,
  Select,
  Spinner,
} from '../components/ui/primitives';

/** Workflow Runs (spec §3.3): runs started by this tool, with steps, on-demand logs and links. */
function runBadge(run: WorkflowRun) {
  if (run.status === 'in_progress') return <StatusBadge kind="promotion_running" size="md" />;
  if (run.status === 'queued' || run.status === 'requested' || run.status === 'waiting')
    return <StatusBadge kind="promotion_queued" size="md" />;
  if (run.conclusion === 'failure' || run.conclusion === 'timed_out')
    return <StatusBadge kind="failed" size="md" />;
  if (run.conclusion === 'success')
    return (
      <span
        className="chip bg-ink-800 px-2.5 py-1 text-xs text-emerald-300 ring-1 ring-inset ring-emerald-800/70"
        title="A successful workflow run is not an integration. Integration is only claimed from a provenance-bearing merge commit."
      >
        <CircleCheck className="h-3.5 w-3.5" aria-hidden="true" />
        Workflow succeeded
      </span>
    );
  return <StatusBadge kind="unknown" label={run.conclusion ?? run.status} size="md" />;
}

const LEVEL_STYLE: Record<string, string> = {
  error: 'text-rose-300',
  warn: 'text-amber-300',
  info: 'text-slate-300',
  debug: 'text-slate-500',
};

export function WorkflowRuns() {
  const app = useApp();
  const [subsystemFilter, setSubsystemFilter] = useState<string>('all');
  const [targetFilter, setTargetFilter] = useState<string>('all');
  const [selectedRunId, setSelectedRunId] = useState<number | null>(null);

  const runs = useAsyncData(
    () => api.listWorkflowRuns(subsystemFilter === 'all' ? null : subsystemFilter, null, 50),
    [subsystemFilter, app.dataVersion],
  );

  const filtered = (runs.data?.runs ?? []).filter(
    (run) => targetFilter === 'all' || run.targetBranch === targetFilter,
  );

  // Derive the visible selection during render: keep the user's choice while it
  // is still in the filtered list, otherwise fall back to the first run.
  const selectedRun = filtered.find((run) => run.id === selectedRunId) ?? filtered[0] ?? null;
  const activeRunId = selectedRun?.id ?? null;

  const details = useAsyncData(
    () => (activeRunId ? api.getWorkflowRunDetails(activeRunId) : Promise.resolve(null)),
    [activeRunId, app.dataVersion],
  );

  const branches = (app.targetBranches.data?.branches ?? []).map((branch) => branch.name);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pb-3 pt-4">
        <div>
          <h1 className="flex items-center gap-2 text-xl font-semibold tracking-tight text-white">
            <Workflow className="h-5 w-5 text-brand-400" aria-hidden="true" />
            Workflow Runs
          </h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Promotion workflow runs started by this tool
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Select
            aria-label="Filter runs by subsystem"
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
          <Select
            aria-label="Filter runs by target branch"
            className="w-[190px]"
            value={targetFilter}
            onChange={(event) => setTargetFilter(event.target.value)}
            options={[
              { value: 'all', label: 'All target branches' },
              ...branches.map((branch) => ({ value: branch, label: branch })),
            ]}
          />
          <Button
            variant="secondary"
            icon={RefreshCw}
            onClick={() => runs.reload()}
            title="Refetch workflow runs"
          >
            Refresh
          </Button>
        </div>
      </header>

      <div className="grid min-h-0 flex-1 gap-3 px-3 pb-3 xl:grid-cols-[400px_minmax(0,1fr)]">
        <Panel
          className="min-h-0"
          title="Runs"
          subtitle={`${filtered.length} of ${runs.data?.total ?? 0}`}
          bodyClassName="scroll-area"
        >
          {runs.loading && filtered.length === 0 ? (
            <LoadingState label="Loading runs…" />
          ) : runs.error ? (
            <ErrorState error={runs.error} onRetry={runs.reload} compact />
          ) : filtered.length === 0 ? (
            <EmptyState
              icon={Workflow}
              title="No runs match these filters"
              message="Runs appear here after a promotion is dispatched. Adjust the filters or dispatch a promotion."
            />
          ) : (
            <ul className="divide-y divide-ink-800/70">
              {filtered.map((run) => {
                const active = run.id === activeRunId;
                return (
                  <li key={run.id}>
                    <button
                      type="button"
                      onClick={() => setSelectedRunId(run.id)}
                      aria-current={active ? 'true' : undefined}
                      className={cx(
                        'w-full px-4 py-3 text-left transition-colors',
                        active ? 'bg-ink-800/80' : 'hover:bg-ink-800/50',
                      )}
                    >
                      <div className="flex items-center gap-2">
                        {runBadge(run)}
                        <span className="mono ml-auto text-2xs text-slate-500">
                          #{run.runNumber}
                        </span>
                      </div>
                      <p className="mt-1.5 truncate text-xs font-medium text-slate-100">
                        {run.displayTitle}
                      </p>
                      <p className="mono mt-0.5 truncate text-2xs text-slate-500">
                        {run.subsystemId} → {run.targetBranch} · {run.mode}
                      </p>
                      <p className="mt-0.5 text-2xs text-slate-500">
                        {run.actor} · {formatDateTime(run.updatedAt)}
                      </p>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </Panel>

        <Panel
          className="min-h-0"
          title="Run detail"
          subtitle={details.data ? `${details.data.run.displayTitle}` : 'Select a run'}
          actions={
            details.data ? (
              <IconButton
                icon={ExternalLink}
                label="Open run on GitHub"
                onClick={() => app.openExternal(details.data?.run.htmlUrl ?? '')}
              />
            ) : null
          }
          bodyClassName="scroll-area"
        >
          {!activeRunId ? (
            <EmptyState
              icon={ScrollText}
              title="No run selected"
              message="Pick a run on the left to inspect its steps and logs."
            />
          ) : details.loading && !details.data ? (
            <LoadingState label="Loading run detail…" />
          ) : details.error ? (
            <ErrorState error={details.error} onRetry={details.reload} />
          ) : details.data ? (
            <div className="space-y-4 p-4">
              <div className="flex flex-wrap items-center gap-2">
                {runBadge(details.data.run)}
                <span className="chip bg-ink-800 text-slate-300">
                  target {details.data.run.targetBranch}
                </span>
                <span className="chip bg-ink-800 text-slate-300">mode {details.data.run.mode}</span>
                <span className="chip bg-ink-800 text-slate-300">{details.data.run.event}</span>
              </div>

              <dl className="rounded-lg border border-ink-700/60 bg-ink-850/50 px-3 py-1">
                <KeyValue label="Run id">
                  <span className="mono">{details.data.run.id}</span>
                </KeyValue>
                <KeyValue label="Workflow definition ref">
                  <span className="mono">{details.data.run.headBranch}</span>
                </KeyValue>
                <KeyValue label="PR base (target branch)">
                  <span className="mono text-brand-200">{details.data.run.targetBranch}</span>
                </KeyValue>
                <KeyValue label="Requested by">{details.data.run.actor}</KeyValue>
                <KeyValue label="Created">{formatDateTime(details.data.run.createdAt)}</KeyValue>
                <KeyValue label="Source SHAs">
                  <span className="mono">
                    {details.data.run.sourceShas.map((sha) => sha.slice(0, 7)).join(', ')}
                  </span>
                </KeyValue>
              </dl>

              {details.data.run.status !== 'completed' ? (
                <InlineAlert tone="info" title="Status will not jump to Integrated">
                  While the run is active the SHA shows{' '}
                  <span className="font-semibold">Promotion queued</span> or{' '}
                  <span className="font-semibold">Promotion running</span>. Integration is only
                  recorded from reachable Git history after the PR is merged.
                </InlineAlert>
              ) : null}

              <section className="space-y-2">
                <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-400">
                  Jobs and steps
                </h3>
                {details.data.jobs.length === 0 ? (
                  <p className="text-2xs text-slate-500">
                    No job detail is available for this run.
                  </p>
                ) : (
                  details.data.jobs.map((job) => (
                    <div
                      key={job.id}
                      className="rounded-lg border border-ink-700/60 bg-ink-850/50 p-3"
                    >
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-xs font-semibold text-slate-100">{job.name}</span>
                        {job.status === 'in_progress' ? (
                          <span className="flex items-center gap-1.5 text-2xs text-orange-300">
                            <Spinner className="h-3.5 w-3.5" /> running
                          </span>
                        ) : (
                          <span className="text-2xs text-slate-400">
                            {job.conclusion ?? job.status}
                          </span>
                        )}
                        <span className="mono ml-auto text-2xs text-slate-500">
                          {job.startedAt ? formatDateTime(job.startedAt) : ''}
                        </span>
                      </div>
                      <ol className="mt-2 space-y-1">
                        {job.steps.map((step) => (
                          <li key={step.number} className="flex items-center gap-2 text-2xs">
                            <span className="mono w-5 shrink-0 text-right text-slate-500">
                              {step.number}
                            </span>
                            {step.status === 'completed' && step.conclusion === 'success' ? (
                              <CircleCheck
                                className="h-3.5 w-3.5 shrink-0 text-emerald-400"
                                aria-hidden="true"
                              />
                            ) : step.status === 'completed' && step.conclusion !== 'success' ? (
                              <TriangleAlert
                                className="h-3.5 w-3.5 shrink-0 text-rose-400"
                                aria-hidden="true"
                              />
                            ) : (
                              <LoaderCircle
                                className="h-3.5 w-3.5 shrink-0 animate-spin text-brand-400"
                                aria-hidden="true"
                              />
                            )}
                            <span className="min-w-0 flex-1 truncate text-slate-300">
                              {step.name}
                            </span>
                            <span className="shrink-0 text-slate-500">
                              {step.conclusion ?? step.status}
                            </span>
                          </li>
                        ))}
                      </ol>
                    </div>
                  ))
                )}
              </section>

              <section className="space-y-2">
                <div className="flex items-center gap-2">
                  <ScrollText className="h-4 w-4 text-slate-400" aria-hidden="true" />
                  <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-400">
                    Logs
                  </h3>
                  {details.data.logsTruncated ? (
                    <span className="chip bg-amber-950/60 text-amber-200">bounded</span>
                  ) : null}
                </div>
                {details.data.logs.length === 0 ? (
                  <p className="text-2xs text-slate-500">
                    Logs are fetched on demand.{' '}
                    {details.data.logsUrl ? 'Open the run on GitHub for the full log.' : ''}
                  </p>
                ) : (
                  <pre className="mono max-h-[320px] overflow-auto rounded-lg border border-ink-700/60 bg-ink-925 p-3 text-2xs leading-relaxed">
                    {details.data.logs.map((line) => (
                      <div
                        key={`${line.ts}-${line.text}`}
                        className={cx('whitespace-pre-wrap', LEVEL_STYLE[line.level])}
                      >
                        {formatDateTime(line.ts)} [{line.level}] {line.text}
                      </div>
                    ))}
                  </pre>
                )}
                {details.data.artifactNote ? (
                  <p className="text-2xs text-slate-500">{details.data.artifactNote}</p>
                ) : null}
              </section>
            </div>
          ) : null}
        </Panel>
      </div>
    </div>
  );
}

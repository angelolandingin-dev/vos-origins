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
        className="chip text-status-applied-ink"
        title="A successful run is not an integration — only a merged provenance commit counts."
      >
        <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-status-applied" />
        Workflow succeeded
      </span>
    );
  return <StatusBadge kind="unknown" label={run.conclusion ?? run.status} size="md" />;
}

const LEVEL_STYLE: Record<string, string> = {
  error: 'text-status-blocked-ink',
  warn: 'text-status-unknown-ink',
  info: 'text-fg',
  debug: 'text-disabled',
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
      <header className="flex min-h-[50px] flex-wrap items-center gap-3 border-b border-line px-5 py-2">
        <h1 className="text-[14px] font-semibold text-fg">Workflow Runs</h1>
        <div className="ml-auto flex flex-wrap items-center gap-2">
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

      <div className="grid min-h-0 flex-1 gap-3 p-3 xl:grid-cols-[400px_minmax(0,1fr)]">
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
              message="Dispatch a promotion to create the first run."
            />
          ) : (
            <ul className="divide-y divide-line">
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
                        active ? 'bg-selection' : 'hover:bg-hover',
                      )}
                    >
                      <div className="flex items-center gap-2">
                        {runBadge(run)}
                        <span className="mono ml-auto text-disabled">#{run.runNumber}</span>
                      </div>
                      <p className="mt-1.5 truncate text-xs font-medium text-fg">
                        {run.displayTitle}
                      </p>
                      <p className="mono mt-0.5 truncate text-muted">
                        {run.subsystemId} → {run.targetBranch} · {run.mode}
                      </p>
                      <p className="mt-0.5 truncate text-[11px] text-muted">
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
              message="Pick a run to inspect its steps and logs."
            />
          ) : details.loading && !details.data ? (
            <LoadingState label="Loading run detail…" />
          ) : details.error ? (
            <ErrorState error={details.error} onRetry={details.reload} />
          ) : details.data ? (
            <div className="space-y-4 p-4">
              <div className="flex flex-wrap items-center gap-2">
                {runBadge(details.data.run)}
                <span className="chip">target {details.data.run.targetBranch}</span>
                <span className="chip">mode {details.data.run.mode}</span>
                <span className="chip">{details.data.run.event}</span>
              </div>

              <dl className="rounded-md border border-line bg-raised px-3 py-1">
                <KeyValue label="Run id">
                  <span className="mono">{details.data.run.id}</span>
                </KeyValue>
                <KeyValue label="Workflow definition ref">
                  <span className="mono">{details.data.run.headBranch}</span>
                </KeyValue>
                <KeyValue label="PR base (target branch)">
                  <span className="mono text-accent-ink">{details.data.run.targetBranch}</span>
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
                <InlineAlert
                  tone="info"
                  title="Integration is recorded only from reachable Git history after the merge"
                />
              ) : null}

              <section className="space-y-2">
                <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Jobs and steps
                </h3>
                {details.data.jobs.length === 0 ? (
                  <p className="text-[11px] text-muted">No job detail is available for this run.</p>
                ) : (
                  details.data.jobs.map((job) => (
                    <div key={job.id} className="rounded-md border border-line bg-raised p-3">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="truncate text-xs font-semibold text-fg">{job.name}</span>
                        {job.status === 'in_progress' ? (
                          <span className="flex items-center gap-1.5 text-[11px] text-status-divergent-ink">
                            <Spinner className="h-3.5 w-3.5" /> running
                          </span>
                        ) : (
                          <span className="text-[11px] text-muted">
                            {job.conclusion ?? job.status}
                          </span>
                        )}
                        <span className="mono ml-auto text-disabled">
                          {job.startedAt ? formatDateTime(job.startedAt) : ''}
                        </span>
                      </div>
                      <ol className="mt-2 space-y-1">
                        {job.steps.map((step) => (
                          <li
                            key={step.number}
                            className="flex h-[29px] items-center gap-2 text-[11px]"
                          >
                            <span className="mono w-5 shrink-0 text-right text-disabled">
                              {step.number}
                            </span>
                            {step.status === 'completed' && step.conclusion === 'success' ? (
                              <CircleCheck
                                className="h-3.5 w-3.5 shrink-0 text-status-applied"
                                aria-hidden="true"
                              />
                            ) : step.status === 'completed' && step.conclusion !== 'success' ? (
                              <TriangleAlert
                                className="h-3.5 w-3.5 shrink-0 text-status-blocked"
                                aria-hidden="true"
                              />
                            ) : (
                              <LoaderCircle
                                className="h-3.5 w-3.5 shrink-0 animate-spin text-accent"
                                aria-hidden="true"
                              />
                            )}
                            <span className="min-w-0 flex-1 truncate text-fg">{step.name}</span>
                            <span className="mono shrink-0 text-right text-disabled tabular-nums">
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
                  <ScrollText className="h-4 w-4 text-muted" aria-hidden="true" />
                  <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                    Logs
                  </h3>
                  {details.data.logsTruncated ? (
                    <span className="chip text-status-unknown-ink">bounded</span>
                  ) : null}
                </div>
                {details.data.logs.length === 0 ? (
                  <p className="text-[11px] text-muted">Logs are fetched on demand.</p>
                ) : (
                  <pre className="mono max-h-[320px] overflow-auto rounded-md border border-line bg-ground p-3 leading-relaxed">
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
                  <p className="truncate text-[11px] text-muted">{details.data.artifactNote}</p>
                ) : null}
              </section>
            </div>
          ) : null}
        </Panel>
      </div>
    </div>
  );
}

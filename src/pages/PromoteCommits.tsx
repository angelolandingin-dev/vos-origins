import { useState } from 'react';
import { GitPullRequestArrow, Package, ShieldAlert, TriangleAlert } from 'lucide-react';

import { api } from '../api/invoke';
import type {
  CommitStatusKind,
  DependencyChange,
  PromotionMode,
  SubsystemSummary,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { CommitTable } from '../components/CommitTable';
import { CommitStatusBadge } from '../components/StatusBadge';
import { cx } from '../components/ui/cx';
import {
  Button,
  EmptyState,
  InlineAlert,
  KeyValue,
  LoadingState,
  Panel,
  Select,
} from '../components/ui/primitives';

/** Promote Commits (spec §§3.3/5.1). Hands off to the shared dialog so the integration-PR acknowledgement lives in one place. */
export function PromoteCommits() {
  const app = useApp();
  const subsystem = app.selectedSubsystem;

  if (!subsystem) {
    return (
      <div className="p-5">
        <EmptyState
          icon={GitPullRequestArrow}
          title="Select a subsystem first"
          message="Open Subsystems or the Commit Browser and pick the subsystem whose commits you want to promote."
          action={
            <Button variant="primary" onClick={() => app.navigate('subsystems')}>
              Open Subsystems
            </Button>
          }
        />
      </div>
    );
  }

  return <PromoteCommitsView subsystem={subsystem} />;
}

/** Filter state that survives a subsystem / target-branch switch. */
function PromoteCommitsView({ subsystem }: { subsystem: SubsystemSummary }) {
  const app = useApp();
  const [search, setSearch] = useState('');
  const [statusFilter, setStatusFilter] = useState<CommitStatusKind | 'all'>('eligible');
  const [sort, setSort] = useState<'newest' | 'oldest'>('newest');
  const [pageSize, setPageSize] = useState(10);
  const [mode, setMode] = useState<PromotionMode>('individual');

  return (
    <PromoteCommitsPanel
      key={JSON.stringify([subsystem.id, app.targetBranch])}
      subsystem={subsystem}
      search={search}
      onSearchChange={setSearch}
      statusFilter={statusFilter}
      onStatusFilterChange={setStatusFilter}
      sort={sort}
      onSortChange={setSort}
      pageSize={pageSize}
      onPageSizeChange={setPageSize}
      mode={mode}
      onModeChange={setMode}
    />
  );
}

interface PromoteCommitsPanelProps {
  subsystem: SubsystemSummary;
  search: string;
  onSearchChange: (value: string) => void;
  statusFilter: CommitStatusKind | 'all';
  onStatusFilterChange: (value: CommitStatusKind | 'all') => void;
  sort: 'newest' | 'oldest';
  onSortChange: (value: 'newest' | 'oldest') => void;
  pageSize: number;
  onPageSizeChange: (size: number) => void;
  mode: PromotionMode;
  onModeChange: (mode: PromotionMode) => void;
}

function PromoteCommitsPanel({
  subsystem,
  search,
  onSearchChange,
  statusFilter,
  onStatusFilterChange,
  sort,
  onSortChange,
  pageSize,
  onPageSizeChange,
  mode,
  onModeChange,
}: PromoteCommitsPanelProps) {
  const app = useApp();
  const [pageNumber, setPageNumber] = useState(1);

  // Changing any filter restarts paging from the first page.
  const handleSearchChange = (value: string) => {
    onSearchChange(value);
    setPageNumber(1);
  };
  const handleStatusFilterChange = (value: CommitStatusKind | 'all') => {
    onStatusFilterChange(value);
    setPageNumber(1);
  };
  const handleSortChange = (value: 'newest' | 'oldest') => {
    onSortChange(value);
    setPageNumber(1);
  };
  const handlePageSizeChange = (size: number) => {
    onPageSizeChange(size);
    setPageNumber(1);
  };

  const commitPage = useAsyncData(
    () =>
      subsystem
        ? api.getSubsystemCommits({
            subsystemId: subsystem.id,
            targetBranch: app.targetBranch,
            page: pageNumber,
            pageSize,
            search,
            statusFilter: statusFilter === 'all' ? null : statusFilter,
            sort,
          })
        : Promise.resolve(null),
    [
      subsystem.id,
      app.targetBranch,
      pageNumber,
      pageSize,
      search,
      statusFilter,
      sort,
      app.dataVersion,
    ],
  );

  const listing = commitPage.data?.commits ?? [];
  const selectedCommits = listing.filter((commit) => app.selectedShas.includes(commit.sha));

  const dependencies = useAsyncData(async () => {
    if (app.selectedShas.length === 0) return [] as DependencyChange[];
    const details = await Promise.all(
      app.selectedShas.map((sha) => api.getCommitDetails(subsystem.id, sha, app.targetBranch)),
    );
    return details.flatMap((detail) => detail.dependencies);
  }, [subsystem.id, app.targetBranch, app.selectedShas.join(','), app.dataVersion]);

  const conflicting = (dependencies.data ?? []).filter((dependency) => dependency.conflict);
  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-start justify-between gap-3 px-5 pb-3 pt-4">
        <div className="min-w-0">
          <h1 className="text-xl font-semibold tracking-tight text-white">Promote Commits</h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Opens an integration PR against a chosen main-system branch — never a direct push, never
            a production deploy
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Select
            aria-label="Source subsystem"
            className="w-[230px]"
            value={subsystem.id}
            onChange={(event) => app.selectSubsystem(event.target.value)}
            options={(app.subsystems.data ?? []).map((entry) => ({
              value: entry.id,
              label: entry.name,
            }))}
          />
          <Select
            aria-label="Authorized target main-system branch"
            className="w-[200px]"
            value={app.targetBranch}
            onChange={(event) => app.setTargetBranch(event.target.value)}
            options={branches.map((branch) => ({
              value: branch.name,
              label: `target: ${branch.name}`,
            }))}
          />
        </div>
      </header>

      <div className="grid min-h-0 flex-1 gap-3 px-3 pb-3 xl:grid-cols-[minmax(0,1fr)_380px]">
        <Panel
          className="min-h-0"
          title="Select commits"
          subtitle={`${subsystem.repository}@${subsystem.sourceBranch} · eligible commits are shown first`}
          bodyClassName="flex min-h-0 flex-1 flex-col"
        >
          <CommitTable
            page={commitPage.data}
            loading={commitPage.loading}
            error={commitPage.error}
            onRetry={commitPage.reload}
            targetBranch={app.targetBranch}
            search={search}
            onSearchChange={handleSearchChange}
            statusFilter={statusFilter}
            onStatusFilterChange={handleStatusFilterChange}
            sort={sort}
            onSortChange={handleSortChange}
            pageNumber={pageNumber}
            onPageChange={setPageNumber}
            pageSize={pageSize}
            onPageSizeChange={handlePageSizeChange}
            selectedShas={app.selectedShas}
            onToggleSha={app.toggleSha}
            onSelectShas={app.setSelectedShas}
            focusedSha={app.focusedSha}
            onFocusSha={app.setFocusedSha}
            onPromoteSelected={() => app.openPromotionDialog(app.selectedShas, selectedCommits)}
            onClearSelection={app.clearSelection}
            detailsVisible={false}
            onToggleDetails={() => undefined}
          />
        </Panel>

        <div className="scroll-area min-h-0 space-y-3">
          <Panel title="Promotion summary" subtitle="Everything the workflow will receive">
            <div className="space-y-3 p-3">
              <dl className="rounded-lg border border-ink-700/60 bg-ink-850/50 px-3 py-1">
                <KeyValue label="Source subsystem">
                  <span className="mono">{subsystem.id}</span>
                </KeyValue>
                <KeyValue label="Source repo">
                  <span className="mono">{subsystem.repository}</span>
                </KeyValue>
                <KeyValue label="Source branch">
                  <span className="mono">{subsystem.sourceBranch}</span>
                </KeyValue>
                <KeyValue label="Target branch">
                  <span className="mono text-brand-200">{app.targetBranch}</span>
                </KeyValue>
                <KeyValue label="Mode">{mode}</KeyValue>
                <KeyValue label="Selected SHAs">{app.selectedShas.length}</KeyValue>
              </dl>

              <label className="block space-y-1.5">
                <span className="text-xs font-semibold uppercase tracking-wide text-slate-400">
                  Mode
                </span>
                <Select
                  aria-label="Promotion mode"
                  value={mode}
                  onChange={(event) => onModeChange(event.target.value as PromotionMode)}
                  options={[
                    { value: 'individual', label: 'Individual (first-parent delta per SHA)' },
                    { value: 'cumulative', label: 'Cumulative (requires an explicit baseline)' },
                  ]}
                />
              </label>

              {app.selectedShas.length === 0 ? (
                <InlineAlert tone="info" title="No commits selected">
                  Tick commits in the table. Only the ticked SHAs are promoted.
                </InlineAlert>
              ) : (
                <ul className="space-y-1.5">
                  {app.selectedShas.map((sha) => {
                    const commit = listing.find((candidate) => candidate.sha === sha);
                    return (
                      <li
                        key={sha}
                        className="flex flex-wrap items-center gap-2 rounded-lg border border-ink-700/60 bg-ink-850/50 px-2.5 py-2"
                      >
                        <span className="mono text-2xs text-brand-300" title={sha}>
                          {sha.slice(0, 7)}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-2xs text-slate-200">
                          {commit?.subject ?? 'Not on this page'}
                        </span>
                        {commit ? <CommitStatusBadge status={commit.status} /> : null}
                      </li>
                    );
                  })}
                </ul>
              )}

              <Button
                variant="primary"
                className="w-full"
                icon={GitPullRequestArrow}
                disabled={app.selectedShas.length === 0}
                onClick={() => app.openPromotionDialog(app.selectedShas, selectedCommits)}
              >
                Continue to confirmation
              </Button>
            </div>
          </Panel>

          <Panel title="Dependency and prerequisite review">
            <div className="space-y-2 p-3">
              {dependencies.loading ? (
                <LoadingState label="Loading dependency deltas…" />
              ) : (dependencies.data ?? []).length === 0 ? (
                <p className="text-2xs text-slate-500">
                  No manifest changes are part of the current selection.
                </p>
              ) : (
                <ul className="space-y-1.5">
                  {(dependencies.data ?? []).map((dependency) => (
                    <li
                      key={`${dependency.section}-${dependency.name}`}
                      className={cx(
                        'rounded-lg border px-2.5 py-2 text-2xs',
                        dependency.conflict
                          ? 'border-rose-900/70 bg-rose-950/30 text-rose-200'
                          : 'border-ink-700/60 bg-ink-850/50 text-slate-300',
                      )}
                    >
                      <span className="mono text-slate-100">{dependency.name}</span>{' '}
                      <span className="text-slate-500">{dependency.section}</span>{' '}
                      <span>
                        {dependency.fromRange ?? '(absent)'} → {dependency.toRange ?? '(removed)'}
                      </span>
                    </li>
                  ))}
                </ul>
              )}

              {conflicting.length > 0 ? (
                <InlineAlert tone="danger" title="Incompatible dependency range">
                  The promotion fails closed. Resolve the shared range in the main-system manifest
                  or drop the conflicting commit.
                </InlineAlert>
              ) : null}

              {selectedCommits.some((commit) => commit.touchesManifest) ? (
                <InlineAlert tone="warning" title="This selection changes package.json">
                  <span className="flex items-center gap-1.5">
                    <Package className="h-3.5 w-3.5" aria-hidden="true" />
                    The manifest delta is merged and the target lockfile is regenerated. The
                    subsystem lockfile is never copied.
                  </span>
                </InlineAlert>
              ) : null}

              <InlineAlert tone="info" title="Prerequisites are not inferred">
                <span className="flex items-center gap-1.5">
                  <TriangleAlert className="h-3.5 w-3.5" aria-hidden="true" />
                  Selecting an implementation commit without its dependency commit does not import
                  the dependency. Add it explicitly or resolve it in the integration PR.
                </span>
              </InlineAlert>
            </div>
          </Panel>

          <Panel title="What happens next">
            <ul className="space-y-2 p-3 text-2xs leading-relaxed text-slate-400">
              <li className="flex gap-2">
                <ShieldAlert
                  className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-300"
                  aria-hidden="true"
                />
                <span className="flex-1">
                  <span className="font-semibold text-slate-200">Confirm &amp; dispatch.</span> A
                  GitHub Actions run creates a temporary integration branch from the current{' '}
                  <span className="mono">{app.targetBranch}</span> tip and applies the selected
                  deltas.
                </span>
              </li>
              <li className="flex gap-2">
                <GitPullRequestArrow
                  className="mt-0.5 h-3.5 w-3.5 shrink-0 text-brand-300"
                  aria-hidden="true"
                />
                <span className="flex-1">
                  <span className="font-semibold text-slate-200">Integration PR.</span> The PR base
                  is exactly <span className="mono">{app.targetBranch}</span>. The run ref is only
                  the workflow definition ref.
                </span>
              </li>
              <li className="flex gap-2">
                <Package
                  className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-400"
                  aria-hidden="true"
                />
                <span className="flex-1">
                  <span className="font-semibold text-slate-200">Status honesty.</span> The SHA
                  stays <span className="font-semibold">Promotion queued</span> →{' '}
                  <span className="font-semibold">PR open</span> and only becomes{' '}
                  <span className="font-semibold">Integrated</span> when a provenance commit is
                  reachable from <span className="mono">{app.targetBranch}</span>.
                </span>
              </li>
            </ul>
          </Panel>
        </div>
      </div>
    </div>
  );
}

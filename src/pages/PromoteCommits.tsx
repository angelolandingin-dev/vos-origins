import { useEffect, useMemo, useState } from 'react';
import { GitPullRequestArrow, Package, TriangleAlert } from 'lucide-react';

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
          message="Open the Commit Browser and pick the subsystem whose commits you want to promote."
          action={
            <Button variant="primary" onClick={() => app.navigate('commit-browser')}>
              Open Commit Browser
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

  const listing = useMemo(() => commitPage.data?.commits ?? [], [commitPage.data]);
  const { rememberSelectedCommits } = app;
  useEffect(() => {
    rememberSelectedCommits(listing);
  }, [listing, rememberSelectedCommits]);
  const selectedCommits = app.selectedCommits;

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
      <header className="flex flex-wrap items-center justify-between gap-3 px-5 pb-3 pt-4">
        <div className="min-w-0">
          <h1 className="text-[14px] font-semibold text-fg">Promote Commits</h1>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Select
            ariaLabel="Source subsystem"
            className="w-[230px]"
            value={subsystem.id}
            onValueChange={(next) => app.selectSubsystem(next)}
            options={(app.subsystems.data ?? []).map((entry) => ({
              value: entry.id,
              label: entry.name,
            }))}
          />
          <Select
            ariaLabel="Authorized target main-system branch"
            className="w-[200px]"
            value={app.targetBranch}
            onValueChange={(next) => app.setTargetBranch(next)}
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
          subtitle={`${subsystem.repository}@${subsystem.sourceBranch} · eligible first · target ${app.targetBranch}`}
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
          />
        </Panel>

        <div className="scroll-area min-h-0 space-y-3">
          <Panel title="Promotion summary" subtitle={`Selection for ${app.targetBranch}`}>
            <div className="space-y-3 p-3">
              <dl className="divide-y divide-line border-y border-line px-1">
                <KeyValue label="Source subsystem">
                  <span className="mono text-right">{subsystem.id}</span>
                </KeyValue>
                <KeyValue label="Source branch">
                  <span className="mono text-right">{subsystem.sourceBranch}</span>
                </KeyValue>
                <KeyValue label="Target branch">
                  <span className="mono text-right text-accent-ink">{app.targetBranch}</span>
                </KeyValue>
                <KeyValue label="Mode">{mode}</KeyValue>
                <KeyValue label="Selected SHAs">
                  <span className="mono text-right">{app.selectedShas.length}</span>
                </KeyValue>
              </dl>

              <label className="block space-y-1.5">
                <span className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Mode
                </span>
                <Select
                  ariaLabel="Promotion mode"
                  value={mode}
                  onValueChange={(next) => onModeChange(next as PromotionMode)}
                  options={[
                    { value: 'individual', label: 'Individual (recommended)' },
                    { value: 'cumulative', label: 'Cumulative (requires an explicit baseline)' },
                  ]}
                />
              </label>

              {app.selectedShas.length === 0 ? (
                <InlineAlert tone="info" title="No commits selected">
                  Tick commits in the table.
                </InlineAlert>
              ) : (
                <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                  {app.selectedShas.map((sha) => {
                    // The selection carries its own summaries, so a selected commit
                    // keeps its subject and status whatever page or filter is shown.
                    const commit =
                      selectedCommits.find((candidate) => candidate.sha === sha) ??
                      app.promotion.commits.find((candidate) => candidate.sha === sha) ??
                      listing.find((candidate) => candidate.sha === sha);
                    return (
                      <li key={sha} className="flex h-[29px] items-center gap-2 px-2.5">
                        <span className="mono shrink-0 text-[11px] text-accent-ink" title={sha}>
                          {sha.slice(0, 7)}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-[11px] text-fg">
                          {commit?.subject ?? ''}
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

          <Panel title="Dependency review">
            <div className="space-y-2 p-3">
              {dependencies.loading ? (
                <LoadingState label="Loading dependency changes…" />
              ) : (dependencies.data ?? []).length === 0 ? (
                <p className="text-[11px] text-muted">No manifest changes in this selection.</p>
              ) : (
                <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                  {(dependencies.data ?? []).map((dependency) => (
                    <li
                      key={`${dependency.section}-${dependency.name}`}
                      className="flex h-[29px] items-center gap-2 px-2.5 text-[11px]"
                    >
                      {dependency.conflict ? (
                        <TriangleAlert
                          className="h-3 w-3 shrink-0 text-status-blocked"
                          aria-hidden="true"
                        />
                      ) : null}
                      <span className="mono truncate text-fg">{dependency.name}</span>
                      <span className="shrink-0 text-muted">{dependency.section}</span>
                      <span className="mono ml-auto shrink-0 text-right text-muted">
                        {dependency.fromRange ?? '(absent)'} → {dependency.toRange ?? '(removed)'}
                      </span>
                    </li>
                  ))}
                </ul>
              )}

              {conflicting.length > 0 ? (
                <InlineAlert tone="danger" title="Incompatible dependency range">
                  The promotion fails closed. Resolve the range or drop the commit.
                </InlineAlert>
              ) : null}

              {selectedCommits.some((commit) => commit.touchesManifest) ? (
                <InlineAlert tone="warning" title="This selection changes package.json">
                  <span className="flex items-center gap-1.5">
                    <Package className="h-3.5 w-3.5" aria-hidden="true" />
                    The target lockfile is regenerated.
                  </span>
                </InlineAlert>
              ) : null}
            </div>
          </Panel>

          <div className="border-t border-line px-1 pt-2">
            <p className="text-[11px] leading-relaxed text-muted">
              Promotion commits locally; pushing is manual.
            </p>
            <p className="text-[11px] leading-relaxed text-muted">Status is per target branch.</p>
          </div>
        </div>
      </div>
    </div>
  );
}

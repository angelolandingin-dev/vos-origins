import { useEffect, useMemo, useState } from 'react';
import {
  Boxes,
  ExternalLink,
  FileText,
  GitBranch,
  GitPullRequestArrow,
  History,
  Package,
  RefreshCw,
  Settings as SettingsIcon,
  Workflow,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { CommitStatusKind, WorkflowRun } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { CommitDetailsPanel } from './CommitDetails';
import { CommitTable } from './CommitTable';
import { DiffViewer } from './DiffViewer';
import { StatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
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
  Tabs,
} from './ui/primitives';

/**
 * Subsystem + commit workspace (spec §3.2). In narrow windows the details panel
 * becomes a slide-over so no required action or message is ever unreachable.
 */

type SourceTab = 'commits' | 'changes' | 'dependencies' | 'runs' | 'settings';
type BottomTab = 'diff' | 'logs' | 'prs';

function runChip(run: WorkflowRun) {
  if (run.status === 'in_progress')
    return <StatusBadge kind="promotion_running" label={`Run #${run.runNumber} running`} />;
  if (run.status === 'queued' || run.status === 'requested' || run.status === 'waiting')
    return <StatusBadge kind="promotion_queued" label={`Run #${run.runNumber} queued`} />;
  if (run.conclusion === 'failure' || run.conclusion === 'timed_out')
    return <StatusBadge kind="failed" label={`Run #${run.runNumber} failed`} />;
  if (run.conclusion === 'success')
    return (
      <span className="meta text-status-applied-ink" title="Workflow succeeded">
        <span aria-hidden="true" className="h-1.5 w-1.5 rounded-full bg-status-applied" />
        Run #{run.runNumber} succeeded
      </span>
    );
  return (
    <span className="meta" title={`Conclusion: ${run.conclusion ?? run.status}`}>
      Run #{run.runNumber} {run.conclusion ?? run.status}
    </span>
  );
}

export function CommitWorkspace() {
  return <CommitWorkspaceView />;
}

/** Filter/panel state keyed by working context, so a subsystem/branch switch resets paging without effect-written state. */
function CommitWorkspaceView() {
  const app = useApp();
  const [search, setSearch] = useState('');
  const [statusFilter, setStatusFilter] = useState<CommitStatusKind | 'all'>('all');
  const [sort, setSort] = useState<'newest' | 'oldest'>('newest');
  const [pageSize, setPageSize] = useState(10);
  // Closed by default below the compact breakpoint, where the panel renders as an
  // overlay that would cover the tab bar and half the workspace on arrival. Uses
  // the same query (and the same guard) as the panel's own `isWide` derivation.
  const [detailsVisible, setDetailsVisible] = useState(() =>
    typeof window === 'undefined' || typeof window.matchMedia !== 'function'
      ? true
      : window.matchMedia('(min-width: 1280px)').matches,
  );
  const [sourceTab, setSourceTab] = useState<SourceTab>('commits');

  return (
    <CommitWorkspacePanel
      key={JSON.stringify([app.selectedSubsystemId, app.targetBranch])}
      search={search}
      onSearchChange={setSearch}
      statusFilter={statusFilter}
      onStatusFilterChange={setStatusFilter}
      sort={sort}
      onSortChange={setSort}
      pageSize={pageSize}
      onPageSizeChange={setPageSize}
      detailsVisible={detailsVisible}
      onDetailsVisibleChange={setDetailsVisible}
      sourceTab={sourceTab}
      onSourceTabChange={setSourceTab}
    />
  );
}

interface CommitWorkspacePanelProps {
  search: string;
  onSearchChange: (value: string) => void;
  statusFilter: CommitStatusKind | 'all';
  onStatusFilterChange: (value: CommitStatusKind | 'all') => void;
  sort: 'newest' | 'oldest';
  onSortChange: (value: 'newest' | 'oldest') => void;
  pageSize: number;
  onPageSizeChange: (size: number) => void;
  detailsVisible: boolean;
  onDetailsVisibleChange: (visible: boolean) => void;
  sourceTab: SourceTab;
  onSourceTabChange: (tab: SourceTab) => void;
}

function CommitWorkspacePanel({
  search,
  onSearchChange,
  statusFilter,
  onStatusFilterChange,
  sort,
  onSortChange,
  pageSize,
  onPageSizeChange,
  detailsVisible,
  onDetailsVisibleChange,
  sourceTab,
  onSourceTabChange,
}: CommitWorkspacePanelProps) {
  const app = useApp();
  const subsystem = app.selectedSubsystem;
  const [isWide, setIsWide] = useState(() =>
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia('(min-width: 1280px)').matches
      : true,
  );
  const [pageNumber, setPageNumber] = useState(1);
  const [bottomTab, setBottomTab] = useState<BottomTab>('diff');
  const [diffPath, setDiffPath] = useState<string | null>(null);

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const list = window.matchMedia('(min-width: 1280px)');
    const update = () => setIsWide(list.matches);
    update();
    list.addEventListener('change', update);
    return () => list.removeEventListener('change', update);
  }, []);

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
      subsystem?.id,
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
  const focusedSha = useMemo(() => {
    if (app.focusedSha && listing.some((commit) => commit.sha === app.focusedSha))
      return app.focusedSha;
    return listing[0]?.sha ?? app.focusedSha;
  }, [app.focusedSha, listing]);

  const details = useAsyncData(
    () =>
      subsystem && focusedSha
        ? api.getCommitDetails(subsystem.id, focusedSha, app.targetBranch)
        : Promise.resolve(null),
    [subsystem?.id, focusedSha, app.targetBranch, app.dataVersion],
  );

  const diff = useAsyncData(
    () =>
      subsystem && focusedSha
        ? api.getCommitDiff(subsystem.id, focusedSha, diffPath, app.targetBranch)
        : Promise.resolve(null),
    [subsystem?.id, focusedSha, diffPath, app.targetBranch, app.dataVersion],
  );

  const runs = useAsyncData(
    () => (subsystem ? api.listWorkflowRuns(subsystem.id, null, 25) : Promise.resolve(null)),
    [subsystem?.id, app.dataVersion],
  );

  const commitRuns = (runs.data?.runs ?? []).filter((run) =>
    focusedSha ? run.sourceShas.includes(focusedSha) : false,
  );
  const activeDiffPath = diffPath ?? diff.data?.filePath ?? null;

  const showDiff = (sha: string, filePath: string) => {
    app.setFocusedSha(sha);
    setDiffPath(filePath || null);
    setBottomTab('diff');
  };

  const promote = (sha: string) => {
    const commit = listing.find((candidate) => candidate.sha === sha);
    app.openPromotionDialog([sha], commit ? [commit] : []);
  };

  const promoteSelection = () => {
    const selected = listing.filter((commit) => app.selectedShas.includes(commit.sha));
    app.openPromotionDialog(app.selectedShas, selected);
  };

  if (!subsystem) {
    return (
      <div className="p-5">
        {app.subsystems.error ? (
          <ErrorState error={app.subsystems.error} onRetry={app.refreshSubsystems} />
        ) : app.subsystems.loading ? (
          <LoadingState label="Loading the subsystem registry…" />
        ) : (
          <EmptyState
            icon={Boxes}
            title="No subsystem is selected"
            message="Choose a registered subsystem to browse its protected source branch."
          />
        )}
      </div>
    );
  }

  const detailsPanel = (
    <CommitDetailsPanel
      details={details.data}
      loading={details.loading}
      error={details.error}
      onRetry={details.reload}
      targetBranch={app.targetBranch}
      selectedShas={app.selectedShas}
      onPromote={promote}
      onPromoteSelection={promoteSelection}
      onShowDiff={showDiff}
      activeDiffPath={activeDiffPath}
      onOpenExternal={app.openExternal}
      onClose={isWide ? undefined : () => onDetailsVisibleChange(false)}
      mainRepository={app.auth.data?.mainRepository ?? null}
    />
  );

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 gap-2.5">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-2.5">
        <Panel
          className="min-h-0 flex-1"
          bodyClassName="flex min-h-0 flex-1 flex-col"
          title={
            <span className="flex items-center gap-2">
              <span className="flex h-4 w-4 items-center justify-center text-muted">
                <Boxes className="h-4 w-4" aria-hidden="true" />
              </span>
              <span className="truncate">{subsystem.name}</span>
            </span>
          }
          subtitle={<span className="mono">{subsystem.repository}</span>}
          actions={
            <>
              <span className="hidden items-center gap-2 lg:flex">
                <GitBranch className="h-4 w-4 text-disabled" aria-hidden="true" />
                <Select
                  aria-label="Subsystem source branch"
                  className="w-[150px]"
                  value={subsystem.sourceBranch}
                  onChange={() => undefined}
                  options={[{ value: subsystem.sourceBranch, label: subsystem.sourceBranch }]}
                  title="The source branch is fixed by the registry entry."
                />
              </span>
              <Button
                size="sm"
                variant="secondary"
                icon={ExternalLink}
                onClick={() =>
                  app.openExternal(
                    `https://github.com/${subsystem.repository}/tree/${subsystem.sourceBranch}`,
                  )
                }
                title="Open the subsystem repository on GitHub"
              >
                View on GitHub
              </Button>
            </>
          }
        >
          <Tabs
            ariaLabel="Selected subsystem sections"
            value={sourceTab}
            onChange={(id) => onSourceTabChange(id as SourceTab)}
            items={[
              { id: 'commits', label: 'Commits', count: commitPage.data?.total },
              { id: 'changes', label: 'Changes', count: details.data?.files.length },
              {
                id: 'dependencies',
                label: 'Dependencies',
                count: details.data?.dependencies.length,
              },
              { id: 'runs', label: 'Workflow Runs', count: commitRuns.length },
              { id: 'settings', label: 'Settings' },
            ]}
          />

          {sourceTab === 'commits' ? (
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
              focusedSha={focusedSha}
              onFocusSha={app.setFocusedSha}
              onPromoteSelected={promoteSelection}
              onClearSelection={app.clearSelection}
              detailsVisible={detailsVisible}
              onToggleDetails={() => onDetailsVisibleChange(!detailsVisible)}
              emptyMessage={`No commits were returned from ${subsystem.repository}@${subsystem.sourceBranch}. Check the registry entry and branch protection.`}
            />
          ) : null}

          {sourceTab === 'changes' ? (
            <div className="scroll-area min-h-0 flex-1 p-2.5">
              {details.loading && !details.data ? (
                <LoadingState label="Loading changed files…" />
              ) : details.data?.files.length ? (
                <ul className="space-y-1">
                  {details.data.files.map((file) => (
                    <li key={file.path}>
                      <button
                        type="button"
                        onClick={() => showDiff(details.data!.sha, file.path)}
                        className="flex h-[29px] w-full items-center gap-2.5 rounded-md border border-line bg-surface px-2.5 text-left hover:bg-hover"
                      >
                        <FileText className="h-4 w-4 shrink-0 text-disabled" aria-hidden="true" />
                        <span className="mono min-w-0 flex-1 truncate text-fg tabular-nums">
                          {file.path}
                        </span>
                        <span className="meta">{file.change}</span>
                        <span className="mono w-12 shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                          +{file.additions}
                        </span>
                        <span className="mono w-12 shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                          -{file.deletions}
                        </span>
                      </button>
                      {file.policyNote ? (
                        <p
                          className="mono mt-0.5 truncate pl-7 text-[11px] text-status-unknown-ink"
                          title={file.policyNote}
                        >
                          {file.policyNote}
                        </p>
                      ) : null}
                    </li>
                  ))}
                </ul>
              ) : (
                <EmptyState
                  icon={FileText}
                  title="No changed files"
                  message="Select a commit to inspect its file list."
                />
              )}
            </div>
          ) : null}

          {sourceTab === 'dependencies' ? (
            <div className="scroll-area min-h-0 flex-1 space-y-3 p-2.5">
              {details.data?.dependencies.length ? (
                <ul className="space-y-2">
                  {details.data.dependencies.map((dependency) => (
                    <li
                      key={`${dependency.section}-${dependency.name}`}
                      className={cx(
                        'rounded-md border border-line p-2.5 text-xs',
                        dependency.conflict
                          ? 'bg-status-blocked-soft text-status-blocked-ink'
                          : 'bg-surface text-muted',
                      )}
                    >
                      <div className="flex items-center gap-2">
                        <Package className="h-4 w-4 shrink-0 text-disabled" aria-hidden="true" />
                        <span className="mono truncate text-fg">{dependency.name}</span>
                        <span className="meta">{dependency.section}</span>
                        <span className="ml-auto tracking-wide text-disabled uppercase">
                          {dependency.change}
                        </span>
                      </div>
                      <p className="mono mt-1.5 truncate tabular-nums">
                        {dependency.fromRange ?? '(absent)'} → {dependency.toRange ?? '(removed)'}
                      </p>
                      {dependency.note ? (
                        <p className="mt-1 truncate text-muted" title={dependency.note}>
                          {dependency.note}
                        </p>
                      ) : null}
                    </li>
                  ))}
                </ul>
              ) : (
                <EmptyState
                  icon={Package}
                  title="No manifest changes in this commit"
                  message="Promote a package commit together with the commits that use it."
                />
              )}
            </div>
          ) : null}

          {sourceTab === 'runs' ? (
            <div className="scroll-area min-h-0 flex-1 p-2.5">
              {runs.loading && !runs.data ? (
                <LoadingState label="Loading workflow runs…" />
              ) : runs.error ? (
                <ErrorState error={runs.error} onRetry={runs.reload} />
              ) : (runs.data?.runs.length ?? 0) === 0 ? (
                <EmptyState
                  icon={Workflow}
                  title="No runs for this subsystem yet"
                  message="Runs appear here after a promotion is dispatched from this tool."
                />
              ) : (
                <ul className="space-y-2">
                  {(runs.data?.runs ?? []).map((run) => (
                    <li
                      key={run.id}
                      className="flex h-[50px] flex-wrap items-center gap-2 rounded-md border border-line bg-surface px-2.5"
                    >
                      {runChip(run)}
                      <span className="min-w-0 flex-1 truncate text-xs text-fg">
                        {run.displayTitle}
                      </span>
                      <span className="meta">target {run.targetBranch}</span>
                      <span className="mono text-right text-[11px] text-disabled tabular-nums">
                        {formatDateTime(run.updatedAt)}
                      </span>
                      <IconButton
                        icon={ExternalLink}
                        label="Open run on GitHub"
                        onClick={() => app.openExternal(run.htmlUrl)}
                      />
                    </li>
                  ))}
                </ul>
              )}
            </div>
          ) : null}

          {sourceTab === 'settings' ? (
            <div className="scroll-area min-h-0 flex-1 space-y-3 p-2.5">
              <dl className="rounded-md border border-line bg-raised px-3 py-1">
                <KeyValue label="Registry id">
                  <span className="mono">{subsystem.id}</span>
                </KeyValue>
                <KeyValue label="Repository">
                  <span className="mono">{subsystem.repository}</span>
                </KeyValue>
                <KeyValue label="Source branch">
                  <span className="mono">{subsystem.sourceBranch}</span>
                </KeyValue>
                <KeyValue label="Path strategy">{subsystem.pathStrategy}</KeyValue>
                <KeyValue label="Dependency strategy">{subsystem.dependencyStrategy}</KeyValue>
                <KeyValue label="Registered">{formatDateTime(subsystem.registeredAt)}</KeyValue>
                <KeyValue label="Last verified">{formatDateTime(subsystem.lastSyncedAt)}</KeyValue>
                <KeyValue label="Enabled">{subsystem.enabled ? 'yes' : 'no'}</KeyValue>
              </dl>

              <div className="rounded-md border border-line bg-raised p-2.5">
                <p className="flex items-center gap-2 text-xs font-semibold text-fg">
                  <SettingsIcon className="h-4 w-4 text-disabled" aria-hidden="true" />
                  Integration policy
                </p>
                <dl className="mt-1">
                  <KeyValue label="Application files">{subsystem.policy.applicationFiles}</KeyValue>
                  <KeyValue label="package.json">{subsystem.policy.packageJson}</KeyValue>
                  <KeyValue label="Lockfiles">{subsystem.policy.lockfiles}</KeyValue>
                  <KeyValue label="Shared configuration">
                    {subsystem.policy.sharedConfiguration}
                  </KeyValue>
                  <KeyValue label="GitHub workflows">{subsystem.policy.githubWorkflows}</KeyValue>
                  <KeyValue label="Secret files">{subsystem.policy.secretFiles}</KeyValue>
                </dl>
              </div>

              <InlineAlert tone="warning" title="Disabling is a registry change">
                Turn it off under Settings → Main system registry.
              </InlineAlert>
            </div>
          ) : null}
        </Panel>

        <Panel
          className="h-[290px] shrink-0"
          bodyClassName="flex min-h-0 flex-1 flex-col"
          headerClassName="px-2 py-0"
          title={undefined}
        >
          <Tabs
            ariaLabel="Diff, workflow logs and related pull requests"
            value={bottomTab}
            onChange={(id) => setBottomTab(id as BottomTab)}
            items={[
              { id: 'diff', label: 'Diff' },
              { id: 'logs', label: 'Workflow Logs', count: commitRuns.length || undefined },
              {
                id: 'prs',
                label: 'Related Pull Requests',
                count: details.data?.relatedPullRequests.length,
              },
            ]}
          />

          {bottomTab === 'diff' ? (
            <DiffViewer
              diff={diff.data}
              loading={diff.loading}
              error={diff.error}
              onRetry={diff.reload}
              files={details.data?.files ?? []}
              selectedPath={activeDiffPath}
              onSelectPath={setDiffPath}
              onOpenExternal={app.openExternal}
            />
          ) : null}

          {bottomTab === 'logs' ? (
            <div className="scroll-area min-h-0 flex-1 p-2.5">
              {commitRuns.length === 0 ? (
                <EmptyState
                  icon={History}
                  title="No workflow run for this commit"
                  message="Dispatch a promotion to create one."
                  action={
                    <Button size="sm" icon={RefreshCw} onClick={runs.reload}>
                      Refresh runs
                    </Button>
                  }
                />
              ) : (
                <ul className="space-y-2">
                  {commitRuns.map((run) => (
                    <li key={run.id} className="rounded-md border border-line bg-surface p-2.5">
                      <div className="flex flex-wrap items-center gap-2">
                        {runChip(run)}
                        <span className="min-w-0 flex-1 truncate text-xs text-fg">
                          {run.displayTitle}
                        </span>
                        <span className="mono text-disabled">{run.headBranch}</span>
                        <IconButton
                          icon={ExternalLink}
                          label="Open run on GitHub"
                          onClick={() => app.openExternal(run.htmlUrl)}
                        />
                      </div>
                      <p className="mt-1.5 truncate text-[11px] text-muted">
                        Requested by {run.actor} · target {run.targetBranch} · mode {run.mode} ·{' '}
                        {formatDateTime(run.createdAt)}
                      </p>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          ) : null}

          {bottomTab === 'prs' ? (
            <div className="scroll-area min-h-0 flex-1 p-2.5">
              {details.data?.relatedPullRequests.length ? (
                <ul className="space-y-2">
                  {details.data.relatedPullRequests.map((pr) => (
                    <li
                      key={`${pr.repository}-${pr.number}`}
                      className="flex h-[50px] flex-wrap items-center gap-2 rounded-md border border-line bg-surface px-2.5"
                    >
                      <StatusBadge
                        kind={
                          pr.state === 'merged'
                            ? 'integrated'
                            : pr.state === 'open'
                              ? 'pr_open'
                              : 'unknown'
                        }
                        label={`PR #${pr.number} ${pr.state}`}
                      />
                      <span className="min-w-0 flex-1 truncate text-xs text-fg">{pr.title}</span>
                      <span className="mono text-disabled">{pr.repository}</span>
                      <span className="meta">base {pr.baseBranch}</span>
                      <IconButton
                        icon={ExternalLink}
                        label="Open pull request"
                        onClick={() => app.openExternal(pr.htmlUrl)}
                      />
                    </li>
                  ))}
                  {details.data.status.evidence?.pullRequestNumber &&
                  app.auth.data?.mainRepository ? (
                    <li className="flex h-[50px] flex-wrap items-center gap-2 rounded-md border border-line bg-surface px-2.5">
                      <StatusBadge
                        kind="integrated"
                        label={`Integration PR #${details.data.status.evidence.pullRequestNumber}`}
                      />
                      <span className="min-w-0 flex-1 truncate text-xs text-fg">
                        Integration record for this SHA
                      </span>
                      <span className="mono text-disabled">{app.auth.data.mainRepository}</span>
                      <IconButton
                        icon={ExternalLink}
                        label="Open integration PR"
                        onClick={() =>
                          app.openExternal(
                            `https://github.com/${app.auth.data?.mainRepository}/pull/${details.data?.status.evidence?.pullRequestNumber}`,
                          )
                        }
                      />
                    </li>
                  ) : null}
                </ul>
              ) : (
                <EmptyState
                  icon={GitPullRequestArrow}
                  title="No related pull requests"
                  message="Source and integration PRs for this commit appear here."
                />
              )}
            </div>
          ) : null}
        </Panel>
      </div>

      {isWide ? (
        detailsVisible ? (
          <Panel
            as="aside"
            className="w-[340px] shrink-0"
            bodyClassName="flex min-h-0 flex-1 flex-col"
          >
            {detailsPanel}
          </Panel>
        ) : null
      ) : detailsVisible ? (
        <>
          <button
            type="button"
            aria-label="Hide commit details"
            className="fixed inset-0 z-30 bg-scrim"
            onClick={() => onDetailsVisibleChange(false)}
          />
          <aside className="fixed top-16 right-0 bottom-0 z-40 flex w-[380px] max-w-full flex-col border-l border-line bg-surface">
            {detailsPanel}
          </aside>
        </>
      ) : null}
    </div>
  );
}

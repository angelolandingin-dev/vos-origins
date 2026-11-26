import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Boxes,
  ExternalLink,
  FileText,
  GitBranch,
  GitPullRequestArrow,
  Package,
  Settings as SettingsIcon,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { CommitStatusKind } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { CommitDiffView } from './CommitDiffView';
import { CommitTable } from './CommitTable';
import { ChangeKindIcon } from './ChangeKindIcon';
import { PathLabel } from './PathLabel';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import {
  Button,
  EmptyState,
  ErrorState,
  InlineAlert,
  KeyValue,
  LoadingState,
  Tabs,
} from './ui/primitives';
import {
  COMMIT_RAIL_DEFAULT_WIDTH,
  COMMIT_RAIL_MIN_WIDTH,
  clampCommitRailWidth,
  commitRailMaxWidth,
} from './ui/split-pane';
import { SplitResizer } from './ui/split-resizer';

type SourceTab = 'commits' | 'changes' | 'dependencies' | 'settings';

export function CommitWorkspace() {
  return <CommitWorkspaceView />;
}

function CommitWorkspaceView() {
  const app = useApp();
  const [search, setSearch] = useState('');
  const [statusFilter, setStatusFilter] = useState<CommitStatusKind | 'all'>('all');
  const [sort, setSort] = useState<'newest' | 'oldest'>('newest');
  const [pageSize, setPageSize] = useState(25);
  // The diff region sits beside the commit list as a permanent rail on wide
  // screens (and as a dismissible overlay below the compact breakpoint). Row
  // clicks only move focus; they never open or close the panel.
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
  // The file selected in the diff view's changed-files column, tagged with the
  // commit it belongs to so a commit switch falls back to that commit's first
  // file rather than a stale path. The Changes tab opens a file through it.
  const [selectedFile, setSelectedFile] = useState<{ sha: string; path: string } | null>(null);
  // Excluded paths persist across inspected commits while building a multi-commit selection.
  const [excludedPaths, setExcludedPaths] = useState<string[]>([]);
  // The commit rail's own width, separate from the diff splitter's. Its ceiling
  // depends on the live pane because the changed-files column and the diff keep
  // their own floors to its right.
  const [railWidth, setRailWidth] = useState(COMMIT_RAIL_DEFAULT_WIDTH);
  const [paneWidth, setPaneWidth] = useState(0);
  const columnsRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const list = window.matchMedia('(min-width: 1280px)');
    const update = () => setIsWide(list.matches);
    update();
    list.addEventListener('change', update);
    return () => list.removeEventListener('change', update);
  }, []);

  useEffect(() => {
    const element = columnsRef.current;
    if (!element) return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) setPaneWidth(entry.contentRect.width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [sourceTab]);

  const railMax = commitRailMaxWidth(paneWidth);
  const visibleRailWidth = clampCommitRailWidth(railWidth, paneWidth);

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
  const { rememberSelectedCommits } = app;
  useEffect(() => {
    rememberSelectedCommits(listing);
  }, [listing, rememberSelectedCommits]);

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

  const handleFocusSha = (sha: string) => {
    app.setFocusedSha(sha);
  };

  const openFile = (sha: string, filePath: string) => {
    app.setFocusedSha(sha);
    setSelectedFile({ sha, path: filePath });
    onSourceTabChange('commits');
  };

  const currentSha = details.data?.sha ?? focusedSha ?? '';
  const hasMultiSelection = app.selectedShas.length > 0;

  const activeExcluded = useMemo(() => {
    if (hasMultiSelection) return excludedPaths;
    const files = details.data?.files;
    if (!files) return excludedPaths;
    return excludedPaths.filter((path) => files.some((file) => file.path === path));
  }, [hasMultiSelection, excludedPaths, details.data?.files]);

  const openConfirmation = () => {
    if (hasMultiSelection) {
      app.openPromotionDialog(app.selectedShas, app.selectedCommits, {
        excludedPaths,
      });
      return;
    }
    if (!details.data) return;
    const commit = listing.find((candidate) => candidate.sha === details.data!.sha);
    app.openPromotionDialog([details.data.sha], commit ? [commit] : [], {
      excludedPaths: activeExcluded,
    });
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

  const toggleExcluded = (path: string) => {
    setExcludedPaths((prev) =>
      prev.includes(path) ? prev.filter((entry) => entry !== path) : [...prev, path],
    );
  };

  const clearSelection = () => {
    app.clearSelection();
    setExcludedPaths([]);
  };

  const statusKind = details.data?.status.kind;
  const isPromotable = statusKind === 'eligible' || statusKind === 'reverted';
  const canPromote = hasMultiSelection || Boolean(details.data && isPromotable);

  const diffPanel = (
    <CommitDiffView
      subsystemId={subsystem.id}
      sha={currentSha}
      targetBranch={app.targetBranch}
      details={details.data}
      loading={details.loading}
      error={details.error}
      onRetry={details.reload}
      selectedPath={
        selectedFile && selectedFile.sha === details.data?.sha ? selectedFile.path : null
      }
      onSelectPath={(path) => {
        if (details.data) setSelectedFile({ sha: details.data.sha, path });
      }}
      excludedPaths={activeExcluded}
      onToggleExcluded={toggleExcluded}
      mainRepository={app.auth.data?.mainRepository ?? null}
    />
  );

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-surface">
      <div className="flex min-h-[38px] flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-3 py-1">
        <Tabs
          ariaLabel="Selected subsystem sections"
          value={sourceTab}
          onChange={(id) => onSourceTabChange(id as SourceTab)}
          className="h-[29px] border-b-0 px-0"
          items={[
            { id: 'commits', label: 'Commits', count: commitPage.data?.total },
            { id: 'changes', label: 'Changes', count: details.data?.files.length },
            {
              id: 'dependencies',
              label: 'Dependencies',
              count: details.data?.dependencies.length,
            },
            { id: 'settings', label: 'Settings' },
          ]}
        />

        <div className="flex shrink-0 items-center gap-2">
          <span
            className="mono hidden text-[11px] text-muted lg:inline"
            title={subsystem.repository}
          >
            {subsystem.repository}
          </span>
          <span className="hidden items-center gap-1.5 text-[11px] text-muted lg:flex">
            <GitBranch className="h-3.5 w-3.5 text-disabled" aria-hidden="true" />
            <span
              className="mono inline-flex h-[25px] max-w-[150px] items-center truncate rounded-md border border-line bg-surface px-2 text-[11px] text-muted"
              title="The source branch is fixed by the registry entry."
            >
              {subsystem.sourceBranch}
            </span>
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
        </div>
      </div>

      {sourceTab === 'commits' ? (
        <div ref={columnsRef} className="flex min-h-0 flex-1">
          <div
            className={cx(
              'flex min-h-0 min-w-0 flex-col border-line',
              isWide ? 'shrink-0' : 'flex-1',
              // The resizer is the divider when the diff is beside it.
              isWide && !detailsVisible && 'border-r',
            )}
            style={isWide ? { width: visibleRailWidth } : undefined}
          >
            <CommitTable
              layout="sidebar"
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
              onClearSelection={clearSelection}
              focusedSha={focusedSha}
              onFocusSha={handleFocusSha}
              detailsVisible={detailsVisible}
              onToggleDetails={() => onDetailsVisibleChange(!detailsVisible)}
              emptyMessage={`No commits were returned from ${subsystem.repository}@${subsystem.sourceBranch}. Check the registry entry and branch protection.`}
            />
          </div>
          {isWide && detailsVisible ? (
            <SplitResizer
              width={visibleRailWidth}
              onWidthChange={setRailWidth}
              min={COMMIT_RAIL_MIN_WIDTH}
              max={railMax}
              label="Resize the commit list"
            />
          ) : null}
          {isWide && detailsVisible ? diffPanel : null}
        </div>
      ) : null}

      {sourceTab === 'changes' ? (
        <div className="scroll-area min-h-0 flex-1">
          {details.loading && !details.data ? (
            <LoadingState label="Loading changed files…" />
          ) : details.data?.files.length ? (
            <ul className="divide-y divide-line">
              {details.data.files.map((file) => (
                <li key={file.path}>
                  <button
                    type="button"
                    onClick={() => openFile(details.data!.sha, file.path)}
                    className="row-hover flex h-[29px] w-full items-center gap-2.5 px-3 text-left"
                  >
                    <ChangeKindIcon change={file.change} />
                    <PathLabel path={file.path} className="min-w-0 flex-1" />
                    {file.policyNote ? (
                      <span
                        className="mono hidden max-w-[260px] truncate text-[11px] text-status-unknown-ink md:inline"
                        title={file.policyNote}
                      >
                        {file.policyNote}
                      </span>
                    ) : null}
                    <span className="mono w-12 shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                      +{file.additions}
                    </span>
                    <span className="mono w-12 shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                      -{file.deletions}
                    </span>
                  </button>
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
        <div className="scroll-area min-h-0 flex-1">
          {details.data?.dependencies.length ? (
            <ul className="divide-y divide-line">
              {details.data.dependencies.map((dependency) => (
                <li
                  key={`${dependency.section}-${dependency.name}`}
                  className={cx(
                    'flex flex-col gap-1 px-4 py-2.5 text-xs',
                    dependency.conflict
                      ? 'bg-status-blocked-soft text-status-blocked-ink'
                      : 'bg-surface text-muted',
                  )}
                >
                  <div className="flex items-center gap-2">
                    <Package className="h-3.5 w-3.5 shrink-0 text-disabled" aria-hidden="true" />
                    <span className="mono truncate font-medium text-fg">{dependency.name}</span>
                    <span className="meta">{dependency.section}</span>
                    <span className="ml-auto tracking-wide text-disabled uppercase">
                      {dependency.change}
                    </span>
                  </div>
                  <p className="mono truncate pl-5 text-[11px] tabular-nums">
                    {dependency.fromRange ?? '(absent)'} → {dependency.toRange ?? '(removed)'}
                  </p>
                  {dependency.note ? (
                    <p className="truncate pl-5 text-[11px] text-muted" title={dependency.note}>
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

      {sourceTab === 'settings' ? (
        <div className="scroll-area min-h-0 flex-1 p-4">
          <div className="max-w-2xl space-y-4">
            <dl className="divide-y divide-line rounded-md border border-line bg-surface px-3">
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

            <div className="rounded-md border border-line bg-surface">
              <div className="flex items-center gap-2 border-b border-line bg-raised px-3 py-2 text-xs font-semibold text-fg">
                <SettingsIcon className="h-3.5 w-3.5 text-disabled" aria-hidden="true" />
                Integration policy
              </div>
              <dl className="divide-y divide-line px-3">
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
        </div>
      ) : null}

      <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
        <div className="min-w-0 text-[11px] text-muted">
          {hasMultiSelection ? (
            <span className="mono tabular-nums">
              {app.selectedShas.length} commit{app.selectedShas.length === 1 ? '' : 's'} selected ·{' '}
              {app.promotion.mode} · target {app.targetBranch}
              {excludedPaths.length > 0 ? ` · ${excludedPaths.length} excluded` : ''}
            </span>
          ) : !details.data ? (
            <span>
              Click a commit to inspect its diff, or tick checkboxes to promote multiple commits to{' '}
              <span className="mono">{app.targetBranch}</span>.
            </span>
          ) : (
            <span className="mono tabular-nums">
              {details.data.shortSha} · {details.data.files.length} file
              {details.data.files.length === 1 ? '' : 's'} · target {app.targetBranch}
              {activeExcluded.length > 0 ? ` · ${activeExcluded.length} excluded` : ''}
              {!isPromotable ? ' · tick eligible commits on the left to build a promotion' : ''}
            </span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {hasMultiSelection ? (
            <Button variant="ghost" size="sm" onClick={clearSelection}>
              Clear selection
            </Button>
          ) : null}
          <Button
            variant="primary"
            icon={GitPullRequestArrow}
            disabled={!canPromote}
            onClick={openConfirmation}
            title={
              hasMultiSelection
                ? `Open promotion confirmation for ${app.selectedShas.length} selected commit(s)`
                : !details.data
                  ? 'Select or focus a commit first'
                  : isPromotable
                    ? `Open promotion confirmation for ${details.data.shortSha}`
                    : 'Only eligible (or previously reverted) commits can be promoted'
            }
          >
            {hasMultiSelection
              ? `Promote ${app.selectedShas.length} commit${app.selectedShas.length === 1 ? '' : 's'}`
              : details.data && isPromotable
                ? activeExcluded.length > 0
                  ? `Promote ${details.data.shortSha} (${activeExcluded.length} excluded)`
                  : `Promote ${details.data.shortSha}`
                : 'Continue to confirmation'}
          </Button>
        </div>
      </footer>

      {!isWide && detailsVisible ? (
        <>
          <button
            type="button"
            aria-label="Hide commit details"
            className="fixed inset-0 z-30 bg-scrim"
            onClick={() => onDetailsVisibleChange(false)}
          />
          <aside className="fixed top-16 right-0 bottom-0 z-40 flex w-[720px] max-w-full flex-col border-l border-line bg-surface">
            {diffPanel}
          </aside>
        </>
      ) : null}
    </div>
  );
}

import { useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  ArrowDownUp,
  ArrowUpDown,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  GitPullRequestArrow,
  RefreshCw,
  Search,
  SquareCheck,
  TriangleAlert,
  X,
} from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import type { ApiError } from '../api/invoke';
import type {
  DistributionBlocker,
  DistributionDependency,
  DistributionOutcome,
  DistributionPlan,
  DistributionPreview,
  DistributionRunResult,
  DistributionSkip,
  PromotionProgress,
  TrunkCommit,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { ChangeKindIcon } from '../components/ChangeKindIcon';
import { PathLabel } from '../components/PathLabel';
import { cx } from '../components/ui/cx';
import { formatDate, formatDateTime } from '../components/ui/format';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  Field,
  IconButton,
  InlineAlert,
  KeyValue,
  LoadingState,
  Panel,
  Select,
  Spinner,
} from '../components/ui/primitives';

/** Distribute: send the paths touched by selected trunk commits down to subsystem pull requests. */
export function Distribute() {
  const app = useApp();
  const [branchChoice, setBranchChoice] = useState<string | null>(null);
  const [selectedShas, setSelectedShas] = useState<string[]>([]);
  const [expandedSha, setExpandedSha] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<'newest' | 'oldest'>('newest');
  const [pageSize, setPageSize] = useState(25);
  const [pageNumber, setPageNumber] = useState(1);
  const [subsystemChoice, setSubsystemChoice] = useState<string[] | null>(null);
  const [message, setMessage] = useState('');
  const [preview, setPreview] = useState<DistributionPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState<ApiError | null>(null);
  const [excluded, setExcluded] = useState<Record<string, string[]>>({});
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<PromotionProgress[]>([]);
  const [runError, setRunError] = useState<ApiError | null>(null);
  const [result, setResult] = useState<DistributionRunResult | null>(null);

  const targetBranches = app.targetBranches;
  const trunkBranch = branchChoice ?? targetBranches.data?.targetBranch ?? '';

  const commitQuery = useAsyncData<TrunkCommit[]>(
    () => (trunkBranch ? api.listTrunkCommits(trunkBranch) : Promise.resolve([])),
    [trunkBranch],
  );
  const commits = useMemo(() => commitQuery.data ?? [], [commitQuery.data]);
  const commitsLoading = trunkBranch !== '' && commitQuery.loading;
  const commitError = trunkBranch === '' ? null : commitQuery.error;

  const visibleCommits = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const filtered =
      needle === ''
        ? commits
        : commits.filter((commit) =>
            `${commit.subject} ${commit.shortSha} ${commit.sha} ${commit.authorName}`
              .toLowerCase()
              .includes(needle),
          );
    return sort === 'newest' ? filtered : [...filtered].reverse();
  }, [commits, search, sort]);

  const totalPages = Math.max(1, Math.ceil(visibleCommits.length / pageSize));
  const page = Math.min(pageNumber, totalPages);
  const pagedCommits = visibleCommits.slice((page - 1) * pageSize, page * pageSize);
  const rangeStart = visibleCommits.length === 0 ? 0 : (page - 1) * pageSize + 1;
  const rangeEnd = Math.min(page * pageSize, visibleCommits.length);

  const enabledSubsystems = useMemo(
    () => (app.subsystems.data ?? []).filter((subsystem) => subsystem.enabled),
    [app.subsystems.data],
  );
  const enabledIds = useMemo(
    () => enabledSubsystems.map((subsystem) => subsystem.id),
    [enabledSubsystems],
  );
  const selectedIds = useMemo(() => {
    const chosen = subsystemChoice ?? enabledIds;
    return chosen.filter((id) => enabledIds.includes(id));
  }, [subsystemChoice, enabledIds]);

  const selectedCommits = useMemo(
    () => commits.filter((commit) => selectedShas.includes(commit.sha)),
    [commits, selectedShas],
  );
  const commitShas = selectedCommits.map((commit) => commit.sha);
  const readyCount = preview
    ? preview.subsystems.filter((plan) => plan.blockers.length === 0).length
    : 0;
  const locked = previewLoading || running || result !== null;
  const canPreview =
    trunkBranch !== '' && commitShas.length > 0 && selectedIds.length > 0 && !locked;
  const canRun = preview !== null && readyCount > 0 && !locked;
  const latestProgress = progress.length > 0 ? (progress[progress.length - 1] ?? null) : null;
  const nothingToDistribute =
    preview !== null &&
    preview.subsystems.every(
      (plan) =>
        plan.paths.length === 0 &&
        plan.dependencies.length === 0 &&
        plan.skipped.length === 0 &&
        plan.blockers.length === 0,
    );

  const branchOptions = (targetBranches.data?.branches ?? []).map((branch) => ({
    value: branch.name,
    label: `${branch.name}${branch.isDefault ? ' (default)' : ''}`,
    disabled: !branch.allowed,
    title: branch.allowed ? undefined : (branch.disallowedReason ?? 'Not permitted'),
  }));

  const resetDistribution = () => {
    setPreview(null);
    setPreviewError(null);
    setExcluded({});
    setProgress([]);
    setRunError(null);
    setResult(null);
  };

  const changeTrunkBranch = (next: string) => {
    setBranchChoice(next);
    setSelectedShas([]);
    setExpandedSha(null);
    setSearch('');
    setPageNumber(1);
    resetDistribution();
  };

  const refreshCommits = () => {
    setSelectedShas([]);
    setExpandedSha(null);
    setPageNumber(1);
    resetDistribution();
    commitQuery.reload();
  };

  const toggleCommit = (sha: string) => {
    setSelectedShas((current) =>
      current.includes(sha) ? current.filter((value) => value !== sha) : [...current, sha],
    );
    resetDistribution();
  };

  const toggleSubsystem = (id: string) => {
    const base = subsystemChoice ?? enabledIds;
    setSubsystemChoice(base.includes(id) ? base.filter((value) => value !== id) : [...base, id]);
    resetDistribution();
  };

  const selectAllSubsystems = () => {
    setSubsystemChoice(enabledIds);
    resetDistribution();
  };

  const selectNoSubsystems = () => {
    setSubsystemChoice([]);
    resetDistribution();
  };

  const excludedFor = (plan: DistributionPlan) => excluded[plan.subsystemId] ?? [];

  const togglePath = (plan: DistributionPlan, path: string, included: boolean) => {
    setExcluded((current) => {
      const planPaths = current[plan.subsystemId] ?? [];
      const nextPaths = included
        ? planPaths.filter((value) => value !== path)
        : planPaths.includes(path)
          ? planPaths
          : [...planPaths, path];
      return { ...current, [plan.subsystemId]: nextPaths };
    });
  };

  const loadPreview = () => {
    if (!canPreview) return;
    resetDistribution();
    setPreviewLoading(true);
    api
      .previewDistribution({ mainBranch: trunkBranch, subsystemIds: selectedIds, commitShas })
      .then((next) => {
        setPreview(next);
        setPreviewLoading(false);
      })
      .catch((error: unknown) => {
        setPreviewError(asApiError(error));
        setPreviewLoading(false);
      });
  };

  const startDistribution = () => {
    if (!canRun || !preview) return;
    const runnable = preview.subsystems.filter((plan) => plan.blockers.length === 0);
    const trimmedMessage = message.trim();
    const request = {
      mainBranch: preview.mainBranch,
      message: trimmedMessage === '' ? null : trimmedMessage,
      commitShas,
      subsystems: runnable.map((plan) => ({
        subsystemId: plan.subsystemId,
        excludedPaths: excludedFor(plan),
      })),
    };
    setRunError(null);
    setProgress([]);
    setRunning(true);

    // Listen before invoking so the first per-subsystem event is not missed.
    let stopListening: (() => void) | null = null;
    let done = false;
    const finishListening = () => {
      done = true;
      stopListening?.();
      stopListening = null;
    };
    void listen<PromotionProgress>('distribution-progress', (event) => {
      if (done) return;
      setProgress((current) => [...current, event.payload]);
    })
      .then((stop) => {
        if (done) stop();
        else stopListening = stop;
      })
      .catch(() => {})
      .then(() => api.runDistribution(request))
      .then((next) => {
        finishListening();
        setResult(next);
        setRunning(false);
        app.bumpDataVersion();
      })
      .catch((error: unknown) => {
        finishListening();
        setRunError(asApiError(error));
        setRunning(false);
      });
  };

  const clearSelection = () => {
    setSelectedShas([]);
    setExpandedSha(null);
    resetDistribution();
  };

  const resetAll = () => {
    setSelectedShas([]);
    setExpandedSha(null);
    setMessage('');
    resetDistribution();
  };

  const openPullRequest = (url: string) => {
    app.openExternal(url);
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-center justify-between gap-3 px-5 pb-3 pt-4">
        <h1 className="text-[14px] font-semibold text-fg">Distribute</h1>
        <div className="flex items-center gap-2">
          <span className="text-[11px] text-muted">Trunk branch</span>
          <Select
            ariaLabel="Trunk branch"
            value={trunkBranch}
            onValueChange={changeTrunkBranch}
            options={branchOptions}
            disabled={locked || targetBranches.loading || branchOptions.length === 0}
            className="w-[220px]"
            title={targetBranches.error?.message ?? 'Branch the commits are taken from'}
          />
          {targetBranches.error ? (
            <button
              type="button"
              onClick={targetBranches.reload}
              className="text-[11px] text-status-blocked-ink underline"
            >
              Retry branches
            </button>
          ) : null}
        </div>
      </header>

      <div className="grid min-h-0 flex-1 gap-3 px-3 pb-3 xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="scroll-area min-h-0 space-y-3">
          <Panel
            title="Commits"
            subtitle={
              commitsLoading
                ? trunkBranch
                : commits.length > 0
                  ? `${selectedShas.length} of ${commits.length} selected`
                  : trunkBranch
            }
            actions={
              <Button
                variant="ghost"
                size="sm"
                icon={RefreshCw}
                disabled={locked || trunkBranch === ''}
                onClick={refreshCommits}
              >
                Refresh
              </Button>
            }
          >
            {targetBranches.loading && trunkBranch === '' ? (
              <LoadingState label="Loading branches" />
            ) : trunkBranch === '' ? (
              <EmptyState title="No trunk branch available" />
            ) : commitsLoading ? (
              <LoadingState label="Loading commits" />
            ) : commitError ? (
              <ErrorState compact error={commitError} onRetry={refreshCommits} />
            ) : commits.length === 0 ? (
              <EmptyState title="No commits on this branch" />
            ) : (
              <>
                <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
                  <div className="relative min-w-[180px] flex-1">
                    <Search
                      className="pointer-events-none absolute top-1/2 left-3 h-4 w-4 -translate-y-1/2 text-disabled"
                      aria-hidden="true"
                    />
                    <input
                      type="search"
                      value={search}
                      onChange={(event) => {
                        setSearch(event.target.value);
                        setPageNumber(1);
                      }}
                      placeholder="Search commits…"
                      aria-label="Search commits by message, SHA or author"
                      className="input pl-9"
                      disabled={locked}
                    />
                  </div>
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={sort === 'newest' ? ArrowDownUp : ArrowUpDown}
                    disabled={locked}
                    onClick={() => {
                      setSort(sort === 'newest' ? 'oldest' : 'newest');
                      setPageNumber(1);
                    }}
                    title={
                      sort === 'newest'
                        ? 'Sorted newest first — click for oldest first'
                        : 'Sorted oldest first — click for newest first'
                    }
                  >
                    {sort === 'newest' ? 'Newest' : 'Oldest'}
                  </Button>
                  {selectedShas.length > 0 ? (
                    <>
                      <span className="meta text-accent-ink tabular-nums">
                        <SquareCheck className="h-2.5 w-2.5" aria-hidden="true" />
                        {selectedShas.length} selected
                      </span>
                      <IconButton
                        icon={X}
                        label="Clear commit selection"
                        onClick={clearSelection}
                      />
                    </>
                  ) : null}
                </div>

                <div className="scroll-area max-h-[440px]">
                  {visibleCommits.length === 0 ? (
                    <EmptyState
                      icon={Search}
                      title="No commits match the search"
                      message="Clear the search to see the full history of this branch."
                      action={
                        <Button size="sm" onClick={() => setSearch('')}>
                          Clear search
                        </Button>
                      }
                    />
                  ) : (
                    <table className="w-full border-separate border-spacing-0">
                      <caption className="sr-only">
                        Commits on trunk branch {trunkBranch}. Selecting commits offers the paths
                        they touched to every selected subsystem.
                      </caption>
                      <thead className="sticky top-0 z-10">
                        <tr className="table-head">
                          <th scope="col" className="w-10 py-2 pl-2.5 text-left">
                            <span className="sr-only">Selected</span>
                          </th>
                          <th scope="col" className="w-6 py-2" />
                          <th scope="col" className="w-24 py-2 pr-2.5 text-right">
                            SHA
                          </th>
                          <th scope="col" className="py-2 pr-2.5 text-left">
                            Message
                          </th>
                          <th
                            scope="col"
                            className="hidden w-40 py-2 pr-2.5 text-left xl:table-cell"
                          >
                            Author
                          </th>
                          <th
                            scope="col"
                            className="hidden w-28 py-2 pr-2.5 text-right xl:table-cell"
                          >
                            Committed
                          </th>
                          <th scope="col" className="w-24 py-2 pr-2.5 text-right">
                            Files
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {pagedCommits.map((commit) => (
                          <CommitRow
                            key={commit.sha}
                            commit={commit}
                            selected={selectedShas.includes(commit.sha)}
                            expanded={expandedSha === commit.sha}
                            locked={locked}
                            onToggle={() => toggleCommit(commit.sha)}
                            onExpand={() =>
                              setExpandedSha(expandedSha === commit.sha ? null : commit.sha)
                            }
                          />
                        ))}
                      </tbody>
                    </table>
                  )}
                </div>

                <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line px-2.5 py-2 text-[11px] text-muted">
                  <span className="truncate">
                    {visibleCommits.length === 0
                      ? 'No commits'
                      : `Showing ${rangeStart}–${rangeEnd} of ${visibleCommits.length}`}
                    {visibleCommits.length === commits.length
                      ? ''
                      : ` (${commits.length} on branch)`}
                  </span>
                  <div className="flex items-center gap-2">
                    <label className="flex items-center gap-1.5 text-[11px] text-muted">
                      Rows
                      <Select
                        className="w-[76px]"
                        value={String(pageSize)}
                        onValueChange={(next) => {
                          setPageSize(Number(next));
                          setPageNumber(1);
                        }}
                        ariaLabel="Rows per page"
                        options={[25, 50, 100].map((size) => ({
                          value: String(size),
                          label: String(size),
                        }))}
                      />
                    </label>
                    <IconButton
                      icon={ChevronLeft}
                      label="Previous page"
                      disabled={page <= 1}
                      onClick={() => setPageNumber(Math.max(1, page - 1))}
                    />
                    <span className="mono text-right text-muted tabular-nums">
                      {page} / {totalPages}
                    </span>
                    <IconButton
                      icon={ChevronRight}
                      label="Next page"
                      disabled={page >= totalPages}
                      onClick={() => setPageNumber(Math.min(totalPages, page + 1))}
                    />
                  </div>
                </div>
              </>
            )}
          </Panel>

          {previewError ? (
            <Panel title="Preview" bodyClassName="p-3">
              <ErrorState compact error={previewError} onRetry={loadPreview} />
            </Panel>
          ) : previewLoading ? (
            <Panel title="Preview">
              <LoadingState label="Building the distribution preview" />
            </Panel>
          ) : result ? (
            <Panel
              title="Results"
              subtitle={`${result.outcomes.filter((outcome) => outcome.ok).length} of ${result.outcomes.length} branches pushed · ${result.mainBranch}`}
              bodyClassName="divide-y divide-line"
            >
              {result.outcomes.map((outcome) => (
                <OutcomeSection
                  key={outcome.subsystemId}
                  outcome={outcome}
                  name={
                    preview?.subsystems.find((plan) => plan.subsystemId === outcome.subsystemId)
                      ?.name ?? outcome.subsystemId
                  }
                  onOpenPr={openPullRequest}
                />
              ))}
            </Panel>
          ) : preview ? (
            preview.subsystems.length === 0 || nothingToDistribute ? (
              <EmptyState
                icon={GitPullRequestArrow}
                title="Nothing to distribute"
                message="The selected commits touch no files in the selected subsystems."
              />
            ) : (
              <div className="space-y-3">
                {preview.subsystems.map((plan) => (
                  <PlanCard
                    key={plan.subsystemId}
                    plan={plan}
                    excludedPaths={excludedFor(plan)}
                    locked={locked}
                    onTogglePath={(path, included) => togglePath(plan, path, included)}
                  />
                ))}
              </div>
            )
          ) : (
            <EmptyState
              icon={GitPullRequestArrow}
              title="Nothing previewed"
              message="Pick trunk commits and subsystems, then preview what each branch receives."
            />
          )}
        </div>

        <div className="scroll-area min-h-0 space-y-3">
          <Panel
            title="Subsystems"
            subtitle={`${selectedIds.length} of ${enabledSubsystems.length} selected`}
            actions={
              <>
                <Button variant="ghost" size="sm" disabled={locked} onClick={selectAllSubsystems}>
                  All
                </Button>
                <Button variant="ghost" size="sm" disabled={locked} onClick={selectNoSubsystems}>
                  None
                </Button>
              </>
            }
          >
            {app.subsystems.loading && enabledSubsystems.length === 0 ? (
              <LoadingState label="Loading subsystems" />
            ) : app.subsystems.error ? (
              <ErrorState compact error={app.subsystems.error} onRetry={app.subsystems.reload} />
            ) : enabledSubsystems.length === 0 ? (
              <EmptyState title="No enabled subsystems" />
            ) : (
              <ul className="scroll-area max-h-[280px] divide-y divide-line">
                {enabledSubsystems.map((subsystem) => (
                  <li key={subsystem.id} className="flex h-[29px] items-center gap-2 px-2.5">
                    <Checkbox
                      checked={selectedIds.includes(subsystem.id)}
                      disabled={locked}
                      onChange={() => toggleSubsystem(subsystem.id)}
                      label={`Select ${subsystem.name}`}
                    />
                    <span
                      className="min-w-0 flex-1 truncate text-xs text-fg"
                      title={subsystem.name}
                    >
                      {subsystem.name}
                    </span>
                    <span
                      className="mono shrink-0 text-[11px] text-muted"
                      title={subsystem.sourceBranch}
                    >
                      {subsystem.sourceBranch}
                    </span>
                  </li>
                ))}
              </ul>
            )}
          </Panel>

          <Panel
            title="Distribution"
            actions={
              preview !== null || result !== null ? (
                <Button variant="ghost" size="sm" onClick={resetAll}>
                  New distribution
                </Button>
              ) : null
            }
          >
            <div className="space-y-3 p-3">
              <dl>
                <KeyValue label="Trunk">
                  <span className="mono">{trunkBranch || '—'}</span>
                </KeyValue>
                <KeyValue label="Commits">{commitShas.length}</KeyValue>
                <KeyValue label="Subsystems">{selectedIds.length}</KeyValue>
                <KeyValue label="Ready">
                  {preview ? `${readyCount} of ${preview.subsystems.length}` : 'Not previewed'}
                </KeyValue>
              </dl>

              <Field
                label="Commit message"
                htmlFor="distribute-message"
                hint="Blank uses the default subject."
              >
                <textarea
                  id="distribute-message"
                  rows={3}
                  className="input min-h-[76px] py-2"
                  placeholder="Optional"
                  value={message}
                  disabled={locked}
                  onChange={(event) => setMessage(event.target.value)}
                />
              </Field>

              <div className="grid grid-cols-2 gap-2">
                <Button
                  variant="secondary"
                  onClick={loadPreview}
                  disabled={!canPreview}
                  loading={previewLoading}
                >
                  Preview
                </Button>
                <Button
                  variant="primary"
                  onClick={startDistribution}
                  disabled={!canRun}
                  loading={running}
                  title={
                    preview && readyCount === 0 ? 'Every previewed subsystem is blocked' : undefined
                  }
                >
                  Distribute
                </Button>
              </div>

              {running ? (
                <div className="space-y-1" role="status" aria-live="polite">
                  <div className="flex items-center justify-between gap-2 text-[11px]">
                    <span className="flex min-w-0 items-center gap-1.5 text-fg">
                      <Spinner className="h-3 w-3" />
                      <span className="truncate">
                        {latestProgress?.label ?? 'Starting distribution'}
                      </span>
                    </span>
                    {latestProgress?.index != null && latestProgress.total != null ? (
                      <span className="mono shrink-0 text-muted">
                        {latestProgress.index}/{latestProgress.total}
                      </span>
                    ) : null}
                  </div>
                  <div className="h-[3px] overflow-hidden rounded-full bg-line">
                    <div
                      className="h-full bg-accent transition-[width]"
                      style={{ width: `${progressPercent(latestProgress)}%` }}
                    />
                  </div>
                  {latestProgress?.detail ? (
                    <p className="truncate text-[11px] text-muted">{latestProgress.detail}</p>
                  ) : null}
                </div>
              ) : null}

              {runError ? <ErrorState compact error={runError} /> : null}
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}

function progressPercent(progress: PromotionProgress | null): number {
  if (!progress || progress.index == null || progress.total == null || progress.total <= 0)
    return 4;
  return Math.max(4, Math.min(100, Math.round((progress.index / progress.total) * 100)));
}

function StatusMark({ blocked }: { blocked: boolean }) {
  return (
    <span className="flex items-center gap-1.5 text-[11px] text-muted">
      <span
        className={cx(
          'h-1.5 w-1.5 rounded-full',
          blocked ? 'bg-status-blocked' : 'bg-status-eligible',
        )}
        aria-hidden="true"
      />
      {blocked ? 'Blocked' : 'Ready'}
    </span>
  );
}

function CommitRow({
  commit,
  selected,
  expanded,
  locked,
  onToggle,
  onExpand,
}: {
  commit: TrunkCommit;
  selected: boolean;
  expanded: boolean;
  locked: boolean;
  onToggle: () => void;
  onExpand: () => void;
}) {
  return (
    <>
      <tr
        onClick={onExpand}
        className={cx(
          'row-hover h-[50px] cursor-pointer border-b border-line',
          selected && 'bg-selection',
        )}
      >
        <td className="py-2 pl-2.5 align-top">
          {/* Ticking a box must not also expand the row. */}
          <span onClick={(event) => event.stopPropagation()}>
            <Checkbox
              checked={selected}
              disabled={locked}
              onChange={onToggle}
              label={`Select commit ${commit.shortSha} – ${commit.subject}`}
            />
          </span>
        </td>
        <td className="relative w-6 align-top">
          <span
            aria-hidden="true"
            className="absolute top-0 bottom-0 left-1/2 w-px -translate-x-1/2 bg-line"
          />
          <span
            aria-hidden="true"
            className="absolute top-4 left-1/2 h-2.5 w-2.5 -translate-x-1/2 rounded-full bg-accent ring-2 ring-accent/30"
          />
        </td>
        <td className="py-2 pr-2.5 text-right align-top">
          <span className="mono text-[11px] text-accent-ink" title={commit.sha}>
            {commit.shortSha}
          </span>
        </td>
        <td className="max-w-[52ch] py-2 pr-2.5 align-top">
          <span className="block truncate text-xs text-fg" title={commit.subject}>
            {commit.subject}
          </span>
          <p
            className="mt-0.5 truncate text-[11px] text-muted"
            title={`${commit.authorName} · ${formatDateTime(commit.committedAt)}`}
          >
            {commit.authorName} · {formatDateTime(commit.committedAt)}
          </p>
        </td>
        <td className="hidden max-w-40 py-2 pr-2.5 align-top text-xs text-muted xl:table-cell">
          <span className="block truncate">{commit.authorName}</span>
        </td>
        <td className="mono hidden py-2 pr-2.5 text-right text-muted tabular-nums xl:table-cell">
          {formatDate(commit.committedAt)}
        </td>
        <td className="py-2 pr-2.5 align-top">
          <div className="flex items-center justify-end gap-1.5">
            <span
              className="text-[11px] text-muted tabular-nums"
              title={`${commit.filesChanged} files changed`}
            >
              {commit.filesChanged}
            </span>
            <span onClick={(event) => event.stopPropagation()}>
              <IconButton
                icon={ChevronDown}
                label={
                  expanded ? `Hide paths of ${commit.shortSha}` : `Show paths of ${commit.shortSha}`
                }
                aria-expanded={expanded}
                className={cx('h-[21px] w-[21px]', expanded && 'text-fg')}
                onClick={onExpand}
              />
            </span>
          </div>
        </td>
      </tr>
      {expanded ? (
        <tr className="bg-raised">
          <td colSpan={7} className="border-b border-line p-0">
            <ul className="scroll-area max-h-44 px-2.5 py-1">
              {commit.paths.map((entry) => (
                <li key={entry.path} className="flex h-[25px] items-center gap-2">
                  <ChangeKindIcon change={entry.change} />
                  <PathLabel path={entry.path} className="text-[11px]" />
                </li>
              ))}
            </ul>
          </td>
        </tr>
      ) : null}
    </>
  );
}

function BlockerList({ blockers }: { blockers: DistributionBlocker[] }) {
  if (blockers.length === 0) return null;
  return (
    <div className="space-y-1.5">
      {blockers.map((blocker, index) => (
        <InlineAlert key={`${blocker.code}-${index}`} tone="danger" title={blocker.message}>
          {blocker.details ? (
            <pre className="mono max-h-40 overflow-auto whitespace-pre-wrap break-words text-[11px] text-muted">
              {blocker.details}
            </pre>
          ) : null}
        </InlineAlert>
      ))}
    </div>
  );
}

function WarningsList({ warnings }: { warnings: string[] }) {
  if (warnings.length === 0) return null;
  return (
    <ul className="space-y-1">
      {warnings.map((warning, index) => (
        <li key={`${warning}-${index}`} className="flex gap-1.5 text-[11px] text-muted">
          <TriangleAlert
            className="mt-px h-3 w-3 shrink-0 text-status-unknown"
            aria-hidden="true"
          />
          <span className="min-w-0 break-words">{warning}</span>
        </li>
      ))}
    </ul>
  );
}

function SkippedList({ skipped }: { skipped: DistributionSkip[] }) {
  return (
    <details className="group">
      <summary className="flex h-[25px] cursor-pointer list-none items-center gap-1.5 text-[11px] text-muted">
        <ChevronDown
          className="h-3 w-3 transition-transform group-open:rotate-180"
          aria-hidden="true"
        />
        {skipped.length} paths skipped
      </summary>
      <ul className="mt-1 divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
        {skipped.map((entry, index) => (
          <li
            key={`${entry.path}-${index}`}
            className="flex min-h-[25px] items-center gap-2 px-2.5 py-1"
          >
            <span className="min-w-0 flex-1">
              <PathLabel path={entry.path} muted className="text-[11px]" />
            </span>
            <span
              className="max-w-[55%] shrink-0 truncate text-[11px] text-muted"
              title={entry.reason}
            >
              {entry.reason}
            </span>
          </li>
        ))}
      </ul>
    </details>
  );
}

function DependencyRow({ dependency }: { dependency: DistributionDependency }) {
  const from = dependency.fromSection
    ? `${dependency.fromSection}${dependency.fromSpec ? ` ${dependency.fromSpec}` : ''}`
    : 'not present';
  const to = `${dependency.section}${dependency.spec ? ` ${dependency.spec}` : ''}`;
  return (
    <li className="flex h-[29px] items-center gap-2 px-2.5 text-[11px]">
      <span className="mono w-[46px] shrink-0 uppercase text-muted">{dependency.action}</span>
      <span className="mono min-w-0 flex-1 truncate text-fg" title={dependency.name}>
        {dependency.name}
      </span>
      <span className="mono shrink-0 truncate text-muted" title={`${from} → ${to}`}>
        {from} → {to}
      </span>
    </li>
  );
}

function PlanCard({
  plan,
  excludedPaths,
  locked,
  onTogglePath,
}: {
  plan: DistributionPlan;
  excludedPaths: string[];
  locked: boolean;
  onTogglePath: (path: string, included: boolean) => void;
}) {
  const blocked = plan.blockers.length > 0;
  const includedCount = plan.paths.filter((path) => !excludedPaths.includes(path)).length;
  return (
    <Panel
      title={plan.name}
      subtitle={`${plan.sourceBranch} → ${plan.branch}`}
      actions={
        <>
          <span className="mono text-[11px] text-muted tabular-nums">
            {includedCount}/{plan.paths.length} included
          </span>
          <StatusMark blocked={blocked} />
        </>
      }
      bodyClassName="space-y-2 p-3"
    >
      {blocked ? <BlockerList blockers={plan.blockers} /> : null}
      <WarningsList warnings={plan.warnings} />
      {plan.paths.length === 0 ? (
        <p className="text-[11px] text-muted">No paths to distribute</p>
      ) : (
        <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
          {plan.paths.map((path) => (
            <li key={path} className="flex h-[25px] items-center gap-2 px-2.5">
              <Checkbox
                checked={!excludedPaths.includes(path)}
                disabled={locked || blocked}
                onChange={(event) => onTogglePath(path, event.target.checked)}
                label={`Include ${path} in ${plan.name}`}
              />
              <span className="min-w-0 flex-1">
                <PathLabel path={path} className="text-[11px]" />
              </span>
            </li>
          ))}
        </ul>
      )}
      {plan.skipped.length > 0 ? <SkippedList skipped={plan.skipped} /> : null}
      {plan.dependencies.length > 0 ? (
        <div className="space-y-1">
          <span className="text-[11px] font-semibold tracking-wide text-muted uppercase">
            Dependencies
          </span>
          <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
            {plan.dependencies.map((dependency) => (
              <DependencyRow
                key={`${dependency.section}:${dependency.name}`}
                dependency={dependency}
              />
            ))}
          </ul>
        </div>
      ) : null}
    </Panel>
  );
}

function OutcomeSection({
  outcome,
  name,
  onOpenPr,
}: {
  outcome: DistributionOutcome;
  name: string;
  onOpenPr: (url: string) => void;
}) {
  const pullRequest = outcome.pullRequest;
  const url = pullRequest?.url ?? null;
  const commitSha = outcome.commitSha;
  return (
    <div className="space-y-2 p-3">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className={cx(
              'h-1.5 w-1.5 shrink-0 rounded-full',
              outcome.ok ? 'bg-status-applied' : 'bg-status-blocked',
            )}
            aria-hidden="true"
          />
          <span className="truncate text-xs font-medium text-fg" title={name}>
            {name}
          </span>
          <span
            className={cx(
              'shrink-0 text-[11px]',
              outcome.ok ? 'text-status-applied-ink' : 'text-status-blocked-ink',
            )}
          >
            {outcome.ok
              ? pullRequest?.number != null
                ? `Pull request #${pullRequest.number} ${pullRequest.state}`
                : 'Pushed'
              : 'Blocked'}
          </span>
        </div>
        {url ? (
          <Button variant="secondary" size="sm" icon={ExternalLink} onClick={() => onOpenPr(url)}>
            Open pull request
          </Button>
        ) : null}
      </div>

      <BlockerList blockers={outcome.blockers} />
      <WarningsList warnings={outcome.warnings} />
      {outcome.message ? <p className="text-[11px] text-muted">{outcome.message}</p> : null}

      <dl>
        <KeyValue label="Branch">
          <span className="mono">{outcome.branch}</span>
        </KeyValue>
        <KeyValue label="Commit">
          {commitSha ? <span className="mono">{commitSha.slice(0, 7)}</span> : '—'}
        </KeyValue>
        <KeyValue label="Applied">{outcome.applied.length} paths</KeyValue>
        {pullRequest ? (
          <KeyValue label="Target">
            <span className="mono">
              {pullRequest.base} ← {pullRequest.head}
            </span>
          </KeyValue>
        ) : null}
      </dl>

      {outcome.dependencies.length > 0 ? (
        <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
          {outcome.dependencies.map((dependency) => (
            <DependencyRow
              key={`${dependency.section}:${dependency.name}`}
              dependency={dependency}
            />
          ))}
        </ul>
      ) : null}
    </div>
  );
}

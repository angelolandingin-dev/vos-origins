import { useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import {
  ArrowDownToLine,
  ArrowDownUp,
  ArrowUpDown,
  CheckCircle2,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ExternalLink,
  GitMerge,
  GitPullRequestArrow,
  RefreshCw,
  Search,
  TriangleAlert,
} from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import type { ApiError } from '../api/invoke';
import type {
  DistributionBlocker,
  DistributionDependency,
  DistributionOutcome,
  DistributionPathAction,
  DistributionPlan,
  DistributionSkip,
  MainCommitDiff,
  PromotionProgress,
  TrunkCommit,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { ChangeKindIcon } from '../components/ChangeKindIcon';
import { DiffViewer } from '../components/DiffViewer';
import { ExecutionLogDisclosure, ExecutionProgressView } from '../components/ExecutionProgressView';
import { appendProgressStep, type ExecutionStage } from '../components/executionProgress';
import { PathLabel } from '../components/PathLabel';
import { SubsystemSelector } from '../components/SubsystemSelector';
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
  Select,
  Spinner,
} from '../components/ui/primitives';
import { splitWidthStyle } from '../components/ui/split-pane';
import { SplitResizer } from '../components/ui/split-resizer';

const DISTRIBUTION_STAGES: ReadonlyArray<ExecutionStage> = [
  { id: 'prepare', label: 'Preparing checkouts' },
  { id: 'plan', label: 'Validating distribution plan' },
  { id: 'apply', label: 'Applying files and opening pull request' },
  { id: 'done', label: 'Finished' },
];

/** Distribute: move one trunk commit's changes into one subsystem via a pull request. */
export function Distribute() {
  const app = useApp();
  const [summaryWidth, setSummaryWidth] = useState(520);
  const [selectedSha, setSelectedSha] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [sort, setSort] = useState<'newest' | 'oldest'>('newest');
  const [pageSize, setPageSize] = useState(25);
  const [pageNumber, setPageNumber] = useState(1);

  const [excludedPaths, setExcludedPaths] = useState<string[]>([]);
  const [inspectedPath, setInspectedPath] = useState<string | null>(null);
  const [diffMode, setDiffMode] = useState<'unified' | 'side'>('unified');

  const [message, setMessage] = useState('');
  const [running, setRunning] = useState(false);
  const [progressSteps, setProgressSteps] = useState<PromotionProgress[]>([]);
  const [runError, setRunError] = useState<ApiError | null>(null);
  const [outcome, setOutcome] = useState<DistributionOutcome | null>(null);
  const latestProgress = progressSteps[progressSteps.length - 1] ?? null;

  const targetBranches = app.targetBranches;
  const trunkBranch = app.targetBranch || targetBranches.data?.targetBranch || '';

  const enabledSubsystems = useMemo(
    () => (app.subsystems.data ?? []).filter((subsystem) => subsystem.enabled),
    [app.subsystems.data],
  );
  const subsystemId = useMemo(() => {
    if (
      app.selectedSubsystem?.enabled &&
      enabledSubsystems.some((s) => s.id === app.selectedSubsystem?.id)
    ) {
      return app.selectedSubsystem.id;
    }
    return enabledSubsystems[0]?.id ?? '';
  }, [enabledSubsystems, app.selectedSubsystem]);

  const selectedSubsystem = useMemo(
    () => enabledSubsystems.find((s) => s.id === subsystemId) ?? null,
    [enabledSubsystems, subsystemId],
  );

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

  const selectedCommit = useMemo(
    () => (selectedSha ? (commits.find((c) => c.sha === selectedSha) ?? null) : null),
    [commits, selectedSha],
  );

  const excludedKey = excludedPaths.join('\0');
  const planQuery = useAsyncData<DistributionPlan | null>(
    () =>
      trunkBranch && subsystemId && selectedSha
        ? api.previewDistribution({
            mainBranch: trunkBranch,
            subsystemId,
            commitSha: selectedSha,
            excludedPaths,
          })
        : Promise.resolve(null),
    [trunkBranch, subsystemId, selectedSha, excludedKey],
  );
  const plan = trunkBranch && subsystemId && selectedSha ? planQuery.data : null;
  const planLoading = Boolean(trunkBranch && subsystemId && selectedSha && planQuery.loading);
  const planError = trunkBranch && subsystemId && selectedSha ? planQuery.error : null;

  const diffActive = Boolean(trunkBranch && selectedSha && inspectedPath);
  const diffQuery = useAsyncData<MainCommitDiff | null>(
    () =>
      trunkBranch && selectedSha && diffActive
        ? api.getMainCommitDiff({ targetBranch: trunkBranch, mainCommitSha: selectedSha })
        : Promise.resolve(null),
    [trunkBranch, selectedSha, diffActive],
  );
  const commitDiff = diffActive ? diffQuery.data : null;
  const diffLoading = diffActive && diffQuery.loading;
  const diffError = diffActive ? diffQuery.error : null;

  const includedFiles = useMemo(
    () => (plan ? plan.files.filter((file) => !excludedPaths.includes(file.path)) : []),
    [plan, excludedPaths],
  );

  const hasChanges = plan !== null && (includedFiles.length > 0 || plan.dependencies.length > 0);
  const isBlocked = plan !== null && plan.blockers.length > 0;
  const locked = running;
  const canRun =
    plan !== null && !planLoading && !isBlocked && hasChanges && !running && outcome === null;

  const branchOptions = (targetBranches.data?.branches ?? []).map((branch) => ({
    value: branch.name,
    label: `${branch.name}${branch.isDefault ? ' (default)' : ''}`,
    disabled: !branch.allowed,
    title: branch.allowed ? undefined : (branch.disallowedReason ?? 'Not permitted'),
  }));

  const resetRunState = () => {
    setExcludedPaths([]);
    setInspectedPath(null);
    setProgressSteps([]);
    setRunError(null);
    setOutcome(null);
  };

  const changeTrunkBranch = (next: string) => {
    app.setTargetBranch(next);
    setSelectedSha(null);
    setSearch('');
    setPageNumber(1);
    setMessage('');
    resetRunState();
  };

  const changeSubsystem = (next: string) => {
    app.selectSubsystem(next);
    resetRunState();
  };

  const selectCommit = (sha: string) => {
    if (locked) return;
    setSelectedSha((current) => (current === sha ? null : sha));
    setMessage('');
    resetRunState();
  };

  const refreshCommits = () => {
    setSelectedSha(null);
    setPageNumber(1);
    resetRunState();
    commitQuery.reload();
  };

  const togglePath = (path: string, included: boolean) => {
    setOutcome(null);
    setRunError(null);
    setExcludedPaths((current) =>
      included
        ? current.filter((value) => value !== path)
        : current.includes(path)
          ? current
          : [...current, path],
    );
  };

  const selectAllFiles = () => {
    setOutcome(null);
    setRunError(null);
    setExcludedPaths([]);
  };

  const selectNoFiles = () => {
    if (!plan) return;
    setOutcome(null);
    setRunError(null);
    setExcludedPaths(plan.files.map((f) => f.path));
  };

  const startDistribution = () => {
    if (!canRun || !plan) return;
    const trimmedMessage = message.trim();
    const request = {
      mainBranch: plan.mainBranch,
      subsystemId: plan.subsystemId,
      commitSha: plan.commitSha,
      excludedPaths,
      message: trimmedMessage === '' ? null : trimmedMessage,
    };
    setRunError(null);
    setProgressSteps([]);
    setRunning(true);

    let stopListening: (() => void) | null = null;
    let done = false;
    const finishListening = () => {
      done = true;
      stopListening?.();
      stopListening = null;
    };
    void listen<PromotionProgress>('distribution-progress', (event) => {
      if (done) return;
      setProgressSteps((current) => appendProgressStep(current, event.payload));
    })
      .then((stop) => {
        if (done) stop();
        else stopListening = stop;
      })
      .catch(() => {})
      .then(() => api.runDistribution(request))
      .then((next) => {
        finishListening();
        setOutcome(next);
        setRunning(false);
        app.bumpDataVersion();
      })
      .catch((error: unknown) => {
        finishListening();
        setRunError(asApiError(error));
        setRunning(false);
      });
  };

  const inspectedDiff = useMemo(() => {
    if (!inspectedPath || !commitDiff || commitDiff.mainCommitSha !== selectedSha) return null;
    return commitDiff.files.find((f) => f.path === inspectedPath) ?? null;
  }, [inspectedPath, commitDiff, selectedSha]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-2.5">
        <div className="flex items-center gap-2">
          <h1 className="text-[14px] font-semibold text-fg">Distribute</h1>
          <span className="text-[11px] text-muted">
            One trunk commit → one subsystem pull request
          </span>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-muted">Trunk</span>
            <Select
              ariaLabel="Trunk branch"
              value={trunkBranch}
              onValueChange={changeTrunkBranch}
              options={branchOptions}
              disabled={locked || targetBranches.loading || branchOptions.length === 0}
              className="w-[180px]"
              title={targetBranches.error?.message ?? 'Branch the commit is taken from'}
            />
          </div>
          <SubsystemSelector
            subsystems={enabledSubsystems}
            selectedId={subsystemId || null}
            onSelect={changeSubsystem}
            disabled={locked || app.subsystems.loading || enabledSubsystems.length === 0}
          />
          <Button
            variant="secondary"
            icon={ArrowDownToLine}
            loading={app.pullingCheckouts}
            disabled={locked || app.pullingCheckouts}
            onClick={() => app.pullLatest(subsystemId || null)}
            title="Fetch the latest refs for this subsystem and main"
          >
            {app.pullingCheckouts ? 'Pulling…' : 'Pull latest'}
          </Button>
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

      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        {/* Left: trunk commit picker (single selection) */}
        <section
          aria-label="Trunk commits"
          className="flex min-h-0 min-w-0 flex-1 flex-col border-b border-line bg-surface md:border-b-0"
        >
          <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h2 className="truncate text-xs font-semibold text-fg">Trunk commits</h2>
                {commitsLoading && commits.length > 0 ? (
                  <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted">
                    <Spinner className="h-3 w-3" />
                    Updating…
                  </span>
                ) : null}
              </div>
              <p className="mono truncate text-[11px] text-muted">
                {commits.length > 0 ? `${commits.length} on ${trunkBranch}` : trunkBranch}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              icon={RefreshCw}
              loading={commitsLoading}
              disabled={locked || trunkBranch === '' || commitsLoading}
              onClick={refreshCommits}
            >
              Refresh
            </Button>
          </div>

          {targetBranches.loading && trunkBranch === '' ? (
            <LoadingState label="Loading branches" />
          ) : trunkBranch === '' ? (
            <EmptyState title="No trunk branch available" />
          ) : commitsLoading && commits.length === 0 ? (
            <LoadingState label="Loading trunk commits" />
          ) : commitError ? (
            <ErrorState compact error={commitError} onRetry={refreshCommits} />
          ) : commits.length === 0 ? (
            <EmptyState title="No commits on this branch" />
          ) : (
            <>
              <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
                <div className="relative min-w-[180px] flex-1">
                  <Search
                    className="pointer-events-none absolute top-1/2 left-3 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                    aria-hidden="true"
                  />
                  <input
                    type="search"
                    value={search}
                    onChange={(event) => {
                      setSearch(event.target.value);
                      setPageNumber(1);
                    }}
                    placeholder="Search commits by message, SHA or author…"
                    aria-label="Search commits by message, SHA or author"
                    className="input pl-8"
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
                >
                  {sort === 'newest' ? 'Newest' : 'Oldest'}
                </Button>
              </div>

              {commitsLoading ? (
                <div
                  role="progressbar"
                  aria-label="Refreshing trunk commits"
                  className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
                >
                  <div className="h-full w-1/2 animate-pulse bg-accent" />
                </div>
              ) : null}

              <div
                className={cx(
                  'scroll-area min-h-0 flex-1 transition-opacity',
                  commitsLoading && 'pointer-events-none opacity-55',
                )}
              >
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
                      Commits on trunk branch {trunkBranch}. Select one commit to preview and
                      distribute to the chosen subsystem.
                    </caption>
                    <thead className="sticky top-0 z-10">
                      <tr className="table-head">
                        <th scope="col" className="w-10 py-2 pl-2.5 text-left">
                          <span className="sr-only">Selected</span>
                        </th>
                        <th scope="col" className="w-20 py-2 pr-2.5 text-right">
                          SHA
                        </th>
                        <th scope="col" className="py-2 pr-2.5 text-left">
                          Message
                        </th>
                        <th
                          scope="col"
                          className="hidden w-32 py-2 pr-2.5 text-left 2xl:table-cell"
                        >
                          Author
                        </th>
                        <th
                          scope="col"
                          className="hidden w-24 py-2 pr-2.5 text-right 2xl:table-cell"
                        >
                          Committed
                        </th>
                        <th scope="col" className="w-16 py-2 pr-2.5 text-right">
                          Files
                        </th>
                      </tr>
                    </thead>
                    <tbody>
                      {pagedCommits.map((commit) => (
                        <CommitRow
                          key={commit.sha}
                          commit={commit}
                          selected={selectedSha === commit.sha}
                          locked={locked}
                          onSelect={() => selectCommit(commit.sha)}
                        />
                      ))}
                    </tbody>
                  </table>
                )}
              </div>

              <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-3 py-2 text-[11px] text-muted">
                <span className="truncate">
                  {visibleCommits.length === 0
                    ? 'No commits'
                    : `Showing ${rangeStart}–${rangeEnd} of ${visibleCommits.length}`}
                  {visibleCommits.length === commits.length ? '' : ` (${commits.length} on branch)`}
                </span>
                <div className="flex items-center gap-2">
                  <label className="flex items-center gap-1.5 text-[11px] text-muted">
                    Rows
                    <Select
                      className="w-[72px]"
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
        </section>

        <SplitResizer
          edge="right"
          width={summaryWidth}
          onWidthChange={setSummaryWidth}
          label="Resize distribution panel"
          min={400}
          max={900}
        />

        {/* Right: live plan preview, diff inspection, and PR execution */}
        <section
          aria-label="Distribution plan"
          style={splitWidthStyle(summaryWidth)}
          className="flex min-h-0 flex-col bg-surface md:w-[var(--split-width)] md:shrink-0 md:border-l md:border-line"
        >
          {!selectedSubsystem ? (
            <EmptyState
              icon={GitPullRequestArrow}
              title="No enabled subsystem"
              message="Enable or register a subsystem in Settings before distributing trunk commits."
            />
          ) : !selectedCommit ? (
            <EmptyState
              icon={GitPullRequestArrow}
              title="Select a trunk commit"
              message={`Pick a commit from ${trunkBranch || 'the trunk branch'} to preview what ${selectedSubsystem.name} will receive.`}
            />
          ) : outcome ? (
            <OutcomePanel
              outcome={outcome}
              subsystemName={selectedSubsystem.name}
              progressSteps={progressSteps}
              onReset={() => {
                setOutcome(null);
                setProgressSteps([]);
                planQuery.reload();
              }}
            />
          ) : (
            <>
              <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
                <div className="min-w-0">
                  <h2 className="truncate text-xs font-semibold text-fg">
                    {selectedSubsystem.name}
                  </h2>
                  <p className="mono truncate text-[11px] text-muted">
                    {plan
                      ? `${plan.sourceBranch} ← ${plan.branch}`
                      : `${selectedSubsystem.sourceBranch} · ${selectedCommit.shortSha}`}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  {plan ? <PlanStatusMark blocked={isBlocked} hasChanges={hasChanges} /> : null}
                  <IconButton
                    icon={RefreshCw}
                    label="Refresh preview"
                    disabled={planLoading || running}
                    onClick={planQuery.reload}
                  />
                </div>
              </div>

              {planLoading && plan ? (
                <div
                  role="progressbar"
                  aria-label="Updating distribution plan"
                  className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
                >
                  <div className="h-full w-1/2 animate-pulse bg-accent" />
                </div>
              ) : null}

              <div
                className={cx(
                  'scroll-area min-h-0 flex-1 divide-y divide-line transition-opacity',
                  planLoading && plan && 'pointer-events-none opacity-55',
                )}
              >
                <div className="space-y-1 p-3.5">
                  <dl>
                    <KeyValue label="Commit">
                      <span className="mono text-accent-ink" title={selectedCommit.sha}>
                        {selectedCommit.shortSha}
                      </span>{' '}
                      <span className="text-fg">{selectedCommit.subject}</span>
                    </KeyValue>
                    <KeyValue label="Target PR">
                      <span className="mono">
                        {selectedSubsystem.sourceBranch} ←{' '}
                        {plan?.branch ?? `distribute/${trunkBranch}-${selectedCommit.shortSha}`}
                      </span>
                    </KeyValue>
                  </dl>
                </div>

                {planLoading && !plan ? (
                  <LoadingState label="Building distribution preview" />
                ) : planError ? (
                  <div className="p-3.5">
                    <ErrorState compact error={planError} onRetry={planQuery.reload} />
                  </div>
                ) : plan ? (
                  <>
                    {plan.blockers.length > 0 || plan.warnings.length > 0 ? (
                      <div className="space-y-2 p-3.5">
                        <BlockerList blockers={plan.blockers} />
                        <WarningsList warnings={plan.warnings} />
                      </div>
                    ) : null}

                    <div className="space-y-2 p-3.5">
                      <div className="flex items-center justify-between gap-2">
                        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                          Files ({includedFiles.length}/{plan.files.length})
                        </h3>
                        {plan.files.length > 0 ? (
                          <div className="flex items-center gap-1">
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={locked || isBlocked || excludedPaths.length === 0}
                              onClick={selectAllFiles}
                            >
                              All
                            </Button>
                            <Button
                              variant="ghost"
                              size="sm"
                              disabled={
                                locked || isBlocked || excludedPaths.length === plan.files.length
                              }
                              onClick={selectNoFiles}
                            >
                              None
                            </Button>
                          </div>
                        ) : null}
                      </div>

                      {plan.files.length === 0 ? (
                        <p className="text-[11px] text-muted">
                          No file changes needed for this subsystem.
                        </p>
                      ) : (
                        <ul className="scroll-area max-h-[260px] divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
                          {plan.files.map((file) => {
                            const included = !excludedPaths.includes(file.path);
                            const inspected = inspectedPath === file.path;
                            return (
                              <li
                                key={file.path}
                                onClick={() => setInspectedPath(inspected ? null : file.path)}
                                title="Click to view the trunk commit diff for this file"
                                className={cx(
                                  'row-hover flex h-[29px] cursor-pointer items-center gap-2 px-2.5',
                                  inspected && 'bg-selection',
                                )}
                              >
                                <span
                                  onClick={(event) => event.stopPropagation()}
                                  className="flex shrink-0 items-center"
                                >
                                  <Checkbox
                                    checked={included}
                                    disabled={locked || isBlocked}
                                    onChange={(event) =>
                                      togglePath(file.path, event.target.checked)
                                    }
                                    label={`Include ${file.path}`}
                                  />
                                </span>
                                <ChangeKindIcon change={file.trunkChange} />
                                <PathLabel
                                  path={file.path}
                                  muted={!included}
                                  className="min-w-0 flex-1 text-[11px]"
                                />
                                <ActionBadge action={file.action} />
                              </li>
                            );
                          })}
                        </ul>
                      )}
                    </div>

                    {inspectedPath ? (
                      <div className="space-y-2 p-3.5">
                        <div className="flex items-center justify-between gap-2">
                          <span className="min-w-0 truncate text-[11px] font-medium text-fg">
                            Commit diff: <span className="mono">{inspectedPath}</span>
                          </span>
                          <div className="flex shrink-0 items-center gap-2">
                            <div
                              role="group"
                              aria-label="Diff view mode"
                              className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
                            >
                              {(['unified', 'side'] as const).map((mode, idx) => (
                                <button
                                  key={mode}
                                  type="button"
                                  onClick={() => setDiffMode(mode)}
                                  className={cx(
                                    'px-2 text-[11px] transition-colors',
                                    idx > 0 && 'border-l border-line',
                                    diffMode === mode
                                      ? 'bg-raised font-semibold text-fg'
                                      : 'text-muted hover:bg-hover hover:text-fg',
                                  )}
                                >
                                  {mode === 'unified' ? 'Unified' : 'Split'}
                                </button>
                              ))}
                            </div>
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => setInspectedPath(null)}
                            >
                              Close
                            </Button>
                          </div>
                        </div>
                        <div className="max-h-[420px] overflow-auto rounded-md border border-line bg-surface">
                          <DiffViewer
                            diff={inspectedDiff}
                            loading={diffLoading}
                            error={diffError}
                            onRetry={diffQuery.reload}
                            mode={diffMode}
                            onOpenExternal={(url) => app.openExternal(url)}
                          />
                        </div>
                      </div>
                    ) : null}

                    {plan.skipped.length > 0 ? (
                      <div className="p-3.5">
                        <SkippedList skipped={plan.skipped} />
                      </div>
                    ) : null}

                    {plan.dependencies.length > 0 ? (
                      <div className="space-y-2 p-3.5">
                        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                          Dependencies ({plan.dependencies.length})
                        </h3>
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

                    <div className="space-y-2.5 p-3.5">
                      <Field
                        label="Commit & PR subject"
                        htmlFor="distribute-message"
                        hint="Blank uses the trunk commit's subject."
                      >
                        <input
                          id="distribute-message"
                          type="text"
                          className="input"
                          placeholder={plan.commitSubject}
                          value={message}
                          disabled={locked}
                          onChange={(event) => setMessage(event.target.value)}
                        />
                      </Field>

                      {running || (runError && progressSteps.length > 0) ? (
                        <ExecutionProgressView
                          title="Distribution progress"
                          stages={DISTRIBUTION_STAGES}
                          steps={progressSteps}
                          outcome={running ? 'running' : runError ? 'failed' : 'success'}
                        />
                      ) : null}

                      {runError ? <ErrorState compact error={runError} /> : null}
                    </div>
                  </>
                ) : null}
              </div>
            </>
          )}
        </section>
      </div>

      <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
        <div className="min-w-0 text-[11px] text-muted">
          {running ? (
            <span className="flex items-center gap-1.5 text-fg">
              <Spinner className="h-3 w-3" />
              <span className="truncate">{latestProgress?.label ?? 'Starting distribution…'}</span>
              {latestProgress?.index != null && latestProgress.total != null ? (
                <span className="mono shrink-0 text-muted tabular-nums">
                  ({latestProgress.index}/{latestProgress.total})
                </span>
              ) : null}
            </span>
          ) : outcome ? (
            <span>
              {outcome.ok
                ? outcome.pullRequest?.number != null
                  ? `Pull request #${outcome.pullRequest.number} ${outcome.pullRequest.state} in ${selectedSubsystem?.name ?? 'subsystem'}`
                  : `Branch ${outcome.branch} pushed`
                : 'Distribution blocked'}
            </span>
          ) : !selectedSubsystem ? (
            <span>Enable or register a subsystem in Settings before distributing commits.</span>
          ) : !selectedCommit ? (
            <span>
              Select a commit from <span className="mono">{trunkBranch || 'the trunk branch'}</span>{' '}
              to distribute to {selectedSubsystem.name}.
            </span>
          ) : planLoading && !plan ? (
            <span>Building distribution preview for {selectedSubsystem.name}…</span>
          ) : planError ? (
            <span className="text-status-blocked-ink">
              Preview failed — retry to inspect changes.
            </span>
          ) : plan ? (
            isBlocked ? (
              <span className="text-status-blocked-ink">
                {plan.blockers.length} blocker{plan.blockers.length === 1 ? '' : 's'} — resolve
                before opening PR.
              </span>
            ) : !hasChanges ? (
              <span>No file or dependency changes selected for {selectedSubsystem.name}.</span>
            ) : (
              <span className="mono tabular-nums">
                {selectedCommit.shortSha} → {selectedSubsystem.name} (
                {selectedSubsystem.sourceBranch}) · {includedFiles.length} file
                {includedFiles.length === 1 ? '' : 's'}
                {plan.dependencies.length > 0
                  ? ` · ${plan.dependencies.length} dep${plan.dependencies.length === 1 ? '' : 's'}`
                  : ''}
              </span>
            )
          ) : null}
        </div>
        {outcome ? (
          <div className="flex shrink-0 items-center gap-2">
            <Button
              variant="secondary"
              onClick={() => {
                setOutcome(null);
                planQuery.reload();
              }}
            >
              Done
            </Button>
            {outcome.pullRequest?.url ? (
              <Button
                variant="primary"
                icon={ExternalLink}
                onClick={() => {
                  const prUrl = outcome.pullRequest?.url;
                  if (prUrl) app.openExternal(prUrl);
                }}
              >
                Open Pull Request
              </Button>
            ) : null}
          </div>
        ) : (
          <Button
            variant="primary"
            icon={GitPullRequestArrow}
            onClick={startDistribution}
            disabled={!canRun}
            loading={running}
            title={
              isBlocked
                ? 'Resolve blockers before distributing'
                : !hasChanges
                  ? 'Select at least one file or dependency change'
                  : undefined
            }
          >
            {selectedSubsystem
              ? `Open Pull Request in ${selectedSubsystem.name}`
              : 'Open Pull Request'}
          </Button>
        )}
      </footer>
    </div>
  );
}

function PlanStatusMark({ blocked, hasChanges }: { blocked: boolean; hasChanges: boolean }) {
  const dot = blocked
    ? 'bg-status-blocked'
    : hasChanges
      ? 'bg-status-eligible'
      : 'bg-status-superseded';
  const label = blocked ? 'Blocked' : hasChanges ? 'Ready' : 'No changes';
  return (
    <span className="flex items-center gap-1.5 text-[11px] text-muted">
      <span className={cx('h-1.5 w-1.5 rounded-full', dot)} aria-hidden="true" />
      {label}
    </span>
  );
}

function ActionBadge({ action }: { action: DistributionPathAction }) {
  const styles: Record<DistributionPathAction, string> = {
    add: 'text-status-applied-ink',
    update: 'text-accent-ink',
    delete: 'text-status-blocked-ink',
  };
  return (
    <span className={cx('mono shrink-0 text-[10px] uppercase', styles[action])}>{action}</span>
  );
}

function CommitRow({
  commit,
  selected,
  locked,
  onSelect,
}: {
  commit: TrunkCommit;
  selected: boolean;
  locked: boolean;
  onSelect: () => void;
}) {
  return (
    <tr
      onClick={locked ? undefined : onSelect}
      aria-selected={selected}
      className={cx(
        'row-hover h-[48px] border-b border-line',
        !locked && 'cursor-pointer',
        selected && 'bg-selection',
      )}
    >
      <td className="py-2 pl-2.5 align-top">
        <span onClick={(event) => event.stopPropagation()}>
          <Checkbox
            checked={selected}
            disabled={locked}
            onChange={onSelect}
            label={`Select commit ${commit.shortSha} – ${commit.subject}`}
          />
        </span>
      </td>
      <td className="py-2 pr-2.5 text-right align-top">
        <span className="mono text-[11px] text-accent-ink" title={commit.sha}>
          {commit.shortSha}
        </span>
      </td>
      <td className="max-w-[46ch] py-2 pr-2.5 align-top">
        <div className="flex items-center gap-1.5">
          {commit.isMerge ? (
            <GitMerge className="h-3 w-3 shrink-0 text-muted" aria-label="Merge commit" />
          ) : null}
          <span className="block truncate text-xs text-fg" title={commit.subject}>
            {commit.subject}
          </span>
        </div>
        <p
          className="mt-0.5 truncate text-[11px] text-muted"
          title={`${commit.authorName} · ${formatDateTime(commit.committedAt)}`}
        >
          {commit.authorName} · {formatDateTime(commit.committedAt)}
        </p>
      </td>
      <td className="hidden max-w-32 py-2 pr-2.5 align-top text-xs text-muted 2xl:table-cell">
        <span className="block truncate">{commit.authorName}</span>
      </td>
      <td className="mono hidden py-2 pr-2.5 text-right align-top text-[11px] text-muted tabular-nums 2xl:table-cell">
        {formatDate(commit.committedAt)}
      </td>
      <td className="py-2 pr-2.5 text-right align-top">
        <span
          className="text-[11px] text-muted tabular-nums"
          title={`${commit.filesChanged} files changed`}
        >
          {commit.filesChanged}
        </span>
      </td>
    </tr>
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
        {skipped.length} path{skipped.length === 1 ? '' : 's'} skipped
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
    <li className="flex h-[27px] items-center gap-2 px-2.5 text-[11px]">
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

function OutcomePathList({ title, paths }: { title: string; paths: string[] }) {
  if (paths.length === 0) return null;
  return (
    <details className="group overflow-hidden rounded-md border border-line bg-surface">
      <summary className="flex h-[29px] cursor-pointer items-center justify-between gap-2 px-2.5 text-[11px] select-none hover:bg-hover">
        <span className="flex items-center gap-1.5 font-semibold tracking-wide text-muted uppercase">
          <ChevronRight
            className="h-3.5 w-3.5 text-disabled transition-transform group-open:rotate-90"
            aria-hidden="true"
          />
          {title}
        </span>
        <span className="mono text-muted tabular-nums">
          {paths.length} path{paths.length === 1 ? '' : 's'}
        </span>
      </summary>
      <ul className="max-h-40 divide-y divide-line overflow-auto border-t border-line">
        {paths.map((path) => (
          <li key={path} className="flex h-[26px] items-center px-2.5">
            <PathLabel path={path} className="min-w-0 flex-1 text-[11px]" />
          </li>
        ))}
      </ul>
    </details>
  );
}

function OutcomePanel({
  outcome,
  subsystemName,
  progressSteps,
  onReset,
}: {
  outcome: DistributionOutcome;
  subsystemName: string;
  progressSteps: PromotionProgress[];
  onReset: () => void;
}) {
  const pullRequest = outcome.pullRequest;
  const commitSha = outcome.commitSha;
  const statusBadge = !outcome.ok
    ? 'Blocked'
    : pullRequest?.number != null
      ? `PR #${pullRequest.number} ${pullRequest.state}`
      : 'Branch pushed';

  return (
    <>
      <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
        <div className="min-w-0">
          <h2 className="truncate text-xs font-semibold text-fg">{subsystemName}</h2>
          <p className="truncate text-[11px] text-muted">{statusBadge}</p>
        </div>
        <Button variant="ghost" size="sm" onClick={onReset}>
          Done
        </Button>
      </div>
      <div className="scroll-area min-h-0 flex-1 space-y-4 p-3.5">
        <div
          className={cx(
            'flex items-start gap-2.5 rounded-md border border-l-[3px] px-3.5 py-3',
            outcome.ok
              ? 'border-status-applied/40 border-l-status-applied bg-status-applied-soft'
              : 'border-status-blocked/40 border-l-status-blocked bg-status-blocked-soft',
          )}
        >
          {outcome.ok ? (
            <CheckCircle2
              className="mt-0.5 h-4 w-4 shrink-0 text-status-applied-ink"
              aria-hidden="true"
            />
          ) : (
            <TriangleAlert
              className="mt-0.5 h-4 w-4 shrink-0 text-status-blocked-ink"
              aria-hidden="true"
            />
          )}
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-2">
              <p
                className={cx(
                  'text-xs font-semibold',
                  outcome.ok ? 'text-status-applied-ink' : 'text-status-blocked-ink',
                )}
              >
                {outcome.ok ? `Distributed to ${subsystemName}` : 'Distribution blocked'}
              </p>
              <span
                className={cx(
                  'mono rounded border px-1.5 py-0.5 text-[10px] font-semibold tracking-wide uppercase',
                  outcome.ok
                    ? 'border-status-applied/40 bg-status-applied/15 text-status-applied-ink'
                    : 'border-status-blocked/40 bg-status-blocked/15 text-status-blocked-ink',
                )}
              >
                {statusBadge}
              </span>
            </div>
            {outcome.message ? (
              <p className="mt-1 text-[11px] leading-relaxed text-fg/85">{outcome.message}</p>
            ) : null}
          </div>
        </div>

        <div className="grid grid-cols-3 divide-x divide-line overflow-hidden rounded-md border border-line bg-surface">
          {[
            { label: 'Written', value: outcome.applied.length, tone: 'text-status-applied-ink' },
            { label: 'Deleted', value: outcome.deleted.length, tone: 'text-status-blocked-ink' },
            { label: 'Deps', value: outcome.dependencies.length, tone: 'text-accent-ink' },
          ].map((stat) => (
            <div key={stat.label} className="px-3 py-2">
              <p className="text-[10px] font-medium text-muted">{stat.label}</p>
              <p
                className={cx(
                  'mono mt-0.5 text-sm font-semibold tabular-nums',
                  stat.value > 0 ? stat.tone : 'text-disabled',
                )}
              >
                {stat.value}
              </p>
            </div>
          ))}
        </div>

        <div className="space-y-1.5">
          <OutcomePathList title="Files written" paths={outcome.applied} />
          <OutcomePathList title="Files deleted" paths={outcome.deleted} />
        </div>

        <BlockerList blockers={outcome.blockers} />
        <WarningsList warnings={outcome.warnings} />

        <dl className="divide-y divide-line rounded-md border border-line bg-surface px-3">
          <KeyValue label="Source commit">
            <span className="mono">{outcome.sourceCommitSha.slice(0, 7)}</span>
          </KeyValue>
          <KeyValue label="Branch">
            <span className="mono">{outcome.branch}</span>
          </KeyValue>
          <KeyValue label="New commit">
            {commitSha ? <span className="mono">{commitSha.slice(0, 7)}</span> : '—'}
          </KeyValue>
          {pullRequest ? (
            <KeyValue label="Target">
              <span className="mono">
                {pullRequest.base} ← {pullRequest.head}
              </span>
            </KeyValue>
          ) : null}
        </dl>

        {outcome.dependencies.length > 0 ? (
          <div className="space-y-1.5">
            <span className="text-[11px] font-semibold tracking-wide text-muted uppercase">
              Dependencies updated
            </span>
            <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
              {outcome.dependencies.map((dependency) => (
                <DependencyRow
                  key={`${dependency.section}:${dependency.name}`}
                  dependency={dependency}
                />
              ))}
            </ul>
          </div>
        ) : null}

        <ExecutionLogDisclosure
          stages={DISTRIBUTION_STAGES}
          steps={progressSteps}
          outcome={outcome.ok ? 'success' : 'failed'}
        />
      </div>
    </>
  );
}

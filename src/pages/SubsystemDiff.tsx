import { useMemo, useState } from 'react';
import {
  ArrowDownToLine,
  FileText,
  GitPullRequestArrow,
  RefreshCw,
  Search,
  TriangleAlert,
} from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import type {
  CommitSummary,
  ReconcileFileAction,
  SubsystemDiffBucket,
  SubsystemDiffCommit,
  SubsystemDiffEntry,
  SubsystemSummary,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { resolveDiffSelection } from '../components/diffResolve';
import { ChangeKindIcon } from '../components/ChangeKindIcon';
import { DiffViewer } from '../components/DiffViewer';
import { PathLabel } from '../components/PathLabel';
import { RevertsNewerLine } from '../components/RevertsNewer';
import { revertsNewerText } from '../components/revertsNewerText';
import { CommitStatusBadge } from '../components/StatusBadge';
import { SubsystemSelector } from '../components/SubsystemSelector';
import { cx } from '../components/ui/cx';
import { formatDate, formatDateTime } from '../components/ui/format';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  InlineAlert,
  LoadingState,
  Select,
  Spinner,
} from '../components/ui/primitives';
import { splitWidthStyle } from '../components/ui/split-pane';
import { SplitResizer } from '../components/ui/split-resizer';

/**
 * Subsystem Diff: the subsystem's tracked tree against the target branch tip,
 * so a partial promotion stops being invisible. One batched backend call per
 * subsystem/branch; clicking a file renders its history from the rows already
 * carried (zero extra spawns). Selection resolves to the union of unpromoted
 * touching commits with the exclusions pre-filled, and hands off to the
 * existing promotion dialog — the same dialog, the same `PromotionRequest`,
 * the same `excludedPaths`.
 */

const HISTORY_LIMIT = 20;

function diffActionFor(entry: SubsystemDiffEntry): ReconcileFileAction {
  if (entry.bucket === 'missing' || entry.change === 'added') return 'add';
  if (entry.change === 'deleted') return 'delete';
  return 'overwrite';
}

const BUCKET_META: Record<SubsystemDiffBucket, { label: string; dot: string }> = {
  missing: { label: 'Missing', dot: 'bg-status-blocked' },
  behind: { label: 'Behind', dot: 'bg-status-eligible' },
  main_side_changed: { label: 'Main-side changed', dot: 'bg-status-unknown' },
  identical: { label: 'Identical', dot: 'bg-status-superseded' },
};

export function BucketMark({
  bucket,
  className = '',
}: {
  bucket: SubsystemDiffBucket;
  className?: string;
}) {
  const meta = BUCKET_META[bucket];
  return (
    <span
      className={`inline-flex shrink-0 items-center gap-1 text-[11px] whitespace-nowrap ${className}`}
      title={
        bucket === 'missing'
          ? 'In the subsystem, absent in main — never promoted.'
          : bucket === 'behind'
            ? "Main's copy is an ancestor version — safe to sync."
            : bucket === 'main_side_changed'
              ? "Main's copy was edited independently — no subsystem commit produced it."
              : 'Same content on both sides.'
      }
    >
      <span aria-hidden="true" className={`h-1.5 w-1.5 shrink-0 rounded-full ${meta.dot}`} />
      {meta.label}
    </span>
  );
}

export function SubsystemDiff({ embedded = false }: { embedded?: boolean } = {}) {
  const app = useApp();
  const subsystem = app.selectedSubsystem;

  if (!subsystem) {
    return (
      <div className="flex h-full min-h-0 flex-1 items-center justify-center p-6">
        {app.subsystems.loading ? (
          <LoadingState label="Loading the subsystem registry…" />
        ) : (
          <EmptyState
            icon={FileText}
            title="Select a subsystem first"
            message="Pick a subsystem to inspect its file drift against the target branch."
          />
        )}
      </div>
    );
  }

  return (
    <SubsystemDiffView
      key={`${subsystem.id}:${app.targetBranch}`}
      subsystem={subsystem}
      embedded={embedded}
    />
  );
}

function SubsystemDiffView({
  subsystem,
  embedded,
}: {
  subsystem: SubsystemSummary;
  embedded: boolean;
}) {
  const app = useApp();
  const [splitWidth, setSplitWidth] = useState(560);
  const [filter, setFilter] = useState('');
  const [bucket, setBucket] = useState<SubsystemDiffBucket | 'all'>('all');
  const [hideIdentical, setHideIdentical] = useState(true);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [focused, setFocused] = useState<string | null>(null);
  const [detailTab, setDetailTab] = useState<'diff' | 'history'>('diff');
  const [diffMode, setDiffMode] = useState<'unified' | 'side'>('unified');
  const [ticked, setTicked] = useState<ReadonlySet<string>>(new Set());
  const [historyAll, setHistoryAll] = useState(false);
  const [resolving, setResolving] = useState(false);

  const diff = useAsyncData(
    () => api.getSubsystemDiff(subsystem.id, app.targetBranch),
    [subsystem.id, app.targetBranch, app.dataVersion],
  );
  const entries = useMemo(() => diff.data?.entries ?? [], [diff.data]);
  const byPath = useMemo(() => new Map(entries.map((entry) => [entry.path, entry])), [entries]);

  const counts = useMemo(() => {
    const tally: Record<SubsystemDiffBucket, number> = {
      missing: 0,
      behind: 0,
      main_side_changed: 0,
      identical: 0,
    };
    for (const entry of entries) tally[entry.bucket] += 1;
    return tally;
  }, [entries]);

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return entries.filter((entry) => {
      if (hideIdentical && entry.bucket === 'identical') return false;
      if (bucket !== 'all' && entry.bucket !== bucket) return false;
      if (needle && !entry.path.toLowerCase().includes(needle)) return false;
      return true;
    });
  }, [entries, filter, bucket, hideIdentical]);

  const resolution = useMemo(
    () => resolveDiffSelection(entries, selected, ticked),
    [entries, selected, ticked],
  );
  const union = useMemo(() => new Set(resolution.commits), [resolution]);
  const focusedEntry = (focused ? byPath.get(focused) : undefined) ?? null;
  const fileDiff = useAsyncData(
    () =>
      focusedEntry && focusedEntry.bucket !== 'identical'
        ? api.getReconcileFileDiff(
            subsystem.id,
            app.targetBranch,
            'subsystem_to_main',
            focusedEntry.path,
            diffActionFor(focusedEntry),
          )
        : Promise.resolve(null),
    [
      subsystem.id,
      app.targetBranch,
      focusedEntry?.path,
      focusedEntry?.bucket,
      focusedEntry?.change,
      app.dataVersion,
    ],
  );
  const currentFileDiff =
    focusedEntry && fileDiff.data?.path === focusedEntry.path ? fileDiff.data : null;
  const selectedReverts = useMemo(
    () => visible.filter((entry) => selected.has(entry.path) && entry.revertsNewer),
    [visible, selected],
  );

  const togglePath = (path: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const toggleTick = (sha: string) => {
    setTicked((current) => {
      const next = new Set(current);
      if (next.has(sha)) next.delete(sha);
      else next.add(sha);
      return next;
    });
  };

  const focusPath = (path: string) => {
    setFocused(path);
    setHistoryAll(false);
  };

  const selectAllMissingBehind = () => {
    setSelected(
      new Set(
        entries
          .filter((entry) => entry.bucket === 'missing' || entry.bucket === 'behind')
          .map((entry) => entry.path),
      ),
    );
  };

  const continueToPromotion = () => {
    if (resolution.commits.length === 0 || resolving) return;
    setResolving(true);
    const shas = resolution.commits;
    void api
      .getSubsystemCommits({
        subsystemId: subsystem.id,
        targetBranch: app.targetBranch,
        page: 1,
        pageSize: Math.min(200, Math.max(5, shas.length)),
        search: '',
        statusFilter: null,
        sort: 'newest',
        shas,
      })
      .then((page) => {
        const fetched = new Map(page.commits.map((commit) => [commit.sha, commit]));
        const ordered = shas
          .map((sha) => fetched.get(sha))
          .filter((commit): commit is CommitSummary => Boolean(commit));
        app.openPromotionDialog(shas, ordered, { excludedPaths: resolution.excludedPaths });
      })
      .catch((error: unknown) => {
        app.pushNotice({
          tone: 'danger',
          title: 'Could not resolve the promotion selection',
          message: asApiError(error).message,
        });
      })
      .finally(() => setResolving(false));
  };

  const branches = (app.targetBranches.data?.branches ?? []).filter((branch) => branch.allowed);
  const continueHint =
    resolution.commits.length === 0
      ? selected.size === 0 && ticked.size === 0
        ? 'Tick files — or commits in the file detail — to resolve a promotion.'
        : 'Every touched commit is already promoted on this branch — nothing left to promote.'
      : `${resolution.commits.length} commit${resolution.commits.length === 1 ? '' : 's'} · ${resolution.excludedPaths.length} pre-excluded`;

  const subsystems = app.subsystems.data ?? [];

  return (
    <div className="flex h-full min-h-0 flex-col">
      {!embedded ? (
        <header className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-2.5">
          <div className="min-w-0">
            <h1 className="text-[14px] font-semibold text-fg">Subsystem Diff</h1>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <SubsystemSelector
              subsystems={subsystems}
              selectedId={subsystem.id}
              onSelect={(next) => app.selectSubsystem(next)}
            />
            <div className="flex items-center gap-1.5">
              <span className="text-[11px] text-muted">Target</span>
              <Select
                ariaLabel="Authorized target main-system branch"
                className="w-[180px]"
                value={app.targetBranch}
                onValueChange={(next) => app.setTargetBranch(next)}
                options={branches.map((branch) => ({
                  value: branch.name,
                  label: branch.name,
                }))}
              />
            </div>
            <Button
              variant="secondary"
              icon={ArrowDownToLine}
              loading={app.pullingCheckouts}
              disabled={app.pullingCheckouts}
              onClick={() => app.pullLatest(subsystem.id)}
              title="Fetch the latest refs for this subsystem and main"
            >
              {app.pullingCheckouts ? 'Pulling…' : 'Pull latest'}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              icon={RefreshCw}
              loading={diff.loading}
              disabled={diff.loading}
              onClick={diff.reload}
              title="Recompute file diff against target branch"
            >
              Refresh
            </Button>
          </div>
        </header>
      ) : null}

      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <section
          aria-label="Files differing from target"
          style={splitWidthStyle(splitWidth)}
          className="flex min-h-0 flex-col border-b border-line bg-surface md:w-[var(--split-width)] md:shrink-0 md:border-r md:border-b-0"
        >
          <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h2 className="truncate text-xs font-semibold text-fg">
                  Files differing from target
                </h2>
                {diff.loading && entries.length > 0 ? (
                  <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted">
                    <Spinner className="h-3 w-3" />
                    Updating…
                  </span>
                ) : null}
              </div>
              <p className="mono truncate text-[11px] text-muted">
                {subsystem.repository}@{subsystem.sourceBranch} · target {app.targetBranch}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <Button
                variant="ghost"
                size="sm"
                onClick={selectAllMissingBehind}
                disabled={diff.loading && entries.length === 0}
              >
                Select missing + behind
              </Button>
              <Button
                variant="ghost"
                size="sm"
                disabled={selected.size === 0 && ticked.size === 0}
                onClick={() => {
                  setSelected(new Set());
                  setTicked(new Set());
                }}
              >
                Clear
              </Button>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
            <div className="relative min-w-[160px] flex-1">
              <Search
                className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                aria-hidden="true"
              />
              <input
                className="input mono h-[25px] pl-8 text-[11px]"
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
                placeholder="Filter paths…"
                aria-label="Filter paths"
                spellCheck={false}
                autoComplete="off"
              />
            </div>
            <div
              role="group"
              aria-label="Filter by diff bucket"
              className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
            >
              {(
                [
                  { id: 'all', label: 'All', count: entries.length },
                  { id: 'missing', label: 'Missing', count: counts.missing },
                  { id: 'behind', label: 'Behind', count: counts.behind },
                  { id: 'main_side_changed', label: 'Main-side', count: counts.main_side_changed },
                  { id: 'identical', label: 'Identical', count: counts.identical },
                ] as const
              ).map((item, idx) => (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => {
                    setBucket(item.id);
                    if (item.id === 'identical') setHideIdentical(false);
                  }}
                  className={cx(
                    'px-2 text-[11px] tabular-nums transition-colors',
                    idx > 0 && 'border-l border-line',
                    bucket === item.id
                      ? 'bg-raised font-semibold text-fg'
                      : 'text-muted hover:bg-hover hover:text-fg',
                  )}
                >
                  {item.label} ({item.count})
                </button>
              ))}
            </div>
            <label className="flex items-center gap-1.5 text-[11px] text-muted">
              <Checkbox
                checked={hideIdentical}
                onChange={(event) => setHideIdentical(event.target.checked)}
                label="Hide identical files"
              />
              <span>Hide identical</span>
            </label>
          </div>

          {diff.loading && entries.length > 0 ? (
            <div
              role="progressbar"
              aria-label="Refreshing subsystem diff"
              className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
            >
              <div className="h-full w-1/2 animate-pulse bg-accent" />
            </div>
          ) : null}

          {diff.loading && entries.length === 0 ? (
            <LoadingState label="Comparing the subsystem tree against the target…" />
          ) : diff.error ? (
            <ErrorState error={diff.error} onRetry={diff.reload} />
          ) : visible.length === 0 ? (
            <EmptyState
              icon={FileText}
              title={entries.length === 0 ? 'No differences' : 'No files match'}
              message={
                entries.length === 0
                  ? 'Every subsystem file matches the target branch.'
                  : 'Loosen the filter or show identical files.'
              }
            />
          ) : (
            <div
              className={cx(
                'scroll-area min-h-0 flex-1 transition-opacity',
                diff.loading && entries.length > 0 && 'pointer-events-none opacity-55',
              )}
            >
              <div className="table-head sticky top-0 z-10 flex h-[25px] items-center gap-2 border-b border-line px-2.5">
                <span className="w-3.5 shrink-0" aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate">File</span>
                <span className="w-[110px] shrink-0">Bucket</span>
                <span className="w-[70px] shrink-0">Change</span>
                <span className="w-[220px] shrink-0">Latest</span>
              </div>
              <ul className="divide-y divide-line">
                {visible.map((entry) => {
                  const latest = entry.commits[0] ?? null;
                  const isFocused = focused === entry.path;
                  return (
                    <li
                      key={entry.path}
                      role="button"
                      tabIndex={0}
                      onClick={() => focusPath(entry.path)}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter' || event.key === ' ') {
                          event.preventDefault();
                          focusPath(entry.path);
                        }
                      }}
                      title={entry.path}
                      className={cx(
                        'row-hover flex h-[29px] cursor-pointer items-center gap-2 px-2.5 focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-accent',
                        isFocused && 'row-selected',
                      )}
                    >
                      <span
                        onClick={(event) => event.stopPropagation()}
                        onKeyDown={(event) => event.stopPropagation()}
                        className="flex shrink-0 items-center"
                      >
                        <Checkbox
                          checked={selected.has(entry.path)}
                          onChange={() => togglePath(entry.path)}
                          label={`Select ${entry.path} for promotion`}
                        />
                      </span>
                      <div className="min-w-0 flex-1">
                        <PathLabel path={entry.path} />
                      </div>
                      {entry.revertsNewer ? (
                        <span title={revertsNewerText(entry.revertsNewer)}>
                          <TriangleAlert
                            className="h-3 w-3 shrink-0 text-status-unknown"
                            aria-hidden="true"
                          />
                        </span>
                      ) : null}
                      <span className="w-[110px] shrink-0">
                        <BucketMark bucket={entry.bucket} />
                      </span>
                      <span className="flex w-[70px] shrink-0 items-center">
                        <ChangeKindIcon change={entry.change} />
                      </span>
                      <span
                        className="flex w-[220px] min-w-0 shrink-0 items-center gap-1.5"
                        title={
                          latest
                            ? `${latest.sha} "${latest.subject}" — ${latest.authorName} · ${formatDateTime(latest.committedAt)}`
                            : 'No touches in the recent source history'
                        }
                      >
                        {latest ? (
                          <>
                            <span className="min-w-0 flex-1 truncate text-[11px] text-fg">
                              {latest.subject}
                            </span>
                            <span className="mono shrink-0 text-[11px] text-muted tabular-nums">
                              {formatDate(latest.committedAt)}
                            </span>
                          </>
                        ) : (
                          <span className="text-[11px] text-muted">—</span>
                        )}
                      </span>
                    </li>
                  );
                })}
              </ul>
            </div>
          )}
        </section>

        <SplitResizer
          width={splitWidth}
          onWidthChange={setSplitWidth}
          label="Resize subsystem diff panels"
          min={380}
          max={900}
        />

        <section
          aria-label="File diff and detail"
          className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface"
        >
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
            <div className="flex min-w-0 flex-1 items-center gap-2">
              {focusedEntry ? (
                <>
                  <ChangeKindIcon change={focusedEntry.change} />
                  <PathLabel path={focusedEntry.path} className="min-w-0 flex-1" />
                  <BucketMark bucket={focusedEntry.bucket} />
                </>
              ) : (
                <div className="min-w-0">
                  <h2 className="truncate text-xs font-semibold text-fg">File diff</h2>
                  <p className="truncate text-[11px] text-muted">
                    Click a file on the left to view its diff
                  </p>
                </div>
              )}
            </div>

            <div className="flex shrink-0 items-center gap-2">
              <div
                role="group"
                aria-label="Detail view"
                className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
              >
                {(
                  [
                    { id: 'diff', label: 'Diff' },
                    {
                      id: 'history',
                      label: focusedEntry ? `History (${focusedEntry.commits.length})` : 'History',
                    },
                  ] as const
                ).map((item, idx) => (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setDetailTab(item.id)}
                    className={cx(
                      'px-2 text-[11px] tabular-nums transition-colors',
                      idx > 0 && 'border-l border-line',
                      detailTab === item.id
                        ? 'bg-raised font-semibold text-fg'
                        : 'text-muted hover:bg-hover hover:text-fg',
                    )}
                  >
                    {item.label}
                  </button>
                ))}
              </div>

              {detailTab === 'diff' && focusedEntry && focusedEntry.bucket !== 'identical' ? (
                <div
                  role="group"
                  aria-label="Diff view mode"
                  className="flex h-[25px] overflow-hidden rounded-md border border-line bg-surface"
                >
                  {(
                    [
                      { id: 'unified', label: 'Unified' },
                      { id: 'side', label: 'Split' },
                    ] as const
                  ).map((item, idx) => (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => setDiffMode(item.id)}
                      className={cx(
                        'px-2 text-[11px] transition-colors',
                        idx > 0 && 'border-l border-line',
                        diffMode === item.id
                          ? 'bg-raised font-semibold text-fg'
                          : 'text-muted hover:bg-hover hover:text-fg',
                      )}
                    >
                      {item.label}
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          </div>

          {!focusedEntry ? (
            <EmptyState
              icon={FileText}
              title="No file selected"
              message="Click any file row on the left to inspect its diff against the target branch."
            />
          ) : detailTab === 'diff' ? (
            <div className="flex min-h-0 flex-1 flex-col">
              {fileDiff.loading && currentFileDiff ? (
                <div
                  role="progressbar"
                  aria-label="Refreshing file diff"
                  className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
                >
                  <div className="h-full w-1/2 animate-pulse bg-accent" />
                </div>
              ) : null}

              <div
                className={cx(
                  'scroll-area min-h-0 flex-1 transition-opacity',
                  fileDiff.loading && currentFileDiff && 'opacity-55',
                )}
              >
                {focusedEntry.revertsNewer ? (
                  <div className="border-b border-line p-2.5">
                    <InlineAlert tone="warning" title="Reverts newer work">
                      <RevertsNewerLine warn={focusedEntry.revertsNewer} />
                    </InlineAlert>
                  </div>
                ) : null}

                {focusedEntry.bucket === 'identical' ? (
                  <EmptyState
                    icon={FileText}
                    title="No content differences"
                    message="The subsystem copy and target branch copy are identical."
                  />
                ) : (
                  <DiffViewer
                    diff={
                      currentFileDiff
                        ? {
                            language: currentFileDiff.language,
                            hunks: currentFileDiff.hunks,
                            binary: currentFileDiff.binary,
                            truncated: currentFileDiff.truncated,
                            sourceUrl: '',
                            note: currentFileDiff.note,
                          }
                        : null
                    }
                    loading={fileDiff.loading}
                    error={fileDiff.error}
                    onRetry={fileDiff.reload}
                    mode={diffMode}
                    onOpenExternal={app.openExternal}
                  />
                )}
              </div>
            </div>
          ) : (
            <div className="scroll-area min-h-0 flex-1">
              <div className="p-3.5">
                <FileDetail
                  key={focusedEntry.path}
                  entry={focusedEntry}
                  union={union}
                  ticked={ticked}
                  historyAll={historyAll}
                  onHistoryAllChange={setHistoryAll}
                  onToggleTick={toggleTick}
                />
              </div>

              {resolution.mainSideSelected.length > 0 ||
              selectedReverts.length > 0 ||
              resolution.commits.length > 0 ||
              resolution.excludedPaths.length > 0 ? (
                <div className="space-y-2.5 border-t border-line p-3.5">
                  {resolution.mainSideSelected.length > 0 ? (
                    <InlineAlert
                      tone="warning"
                      title={`${resolution.mainSideSelected.length} selected path${resolution.mainSideSelected.length === 1 ? ' was' : 's were'} changed in main only`}
                    >
                      <p>
                        No subsystem commit produced main&apos;s copy — the resolved commits are the
                        subsystem&apos;s own versions, and promoting overwrites main&apos;s copy
                        (recorded as replaced).
                      </p>
                    </InlineAlert>
                  ) : null}

                  {selectedReverts.length > 0 ? (
                    <InlineAlert
                      tone="warning"
                      title={`Reverts newer work — ${selectedReverts.length} selected path${selectedReverts.length === 1 ? '' : 's'}`}
                    >
                      <div className="space-y-1.5">
                        {selectedReverts.map((entry) =>
                          entry.revertsNewer ? (
                            <div key={entry.path}>
                              <PathLabel path={entry.path} muted />
                              <RevertsNewerLine warn={entry.revertsNewer} />
                            </div>
                          ) : null,
                        )}
                      </div>
                    </InlineAlert>
                  ) : null}

                  {resolution.commits.length > 0 ? (
                    <details className="overflow-hidden rounded-md border border-line bg-surface">
                      <summary className="flex h-[29px] cursor-pointer items-center justify-between gap-2 px-2.5 text-[11px]">
                        <span className="font-semibold tracking-wide text-muted uppercase">
                          Resolved commits
                        </span>
                        <span className="mono text-muted tabular-nums">
                          {resolution.commits.length}
                        </span>
                      </summary>
                      <ul className="max-h-32 divide-y divide-line overflow-y-auto border-t border-line">
                        {resolution.commits.map((sha) => (
                          <li
                            key={sha}
                            className="mono flex h-[29px] items-center truncate px-2.5 text-[11px] text-muted tabular-nums"
                            title={sha}
                          >
                            <span className="truncate">{sha.slice(0, 7)}</span>
                          </li>
                        ))}
                      </ul>
                    </details>
                  ) : null}

                  {resolution.excludedPaths.length > 0 ? (
                    <>
                      <details className="overflow-hidden rounded-md border border-line bg-surface">
                        <summary className="flex h-[29px] cursor-pointer items-center justify-between gap-2 px-2.5 text-[11px]">
                          <span className="font-semibold tracking-wide text-muted uppercase">
                            Pre-excluded
                          </span>
                          <span className="mono text-muted tabular-nums">
                            {resolution.excludedPaths.length}
                          </span>
                        </summary>
                        <ul className="max-h-32 divide-y divide-line overflow-y-auto border-t border-line">
                          {resolution.excludedPaths.map((path) => (
                            <li key={path} className="flex h-[29px] items-center px-2.5">
                              <PathLabel path={path} className="flex-1 text-[11px]" muted />
                            </li>
                          ))}
                        </ul>
                      </details>
                      <p className="text-[11px] leading-relaxed text-muted">
                        Adjustable in the confirmation dialog — uncheck any path to include it.
                      </p>
                    </>
                  ) : null}
                </div>
              ) : null}
            </div>
          )}
        </section>
      </div>

      <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
        <div className="min-w-0 text-[11px] text-muted">
          {resolution.commits.length === 0 ? (
            <span>{continueHint}</span>
          ) : (
            <span className="mono tabular-nums">
              {selected.size} file{selected.size === 1 ? '' : 's'} · {resolution.commits.length}{' '}
              commit{resolution.commits.length === 1 ? '' : 's'} · {resolution.excludedPaths.length}{' '}
              pre-excluded
            </span>
          )}
        </div>
        <Button
          variant="primary"
          icon={GitPullRequestArrow}
          loading={resolving}
          disabled={resolution.commits.length === 0 || resolving}
          onClick={continueToPromotion}
        >
          {resolving ? 'Resolving…' : 'Continue to promotion'}
        </Button>
      </footer>
    </div>
  );
}

function FileDetail({
  entry,
  union,
  ticked,
  historyAll,
  onHistoryAllChange,
  onToggleTick,
}: {
  entry: SubsystemDiffEntry;
  union: ReadonlySet<string>;
  ticked: ReadonlySet<string>;
  historyAll: boolean;
  onHistoryAllChange: (all: boolean) => void;
  onToggleTick: (sha: string) => void;
}) {
  const latest: SubsystemDiffCommit | null = entry.commits[0] ?? null;
  const shown = historyAll ? entry.commits : entry.commits.slice(0, HISTORY_LIMIT);

  return (
    <div className="space-y-3">
      <div className="min-w-0">
        <PathLabel path={entry.path} />
        <p className="mt-1 flex items-center gap-2">
          <BucketMark bucket={entry.bucket} />
          <ChangeKindIcon change={entry.change} />
        </p>
      </div>

      {entry.revertsNewer ? (
        <InlineAlert tone="warning" title="Reverts newer work">
          <RevertsNewerLine warn={entry.revertsNewer} />
        </InlineAlert>
      ) : null}

      <section className="space-y-1.5">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">Latest</h3>
        {latest ? (
          <div className="rounded-md border border-line bg-surface px-2.5 py-2">
            <p className="flex items-center gap-2">
              <span className="mono shrink-0 text-[11px] text-accent-ink" title={latest.sha}>
                {latest.shortSha}
              </span>
              <span
                className="min-w-0 flex-1 truncate text-xs font-medium text-fg"
                title={latest.subject}
              >
                {latest.subject}
              </span>
            </p>
            <p className="meta mt-1">
              {latest.authorName} · {formatDateTime(latest.committedAt)}
            </p>
            <p className="mt-1">
              <CommitStatusBadge status={latest.status} />
            </p>
          </div>
        ) : (
          <p className="text-[11px] leading-relaxed text-muted">
            No touches in the recent source history.
          </p>
        )}
      </section>

      <section className="space-y-1.5">
        <div className="flex h-[25px] items-center justify-between gap-2">
          <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">History</h3>
          <span className="mono text-right text-[11px] text-muted tabular-nums">
            {entry.commits.length}
          </span>
        </div>
        {entry.commits.length === 0 ? (
          <p className="text-[11px] leading-relaxed text-muted">
            No touches in the recent source history.
          </p>
        ) : (
          <ul className="divide-y divide-line overflow-hidden rounded-md border border-line bg-surface">
            {shown.map((commit) => {
              const promoted = commit.status.kind === 'integrated';
              const tickedExplicit = ticked.has(commit.sha);
              return (
                <li key={commit.sha} className="flex h-[29px] items-center gap-2 px-2.5">
                  <Checkbox
                    checked={union.has(commit.sha)}
                    disabled={promoted}
                    onChange={() => onToggleTick(commit.sha)}
                    label={
                      promoted
                        ? `${commit.shortSha} is already promoted on this branch`
                        : tickedExplicit || union.has(commit.sha)
                          ? `Remove ${commit.shortSha} from this promotion`
                          : `Promote ${commit.shortSha} ("${commit.subject}")`
                    }
                  />
                  <span
                    className="mono shrink-0 text-[11px] text-accent-ink"
                    title={`${commit.sha} — ${commit.authorName} · ${formatDateTime(commit.committedAt)}`}
                  >
                    {commit.shortSha}
                  </span>
                  <span
                    className="min-w-0 flex-1 truncate text-[11px] text-fg"
                    title={`${commit.subject} — ${commit.authorName} · ${formatDateTime(commit.committedAt)}`}
                  >
                    {commit.subject}
                  </span>
                  <CommitStatusBadge status={commit.status} />
                </li>
              );
            })}
          </ul>
        )}
        {!historyAll && entry.commits.length > HISTORY_LIMIT ? (
          <Button variant="secondary" size="sm" onClick={() => onHistoryAllChange(true)}>
            Show all {entry.commits.length}
          </Button>
        ) : null}
        {entry.commitsTruncated ? (
          <p className="text-[11px] leading-relaxed text-muted">
            Showing the {entry.commits.length} most recent touches.
          </p>
        ) : null}
        {entry.commits.length > 0 ? (
          <p className="text-[11px] leading-relaxed text-muted">History since the last rename.</p>
        ) : null}
      </section>
    </div>
  );
}

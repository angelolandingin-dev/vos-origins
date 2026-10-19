import { useMemo, useState } from 'react';
import { FileText, GitPullRequestArrow, TriangleAlert } from 'lucide-react';

import { api, asApiError } from '../api/invoke';
import type {
  CommitSummary,
  SubsystemDiffBucket,
  SubsystemDiffCommit,
  SubsystemDiffEntry,
  SubsystemSummary,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { resolveDiffSelection } from '../components/diffResolve';
import { PathLabel } from '../components/PathLabel';
import { RevertsNewerLine } from '../components/RevertsNewer';
import { revertsNewerText } from '../components/revertsNewerText';
import { CommitStatusBadge } from '../components/StatusBadge';
import { cx } from '../components/ui/cx';
import { formatDate, formatDateTime } from '../components/ui/format';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  InlineAlert,
  KeyValue,
  LoadingState,
  Panel,
  Select,
} from '../components/ui/primitives';

/**
 * Subsystem Diff: the subsystem's tracked tree against the target branch tip,
 * so a partial promotion stops being invisible. One batched backend call per
 * subsystem/branch; clicking a file renders its history from the rows already
 * carried (zero extra spawns). Selection resolves to the union of unpromoted
 * touching commits with the exclusions pre-filled, and hands off to the
 * existing promotion dialog — the same dialog, the same `PromotionRequest`,
 * the same `excludedPaths`.
 */

/** Rows shown before the history "show all" control. The backend caps touches well above this. */
const HISTORY_LIMIT = 20;

const BUCKET_META: Record<SubsystemDiffBucket, { label: string; dot: string }> = {
  missing: { label: 'Missing', dot: 'bg-status-blocked' },
  behind: { label: 'Behind', dot: 'bg-status-eligible' },
  main_side_changed: { label: 'Main-side changed', dot: 'bg-status-unknown' },
  identical: { label: 'Identical', dot: 'bg-status-superseded' },
};

/** Status is a 6px dot plus a word — never a pill or filled badge. */
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

export function SubsystemDiff() {
  const app = useApp();
  const subsystem = app.selectedSubsystem;

  if (!subsystem) {
    return (
      <div className="p-5">
        <EmptyState
          icon={FileText}
          title="Select a subsystem first"
          message="Open the Commit Browser and pick the subsystem whose gap against main you want to see."
          action={
            <Button variant="primary" onClick={() => app.navigate('commit-browser')}>
              Open Commit Browser
            </Button>
          }
        />
      </div>
    );
  }

  return <SubsystemDiffView key={`${subsystem.id}:${app.targetBranch}`} subsystem={subsystem} />;
}

function SubsystemDiffView({ subsystem }: { subsystem: SubsystemSummary }) {
  const app = useApp();
  const [filter, setFilter] = useState('');
  const [bucket, setBucket] = useState<SubsystemDiffBucket | 'all'>('all');
  const [hideIdentical, setHideIdentical] = useState(true);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [focused, setFocused] = useState<string | null>(null);
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

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-center justify-between gap-3 px-5 pb-3 pt-4">
        <div className="min-w-0">
          <h1 className="text-[14px] font-semibold text-fg">Subsystem Diff</h1>
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
          title="Files differing from target"
          subtitle={`${subsystem.repository}@${subsystem.sourceBranch} · target ${app.targetBranch}`}
          bodyClassName="flex min-h-0 flex-1 flex-col"
        >
          <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-1.5">
            <input
              className="input mono h-[25px] min-w-[140px] flex-1 text-[11px]"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              placeholder="Filter paths…"
              aria-label="Filter paths"
              spellCheck={false}
              autoComplete="off"
            />
            <Select
              ariaLabel="Bucket filter"
              className="w-[150px]"
              value={bucket}
              onValueChange={(next) => setBucket(next as SubsystemDiffBucket | 'all')}
              options={[
                { value: 'all', label: 'All buckets' },
                { value: 'missing', label: 'Missing' },
                { value: 'behind', label: 'Behind' },
                { value: 'main_side_changed', label: 'Main-side changed' },
                { value: 'identical', label: 'Identical' },
              ]}
            />
            <label className="flex items-center gap-1.5 text-[11px] text-muted">
              <Checkbox
                checked={hideIdentical}
                onChange={(event) => setHideIdentical(event.target.checked)}
                label="Hide identical files"
              />
              <span>Hide identical</span>
            </label>
            <span className="mono ml-auto shrink-0 text-right text-[11px] text-muted tabular-nums">
              {counts.missing} missing · {counts.behind} behind · {counts.main_side_changed}{' '}
              main-side · {counts.identical} identical
            </span>
          </div>

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
            <div className="scroll-area min-h-0 flex-1">
              <div className="table-head flex h-[25px] items-center gap-2 px-2.5">
                <span className="w-3.5 shrink-0" aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate">File</span>
                <span className="w-[110px] shrink-0">Bucket</span>
                <span className="w-[70px] shrink-0">Change</span>
                <span className="w-[250px] shrink-0">Latest</span>
              </div>
              <ul className="divide-y divide-line">
                {visible.map((entry) => {
                  const latest = entry.commits[0] ?? null;
                  const isFocused = focused === entry.path;
                  return (
                    <li
                      key={entry.path}
                      className={cx(
                        'flex h-[29px] items-center gap-2 px-2.5 row-hover',
                        isFocused && 'row-selected',
                      )}
                    >
                      <Checkbox
                        checked={selected.has(entry.path)}
                        onChange={() => togglePath(entry.path)}
                        label={`Select ${entry.path} for promotion`}
                      />
                      <button
                        type="button"
                        onClick={() => focusPath(entry.path)}
                        title={entry.path}
                        className="min-w-0 flex-1 text-left"
                      >
                        <PathLabel path={entry.path} />
                      </button>
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
                      <span className="w-[70px] shrink-0 truncate text-[11px] text-muted">
                        {entry.change}
                      </span>
                      <span
                        className="flex w-[250px] min-w-0 shrink-0 items-center gap-1.5"
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
        </Panel>

        <div className="scroll-area min-h-0 space-y-3">
          <Panel
            title="File detail"
            subtitle={focusedEntry ? focusedEntry.path : 'Select a file to inspect its history'}
          >
            <div className="space-y-3 p-3">
              {focusedEntry ? (
                <FileDetail
                  key={focusedEntry.path}
                  entry={focusedEntry}
                  union={union}
                  ticked={ticked}
                  historyAll={historyAll}
                  onHistoryAllChange={setHistoryAll}
                  onToggleTick={toggleTick}
                />
              ) : (
                <p className="text-[11px] leading-relaxed text-muted">
                  Click a file to see its latest commit and its history on this branch.
                </p>
              )}
            </div>
          </Panel>

          <Panel title="Promotion selection" subtitle={continueHint}>
            <div className="space-y-3 p-3">
              <dl className="divide-y divide-line border-y border-line px-1">
                <KeyValue label="Files selected">
                  <span className="mono text-right tabular-nums">{selected.size}</span>
                </KeyValue>
                <KeyValue label="Commits resolved">
                  <span className="mono text-right tabular-nums">{resolution.commits.length}</span>
                </KeyValue>
                <KeyValue label="Pre-excluded">
                  <span className="mono text-right tabular-nums">
                    {resolution.excludedPaths.length}
                  </span>
                </KeyValue>
              </dl>

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
                  <ul className="divide-y divide-line border-t border-line max-h-32 overflow-y-auto">
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
                <details className="overflow-hidden rounded-md border border-line bg-surface">
                  <summary className="flex h-[29px] cursor-pointer items-center justify-between gap-2 px-2.5 text-[11px]">
                    <span className="font-semibold tracking-wide text-muted uppercase">
                      Pre-excluded
                    </span>
                    <span className="mono text-muted tabular-nums">
                      {resolution.excludedPaths.length}
                    </span>
                  </summary>
                  <ul className="divide-y divide-line border-t border-line max-h-32 overflow-y-auto">
                    {resolution.excludedPaths.map((path) => (
                      <li key={path} className="flex h-[29px] items-center px-2.5">
                        <PathLabel path={path} className="flex-1 text-[11px]" muted />
                      </li>
                    ))}
                  </ul>
                </details>
              ) : null}
              {resolution.excludedPaths.length > 0 ? (
                <p className="text-[11px] leading-relaxed text-muted">
                  Adjustable in the confirmation dialog — uncheck any path to include it.
                </p>
              ) : null}

              <div className="flex flex-wrap gap-1.5">
                <Button variant="secondary" size="sm" onClick={selectAllMissingBehind}>
                  Select all missing + behind
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
              <Button
                variant="primary"
                className="w-full"
                icon={GitPullRequestArrow}
                loading={resolving}
                disabled={resolution.commits.length === 0 || resolving}
                onClick={continueToPromotion}
              >
                {resolving ? 'Resolving…' : 'Continue to promotion'}
              </Button>
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}

/**
 * The on-click file rail, in the details-rail idiom: path + bucket first
 * (the why), the latest commit prominent, then the newest-first history with
 * each row's per-branch promoted state. History commits can be ticked
 * directly into the same promotion selection — cheap, because it is one set
 * unioned by the resolver the page already runs.
 */
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
          <span className="text-[11px] text-muted">{entry.change}</span>
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

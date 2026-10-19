import { useEffect, useMemo, useRef, useState } from 'react';
import {
  ArrowLeft,
  ChevronDown,
  ChevronRight,
  Columns2,
  FileCode2,
  GitPullRequestArrow,
  Rows3,
  Search,
} from 'lucide-react';

import { api } from '../api/invoke';
import type { ChangedFile, CommitSummary } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { DiffViewer } from './DiffViewer';
import { CommitStatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
  Panel,
} from './ui/primitives';
import { SPLIT_DEFAULT_WIDTH, splitWidthStyle } from './ui/split-pane';
import { SplitResizer } from './ui/split-resizer';

/**
 * Dedicated full-diff view (GitHub Desktop History style). A separate
 * full-workspace surface with a back control — not a corner panel: a
 * changed-files column with per-file include checkboxes, and every changed
 * file's diff stacked continuously in one scroll, each rendered by the shared
 * `DiffViewer`. Per-file diffs mount (and fetch) lazily as they near the
 * viewport; each mounted file renders its engine-bounded hunks in full, with
 * no inner scroll container and no row windowing — the section list holds the
 * view's only diff scroller.
 */

function fileGlyph(file: ChangedFile): { glyph: string; className: string } {
  switch (file.change) {
    case 'added':
      return { glyph: '+', className: 'text-status-applied' };
    case 'deleted':
      return { glyph: '-', className: 'text-status-blocked' };
    case 'renamed':
      return { glyph: '→', className: 'text-status-eligible' };
    default:
      return { glyph: '~', className: 'text-status-unknown-ink' };
  }
}

export interface CommitDiffViewProps {
  subsystemId: string;
  sha: string;
  targetBranch: string;
  initialPath: string | null;
  commit: CommitSummary | null;
  onBack: () => void;
}

export function CommitDiffView({
  subsystemId,
  sha,
  targetBranch,
  initialPath,
  commit,
  onBack,
}: CommitDiffViewProps) {
  const app = useApp();
  const [mode, setMode] = useState<'side' | 'unified'>('unified');
  const [filesWidth, setFilesWidth] = useState(SPLIT_DEFAULT_WIDTH);
  const [filter, setFilter] = useState('');
  const [excluded, setExcluded] = useState<string[]>([]);
  const [collapsed, setCollapsed] = useState<string[]>([]);
  // Paths whose section has come near the viewport. Mounting (and fetching)
  // happens once per path and is never undone, so scrolling back is instant.
  // The requested file mounts immediately so its fetch starts on first paint.
  const [mounted, setMounted] = useState<string[]>(() => (initialPath ? [initialPath] : []));
  const [activeOverride, setActiveOverride] = useState<string | null>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const sectionRefs = useRef(new Map<string, HTMLElement>());
  const initialScrolledRef = useRef(false);

  const details = useAsyncData(
    () => api.getCommitDetails(subsystemId, sha, targetBranch),
    [subsystemId, sha, targetBranch, app.dataVersion],
  );
  const files = useMemo(() => details.data?.files ?? [], [details.data]);
  // Derived during render (never written from an effect): an explicit override
  // wins, then the requested file, then the first changed file.
  const defaultPath =
    (initialPath && files.some((file) => file.path === initialPath)
      ? initialPath
      : files[0]?.path) ?? null;
  const activePath = activeOverride ?? defaultPath;
  const query = filter.trim().toLowerCase();
  const listedFiles = query
    ? files.filter((file) => file.path.toLowerCase().includes(query))
    : files;
  const fileKey = listedFiles.map((file) => file.path).join('\n');

  // Escape returns to the workspace without touching focus or selection.
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onBack();
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  }, [onBack]);

  // Lazily mount per-file diffs as they near the viewport.
  useEffect(() => {
    const scroller = scrollerRef.current;
    if (!scroller || typeof IntersectionObserver !== 'function') {
      setMounted(files.map((file) => file.path));
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        const near: string[] = [];
        for (const entry of entries) {
          if (!entry.isIntersecting) continue;
          const path = (entry.target as HTMLElement).dataset['fileSection'];
          if (path) near.push(path);
        }
        if (near.length === 0) return;
        setMounted((prev) => {
          const next = new Set(prev);
          let changed = false;
          for (const path of near) {
            if (!next.has(path)) {
              next.add(path);
              changed = true;
            }
          }
          return changed ? Array.from(next) : prev;
        });
      },
      { root: scroller, rootMargin: '640px 0px' },
    );
    for (const element of sectionRefs.current.values()) observer.observe(element);
    return () => observer.disconnect();
  }, [fileKey, files]);

  // Opening at a file (View Full Diff, or a per-file affordance) scrolls the
  // diffs column straight to that section once the file list has loaded. This
  // effect only moves the DOM scroll position; all state derives during render.
  useEffect(() => {
    if (initialScrolledRef.current) return;
    if (!initialPath || !files.some((file) => file.path === initialPath)) return;
    initialScrolledRef.current = true;
    window.requestAnimationFrame(() => {
      sectionRefs.current.get(initialPath)?.scrollIntoView({ block: 'start' });
    });
  }, [files, initialPath]);

  const scrollToPath = (path: string) => {
    setActiveOverride(path);
    setMounted((prev) => (prev.includes(path) ? prev : [...prev, path]));
    sectionRefs.current.get(path)?.scrollIntoView({ block: 'start', behavior: 'smooth' });
  };

  const toggleExcluded = (path: string) => {
    setExcluded((prev) =>
      prev.includes(path) ? prev.filter((entry) => entry !== path) : [...prev, path],
    );
  };

  const toggleCollapsed = (path: string) => {
    setCollapsed((prev) =>
      prev.includes(path) ? prev.filter((entry) => entry !== path) : [...prev, path],
    );
  };

  // Unticked files are carried into the promotion request's `excludedPaths`,
  // which the promotion dialog and preflight already honour.
  const promote = () => {
    app.openPromotionDialog([sha], commit ? [commit] : [], { excludedPaths: excluded });
  };

  const statusKind = details.data?.status.kind;
  const isPromotable = statusKind === 'eligible' || statusKind === 'reverted';

  return (
    <Panel
      className="min-h-0 min-w-0 flex-1"
      bodyClassName="flex min-h-0 flex-1 flex-col"
      bodyStyle={splitWidthStyle(filesWidth)}
      title={
        <span className="flex min-w-0 items-center gap-2">
          <Button
            size="sm"
            variant="secondary"
            icon={ArrowLeft}
            onClick={onBack}
            title="Back to commits (Esc)"
          >
            Back
          </Button>
          {details.data ? (
            <>
              <span className="mono shrink-0 text-accent-ink" title={details.data.sha}>
                {details.data.shortSha}
              </span>
              <span
                className="min-w-0 flex-1 truncate text-xs font-medium"
                title={details.data.subject}
              >
                {details.data.subject}
              </span>
            </>
          ) : (
            <span className="mono shrink-0 text-muted" title={sha}>
              {sha.slice(0, 7)}
            </span>
          )}
        </span>
      }
      actions={
        details.data ? (
          <>
            <span className="meta hidden xl:flex">
              {details.data.authorLogin ?? details.data.authorName} ·{' '}
              {formatDateTime(details.data.committedAt)}
            </span>
            <span className="mono shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
              +{details.data.additions}
            </span>
            <span className="mono shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
              −{details.data.deletions}
            </span>
            <CommitStatusBadge status={details.data.status} />
            <Button
              size="sm"
              variant="secondary"
              icon={mode === 'side' ? Columns2 : Rows3}
              onClick={() => setMode(mode === 'side' ? 'unified' : 'side')}
              title={mode === 'side' ? 'Switch to unified diff' : 'Switch to side-by-side diff'}
              aria-label={
                mode === 'side' ? 'Switch to unified diff' : 'Switch to side-by-side diff'
              }
            >
              {mode === 'side' ? 'Side-by-side' : 'Unified'}
            </Button>
            <Button
              size="sm"
              variant="primary"
              icon={GitPullRequestArrow}
              disabled={!isPromotable}
              onClick={promote}
              title={
                isPromotable
                  ? excluded.length > 0
                    ? `Promote this commit without ${excluded.length} excluded file(s)`
                    : 'Promote this commit'
                  : 'Only eligible (or previously reverted) commits can be promoted'
              }
            >
              {excluded.length > 0 ? `Promote (${excluded.length} excluded)` : 'Promote'}
            </Button>
          </>
        ) : null
      }
    >
      {details.loading && !details.data ? (
        <LoadingState label="Loading the full diff…" />
      ) : details.error ? (
        <ErrorState error={details.error} onRetry={details.reload} />
      ) : !details.data || files.length === 0 ? (
        <EmptyState
          icon={FileCode2}
          title="No changed files"
          message="There is no file list for this commit."
        />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col md:flex-row">
          <div className="flex max-h-[220px] min-w-0 shrink-0 flex-col border-b border-line md:max-h-none md:w-[var(--split-width)] md:border-b-0">
            <div className="flex h-[29px] shrink-0 items-center justify-between gap-2 px-2.5">
              <p className="text-[11px] font-semibold tracking-wide text-muted uppercase tabular-nums">
                {files.length} changed file{files.length === 1 ? '' : 's'}
              </p>
              {excluded.length > 0 ? (
                <span className="meta text-status-unknown-ink tabular-nums">
                  {excluded.length} excluded
                </span>
              ) : null}
            </div>
            <div className="relative shrink-0 px-2.5 pb-2">
              <Search
                className="pointer-events-none absolute top-1/2 left-5 h-4 w-4 -translate-y-[calc(50%+4px)] text-disabled"
                aria-hidden="true"
              />
              <input
                type="search"
                value={filter}
                onChange={(event) => setFilter(event.target.value)}
                placeholder="Filter files…"
                aria-label="Filter changed files by path"
                className="input pl-9"
              />
            </div>
            <ul className="scroll-area min-h-0 flex-1 overflow-y-auto px-1 pb-2">
              {listedFiles.map((file) => {
                const glyph = fileGlyph(file);
                const included = !excluded.includes(file.path);
                const active = activePath === file.path;
                return (
                  <li
                    key={file.path}
                    className={cx(
                      'flex min-h-[29px] items-start gap-2 rounded-md px-1.5 py-1',
                      active && 'bg-selection',
                    )}
                  >
                    <Checkbox
                      checked={included}
                      onChange={() => toggleExcluded(file.path)}
                      label={
                        included
                          ? `Exclude ${file.path} from promotion`
                          : `Include ${file.path} in promotion`
                      }
                      className="mt-0.5"
                    />
                    <button
                      type="button"
                      onClick={() => scrollToPath(file.path)}
                      title={`${file.path} — scroll to its diff`}
                      className="flex min-w-0 flex-1 items-start gap-2 rounded-[3px] text-left"
                    >
                      <span
                        className={cx(
                          'w-3 shrink-0 text-center text-sm font-bold',
                          glyph.className,
                        )}
                        aria-hidden="true"
                      >
                        {glyph.glyph}
                      </span>
                      <span
                        className="mono min-w-0 flex-1 truncate text-xs text-fg tabular-nums"
                        title={file.path}
                      >
                        {file.path}
                      </span>
                      <span className="mono w-11 shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                        +{file.additions}
                      </span>
                      <span className="mono w-11 shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                        −{file.deletions}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
            <p className="shrink-0 truncate border-t border-line px-2.5 py-1.5 text-[11px] text-muted">
              {excluded.length > 0
                ? `${excluded.length} unticked file(s) are excluded from promotion.`
                : 'Untick files to exclude them from promotion.'}
            </p>
          </div>

          <SplitResizer
            width={filesWidth}
            onWidthChange={setFilesWidth}
            label="Resize the changed-files column"
          />

          <div
            ref={scrollerRef}
            className="scroll-area min-h-0 min-w-0 flex-1 overflow-y-auto bg-ground"
            tabIndex={0}
            role="region"
            aria-label={`Full diff of ${details.data.shortSha}`}
          >
            {listedFiles.map((file) => {
              const glyph = fileGlyph(file);
              const isCollapsed = collapsed.includes(file.path);
              const isMounted = mounted.includes(file.path);
              const isExcluded = excluded.includes(file.path);
              return (
                <section
                  key={file.path}
                  data-file-section={file.path}
                  ref={(element) => {
                    if (element) sectionRefs.current.set(file.path, element);
                    else sectionRefs.current.delete(file.path);
                  }}
                  aria-label={`Diff of ${file.path}`}
                  className="scroll-mt-2 border-b border-line last:border-b-0"
                >
                  <div className="flex min-h-[29px] items-center gap-2 bg-raised px-2.5 py-1">
                    <IconButton
                      icon={isCollapsed ? ChevronRight : ChevronDown}
                      label={
                        isCollapsed
                          ? `Expand diff of ${file.path}`
                          : `Collapse diff of ${file.path}`
                      }
                      onClick={() => toggleCollapsed(file.path)}
                    />
                    <span
                      className={cx('w-3 shrink-0 text-center text-sm font-bold', glyph.className)}
                      aria-hidden="true"
                    >
                      {glyph.glyph}
                    </span>
                    <span
                      className="mono min-w-0 flex-1 truncate text-xs text-fg tabular-nums"
                      title={file.path}
                    >
                      {file.path}
                    </span>
                    {isExcluded ? (
                      <span className="meta shrink-0 text-status-unknown-ink">excluded</span>
                    ) : null}
                    <span className="mono w-11 shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                      +{file.additions}
                    </span>
                    <span className="mono w-11 shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                      −{file.deletions}
                    </span>
                  </div>
                  {isCollapsed ? null : isMounted ? (
                    <FileDiffSection
                      subsystemId={subsystemId}
                      sha={sha}
                      targetBranch={targetBranch}
                      file={file}
                      mode={mode}
                      onOpenExternal={app.openExternal}
                    />
                  ) : (
                    <div className="flex h-[120px] items-center justify-center">
                      <p className="text-[11px] text-disabled">Diff loads on scroll…</p>
                    </div>
                  )}
                </section>
              );
            })}
          </div>
        </div>
      )}
    </Panel>
  );
}

function FileDiffSection({
  subsystemId,
  sha,
  targetBranch,
  file,
  mode,
  onOpenExternal,
}: {
  subsystemId: string;
  sha: string;
  targetBranch: string;
  file: ChangedFile;
  mode: 'side' | 'unified';
  onOpenExternal: (url: string) => void;
}) {
  const app = useApp();
  const diff = useAsyncData(
    () => api.getCommitDiff(subsystemId, sha, file.path, targetBranch),
    [subsystemId, sha, file.path, targetBranch, app.dataVersion],
  );

  if (diff.loading && !diff.data) return <LoadingState label={`Loading ${file.path}…`} />;
  if (diff.error) return <ErrorState error={diff.error} onRetry={diff.reload} compact />;
  if (!diff.data) {
    return (
      <EmptyState
        icon={FileCode2}
        title="No diff for this file"
        message={file.path}
        className="p-4"
      />
    );
  }

  return (
    <DiffViewer
      diff={diff.data}
      loading={diff.loading}
      error={diff.error}
      onRetry={diff.reload}
      mode={mode}
      onOpenExternal={onOpenExternal}
    />
  );
}

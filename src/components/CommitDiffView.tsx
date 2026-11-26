import { useMemo, useState } from 'react';
import { Check, Copy, ExternalLink, FileCode2, Search } from 'lucide-react';

import type { ApiError } from '../api/invoke';
import { api } from '../api/invoke';
import type { ChangedFile, CommitDetails } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { ChangeKindIcon } from './ChangeKindIcon';
import { CommitDetailsContext } from './CommitDetails';
import { DiffViewer } from './DiffViewer';
import { PathLabel } from './PathLabel';
import { CommitStatusBadge } from './StatusBadge';
import { cx } from './ui/cx';
import {
  Checkbox,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
  Spinner,
} from './ui/primitives';
import { SPLIT_DEFAULT_WIDTH, splitWidthStyle } from './ui/split-pane';
import { SplitResizer } from './ui/split-resizer';

export interface CommitDiffViewProps {
  subsystemId: string;
  sha: string;
  targetBranch: string;
  details: CommitDetails | null;
  loading: boolean;
  error: ApiError | null;
  onRetry: () => void;
  selectedPath: string | null;
  onSelectPath: (path: string) => void;
  excludedPaths: string[];
  onToggleExcluded: (path: string) => void;
  mainRepository: string | null;
}

export function CommitDiffView({
  subsystemId,
  sha,
  targetBranch,
  details,
  loading,
  error,
  onRetry,
  selectedPath,
  onSelectPath,
  excludedPaths,
  onToggleExcluded,
  mainRepository,
}: CommitDiffViewProps) {
  const app = useApp();
  const [mode, setMode] = useState<'side' | 'unified'>('unified');
  const [filesWidth, setFilesWidth] = useState(SPLIT_DEFAULT_WIDTH);
  const [filter, setFilter] = useState('');
  const [copied, setCopied] = useState(false);

  const files = useMemo(() => details?.files ?? [], [details]);
  const query = filter.trim().toLowerCase();
  const listedFiles = query
    ? files.filter((file) => file.path.toLowerCase().includes(query))
    : files;
  const activePath =
    (selectedPath && files.some((file) => file.path === selectedPath)
      ? selectedPath
      : files[0]?.path) ?? null;
  const activeFile = files.find((file) => file.path === activePath) ?? null;
  const activeExcluded = excludedPaths.filter((path) => files.some((file) => file.path === path));

  const toggleExcluded = (path: string) => {
    onToggleExcluded(path);
  };

  const copySha = async () => {
    if (!details) return;
    try {
      await navigator.clipboard?.writeText(details.sha);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <section
      aria-label="Commit diff"
      style={splitWidthStyle(filesWidth)}
      className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface"
    >
      <header className="flex min-h-[38px] flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-3 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          {details ? (
            <>
              <button
                type="button"
                onClick={() => void copySha()}
                title={`${details.sha}\nClick to copy the full SHA (abbreviated values are display-only)`}
                className="mono shrink-0 rounded-[3px] text-xs font-semibold text-accent-ink hover:underline"
              >
                {details.shortSha}
              </button>
              {loading ? (
                <span
                  role="status"
                  aria-label="Loading the newly selected commit"
                  title="Loading the newly selected commit — the view still shows the previous commit until it arrives."
                >
                  <Spinner className="h-3.5 w-3.5" />
                </span>
              ) : null}
              <IconButton
                icon={copied ? Check : Copy}
                label={copied ? 'Full SHA copied' : 'Copy full SHA'}
                onClick={() => void copySha()}
              />
              <IconButton
                icon={ExternalLink}
                label="Open this commit on GitHub"
                onClick={() => app.openExternal(details.sourceUrl)}
              />
              <span
                className="min-w-0 flex-1 truncate text-xs font-medium text-fg"
                title={details.subject}
              >
                {details.subject}
              </span>
            </>
          ) : (
            <span className="mono shrink-0 text-xs text-muted" title={sha}>
              {sha.slice(0, 7)}
            </span>
          )}
        </div>
        {details ? (
          <div className="flex shrink-0 items-center gap-2">
            <span className="mono shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
              +{details.additions}
            </span>
            <span className="mono shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
              −{details.deletions}
            </span>
            <CommitStatusBadge status={details.status} />
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
              ).map((option, idx) => (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => setMode(option.id)}
                  className={cx(
                    'px-2 text-[11px] transition-colors',
                    idx > 0 && 'border-l border-line',
                    mode === option.id
                      ? 'bg-raised font-semibold text-fg'
                      : 'text-muted hover:bg-hover hover:text-fg',
                  )}
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>
        ) : null}
      </header>

      {loading && details ? (
        <div
          role="progressbar"
          aria-label="Refreshing commit diff"
          className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
        >
          <div className="h-full w-1/2 animate-pulse bg-accent" />
        </div>
      ) : null}

      {loading && !details ? (
        <LoadingState label="Loading the commit…" />
      ) : error ? (
        <ErrorState error={error} onRetry={onRetry} />
      ) : !details ? (
        <EmptyState
          icon={FileCode2}
          title="No commit selected"
          message="Pick a commit in the list to inspect its changed files and diff."
        />
      ) : (
        <div
          className={cx(
            'flex min-h-0 flex-1 flex-col transition-opacity md:flex-row',
            loading && 'opacity-55',
          )}
        >
          {/*
            `md:shrink` (not `shrink-0`) lets the files column give width back
            when the window is narrower than the dragged split, so a wide split
            can never push the diff column off-screen. The diff keeps a floor so
            its fixed-width header/row parts cannot be squeezed into a
            horizontal overflow. The commit list is a separate column of the
            workspace, so this split only ever divides what is left of it.
          */}
          <div className="flex max-h-[220px] min-w-0 shrink-0 flex-col border-b border-line md:max-h-none md:w-[var(--split-width)] md:shrink md:border-b-0">
            <div className="flex h-[29px] shrink-0 items-center justify-between gap-2 px-2.5">
              <p className="text-[11px] font-semibold tracking-wide text-muted uppercase tabular-nums">
                {files.length} changed file{files.length === 1 ? '' : 's'}
              </p>
              {activeExcluded.length > 0 ? (
                <span className="meta text-status-unknown-ink tabular-nums">
                  {activeExcluded.length} excluded
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
            <div className="scroll-area min-h-0 flex-1 overflow-y-auto">
              {files.length === 0 ? (
                <p className="px-2.5 pb-2 text-[11px] text-disabled">
                  No file list is available for this commit.
                </p>
              ) : listedFiles.length === 0 ? (
                <p className="px-2.5 pb-2 text-[11px] text-disabled">No files match this filter.</p>
              ) : (
                <ul className="px-1 pb-2">
                  {listedFiles.map((file) => {
                    const included = !activeExcluded.includes(file.path);
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
                          onClick={() => onSelectPath(file.path)}
                          aria-current={active ? 'true' : undefined}
                          title={`${file.path} — show its diff`}
                          className="flex min-w-0 flex-1 items-start gap-2 rounded-[3px] text-left"
                        >
                          <ChangeKindIcon change={file.change} />
                          <PathLabel path={file.path} className="flex-1" />
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
              )}
              <p className="shrink-0 truncate border-t border-line px-2.5 py-1.5 text-[11px] text-muted">
                {activeExcluded.length > 0
                  ? `${activeExcluded.length} unticked file(s) are excluded from promotion.`
                  : 'Untick files to exclude them from promotion.'}
              </p>

              <div className="border-t border-line">
                <CommitDetailsContext
                  details={details}
                  targetBranch={targetBranch}
                  onOpenExternal={app.openExternal}
                  mainRepository={mainRepository}
                />
              </div>
            </div>
          </div>

          <SplitResizer
            width={filesWidth}
            onWidthChange={setFilesWidth}
            label="Resize the changed-files column"
          />

          {/* Exactly one file's diff — a file header and its hunks. */}
          <div
            className="scroll-area min-h-0 min-w-0 flex-1 overflow-y-auto bg-ground md:min-w-[320px]"
            tabIndex={0}
            role="region"
            aria-label={`Diff of ${details.shortSha}`}
          >
            {activeFile ? (
              <>
                <div className="flex min-h-[29px] items-center gap-2 border-b border-line bg-raised px-2.5 py-1">
                  <ChangeKindIcon change={activeFile.change} />
                  <PathLabel path={activeFile.path} className="flex-1" />
                  <span className="mono shrink-0 text-right text-[11px] text-status-applied-ink tabular-nums">
                    +{activeFile.additions}
                  </span>
                  <span className="mono shrink-0 text-right text-[11px] text-status-blocked-ink tabular-nums">
                    −{activeFile.deletions}
                  </span>
                </div>
                {activeFile.previousPath ? (
                  <p
                    className="mono truncate border-b border-line px-2.5 py-1 text-[11px] text-disabled"
                    title={`renamed from ${activeFile.previousPath}`}
                  >
                    renamed from {activeFile.previousPath}
                  </p>
                ) : null}
                <FileDiffSection
                  subsystemId={subsystemId}
                  sha={sha}
                  targetBranch={targetBranch}
                  file={activeFile}
                  mode={mode}
                  onOpenExternal={app.openExternal}
                />
              </>
            ) : (
              <EmptyState
                icon={FileCode2}
                title="No file selected"
                message="Choose a file in the changed-files column to read its diff."
              />
            )}
          </div>
        </div>
      )}
    </section>
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
  // A path the patch could not resolve — a mode-only change — has no text diff;
  // it says so rather than inventing one.
  if (!diff.data) {
    return (
      <EmptyState
        icon={FileCode2}
        title="No text diff for this file"
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

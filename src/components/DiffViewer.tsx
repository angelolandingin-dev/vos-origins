import { useEffect, useRef, useState } from 'react';
import { Columns2, ExternalLink, FileCode2, Rows3, TriangleAlert } from 'lucide-react';

import type { ApiError } from '../api/invoke';
import type { ChangedFile, CommitDiff, DiffHunk, DiffLine } from '../api/types';
import { cx } from './ui/cx';
import {
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  InlineAlert,
  LoadingState,
  Select,
} from './ui/primitives';

/**
 * Read-only bounded diff viewer (spec §3.2): lightweight renderer with windowed rows;
 * over-bound responses flag truncated; the full file stays one click away on GitHub.
 *
 * Fixed comparative measure: every row is exactly ROW_HEIGHT tall and both line-number
 * gutters are a constant width, so the two compared states are read at one scale
 * rather than re-zoomed per view.
 */

const ROW_HEIGHT = 20;
const OVERSCAN = 12;

type Row =
  | { kind: 'hunk'; header: string }
  | { kind: 'unified'; line: DiffLine }
  | { kind: 'side'; left?: DiffLine; right?: DiffLine };

function buildUnifiedRows(hunks: DiffHunk[]): Row[] {
  const rows: Row[] = [];
  for (const hunk of hunks) {
    rows.push({ kind: 'hunk', header: hunk.header });
    for (const line of hunk.lines) rows.push({ kind: 'unified', line });
  }
  return rows;
}

/** Pairs removed/added runs so both sides line up like the approved mockup. */
function buildSideBySideRows(hunks: DiffHunk[]): Row[] {
  const rows: Row[] = [];
  for (const hunk of hunks) {
    rows.push({ kind: 'hunk', header: hunk.header });
    const lines = hunk.lines;
    let index = 0;
    while (index < lines.length) {
      const line = lines[index];
      if (!line) break;
      if (line.kind === 'context') {
        rows.push({ kind: 'side', left: line, right: line });
        index += 1;
        continue;
      }
      const removed: DiffLine[] = [];
      const added: DiffLine[] = [];
      while (index < lines.length && lines[index]?.kind === 'removed') {
        const next = lines[index];
        if (!next) break;
        removed.push(next);
        index += 1;
      }
      while (index < lines.length && lines[index]?.kind === 'added') {
        const next = lines[index];
        if (!next) break;
        added.push(next);
        index += 1;
      }
      if (removed.length === 0 && added.length === 0) {
        index += 1;
        continue;
      }
      const count = Math.max(removed.length, added.length);
      for (let offset = 0; offset < count; offset += 1) {
        rows.push({ kind: 'side', left: removed[offset], right: added[offset] });
      }
    }
  }
  return rows;
}

function lineClasses(kind: DiffLine['kind'] | 'empty'): string {
  switch (kind) {
    case 'added':
      return 'bg-status-applied-soft';
    case 'removed':
      return 'bg-status-blocked-soft';
    case 'empty':
      return 'bg-raised';
    default:
      return '';
  }
}

export interface DiffViewerProps {
  diff: CommitDiff | null;
  loading: boolean;
  error: ApiError | null;
  onRetry: () => void;
  files: ChangedFile[];
  selectedPath: string | null;
  onSelectPath: (path: string) => void;
  onOpenExternal: (url: string) => void;
}

export function DiffViewer({
  diff,
  loading,
  error,
  onRetry,
  files,
  selectedPath,
  onSelectPath,
  onOpenExternal,
}: DiffViewerProps) {
  const [mode, setMode] = useState<'side' | 'unified'>('side');

  if (loading && !diff) return <LoadingState label="Fetching the file diff…" />;
  if (error) return <ErrorState error={error} onRetry={onRetry} compact />;
  if (!diff) {
    return (
      <EmptyState
        icon={FileCode2}
        title="No diff loaded"
        message="Select a file in the commit details panel to fetch its diff on demand."
      />
    );
  }

  const rows = mode === 'side' ? buildSideBySideRows(diff.hunks) : buildUnifiedRows(diff.hunks);
  const total = rows.length;
  const fileOptions = files.map((file) => ({
    value: file.path,
    label: `${file.path}  (+${file.additions}/-${file.deletions})`,
  }));

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
        <FileCode2 className="h-4 w-4 shrink-0 text-disabled" aria-hidden="true" />
        {files.length > 1 ? (
          <Select
            aria-label="Changed file to preview"
            className="w-[320px]"
            value={selectedPath ?? diff.filePath}
            onChange={(event) => onSelectPath(event.target.value)}
            options={
              fileOptions.length > 0
                ? fileOptions
                : [{ value: diff.filePath, label: diff.filePath }]
            }
          />
        ) : (
          <span className="mono min-w-0 flex-1 truncate text-fg" title={diff.filePath}>
            {diff.filePath}
          </span>
        )}

        <span className="chip">{diff.language}</span>
        <span className="mono text-right text-[11px] text-status-applied-ink tabular-nums">
          +{diff.additions}
        </span>
        <span className="mono text-right text-[11px] text-status-blocked-ink tabular-nums">
          -{diff.deletions}
        </span>
        {diff.binary ? <span className="chip text-status-unknown-ink">binary</span> : null}

        <div className="ml-auto flex items-center gap-2">
          <span className="mono text-right text-[11px] text-disabled tabular-nums">
            {total} rows
          </span>
          <Button
            size="sm"
            variant="secondary"
            icon={mode === 'side' ? Columns2 : Rows3}
            onClick={() => setMode(mode === 'side' ? 'unified' : 'side')}
            title={mode === 'side' ? 'Switch to unified diff' : 'Switch to side-by-side diff'}
            aria-label={mode === 'side' ? 'Switch to unified diff' : 'Switch to side-by-side diff'}
          >
            {mode === 'side' ? 'Side-by-side' : 'Unified'}
          </Button>
          <IconButton
            icon={ExternalLink}
            label="Open this file on GitHub"
            onClick={() => onOpenExternal(diff.sourceUrl)}
          />
        </div>
      </div>

      {diff.note ? (
        <div className="px-2.5 pt-2">
          <InlineAlert tone="info" title="How this change is applied">
            {diff.note}
          </InlineAlert>
        </div>
      ) : null}

      {/* Remount resets scroll without an effect that would cascade a render. */}
      <DiffBody key={`${diff.sha}|${diff.filePath}|${mode}`} rows={rows} filePath={diff.filePath} />

      {diff.truncated ? (
        <div className="border-t border-status-unknown/40 bg-status-unknown-soft px-2.5 py-1.5">
          <p className="flex items-center gap-2 text-[11px] text-status-unknown-ink">
            <TriangleAlert className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Diff bounded for responsiveness. Open the file on GitHub for the complete patch.
          </p>
        </div>
      ) : null}
    </div>
  );
}

function DiffBody({ rows, filePath }: { rows: Row[]; filePath: string }) {
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(320);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    const update = () => setViewportHeight(element.clientHeight || 320);
    update();
    if (typeof ResizeObserver === 'function') {
      const observer = new ResizeObserver(update);
      observer.observe(element);
      return () => observer.disconnect();
    }
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  const total = rows.length;
  const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const end = Math.min(total, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN);
  const visible = rows.slice(start, end);

  return (
    <div
      ref={scrollRef}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      className="scroll-area min-h-0 flex-1 overflow-y-auto bg-ground"
      tabIndex={0}
      role="region"
      aria-label={`Diff of ${filePath}`}
    >
      <div style={{ height: total * ROW_HEIGHT }} className="relative">
        <div
          className="absolute inset-x-0"
          style={{ transform: `translateY(${start * ROW_HEIGHT}px)` }}
        >
          {visible.map((row, index) => {
            const key = `${start + index}`;

            if (row.kind === 'hunk') {
              return (
                <div
                  key={key}
                  style={{ height: ROW_HEIGHT }}
                  className="mono flex items-center gap-3 bg-accent-soft px-2.5 text-[11px] text-accent-ink"
                >
                  <span className="truncate">{row.header}</span>
                </div>
              );
            }

            if (row.kind === 'unified') {
              const { line } = row;
              const sign = line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : ' ';
              return (
                <div
                  key={key}
                  style={{ height: ROW_HEIGHT }}
                  className={cx(
                    'mono flex items-center gap-2 px-2 text-[11px] leading-none',
                    lineClasses(line.kind),
                  )}
                >
                  <span className="w-10 shrink-0 text-right text-disabled tabular-nums select-none">
                    {line.oldLine ?? ''}
                  </span>
                  <span className="w-10 shrink-0 text-right text-disabled tabular-nums select-none">
                    {line.newLine ?? ''}
                  </span>
                  <span
                    className={cx(
                      'w-3 shrink-0 text-center select-none',
                      line.kind === 'added'
                        ? 'text-status-applied'
                        : line.kind === 'removed'
                          ? 'text-status-blocked'
                          : 'text-disabled',
                    )}
                  >
                    {sign}
                  </span>
                  <span className="min-w-0 truncate text-fg" title={line.text}>
                    {line.text || ' '}
                  </span>
                </div>
              );
            }

            return (
              <div
                key={key}
                style={{ height: ROW_HEIGHT }}
                className="mono grid grid-cols-2 text-[11px] leading-none"
              >
                {(['left', 'right'] as const).map((side) => {
                  const line = side === 'left' ? row.left : row.right;
                  if (!line) {
                    return (
                      <div
                        key={side}
                        className={cx('flex items-center px-2', lineClasses('empty'))}
                      />
                    );
                  }
                  const lineNumber = side === 'left' ? line.oldLine : line.newLine;
                  const sign = line.kind === 'added' ? '+' : line.kind === 'removed' ? '-' : ' ';
                  return (
                    <div
                      key={side}
                      className={cx(
                        'flex min-w-0 items-center gap-2 px-2',
                        lineClasses(line.kind),
                        side === 'right' && 'border-l border-line',
                      )}
                    >
                      <span className="w-10 shrink-0 text-right text-disabled tabular-nums select-none">
                        {lineNumber ?? ''}
                      </span>
                      <span
                        className={cx(
                          'w-3 shrink-0 text-center select-none',
                          line.kind === 'added'
                            ? 'text-status-applied'
                            : line.kind === 'removed'
                              ? 'text-status-blocked'
                              : 'text-disabled',
                        )}
                      >
                        {sign}
                      </span>
                      <span className="min-w-0 truncate text-fg" title={line.text}>
                        {line.text || ' '}
                      </span>
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

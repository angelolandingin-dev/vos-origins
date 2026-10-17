import { ExternalLink, FileCode2, TriangleAlert } from 'lucide-react';

import type { ApiError } from '../api/invoke';
import type { CommitDiff, DiffHunk, DiffLine } from '../api/types';
import { cx } from './ui/cx';
import { EmptyState, ErrorState, IconButton, InlineAlert, LoadingState } from './ui/primitives';

/**
 * Read-only bounded diff renderer (spec §3.2): renders the engine-bounded
 * hunks in full — no internal scroll container, no row windowing. The single
 * scroller lives in the parent (the diff view's section list); this component
 * must never introduce one of its own. Over-bound responses flag truncated
 * (around 1500 lines per file); the full file stays one click away on GitHub.
 *
 * Fixed comparative measure: every row is exactly ROW_HEIGHT tall and both
 * line-number gutters are a constant width, so the two compared states are
 * read at one scale rather than re-zoomed per view.
 */

const ROW_HEIGHT = 20;

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
  /** Side-by-side/unified mode is owned by the parent, which holds the single toggle. */
  mode: 'side' | 'unified';
  onOpenExternal: (url: string) => void;
}

export function DiffViewer({
  diff,
  loading,
  error,
  onRetry,
  mode,
  onOpenExternal,
}: DiffViewerProps) {
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

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b border-line px-2.5 py-2">
        <span className="meta">{diff.language}</span>
        <span className="mono text-right text-[11px] text-disabled tabular-nums">
          {rows.length} rows
        </span>
        {diff.binary ? <span className="meta text-status-unknown-ink">binary</span> : null}

        <div className="ml-auto flex items-center gap-2">
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

      <DiffRows rows={rows} />

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

/** Full row rendering: every hunk row mounts — the engine bounds the total, not the viewport. */
function DiffRows({ rows }: { rows: Row[] }) {
  return (
    <div className="bg-ground">
      {rows.map((row, index) => {
        if (row.kind === 'hunk') {
          return (
            <div
              key={index}
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
              key={index}
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
            key={index}
            style={{ height: ROW_HEIGHT }}
            className="mono grid grid-cols-2 text-[11px] leading-none"
          >
            {(['left', 'right'] as const).map((side) => {
              const line = side === 'left' ? row.left : row.right;
              if (!line) {
                return (
                  <div key={side} className={cx('flex items-center px-2', lineClasses('empty'))} />
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
  );
}

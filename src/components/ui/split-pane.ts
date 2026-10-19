import type { CSSProperties } from 'react';

/**
 * The geometry contract for the app's one horizontal splitter, shared by both
 * diff surfaces (`CommitDiffView`, `IntegrationEventAuditPanel`): the
 * changed-files column against the diff. The dragged width lives in the parent
 * and is published as the `--split-width` custom property, so the same number
 * can be handed to a responsive (`md:`) width utility and still stack on narrow
 * windows. The handle itself is `SplitResizer` (split-resizer.tsx).
 */

/** Wide enough for a 40-character path; the ceiling is generous because reading a
 *  long path is the point of the column, and the diff keeps its own `min-w-0`. */
export const SPLIT_MIN_WIDTH = 240;
export const SPLIT_MAX_WIDTH = 900;
export const SPLIT_DEFAULT_WIDTH = 340;

export function clampSplitWidth(width: number): number {
  return Math.min(SPLIT_MAX_WIDTH, Math.max(SPLIT_MIN_WIDTH, width));
}

/** Publishes the width for `md:w-[var(--split-width)]` on the resized pane. */
export function splitWidthStyle(width: number): CSSProperties {
  return { '--split-width': `${width}px` } as CSSProperties;
}

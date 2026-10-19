import type { CSSProperties } from 'react';

/** Wide enough for a 40-character path; the diff keeps its own `min-w-0`. */
export const SPLIT_MIN_WIDTH = 240;
export const SPLIT_MAX_WIDTH = 900;
export const SPLIT_DEFAULT_WIDTH = 340;

export function splitWidthStyle(width: number): CSSProperties {
  return { '--split-width': `${width}px` } as CSSProperties;
}

export const COMMIT_RAIL_MIN_WIDTH = 280;
export const COMMIT_RAIL_MAX_WIDTH = 720;
export const COMMIT_RAIL_DEFAULT_WIDTH = 380;

export const COMMIT_RAIL_RESERVED_WIDTH = SPLIT_MIN_WIDTH + 320 + 8;

export function commitRailMaxWidth(paneWidth: number): number {
  // paneWidth is 0 until the first ResizeObserver callback; don't collapse the rail.
  if (paneWidth <= 0) return COMMIT_RAIL_MAX_WIDTH;
  return Math.max(
    COMMIT_RAIL_MIN_WIDTH,
    Math.min(COMMIT_RAIL_MAX_WIDTH, paneWidth - COMMIT_RAIL_RESERVED_WIDTH),
  );
}

export function clampCommitRailWidth(width: number, paneWidth: number): number {
  return Math.min(commitRailMaxWidth(paneWidth), Math.max(COMMIT_RAIL_MIN_WIDTH, width));
}

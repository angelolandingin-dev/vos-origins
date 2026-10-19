import type { CSSProperties } from 'react';

/**
 * Geometry for the diff splitter (changed-files against the diff) and the
 * commit-browser rail. Only the diff pane publishes `--split-width`, so the same
 * number can also feed a responsive (`md:`) width utility; the rail keeps its own
 * inline width and clamp so the two handles never share state.
 */

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

/**
 * The floors the rail must not eat into — changed files (240) and the diff (320) —
 * plus the hairlines and the diff panel's border between them.
 */
export const COMMIT_RAIL_RESERVED_WIDTH = SPLIT_MIN_WIDTH + 320 + 8;

/** Capped by the pane so neither neighbour drops below its floor; never below the rail's own floor, which would invert the clamp. */
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

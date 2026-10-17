import type { SubsystemDiffEntry } from '../api/types';

export interface DiffResolution {
  /** Union of unpromoted touching commits (plus explicitly ticked ones), newest-first. */
  commits: string[];
  /** Other paths those commits touch that are not selected — the exclusions prefill. */
  excludedPaths: string[];
  /** Selected main-side changed paths: flagged, never silently gaining a commit. */
  mainSideSelected: string[];
}

/**
 * TypeScript mirror of the Rust `resolve_selection`: the union of unpromoted
 * touching commits over the selected paths plus explicitly ticked history
 * commits, ordered newest-first by commit date. Set equality with the backend
 * twin is what the acceptance needs; the engine sorts anyway.
 */
export function resolveDiffSelection(
  entries: SubsystemDiffEntry[],
  selected: ReadonlySet<string>,
  ticked: ReadonlySet<string>,
): DiffResolution {
  const byPath = new Map(entries.map((entry) => [entry.path, entry]));
  const promoted = new Set<string>();
  const seen = new Map<string, string>();
  for (const entry of entries) {
    for (const commit of entry.commits) {
      if (commit.status.kind === 'integrated') promoted.add(commit.sha);
      if (!seen.has(commit.sha)) seen.set(commit.sha, commit.committedAt);
    }
  }
  const wanted = new Set<string>();
  for (const sha of ticked) wanted.add(sha);
  for (const path of selected) {
    const entry = byPath.get(path);
    if (!entry) continue;
    for (const commit of entry.commits) wanted.add(commit.sha);
  }
  for (const sha of promoted) wanted.delete(sha);
  const commits = [...wanted].sort(
    (x, y) => (seen.get(y) ?? '').localeCompare(seen.get(x) ?? '') || (x < y ? -1 : 1),
  );
  const touched = new Map<string, string[]>();
  for (const entry of entries) {
    for (const commit of entry.commits) {
      if (!touched.has(commit.sha)) touched.set(commit.sha, commit.touchedPaths);
    }
  }
  const excluded = new Set<string>();
  for (const sha of commits) {
    for (const path of touched.get(sha) ?? []) {
      if (!selected.has(path)) excluded.add(path);
    }
  }
  const mainSideSelected = [...selected]
    .filter((path) => byPath.get(path)?.bucket === 'main_side_changed')
    .sort();
  return { commits, excludedPaths: [...excluded].sort(), mainSideSelected };
}

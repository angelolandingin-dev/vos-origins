import type { ReconcileDirectoryNode } from '../../api/types';

function normalizeFolderPath(raw: string): string {
  return raw
    .trim()
    .replace(/\\/g, '/')
    .replace(/^(\.\/)+/, '')
    .replace(/\/+$/, '');
}

export function isCoveredByAncestor(path: string, selected: readonly string[]): boolean {
  return selected.some((ancestor) => path.startsWith(`${ancestor}/`));
}

export function addFolderWithConsolidation(
  current: readonly string[],
  folder: string,
): { next: string[]; replaced: string[] } {
  const norm = normalizeFolderPath(folder);
  if (!norm) return { next: [...current], replaced: [] };
  if (current.includes(norm) || isCoveredByAncestor(norm, current)) {
    return { next: [...current], replaced: [] };
  }
  const childPrefix = `${norm}/`;
  const replaced = current.filter((existing) => existing.startsWith(childPrefix));
  const kept = current.filter((existing) => !existing.startsWith(childPrefix));
  const next = [...kept, norm].sort((a, b) => a.localeCompare(b));
  return { next, replaced };
}

export function scopesEqual(a: readonly string[], b: readonly string[]): boolean {
  if (a.length !== b.length) return false;
  const sa = [...a].sort((x, y) => x.localeCompare(y));
  const sb = [...b].sort((x, y) => x.localeCompare(y));
  return sa.every((val, idx) => val === sb[idx]);
}

export function folderDriftTotal(node: ReconcileDirectoryNode): number {
  return node.added + node.modified + node.deleted;
}

export function visibleTreeNodes(
  directories: readonly ReconcileDirectoryNode[],
  expanded: ReadonlySet<string>,
  filter: string,
  changedOnly: boolean,
): ReconcileDirectoryNode[] {
  const needle = filter.trim().toLowerCase();
  const hasSearch = needle.length > 0;

  let allowed: Set<string> | null = null;
  if (hasSearch || changedOnly) {
    allowed = new Set<string>();
    const byPath = new Map(directories.map((dir) => [dir.path, dir]));
    for (const dir of directories) {
      const matchesChanged = !changedOnly || folderDriftTotal(dir) > 0;
      const matchesSearch = !hasSearch || dir.path.toLowerCase().includes(needle);
      if (matchesChanged && matchesSearch) {
        let cur: ReconcileDirectoryNode | undefined = dir;
        while (cur) {
          allowed.add(cur.path);
          cur = cur.parent ? byPath.get(cur.parent) : undefined;
        }
      }
    }
  }

  const out: ReconcileDirectoryNode[] = [];
  for (const dir of directories) {
    if (allowed && !allowed.has(dir.path)) continue;
    if (!hasSearch && dir.parent !== null) {
      let ancestor: string | null = dir.parent;
      let parentOpen = true;
      const byPath = new Map(directories.map((d) => [d.path, d]));
      while (ancestor !== null) {
        if (!expanded.has(ancestor)) {
          parentOpen = false;
          break;
        }
        ancestor = byPath.get(ancestor)?.parent ?? null;
      }
      if (!parentOpen) continue;
    }
    out.push(dir);
  }
  return out;
}

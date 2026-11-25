import { useMemo, useRef, useState, type KeyboardEvent } from 'react';
import {
  ArrowRight,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  FileText,
  Folder,
  FolderOpen,
  RotateCcw,
  Save,
  Search,
  Trash2,
  X,
} from 'lucide-react';

import type { ReconcileDirectoryNode, ReconcileTreeSnapshot } from '../../api/types';
import { ChangeKindIcon } from '../ChangeKindIcon';
import { PathLabel } from '../PathLabel';
import { cx } from '../ui/cx';
import { Button, Checkbox, EmptyState, IconButton, InlineAlert, Spinner } from '../ui/primitives';
import { splitWidthStyle } from '../ui/split-pane';
import { SplitResizer } from '../ui/split-resizer';
import { folderDriftTotal, isCoveredByAncestor, scopesEqual, visibleTreeNodes } from './scopeUtils';

export interface DirectorySelectorProps {
  snapshot: ReconcileTreeSnapshot;
  loading: boolean;
  selectedFolders: string[];
  savedFolders: string[];
  saving: boolean;
  saveError: string | null;
  locked: boolean;
  consolidatedNotice: string | null;
  onDismissConsolidated: () => void;
  onToggleFolder: (path: string) => void;
  onRemoveFolder: (path: string) => void;
  onSaveScope: () => void;
  onResetScope: () => void;
  onClearScope: () => void;
  onProceed: () => void;
}

function DriftCounts({
  added,
  modified,
  deleted,
  dim = false,
}: {
  added: number;
  modified: number;
  deleted: number;
  dim?: boolean;
}) {
  if (added === 0 && modified === 0 && deleted === 0) {
    return <span className="mono shrink-0 text-[11px] text-disabled tabular-nums">In sync</span>;
  }
  return (
    <span
      className={cx(
        'mono flex shrink-0 items-center gap-1.5 text-[11px] tabular-nums',
        dim && 'opacity-60',
      )}
    >
      {added > 0 ? <span className="text-status-applied-ink">+{added}</span> : null}
      {modified > 0 ? <span className="text-accent-ink">~{modified}</span> : null}
      {deleted > 0 ? <span className="text-status-blocked-ink">-{deleted}</span> : null}
    </span>
  );
}

function TreeNodeIcon({
  node,
  isOpen = false,
}: {
  node: ReconcileDirectoryNode;
  isOpen?: boolean;
}) {
  if (node.kind === 'file') {
    if (node.added > 0) return <ChangeKindIcon change="added" label="Added" />;
    if (node.modified > 0) return <ChangeKindIcon change="modified" label="Modified" />;
    if (node.deleted > 0) return <ChangeKindIcon change="deleted" label="Deleted" />;
    return <FileText className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden="true" />;
  }
  return isOpen && node.hasChildren ? (
    <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden="true" />
  ) : (
    <Folder className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden="true" />
  );
}

function NodeDriftSummary({ node, dim = false }: { node: ReconcileDirectoryNode; dim?: boolean }) {
  if (node.kind === 'file') {
    if (node.added > 0) {
      return <span className="mono shrink-0 text-[11px] text-status-applied-ink">Added</span>;
    }
    if (node.modified > 0) {
      return <span className="mono shrink-0 text-[11px] text-accent-ink">Modified</span>;
    }
    if (node.deleted > 0) {
      return <span className="mono shrink-0 text-[11px] text-status-blocked-ink">Deleted</span>;
    }
    return <span className="mono shrink-0 text-[11px] text-disabled">In sync</span>;
  }
  return (
    <DriftCounts added={node.added} modified={node.modified} deleted={node.deleted} dim={dim} />
  );
}

export function DirectorySelector({
  snapshot,
  loading,
  selectedFolders,
  savedFolders,
  saving,
  saveError,
  locked,
  consolidatedNotice,
  onDismissConsolidated,
  onToggleFolder,
  onRemoveFolder,
  onSaveScope,
  onResetScope,
  onClearScope,
  onProceed,
}: DirectorySelectorProps) {
  const [splitWidth, setSplitWidth] = useState(500);
  const [search, setSearch] = useState('');
  const [changedOnly, setChangedOnly] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(() => {
    const initial = new Set<string>();
    for (const dir of snapshot.directories) {
      if (dir.kind === 'directory' && dir.depth === 0) initial.add(dir.path);
    }
    for (const folder of selectedFolders) {
      const parts = folder.split('/');
      for (let i = 1; i < parts.length; i += 1) {
        initial.add(parts.slice(0, i).join('/'));
      }
    }
    return initial;
  });
  const [focusedPath, setFocusedPath] = useState<string | null>(null);
  const treeRef = useRef<HTMLDivElement>(null);

  const byPath = useMemo(
    () => new Map<string, ReconcileDirectoryNode>(snapshot.directories.map((d) => [d.path, d])),
    [snapshot.directories],
  );
  const selectedSet = useMemo(() => new Set(selectedFolders), [selectedFolders]);
  const visibleNodes = useMemo(
    () => visibleTreeNodes(snapshot.directories, expanded, search, changedOnly),
    [snapshot.directories, expanded, search, changedOnly],
  );

  const dirty = !scopesEqual(selectedFolders, savedFolders);
  const missingFolders = useMemo(
    () => selectedFolders.filter((folder) => !byPath.has(folder)),
    [selectedFolders, byPath],
  );

  const aggregate = useMemo(() => {
    let added = 0;
    let modified = 0;
    let deleted = 0;
    let unchanged = 0;
    for (const folder of selectedFolders) {
      const node = byPath.get(folder);
      if (!node) continue;
      added += node.added;
      modified += node.modified;
      deleted += node.deleted;
      unchanged += node.unchanged;
    }
    return {
      added,
      modified,
      deleted,
      unchanged,
      changes: added + modified + deleted,
    };
  }, [selectedFolders, byPath]);

  const toggleExpand = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const expandAll = () => {
    setExpanded(new Set(snapshot.directories.filter((d) => d.hasChildren).map((d) => d.path)));
  };

  const collapseAll = () => {
    setExpanded(new Set());
  };

  const handleTreeKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (visibleNodes.length === 0) return;
    const currentIndex = focusedPath ? visibleNodes.findIndex((n) => n.path === focusedPath) : -1;
    const active = currentIndex >= 0 ? visibleNodes[currentIndex] : visibleNodes[0];
    if (!active) return;

    if (event.key === 'ArrowDown') {
      event.preventDefault();
      const next = visibleNodes[Math.min(visibleNodes.length - 1, currentIndex + 1)];
      if (next) setFocusedPath(next.path);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      const prev =
        currentIndex <= 0 ? visibleNodes[0] : visibleNodes[Math.max(0, currentIndex - 1)];
      if (prev) setFocusedPath(prev.path);
    } else if (event.key === 'ArrowRight') {
      event.preventDefault();
      if (active.hasChildren && !expanded.has(active.path)) {
        toggleExpand(active.path);
      } else if (active.hasChildren) {
        const firstChild = visibleNodes[currentIndex + 1];
        if (firstChild && firstChild.parent === active.path) {
          setFocusedPath(firstChild.path);
        }
      }
    } else if (event.key === 'ArrowLeft') {
      event.preventDefault();
      if (active.hasChildren && expanded.has(active.path)) {
        toggleExpand(active.path);
      } else if (active.parent) {
        setFocusedPath(active.parent);
      }
    } else if (event.key === ' ' || event.key === 'Enter') {
      event.preventDefault();
      if (!locked && !isCoveredByAncestor(active.path, selectedFolders)) {
        onToggleFolder(active.path);
      }
    }
  };

  const canProceed =
    selectedFolders.length > 0 &&
    missingFolders.length === 0 &&
    aggregate.changes > 0 &&
    !locked &&
    !loading;

  return (
    <div className="flex min-h-0 flex-1 flex-col" aria-busy={loading}>
      {loading ? (
        <div
          role="progressbar"
          aria-label="Refreshing source tree"
          className="h-[2px] w-full shrink-0 overflow-hidden bg-accent-soft"
        >
          <div className="h-full w-1/2 animate-pulse bg-accent" />
        </div>
      ) : null}
      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        {/* Left panel: browsable source directory and file tree */}
        <section
          aria-label="Source directory"
          style={splitWidthStyle(splitWidth)}
          className="flex min-h-0 flex-col border-b border-line bg-surface md:w-[var(--split-width)] md:shrink-0 md:border-r md:border-b-0"
        >
          <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h2 className="truncate text-xs font-semibold text-fg">Source tree</h2>
                {loading ? (
                  <span className="flex shrink-0 items-center gap-1 text-[11px] text-muted">
                    <Spinner className="h-3 w-3" />
                    Updating…
                  </span>
                ) : null}
              </div>
              <p
                className="mono truncate text-[11px] text-muted"
                title={`${snapshot.sourceRepo} · ${snapshot.sourceBranch} · ${snapshot.sourceSha}`}
              >
                {snapshot.sourceRepo} · {snapshot.sourceBranch} · {snapshot.sourceSha.slice(0, 7)}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-0.5">
              <IconButton
                icon={ChevronsUpDown}
                label="Expand all folders"
                onClick={expandAll}
                disabled={loading || snapshot.directories.length === 0}
              />
              <IconButton
                icon={ChevronsDownUp}
                label="Collapse all folders"
                onClick={collapseAll}
                disabled={loading || expanded.size === 0}
              />
            </div>
          </div>

          <div className="flex items-center gap-2 border-b border-line px-2.5 py-2">
            <div className="relative min-w-0 flex-1">
              <Search
                className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                aria-hidden="true"
              />
              <input
                type="search"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                placeholder="Filter folders and files…"
                aria-label="Filter folders and files"
                className="input pl-8"
              />
            </div>
            <button
              type="button"
              aria-pressed={changedOnly}
              onClick={() => setChangedOnly((value) => !value)}
              className={cx(
                'h-[25px] shrink-0 rounded-md border px-2 text-[11px] transition-colors',
                changedOnly
                  ? 'border-accent bg-accent-soft text-accent-ink'
                  : 'border-line bg-surface text-muted hover:bg-hover hover:text-fg',
              )}
              title="Show only folders or files with differences against the destination"
            >
              Changed only
            </button>
          </div>

          <div
            ref={treeRef}
            role="tree"
            aria-label="Source repository directories and files"
            tabIndex={0}
            onKeyDown={handleTreeKeyDown}
            className={cx(
              'scroll-area min-h-0 flex-1 transition-opacity focus:outline-none',
              loading && 'pointer-events-none opacity-55',
            )}
          >
            {visibleNodes.length === 0 ? (
              <EmptyState
                icon={Search}
                title={
                  snapshot.directories.length === 0
                    ? 'No folders or files found'
                    : 'No items match the current filter'
                }
                message={
                  changedOnly
                    ? 'All folders and files match the destination, or turn off "Changed only" to inspect everything.'
                    : 'Clear the filter to see all repository folders and files.'
                }
                action={
                  search || changedOnly ? (
                    <Button
                      size="sm"
                      onClick={() => {
                        setSearch('');
                        setChangedOnly(false);
                      }}
                    >
                      Reset filters
                    </Button>
                  ) : undefined
                }
              />
            ) : (
              <div className="divide-y divide-line/60">
                {visibleNodes.map((node) => {
                  const isOpen = search.trim().length > 0 || expanded.has(node.path);
                  const directlySelected = selectedSet.has(node.path);
                  const covered = isCoveredByAncestor(node.path, selectedFolders);
                  const focused = focusedPath === node.path;
                  const drift = folderDriftTotal(node);

                  return (
                    <div
                      key={node.path}
                      role="treeitem"
                      aria-expanded={node.hasChildren ? isOpen : undefined}
                      aria-selected={directlySelected}
                      aria-level={node.depth + 1}
                      onClick={() => {
                        setFocusedPath(node.path);
                        treeRef.current?.focus();
                      }}
                      className={cx(
                        'row-hover flex h-[29px] items-center gap-1.5 pr-2.5 select-none',
                        directlySelected && 'bg-selection',
                        focused && 'outline outline-1 -outline-offset-1 outline-accent',
                        covered && 'opacity-65',
                      )}
                      style={{ paddingLeft: `${8 + node.depth * 14}px` }}
                      title={node.path}
                    >
                      {node.hasChildren ? (
                        <button
                          type="button"
                          tabIndex={-1}
                          aria-label={isOpen ? `Collapse ${node.name}` : `Expand ${node.name}`}
                          onClick={(event) => {
                            event.stopPropagation();
                            toggleExpand(node.path);
                          }}
                          className="flex h-5 w-4 shrink-0 items-center justify-center text-muted hover:text-fg"
                        >
                          {isOpen ? (
                            <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
                          ) : (
                            <ChevronRight className="h-3.5 w-3.5" aria-hidden="true" />
                          )}
                        </button>
                      ) : (
                        <span className="h-5 w-4 shrink-0" aria-hidden="true" />
                      )}

                      <span
                        onClick={(event) => event.stopPropagation()}
                        className="flex shrink-0 items-center"
                      >
                        <Checkbox
                          checked={directlySelected || covered}
                          disabled={locked || covered}
                          onChange={() => onToggleFolder(node.path)}
                          label={
                            covered
                              ? `${node.path} (covered by selected parent)`
                              : `Select ${node.path}`
                          }
                        />
                      </span>

                      <TreeNodeIcon node={node} isOpen={isOpen} />

                      <button
                        type="button"
                        tabIndex={-1}
                        disabled={locked || covered}
                        onClick={() => onToggleFolder(node.path)}
                        onDoubleClick={() => {
                          if (node.hasChildren) toggleExpand(node.path);
                        }}
                        className="mono min-w-0 flex-1 truncate text-left text-xs text-fg disabled:cursor-default"
                      >
                        {node.name}
                      </button>

                      {covered ? (
                        <span className="shrink-0 text-[11px] text-muted">Included</span>
                      ) : (
                        <NodeDriftSummary node={node} dim={drift === 0} />
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </section>

        <SplitResizer
          width={splitWidth}
          onWidthChange={setSplitWidth}
          label="Resize directory selector panels"
          min={320}
          max={820}
        />

        {/* Right panel: folders queued for reconciliation & saved scope */}
        <section
          aria-label="Folders to reconcile"
          className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface"
        >
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-3 py-2">
            <div className="min-w-0">
              <h2 className="truncate text-xs font-semibold text-fg">Selected to reconcile</h2>
              <p className="truncate text-[11px] text-muted">
                {selectedFolders.length === 0
                  ? savedFolders.length === 0
                    ? 'Empty · nothing saved yet'
                    : `0 selected · ${savedFolders.length} saved`
                  : `${selectedFolders.length} selected · ${dirty ? 'Unsaved changes' : 'Saved'}`}
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <Button
                variant="ghost"
                size="sm"
                icon={RotateCcw}
                disabled={!dirty || saving || locked}
                onClick={onResetScope}
                title="Revert to saved selection"
              >
                Reset
              </Button>
              <Button
                variant="ghost"
                size="sm"
                icon={Trash2}
                disabled={selectedFolders.length === 0 || saving || locked}
                onClick={onClearScope}
                title="Clear selection"
              >
                Clear
              </Button>
              <Button
                variant="secondary"
                size="sm"
                icon={Save}
                disabled={!dirty || saving || locked}
                loading={saving}
                onClick={onSaveScope}
                title="Save this selection as the default for this subsystem"
              >
                Save
              </Button>
            </div>
          </div>

          <div
            className={cx(
              'scroll-area min-h-0 flex-1 transition-opacity',
              loading && 'pointer-events-none opacity-55',
            )}
          >
            {saveError ? (
              <div className="border-b border-line p-2.5">
                <InlineAlert tone="danger" title="Could not save selection">
                  {saveError}
                </InlineAlert>
              </div>
            ) : null}

            {consolidatedNotice ? (
              <div className="flex items-center justify-between gap-2 border-b border-line bg-accent-soft px-3 py-1.5 text-[11px] text-accent-ink">
                <span className="min-w-0 truncate">{consolidatedNotice}</span>
                <button
                  type="button"
                  onClick={onDismissConsolidated}
                  className="shrink-0 text-accent-ink underline"
                >
                  Dismiss
                </button>
              </div>
            ) : null}

            {selectedFolders.length === 0 ? (
              <EmptyState
                icon={Folder}
                title="No folders or files selected"
                message="Pick folders or individual files from the source tree on the left. Click Save to auto-load this selection next time."
              />
            ) : (
              <ul className="divide-y divide-line">
                {selectedFolders.map((folder) => {
                  const node = byPath.get(folder);
                  const missing = !node;
                  return (
                    <li key={folder} className="row-hover flex h-[29px] items-center gap-2 px-3">
                      {missing ? (
                        <Folder
                          className="h-3.5 w-3.5 shrink-0 text-status-blocked-ink"
                          aria-hidden="true"
                        />
                      ) : (
                        <TreeNodeIcon node={node} />
                      )}
                      <PathLabel path={folder} className="min-w-0 flex-1 text-xs" />

                      {missing ? (
                        <span
                          className="flex shrink-0 items-center gap-1 text-[11px] text-status-blocked-ink"
                          title={`'${folder}' was not found on ${snapshot.sourceBranch}`}
                        >
                          <span
                            className="h-1.5 w-1.5 rounded-full bg-status-blocked"
                            aria-hidden="true"
                          />
                          Missing in source
                        </span>
                      ) : (
                        <NodeDriftSummary node={node} />
                      )}

                      <IconButton
                        icon={X}
                        label={`Remove ${folder}`}
                        disabled={locked}
                        onClick={() => onRemoveFolder(folder)}
                      />
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </section>
      </div>

      {/* Footer action bar */}
      <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
        <div className="flex min-w-0 flex-wrap items-center gap-2 text-[11px] text-muted">
          {selectedFolders.length === 0 ? (
            <span>Select one or more folders or files to preview exact mirror changes.</span>
          ) : missingFolders.length > 0 ? (
            <span className="text-status-blocked-ink">
              Remove or update missing source path(s) before reviewing changes.
            </span>
          ) : aggregate.changes === 0 ? (
            <span>
              {selectedFolders.length} selected · already in sync ({aggregate.unchanged} unchanged)
            </span>
          ) : (
            <span className="mono tabular-nums">
              {selectedFolders.length} selected ·{' '}
              <span className="text-status-applied-ink">+{aggregate.added} added</span> ·{' '}
              <span className="text-accent-ink">~{aggregate.modified} overwritten</span> ·{' '}
              <span className="text-status-blocked-ink">-{aggregate.deleted} deleted</span> ·{' '}
              {aggregate.unchanged} unchanged
            </span>
          )}
        </div>
        <Button
          variant="primary"
          trailingIcon={ArrowRight}
          disabled={!canProceed}
          onClick={onProceed}
        >
          Review changes
        </Button>
      </footer>
    </div>
  );
}

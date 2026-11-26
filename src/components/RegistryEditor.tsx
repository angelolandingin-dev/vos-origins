import { useMemo, useRef, useState } from 'react';
import {
  ArrowUpRight,
  ChevronRight,
  ExternalLink,
  FolderGit2,
  GitBranch,
  Plus,
  RefreshCw,
  Save,
  Search,
  Trash2,
} from 'lucide-react';

import { api, asApiError, type ApiError } from '../api/invoke';
import type {
  RegistryConfig,
  RegistrySnapshot,
  RegistrySubsystem,
  SubsystemSummary,
} from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { CONNECTIVITY_DOT } from './subsystemConnectivity';
import { cx } from './ui/cx';
import { formatDateTime } from './ui/format';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  IconButton,
  InlineAlert,
  LoadingState,
  Tabs,
} from './ui/primitives';
import { splitWidthStyle } from './ui/split-pane';
import { SplitResizer } from './ui/split-resizer';

type RegistryTab = 'subsystems' | 'main';
type StatusFilter = 'all' | 'enabled' | 'disabled';

function splitList(value: string): string[] {
  return value
    .split(',')
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

function splitLines(value: string): string[] {
  return value
    .split('\n')
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

function ListField({
  label,
  hint,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  hint?: string;
  value: string[];
  placeholder?: string;
  onChange: (next: string[]) => void;
}) {
  const [draft, setDraft] = useState(value.join(', '));
  return (
    <div className="space-y-1.5">
      <label className="block">
        <span className="text-xs font-medium text-fg">{label}</span>
        {hint ? <span className="mt-0.5 block text-[11px] text-muted">{hint}</span> : null}
        <input
          className="input mono mt-1.5"
          value={draft}
          placeholder={placeholder}
          spellCheck={false}
          autoComplete="off"
          onChange={(event) => {
            setDraft(event.target.value);
            onChange(splitList(event.target.value));
          }}
        />
      </label>
      {value.length > 0 ? (
        <div className="flex flex-wrap gap-1">
          {value.map((item) => (
            <span
              key={item}
              className="mono rounded border border-line bg-ground px-1.5 py-0.5 text-[10px] text-muted"
            >
              {item}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function PolicyRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 py-1.5 text-[11px]">
      <dt className="text-muted">{label}</dt>
      <dd className="mono truncate text-right text-muted">{value}</dd>
    </div>
  );
}

const DEFAULT_POLICY: RegistrySubsystem['integrationPolicy'] = {
  applicationFiles: 'selected-commit-delta',
  packageJson: 'merge-dependency-delta',
  lockfiles: 'regenerate-in-target',
  sharedConfiguration: 'review-explicitly',
  githubWorkflows: 'main-owned',
  secretFiles: 'never-import',
};

function newSubsystem(): RegistrySubsystem {
  return {
    id: '',
    name: '',
    repository: '',
    sourceBranch: 'master',
    enabled: true,
    pathStrategy: 'preserve-relative-paths',
    dependencyStrategy: 'reconcile',
    integrationPolicy: { ...DEFAULT_POLICY },
  };
}

interface SubsystemRow {
  key: string;
  entry: RegistrySubsystem;
}

/** Settings → register/configure the main system and its subsystems (spec §4, tool-local). */
export function RegistryEditor() {
  const app = useApp();
  const snapshot = useAsyncData(() => api.getRegistry(), [app.dataVersion]);

  if (snapshot.loading && !snapshot.data) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <LoadingState label="Reading the registry…" />
      </div>
    );
  }

  if (snapshot.error) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <ErrorState error={snapshot.error} onRetry={() => snapshot.reload()} compact />
      </div>
    );
  }

  if (snapshot.data) {
    return (
      <RegistryForm
        key={JSON.stringify(snapshot.data.config)}
        snapshot={snapshot.data}
        onReload={() => snapshot.reload()}
        onSaved={() => {
          snapshot.reload();
          app.bumpDataVersion();
        }}
      />
    );
  }

  return null;
}

function RegistryForm({
  snapshot,
  onReload,
  onSaved,
}: {
  snapshot: RegistrySnapshot;
  onReload: () => void;
  onSaved: () => void;
}) {
  const app = useApp();
  const [tab, setTab] = useState<RegistryTab>('subsystems');
  const [config, setConfig] = useState<RegistryConfig>(snapshot.config);
  const [globalFilesDraft, setGlobalFilesDraft] = useState(() =>
    (snapshot.config.globalFiles ?? []).join('\n'),
  );
  // Keys are issued once per row and never derived from editable fields, so edits and
  // removes can't reattach state to the wrong row.
  const [rows, setRows] = useState<SubsystemRow[]>(() =>
    snapshot.config.subsystems.map((entry) => ({ key: crypto.randomUUID(), entry })),
  );
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [subsystemFilter, setSubsystemFilter] = useState('');
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');
  const [inspectorWidth, setInspectorWidth] = useState(440);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);
  const idInputRef = useRef<HTMLInputElement>(null);

  const liveById = useMemo(() => {
    const map = new Map<string, SubsystemSummary>();
    for (const item of app.subsystems.data ?? []) {
      map.set(item.id, item);
    }
    return map;
  }, [app.subsystems.data]);

  const mainRepositoryInUse = app.auth.data?.mainRepository ?? config.mainRepository;
  const localPathInEffect = (config.localMainRepositoryPath ?? '').trim();
  const enabledCount = useMemo(() => rows.filter((row) => row.entry.enabled).length, [rows]);
  const disabledCount = rows.length - enabledCount;

  const filteredRows = useMemo(() => {
    const needle = subsystemFilter.trim().toLowerCase();
    return rows.filter(({ entry }) => {
      if (statusFilter === 'enabled' && !entry.enabled) return false;
      if (statusFilter === 'disabled' && entry.enabled) return false;
      if (!needle) return true;
      return `${entry.id} ${entry.name} ${entry.repository} ${entry.sourceBranch}`
        .toLowerCase()
        .includes(needle);
    });
  }, [rows, subsystemFilter, statusFilter]);

  const activeRow = useMemo(() => {
    if (selectedKey) {
      const match = rows.find((row) => row.key === selectedKey);
      if (match) return match;
    }
    return filteredRows[0] ?? rows[0] ?? null;
  }, [rows, filteredRows, selectedKey]);

  const activeLive = activeRow?.entry.id ? (liveById.get(activeRow.entry.id) ?? null) : null;

  const isDirty = useMemo(() => {
    const payload: RegistryConfig = { ...config, subsystems: rows.map((row) => row.entry) };
    return JSON.stringify(payload) !== JSON.stringify(snapshot.config);
  }, [config, rows, snapshot.config]);

  const patchRow = (key: string, patch: Partial<RegistrySubsystem>) => {
    setRows((current) =>
      current.map((row) => (row.key === key ? { ...row, entry: { ...row.entry, ...patch } } : row)),
    );
  };

  const addRow = () => {
    const key = crypto.randomUUID();
    setTab('subsystems');
    setSubsystemFilter('');
    setStatusFilter('all');
    setRows((current) => [...current, { key, entry: newSubsystem() }]);
    setSelectedKey(key);
    requestAnimationFrame(() => idInputRef.current?.focus());
  };

  const removeRow = (key: string) => {
    setRows((current) => {
      const idx = current.findIndex((row) => row.key === key);
      const next = current.filter((row) => row.key !== key);
      if (selectedKey === key || activeRow?.key === key) {
        const fallback = next[Math.min(idx, next.length - 1)]?.key ?? null;
        setSelectedKey(fallback);
      }
      return next;
    });
  };

  const save = () => {
    setSaving(true);
    setError(null);
    const payload: RegistryConfig = { ...config, subsystems: rows.map((row) => row.entry) };
    void api
      .saveRegistry(payload)
      .then(() => {
        app.pushNotice({
          tone: 'success',
          title: 'Registry saved',
        });
        onSaved();
      })
      .catch((err: unknown) => setError(asApiError(err)))
      .finally(() => setSaving(false));
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-surface">
      <div className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line bg-raised px-5 py-2">
        <Tabs
          ariaLabel="Registry sections"
          value={tab}
          onChange={(id) => setTab(id as RegistryTab)}
          className="h-[32px] border-b-0 px-0"
          items={[
            { id: 'subsystems', label: 'Subsystems', count: rows.length },
            { id: 'main', label: 'Main system & policy' },
          ]}
        />

        <div className="flex shrink-0 items-center gap-2.5">
          <button
            type="button"
            onClick={() => setTab('main')}
            title={`Main repository: ${mainRepositoryInUse || 'unset'}`}
            className="mono hidden items-center gap-1.5 rounded-md border border-line bg-surface px-2.5 py-1 text-[11px] text-muted transition-colors hover:border-line-strong hover:text-fg lg:inline-flex"
          >
            <FolderGit2 className="h-3.5 w-3.5 text-disabled" aria-hidden="true" />
            <span className="max-w-[280px] truncate">{mainRepositoryInUse || 'main unset'}</span>
          </button>
          <Button size="sm" variant="secondary" icon={Plus} onClick={addRow}>
            Add subsystem
          </Button>
        </div>
      </div>

      {tab === 'subsystems' ? (
        <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
          {/* Left: full-bleed subsystem table */}
          <section
            aria-label="Registered subsystems"
            className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface"
          >
            <div className="flex h-[42px] shrink-0 items-center justify-between gap-2 border-b border-line bg-surface px-4">
              <div className="relative w-full max-w-[320px] min-w-[180px] flex-1">
                <Search
                  className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                  aria-hidden="true"
                />
                <input
                  type="search"
                  value={subsystemFilter}
                  onChange={(event) => setSubsystemFilter(event.target.value)}
                  placeholder="Filter by name, id, repository, or branch…"
                  aria-label="Filter subsystems"
                  className="input pl-8"
                />
              </div>

              <div
                role="group"
                aria-label="Filter by enabled status"
                className="flex h-[25px] shrink-0 overflow-hidden rounded-md border border-line bg-surface"
              >
                {(
                  [
                    { id: 'all', label: 'All', count: rows.length },
                    { id: 'enabled', label: 'Enabled', count: enabledCount },
                    { id: 'disabled', label: 'Disabled', count: disabledCount },
                  ] as const
                ).map((item, idx) => (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setStatusFilter(item.id)}
                    className={cx(
                      'px-2.5 text-[11px] tabular-nums transition-colors',
                      idx > 0 && 'border-l border-line',
                      statusFilter === item.id
                        ? 'bg-raised font-semibold text-fg'
                        : 'text-muted hover:bg-hover hover:text-fg',
                    )}
                  >
                    {item.label} ({item.count})
                  </button>
                ))}
              </div>
            </div>

            <div className="scroll-area min-h-0 flex-1">
              {rows.length === 0 ? (
                <EmptyState
                  icon={FolderGit2}
                  title="No subsystems registered"
                  message="Add a subsystem repository to browse commits, promote changes, and reconcile files."
                  action={
                    <Button size="sm" variant="secondary" icon={Plus} onClick={addRow}>
                      Add subsystem
                    </Button>
                  }
                />
              ) : filteredRows.length === 0 ? (
                <EmptyState
                  icon={Search}
                  title="No matching subsystems"
                  message="Clear the search or status filter to see all registered subsystems."
                  action={
                    <Button
                      size="sm"
                      onClick={() => {
                        setSubsystemFilter('');
                        setStatusFilter('all');
                      }}
                    >
                      Reset filters
                    </Button>
                  }
                />
              ) : (
                <table className="w-full border-separate border-spacing-0 text-xs">
                  <thead className="sticky top-0 z-10">
                    <tr className="table-head">
                      <th scope="col" className="w-10 border-b border-line py-2 pl-4 text-left">
                        <span className="sr-only">Enabled</span>
                      </th>
                      <th scope="col" className="border-b border-line py-2 pr-4 text-left">
                        Subsystem
                      </th>
                      <th scope="col" className="border-b border-line py-2 pr-4 text-left">
                        Repository
                      </th>
                      <th scope="col" className="w-28 border-b border-line py-2 pr-4 text-left">
                        Branch
                      </th>
                      <th scope="col" className="w-28 border-b border-line py-2 pr-4 text-left">
                        Status
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {filteredRows.map((row) => {
                      const { entry } = row;
                      const selected = activeRow?.key === row.key;
                      const label = entry.name || entry.id || 'New subsystem';
                      const live = entry.id ? liveById.get(entry.id) : undefined;
                      const connectivity = live ? CONNECTIVITY_DOT[live.connectivity] : null;

                      return (
                        <tr
                          key={row.key}
                          aria-selected={selected}
                          onClick={() => setSelectedKey(row.key)}
                          className={cx(
                            'row-hover cursor-pointer transition-colors',
                            selected && 'bg-selection',
                            !entry.enabled && 'opacity-60',
                          )}
                        >
                          <td
                            className="h-[36px] border-b border-line py-0 pl-4 align-middle"
                            onClick={(event) => event.stopPropagation()}
                          >
                            <Checkbox
                              label={`Enabled: ${label}`}
                              checked={entry.enabled}
                              onChange={(event) =>
                                patchRow(row.key, { enabled: event.target.checked })
                              }
                            />
                          </td>
                          <td className="h-[36px] border-b border-line py-0 pr-4 align-middle">
                            <div className="flex items-center gap-2">
                              <span className="truncate font-medium text-fg">{label}</span>
                              {entry.id ? (
                                <span className="mono shrink-0 rounded border border-line bg-ground px-1.5 py-0.5 text-[10px] text-accent-ink">
                                  {entry.id}
                                </span>
                              ) : (
                                <span className="mono shrink-0 rounded border border-status-unknown/40 bg-status-unknown-soft px-1.5 py-0.5 text-[10px] text-status-unknown-ink">
                                  draft
                                </span>
                              )}
                            </div>
                          </td>
                          <td className="mono h-[36px] border-b border-line py-0 pr-4 align-middle text-[11px] text-muted">
                            <span className="block truncate" title={entry.repository}>
                              {entry.repository || '—'}
                            </span>
                          </td>
                          <td className="mono h-[36px] border-b border-line py-0 pr-4 align-middle text-[11px] text-muted">
                            <span className="inline-flex items-center gap-1.5">
                              <GitBranch
                                className="h-3 w-3 shrink-0 text-disabled"
                                aria-hidden="true"
                              />
                              {entry.sourceBranch || '—'}
                            </span>
                          </td>
                          <td className="h-[36px] border-b border-line py-0 pr-4 align-middle text-[11px] text-muted">
                            <span className="inline-flex items-center gap-1.5">
                              <span
                                aria-hidden="true"
                                className={cx(
                                  'h-1.5 w-1.5 shrink-0 rounded-full',
                                  !entry.enabled
                                    ? 'bg-disabled'
                                    : connectivity
                                      ? connectivity.dot
                                      : 'bg-accent',
                                )}
                              />
                              {!entry.enabled
                                ? 'Disabled'
                                : connectivity
                                  ? connectivity.label
                                  : 'Draft'}
                            </span>
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              )}
            </div>
          </section>

          <SplitResizer
            edge="right"
            width={inspectorWidth}
            onWidthChange={setInspectorWidth}
            label="Resize subsystem inspector"
            min={360}
            max={580}
          />

          {/* Right: always-present subsystem inspector */}
          <aside
            aria-label="Subsystem inspector"
            style={splitWidthStyle(inspectorWidth)}
            className="flex min-h-0 flex-col border-t border-line bg-surface lg:w-[var(--split-width)] lg:shrink-0 lg:border-t-0 lg:border-l"
          >
            {activeRow ? (
              <>
                <div className="flex h-[42px] shrink-0 items-center justify-between gap-2 border-b border-line bg-surface px-4">
                  <div className="flex min-w-0 flex-1 items-center gap-2">
                    <span
                      aria-hidden="true"
                      className={cx(
                        'h-2 w-2 shrink-0 rounded-full',
                        !activeRow.entry.enabled
                          ? 'bg-disabled'
                          : activeLive
                            ? CONNECTIVITY_DOT[activeLive.connectivity].dot
                            : 'bg-accent',
                      )}
                    />
                    <h2 className="truncate text-xs font-semibold text-fg">
                      {activeRow.entry.name || activeRow.entry.id || 'New subsystem'}
                    </h2>
                  </div>

                  <div className="flex shrink-0 items-center gap-1">
                    {activeLive && activeRow.entry.enabled ? (
                      <Button
                        size="sm"
                        variant="ghost"
                        trailingIcon={ArrowUpRight}
                        onClick={() => {
                          app.selectSubsystem(activeLive.id);
                          app.navigate('commit-browser');
                        }}
                      >
                        Browse
                      </Button>
                    ) : null}
                    {activeRow.entry.repository ? (
                      <IconButton
                        icon={ExternalLink}
                        label="Open repository on GitHub"
                        onClick={() =>
                          app.openExternal(
                            `https://github.com/${activeRow.entry.repository}/tree/${activeRow.entry.sourceBranch || 'master'}`,
                          )
                        }
                      />
                    ) : null}
                  </div>
                </div>

                <div className="scroll-area min-h-0 flex-1 divide-y divide-line">
                  <div className="space-y-3.5 px-4 py-4">
                    <p className="truncate text-[11px] text-muted">
                      {activeLive
                        ? `${CONNECTIVITY_DOT[activeLive.connectivity].label} · verified ${formatDateTime(activeLive.lastSyncedAt)}`
                        : 'Unsaved draft — save registry to apply'}
                    </p>

                    <div className="grid grid-cols-[110px_minmax(0,1fr)] gap-2.5">
                      <label className="block">
                        <span className="text-[11px] font-medium text-muted">Id</span>
                        <input
                          ref={idInputRef}
                          className="input mono mt-1"
                          value={activeRow.entry.id}
                          spellCheck={false}
                          autoComplete="off"
                          placeholder="hrm"
                          onChange={(event) => patchRow(activeRow.key, { id: event.target.value })}
                        />
                      </label>
                      <label className="block">
                        <span className="text-[11px] font-medium text-muted">Display name</span>
                        <input
                          className="input mt-1"
                          value={activeRow.entry.name}
                          spellCheck={false}
                          autoComplete="off"
                          placeholder="Human Resource Management"
                          onChange={(event) =>
                            patchRow(activeRow.key, { name: event.target.value })
                          }
                        />
                      </label>
                    </div>

                    <label className="block">
                      <span className="text-[11px] font-medium text-muted">
                        Repository (owner/name)
                      </span>
                      <input
                        className="input mono mt-1"
                        value={activeRow.entry.repository}
                        spellCheck={false}
                        autoComplete="off"
                        placeholder="ORG/hr-management"
                        onChange={(event) =>
                          patchRow(activeRow.key, { repository: event.target.value })
                        }
                      />
                    </label>

                    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-end gap-3">
                      <label className="block">
                        <span className="text-[11px] font-medium text-muted">Source branch</span>
                        <input
                          className="input mono mt-1"
                          value={activeRow.entry.sourceBranch}
                          spellCheck={false}
                          autoComplete="off"
                          placeholder="master"
                          onChange={(event) =>
                            patchRow(activeRow.key, { sourceBranch: event.target.value })
                          }
                        />
                      </label>

                      <label className="flex h-[27px] cursor-pointer items-center gap-2 rounded-md border border-line bg-raised/50 px-2.5 select-none">
                        <Checkbox
                          checked={activeRow.entry.enabled}
                          onChange={(event) =>
                            patchRow(activeRow.key, { enabled: event.target.checked })
                          }
                          label="Enable this subsystem"
                        />
                        <span className="text-[11px] font-medium text-fg">Enabled</span>
                      </label>
                    </div>
                  </div>

                  <details className="group">
                    <summary className="flex h-[36px] cursor-pointer items-center justify-between gap-2 px-4 text-[11px] text-muted select-none hover:bg-hover hover:text-fg">
                      <span className="flex items-center gap-1.5 font-medium">
                        <ChevronRight
                          className="h-3.5 w-3.5 text-disabled transition-transform group-open:rotate-90"
                          aria-hidden="true"
                        />
                        Integration policy
                      </span>
                      <span className="mono text-[11px] text-disabled">
                        {activeRow.entry.pathStrategy}
                      </span>
                    </summary>
                    <dl className="divide-y divide-line/60 border-t border-line/60 bg-raised/20 px-4 py-1">
                      <PolicyRow label="Path strategy" value={activeRow.entry.pathStrategy} />
                      <PolicyRow
                        label="Dependency strategy"
                        value={activeRow.entry.dependencyStrategy}
                      />
                      <PolicyRow
                        label="Application files"
                        value={activeRow.entry.integrationPolicy.applicationFiles}
                      />
                      <PolicyRow
                        label="package.json"
                        value={activeRow.entry.integrationPolicy.packageJson}
                      />
                      <PolicyRow
                        label="Lockfiles"
                        value={activeRow.entry.integrationPolicy.lockfiles}
                      />
                      <PolicyRow
                        label="GitHub workflows"
                        value={activeRow.entry.integrationPolicy.githubWorkflows}
                      />
                      <PolicyRow
                        label="Secret files"
                        value={activeRow.entry.integrationPolicy.secretFiles}
                      />
                    </dl>
                  </details>

                  <div className="flex items-center justify-between gap-3 px-4 py-3">
                    <span className="text-[11px] text-muted">Remove from local registry</span>
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={Trash2}
                      onClick={() => removeRow(activeRow.key)}
                      className="text-status-blocked-ink hover:bg-status-blocked-soft hover:text-status-blocked-ink"
                    >
                      Remove
                    </Button>
                  </div>
                </div>
              </>
            ) : (
              <EmptyState
                icon={FolderGit2}
                title="No subsystem selected"
                message="Pick a subsystem on the left to inspect or edit it, or add a new one."
              />
            )}
          </aside>
        </div>
      ) : (
        <div className="scroll-area min-h-0 flex-1">
          <div className="border-b border-line bg-raised/40 px-5 py-2 text-[11px] text-muted">
            Tool-local configuration — nothing is written into any repository.
          </div>

          <section aria-label="Main system" className="border-b border-line">
            <div className="border-b border-line/60 bg-raised/40 px-5 py-2">
              <h2 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                Main system
              </h2>
              <p className="mt-0.5 text-[11px] text-muted">
                Target repository and local clone used for promotions, diffs, and reconciliation.
              </p>
            </div>
            <div className="max-w-3xl space-y-4 px-5 py-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <label className="block">
                  <span className="text-xs font-medium text-fg">Main repository (owner/name)</span>
                  <input
                    className="input mono mt-1.5"
                    value={config.mainRepository}
                    spellCheck={false}
                    autoComplete="off"
                    onChange={(event) =>
                      setConfig((current) => ({ ...current, mainRepository: event.target.value }))
                    }
                  />
                </label>

                <label className="block">
                  <span className="text-xs font-medium text-fg">Local main repository clone</span>
                  <input
                    className="input mono mt-1.5"
                    value={config.localMainRepositoryPath ?? ''}
                    spellCheck={false}
                    autoComplete="off"
                    placeholder="C:\Users\you\repos\main-system"
                    onChange={(event) =>
                      setConfig((current) => ({
                        ...current,
                        localMainRepositoryPath: event.target.value.trim()
                          ? event.target.value
                          : null,
                      }))
                    }
                  />
                </label>
              </div>

              <p className="mono truncate text-[11px] text-muted">
                Active target: {mainRepositoryInUse || 'unknown'} ·{' '}
                {localPathInEffect || 'managed checkout'}
              </p>

              <label className="block">
                <span className="text-xs font-medium text-fg">Global files (main-owned)</span>
                <span className="mt-0.5 block text-[11px] text-muted">
                  One path or directory prefix per line. Protected from accidental subsystem
                  overwrites.
                </span>
                <textarea
                  className="input mono mt-1.5 h-auto min-h-[104px] py-1.5"
                  rows={4}
                  value={globalFilesDraft}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={'src/app/layout.tsx\nsrc/components/shared/'}
                  aria-label="Global files (main-owned)"
                  onChange={(event) => {
                    setGlobalFilesDraft(event.target.value);
                    setConfig((current) => ({
                      ...current,
                      globalFiles: splitLines(event.target.value),
                    }));
                  }}
                />
              </label>
            </div>
          </section>

          <section aria-label="Promotion policy">
            <div className="border-b border-line/60 bg-raised/40 px-5 py-2">
              <h2 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                Promotion policy
              </h2>
              <p className="mt-0.5 text-[11px] text-muted">
                Local-first — push and pull request creation are opt-in per promotion.
              </p>
            </div>
            <div className="max-w-3xl space-y-4 px-5 py-4">
              <div className="grid gap-4 sm:grid-cols-2">
                <ListField
                  label="Allowed target branches"
                  hint="Comma-separated branch names allowed as promotion targets."
                  placeholder="master, UAT"
                  value={config.promotion.allowedTargetBranches}
                  onChange={(next) =>
                    setConfig((current) => ({
                      ...current,
                      promotion: { ...current.promotion, allowedTargetBranches: next },
                    }))
                  }
                />

                <ListField
                  label="Allowed branch patterns"
                  hint="Safe glob patterns for release branches, e.g. release/*."
                  placeholder="release/*"
                  value={config.promotion.allowedTargetBranchPatterns}
                  onChange={(next) =>
                    setConfig((current) => ({
                      ...current,
                      promotion: { ...current.promotion, allowedTargetBranchPatterns: next },
                    }))
                  }
                />
              </div>

              <label className="flex items-start gap-2.5 pt-1">
                <Checkbox
                  className="mt-0.5"
                  checked={config.promotion.attributeToAccount}
                  onChange={(event) =>
                    setConfig((current) => ({
                      ...current,
                      promotion: {
                        ...current.promotion,
                        attributeToAccount: event.target.checked,
                      },
                    }))
                  }
                  label="Attribute promotions to my GitHub account"
                />
                <span>
                  <span className="text-xs font-medium text-fg">
                    Attribute promotions to my GitHub account
                  </span>
                  <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                    Authors promotion commits as your signed-in GitHub account (via a GitHub noreply
                    address). Unchecked authors them as the tool&apos;s bot identity.
                  </span>
                </span>
              </label>
            </div>
          </section>
        </div>
      )}

      {error ? (
        <div className="border-t border-line px-4 py-2">
          <InlineAlert tone="danger" title={error.message}>
            {error.remedy ?? 'Check the values and try again.'}
          </InlineAlert>
        </div>
      ) : null}

      <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
        <div className="flex min-w-0 flex-1 items-center gap-2 text-[11px] text-muted">
          <span className="mono truncate" title={snapshot.overridePath}>
            {snapshot.overridePath}
          </span>
          {isDirty ? (
            <span className="shrink-0 font-medium text-accent-ink">· Unsaved changes</span>
          ) : null}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button size="sm" variant="ghost" icon={RefreshCw} onClick={onReload} disabled={saving}>
            Reload
          </Button>
          <Button variant="primary" icon={Save} loading={saving} onClick={save} disabled={saving}>
            {saving ? 'Saving…' : 'Save registry'}
          </Button>
        </div>
      </footer>
    </div>
  );
}

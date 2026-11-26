import { useMemo, useState } from 'react';
import {
  ArrowUpRight,
  ExternalLink,
  FolderGit2,
  GitBranch,
  Plus,
  RefreshCw,
  Save,
  Search,
  ShieldCheck,
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
  KeyValue,
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
  const [inspectorWidth, setInspectorWidth] = useState(420);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);

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
  };

  const removeRow = (key: string) => {
    setRows((current) => {
      const idx = current.findIndex((row) => row.key === key);
      const next = current.filter((row) => row.key !== key);
      if (selectedKey === key) {
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
      <div className="flex min-h-[42px] flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-4 py-1">
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
            className="mono hidden items-center gap-1.5 rounded-md border border-line bg-surface px-2 py-1 text-[11px] text-muted transition-colors hover:border-line-strong hover:text-fg xl:inline-flex"
          >
            <FolderGit2 className="h-3 w-3 text-disabled" aria-hidden="true" />
            <span className="max-w-[260px] truncate">{mainRepositoryInUse || 'main unset'}</span>
          </button>
          <Button size="sm" variant="secondary" icon={Plus} onClick={addRow}>
            Add subsystem
          </Button>
        </div>
      </div>

      {tab === 'subsystems' ? (
        <div className="flex min-h-0 flex-1 flex-col md:flex-row">
          {/* Left: filterable subsystem card grid */}
          <section
            aria-label="Registered subsystems"
            className="flex min-h-0 min-w-0 flex-1 flex-col bg-ground/40"
          >
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface px-3.5 py-2">
              <div className="relative min-w-[200px] flex-1">
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
                      'px-2 text-[11px] tabular-nums transition-colors',
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

            <div className="scroll-area min-h-0 flex-1 p-3.5">
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
                <div
                  role="listbox"
                  aria-label="Subsystems"
                  className="grid gap-2.5 sm:grid-cols-2 2xl:grid-cols-3"
                >
                  {filteredRows.map((row) => {
                    const { entry } = row;
                    const selected = activeRow?.key === row.key;
                    const label = entry.name || entry.id || 'New subsystem';
                    const live = entry.id ? liveById.get(entry.id) : undefined;
                    const connectivity = live ? CONNECTIVITY_DOT[live.connectivity] : null;

                    return (
                      <div
                        key={row.key}
                        role="option"
                        tabIndex={0}
                        aria-selected={selected}
                        onClick={() => setSelectedKey(row.key)}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault();
                            setSelectedKey(row.key);
                          }
                        }}
                        className={cx(
                          'group flex cursor-pointer flex-col rounded-md border transition-colors focus-visible:control-focus',
                          selected
                            ? 'border-accent bg-selection/45'
                            : 'border-line bg-surface hover:border-line-strong hover:bg-raised/60',
                          !entry.enabled && 'opacity-65',
                        )}
                      >
                        <div className="flex items-start justify-between gap-2 px-3 pt-2.5 pb-2">
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-1.5">
                              <span
                                aria-hidden="true"
                                title={connectivity?.label ?? 'Not yet saved'}
                                className={cx(
                                  'h-1.5 w-1.5 shrink-0 rounded-full',
                                  connectivity
                                    ? connectivity.dot
                                    : entry.enabled
                                      ? 'bg-accent'
                                      : 'bg-disabled',
                                )}
                              />
                              <span
                                className="truncate text-xs font-semibold text-fg"
                                title={label}
                              >
                                {label}
                              </span>
                            </div>

                            <div className="mt-1.5 flex items-center gap-1.5">
                              <span
                                className={cx(
                                  'mono inline-flex max-w-[120px] truncate rounded border px-1.5 py-0.5 text-[10px] font-medium',
                                  entry.id
                                    ? 'border-line bg-ground text-accent-ink'
                                    : 'border-status-unknown/40 bg-status-unknown-soft text-status-unknown-ink',
                                )}
                              >
                                {entry.id || 'unsaved'}
                              </span>
                              <span className="mono inline-flex min-w-0 items-center gap-1 truncate text-[11px] text-muted">
                                <GitBranch
                                  className="h-3 w-3 shrink-0 text-disabled"
                                  aria-hidden="true"
                                />
                                <span className="truncate">{entry.sourceBranch || '—'}</span>
                              </span>
                            </div>
                          </div>

                          <span
                            onClick={(event) => event.stopPropagation()}
                            className="flex shrink-0 items-center gap-1"
                          >
                            <Checkbox
                              label={`Enabled: ${label}`}
                              checked={entry.enabled}
                              onChange={(event) =>
                                patchRow(row.key, { enabled: event.target.checked })
                              }
                            />
                          </span>
                        </div>

                        <div className="flex items-center justify-between gap-2 border-t border-line/60 bg-raised/35 px-3 py-1.5">
                          <span
                            className="mono min-w-0 flex-1 truncate text-[11px] text-muted"
                            title={entry.repository || 'No repository configured'}
                          >
                            {entry.repository || 'No repository configured'}
                          </span>
                          <span className="shrink-0 text-[10px] text-disabled">
                            {!entry.enabled
                              ? 'Disabled'
                              : connectivity
                                ? connectivity.label
                                : 'Draft'}
                          </span>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          </section>

          <SplitResizer
            edge="right"
            width={inspectorWidth}
            onWidthChange={setInspectorWidth}
            label="Resize subsystem details panel"
            min={340}
            max={580}
          />

          {/* Right: selected subsystem editor */}
          <section
            aria-label="Subsystem details"
            style={splitWidthStyle(inspectorWidth)}
            className="flex min-h-0 flex-col border-t border-line bg-surface md:w-[var(--split-width)] md:shrink-0 md:border-t-0 md:border-l"
          >
            {activeRow ? (
              <>
                <div className="flex items-center justify-between gap-2 border-b border-line bg-raised px-4 py-2.5">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <h2 className="truncate text-xs font-semibold text-fg">
                        {activeRow.entry.name || activeRow.entry.id || 'New subsystem'}
                      </h2>
                      {activeRow.entry.id ? (
                        <span className="mono shrink-0 rounded border border-line bg-ground px-1.5 py-0.5 text-[10px] text-accent-ink">
                          {activeRow.entry.id}
                        </span>
                      ) : null}
                    </div>
                    <p className="mono mt-0.5 truncate text-[11px] text-muted">
                      {activeRow.entry.repository || 'Configure repository and source branch'}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
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
                    <IconButton
                      icon={Trash2}
                      tone="danger"
                      label={`Remove ${activeRow.entry.name || activeRow.entry.id || 'subsystem'}`}
                      onClick={() => removeRow(activeRow.key)}
                    />
                  </div>
                </div>

                {activeLive ? (
                  <div className="flex items-center justify-between gap-2 border-b border-line bg-raised/35 px-4 py-2 text-[11px]">
                    <span className="flex min-w-0 items-center gap-1.5 text-muted">
                      <span
                        aria-hidden="true"
                        className={cx(
                          'h-1.5 w-1.5 shrink-0 rounded-full',
                          CONNECTIVITY_DOT[activeLive.connectivity].dot,
                        )}
                      />
                      <span className="font-medium text-fg">
                        {CONNECTIVITY_DOT[activeLive.connectivity].label}
                      </span>
                      <span className="truncate">
                        · verified {formatDateTime(activeLive.lastSyncedAt)}
                      </span>
                    </span>
                    {activeRow.entry.enabled ? (
                      <button
                        type="button"
                        onClick={() => {
                          app.selectSubsystem(activeLive.id);
                          app.navigate('commit-browser');
                        }}
                        className="inline-flex shrink-0 items-center gap-1 text-[11px] font-medium text-accent-ink hover:underline"
                      >
                        Browse
                        <ArrowUpRight className="h-3 w-3" aria-hidden="true" />
                      </button>
                    ) : null}
                  </div>
                ) : null}

                <div className="scroll-area min-h-0 flex-1 divide-y divide-line">
                  <div className="space-y-3 p-4">
                    <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                      Identity
                    </h3>
                    <div className="grid gap-3 sm:grid-cols-2">
                      <label className="block">
                        <span className="text-xs font-medium text-fg">Id</span>
                        <input
                          className="input mono mt-1"
                          value={activeRow.entry.id}
                          spellCheck={false}
                          autoComplete="off"
                          placeholder="hr-management"
                          onChange={(event) => patchRow(activeRow.key, { id: event.target.value })}
                        />
                      </label>
                      <label className="block">
                        <span className="text-xs font-medium text-fg">Display name</span>
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
                  </div>

                  <div className="space-y-3 p-4">
                    <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                      Repository
                    </h3>
                    <label className="block">
                      <span className="text-xs font-medium text-fg">Repository (owner/name)</span>
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
                    <label className="block">
                      <span className="text-xs font-medium text-fg">Protected source branch</span>
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
                    <label className="flex items-start gap-2.5 rounded-md border border-line bg-raised/50 px-3 py-2.5">
                      <Checkbox
                        className="mt-0.5"
                        checked={activeRow.entry.enabled}
                        onChange={(event) =>
                          patchRow(activeRow.key, { enabled: event.target.checked })
                        }
                        label="Enable this subsystem"
                      />
                      <span>
                        <span className="text-xs font-medium text-fg">Enabled</span>
                        <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                          Show this subsystem in Commit Browser, Subsystem Diff, Promote,
                          Distribute, and Reconcile.
                        </span>
                      </span>
                    </label>
                  </div>

                  <div className="space-y-2 p-4">
                    <div className="flex items-center gap-1.5">
                      <ShieldCheck className="h-3.5 w-3.5 text-disabled" aria-hidden="true" />
                      <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                        Integration policy
                      </h3>
                    </div>
                    <dl className="divide-y divide-line border-y border-line">
                      <KeyValue label="Path strategy">
                        <span className="mono">{activeRow.entry.pathStrategy}</span>
                      </KeyValue>
                      <KeyValue label="Dependency strategy">
                        <span className="mono">{activeRow.entry.dependencyStrategy}</span>
                      </KeyValue>
                      <KeyValue label="Application files">
                        <span className="mono">
                          {activeRow.entry.integrationPolicy.applicationFiles}
                        </span>
                      </KeyValue>
                      <KeyValue label="package.json">
                        <span className="mono">
                          {activeRow.entry.integrationPolicy.packageJson}
                        </span>
                      </KeyValue>
                      <KeyValue label="Lockfiles">
                        <span className="mono">{activeRow.entry.integrationPolicy.lockfiles}</span>
                      </KeyValue>
                      <KeyValue label="GitHub workflows">
                        <span className="mono">
                          {activeRow.entry.integrationPolicy.githubWorkflows}
                        </span>
                      </KeyValue>
                      <KeyValue label="Secret files">
                        <span className="mono">
                          {activeRow.entry.integrationPolicy.secretFiles}
                        </span>
                      </KeyValue>
                    </dl>
                  </div>
                </div>
              </>
            ) : (
              <EmptyState
                icon={FolderGit2}
                title="No subsystem selected"
                message="Pick a subsystem card on the left to edit its repository and source branch, or add a new one."
              />
            )}
          </section>
        </div>
      ) : (
        <div className="scroll-area min-h-0 flex-1 bg-ground/40">
          <div className="border-b border-line bg-surface px-5 py-2.5 text-[11px] text-muted">
            Tool-local configuration — nothing is written into any repository.
          </div>

          <div className="grid gap-5 p-5 xl:grid-cols-2">
            <section
              aria-label="Main system"
              className="flex flex-col rounded-md border border-line bg-surface"
            >
              <div className="border-b border-line bg-raised px-4 py-2.5">
                <h2 className="text-xs font-semibold text-fg">Main system</h2>
                <p className="mt-0.5 text-[11px] text-muted">
                  Target repository and local clone used for promotions, diffs, and reconciliation.
                </p>
              </div>
              <div className="space-y-4 p-4">
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
                  <span className="mt-0.5 block text-[11px] text-muted">
                    Absolute path for local promotion. Leave blank to use a tool-managed checkout.
                  </span>
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

                <div className="rounded-md border border-line bg-raised/50 px-3 py-2 text-[11px] text-muted">
                  <span className="font-medium text-fg">Active target:</span>{' '}
                  <span className="mono">{mainRepositoryInUse || 'unknown'}</span>
                  {' · '}
                  <span className="mono">{localPathInEffect || 'managed checkout'}</span>
                </div>

                <label className="block">
                  <span className="text-xs font-medium text-fg">Global files (main-owned)</span>
                  <span className="mt-0.5 block text-[11px] text-muted">
                    One path or directory prefix per line. Protected from accidental subsystem
                    overwrites.
                  </span>
                  <textarea
                    className="input mono mt-1.5 h-auto min-h-[112px] py-1.5"
                    rows={5}
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

            <section
              aria-label="Promotion policy"
              className="flex flex-col rounded-md border border-line bg-surface"
            >
              <div className="border-b border-line bg-raised px-4 py-2.5">
                <h2 className="text-xs font-semibold text-fg">Promotion policy</h2>
                <p className="mt-0.5 text-[11px] text-muted">
                  Local-first — push and pull request creation are opt-in per promotion.
                </p>
              </div>
              <div className="space-y-4 p-4">
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

                <label className="flex items-start gap-2.5 rounded-md border border-line bg-raised/50 px-3 py-2.5">
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
                      Authors promotion commits as your signed-in GitHub account (via a GitHub
                      noreply address). Unchecked authors them as the tool&apos;s bot identity.
                    </span>
                  </span>
                </label>
              </div>
            </section>
          </div>
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

import { useEffect, useMemo, useState } from 'react';
import {
  ArrowUpRight,
  ExternalLink,
  FolderGit2,
  GitBranch,
  Pencil,
  Plus,
  RefreshCw,
  Save,
  Search,
  Trash2,
  X,
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

interface SubsystemModalState {
  mode: 'add' | 'edit';
  key: string;
  entry: RegistrySubsystem;
}

function SubsystemModal({
  state,
  live,
  onClose,
  onApply,
  onRemove,
  onBrowse,
  onOpenExternal,
}: {
  state: SubsystemModalState;
  live: SubsystemSummary | null;
  onClose: () => void;
  onApply: (entry: RegistrySubsystem) => void;
  onRemove: () => void;
  onBrowse: (id: string) => void;
  onOpenExternal: (url: string) => void;
}) {
  const [draft, setDraft] = useState<RegistrySubsystem>(state.entry);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [onClose]);

  const patch = (next: Partial<RegistrySubsystem>) => {
    setDraft((current) => ({ ...current, ...next }));
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="subsystem-modal-title"
      className="fixed inset-0 z-50 flex items-center justify-center bg-scrim p-4"
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div className="flex max-h-[85vh] w-full max-w-[540px] flex-col overflow-hidden rounded-lg border border-line-strong bg-surface">
        <header className="flex items-center justify-between gap-3 border-b border-line bg-raised px-4 py-3">
          <div className="min-w-0">
            <h2 id="subsystem-modal-title" className="truncate text-xs font-semibold text-fg">
              {state.mode === 'add' ? 'Add subsystem' : draft.name || draft.id || 'Edit subsystem'}
            </h2>
            {live ? (
              <p className="mt-0.5 flex items-center gap-1.5 text-[11px] text-muted">
                <span
                  aria-hidden="true"
                  className={cx(
                    'h-1.5 w-1.5 shrink-0 rounded-full',
                    CONNECTIVITY_DOT[live.connectivity].dot,
                  )}
                />
                <span>{CONNECTIVITY_DOT[live.connectivity].label}</span>
                <span>· verified {formatDateTime(live.lastSyncedAt)}</span>
              </p>
            ) : (
              <p className="mt-0.5 text-[11px] text-muted">
                Configure repository details and protected source branch
              </p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {draft.repository ? (
              <IconButton
                icon={ExternalLink}
                label="Open repository on GitHub"
                onClick={() =>
                  onOpenExternal(
                    `https://github.com/${draft.repository}/tree/${draft.sourceBranch || 'master'}`,
                  )
                }
              />
            ) : null}
            {live && draft.enabled ? (
              <Button
                size="sm"
                variant="ghost"
                trailingIcon={ArrowUpRight}
                onClick={() => {
                  onClose();
                  onBrowse(live.id);
                }}
              >
                Browse
              </Button>
            ) : null}
            <IconButton icon={X} label="Close dialog" onClick={onClose} />
          </div>
        </header>

        <form
          onSubmit={(event) => {
            event.preventDefault();
            onApply(draft);
          }}
          className="flex min-h-0 flex-1 flex-col"
        >
          <div className="scroll-area min-h-0 flex-1 space-y-4 p-4">
            <div className="grid gap-3 sm:grid-cols-2">
              <label className="block">
                <span className="text-xs font-medium text-fg">Id</span>
                <input
                  className="input mono mt-1"
                  value={draft.id}
                  spellCheck={false}
                  autoComplete="off"
                  autoFocus={state.mode === 'add'}
                  placeholder="hr-management"
                  onChange={(event) => patch({ id: event.target.value })}
                />
              </label>
              <label className="block">
                <span className="text-xs font-medium text-fg">Display name</span>
                <input
                  className="input mt-1"
                  value={draft.name}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder="Human Resource Management"
                  onChange={(event) => patch({ name: event.target.value })}
                />
              </label>
            </div>

            <label className="block">
              <span className="text-xs font-medium text-fg">Repository (owner/name)</span>
              <input
                className="input mono mt-1"
                value={draft.repository}
                spellCheck={false}
                autoComplete="off"
                placeholder="ORG/hr-management"
                onChange={(event) => patch({ repository: event.target.value })}
              />
            </label>

            <label className="block">
              <span className="text-xs font-medium text-fg">Protected source branch</span>
              <input
                className="input mono mt-1"
                value={draft.sourceBranch}
                spellCheck={false}
                autoComplete="off"
                placeholder="master"
                onChange={(event) => patch({ sourceBranch: event.target.value })}
              />
            </label>

            <label className="flex items-start gap-2.5 rounded-md border border-line bg-raised/50 px-3 py-2.5">
              <Checkbox
                className="mt-0.5"
                checked={draft.enabled}
                onChange={(event) => patch({ enabled: event.target.checked })}
                label="Enable this subsystem"
              />
              <span>
                <span className="text-xs font-medium text-fg">Enabled</span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                  Show this subsystem in Commit Browser, Subsystem Diff, Promote, Distribute, and
                  Reconcile.
                </span>
              </span>
            </label>

            <details className="overflow-hidden rounded-md border border-line bg-surface">
              <summary className="flex h-[31px] cursor-pointer items-center justify-between px-3 text-[11px] font-semibold tracking-wide text-muted uppercase select-none hover:bg-hover hover:text-fg">
                <span>Integration policy</span>
                <span className="mono font-normal normal-case">{draft.pathStrategy}</span>
              </summary>
              <dl className="divide-y divide-line border-t border-line px-3">
                <KeyValue label="Path strategy">
                  <span className="mono">{draft.pathStrategy}</span>
                </KeyValue>
                <KeyValue label="Dependency strategy">
                  <span className="mono">{draft.dependencyStrategy}</span>
                </KeyValue>
                <KeyValue label="Application files">
                  <span className="mono">{draft.integrationPolicy.applicationFiles}</span>
                </KeyValue>
                <KeyValue label="package.json">
                  <span className="mono">{draft.integrationPolicy.packageJson}</span>
                </KeyValue>
                <KeyValue label="Lockfiles">
                  <span className="mono">{draft.integrationPolicy.lockfiles}</span>
                </KeyValue>
                <KeyValue label="GitHub workflows">
                  <span className="mono">{draft.integrationPolicy.githubWorkflows}</span>
                </KeyValue>
                <KeyValue label="Secret files">
                  <span className="mono">{draft.integrationPolicy.secretFiles}</span>
                </KeyValue>
              </dl>
            </details>
          </div>

          <footer className="flex items-center justify-between gap-2 border-t border-line bg-raised px-4 py-2.5">
            <div>
              {state.mode === 'edit' ? (
                <Button type="button" size="sm" variant="danger" icon={Trash2} onClick={onRemove}>
                  Remove
                </Button>
              ) : null}
            </div>
            <div className="flex items-center gap-2">
              <Button type="button" size="sm" variant="ghost" onClick={onClose}>
                Cancel
              </Button>
              <Button type="submit" size="sm" variant="primary">
                {state.mode === 'add' ? 'Add subsystem' : 'Apply changes'}
              </Button>
            </div>
          </footer>
        </form>
      </div>
    </div>
  );
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
  const [modal, setModal] = useState<SubsystemModalState | null>(null);
  const [subsystemFilter, setSubsystemFilter] = useState('');
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');
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

  const isDirty = useMemo(() => {
    const payload: RegistryConfig = { ...config, subsystems: rows.map((row) => row.entry) };
    return JSON.stringify(payload) !== JSON.stringify(snapshot.config);
  }, [config, rows, snapshot.config]);

  const patchRow = (key: string, patch: Partial<RegistrySubsystem>) => {
    setRows((current) =>
      current.map((row) => (row.key === key ? { ...row, entry: { ...row.entry, ...patch } } : row)),
    );
  };

  const openAddModal = () => {
    setTab('subsystems');
    setModal({
      mode: 'add',
      key: crypto.randomUUID(),
      entry: newSubsystem(),
    });
  };

  const openEditModal = (row: SubsystemRow) => {
    setModal({
      mode: 'edit',
      key: row.key,
      entry: row.entry,
    });
  };

  const applyModal = (entry: RegistrySubsystem) => {
    if (!modal) return;
    if (modal.mode === 'add') {
      setSubsystemFilter('');
      setStatusFilter('all');
      setRows((current) => [...current, { key: modal.key, entry }]);
    } else {
      setRows((current) => current.map((row) => (row.key === modal.key ? { ...row, entry } : row)));
    }
    setModal(null);
  };

  const removeRow = (key: string) => {
    setRows((current) => current.filter((row) => row.key !== key));
    setModal((current) => (current?.key === key ? null : current));
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
            <span className="max-w-[300px] truncate">{mainRepositoryInUse || 'main unset'}</span>
          </button>
          <Button size="sm" variant="secondary" icon={Plus} onClick={openAddModal}>
            Add subsystem
          </Button>
        </div>
      </div>

      {tab === 'subsystems' ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface px-5 py-2.5">
            <div className="relative w-full max-w-[360px] min-w-[200px] flex-1">
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
                  <Button size="sm" variant="secondary" icon={Plus} onClick={openAddModal}>
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
                    <th scope="col" className="w-11 border-b border-line py-2 pl-5 text-left">
                      <span className="sr-only">Enabled</span>
                    </th>
                    <th scope="col" className="border-b border-line py-2 pr-4 text-left">
                      Subsystem
                    </th>
                    <th scope="col" className="border-b border-line py-2 pr-4 text-left">
                      Repository
                    </th>
                    <th scope="col" className="w-32 border-b border-line py-2 pr-4 text-left">
                      Branch
                    </th>
                    <th scope="col" className="w-28 border-b border-line py-2 pr-4 text-left">
                      Status
                    </th>
                    <th scope="col" className="w-36 border-b border-line py-2 pr-5 text-right">
                      <span className="sr-only">Actions</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {filteredRows.map((row) => {
                    const { entry } = row;
                    const label = entry.name || entry.id || 'New subsystem';
                    const live = entry.id ? liveById.get(entry.id) : undefined;
                    const connectivity = live ? CONNECTIVITY_DOT[live.connectivity] : null;

                    return (
                      <tr
                        key={row.key}
                        onClick={() => openEditModal(row)}
                        className={cx(
                          'row-hover cursor-pointer transition-colors',
                          !entry.enabled && 'opacity-60',
                        )}
                      >
                        <td
                          className="h-[38px] border-b border-line py-0 pl-5 align-middle"
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
                        <td className="h-[38px] border-b border-line py-0 pr-4 align-middle">
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
                        <td className="mono h-[38px] border-b border-line py-0 pr-4 align-middle text-[11px] text-muted">
                          <span className="block truncate" title={entry.repository}>
                            {entry.repository || '—'}
                          </span>
                        </td>
                        <td className="mono h-[38px] border-b border-line py-0 pr-4 align-middle text-[11px] text-muted">
                          <span className="inline-flex items-center gap-1.5">
                            <GitBranch
                              className="h-3 w-3 shrink-0 text-disabled"
                              aria-hidden="true"
                            />
                            {entry.sourceBranch || '—'}
                          </span>
                        </td>
                        <td className="h-[38px] border-b border-line py-0 pr-4 align-middle text-[11px] text-muted">
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
                        <td
                          className="h-[38px] border-b border-line py-0 pr-5 text-right align-middle"
                          onClick={(event) => event.stopPropagation()}
                        >
                          <div className="inline-flex items-center gap-1">
                            <Button
                              size="sm"
                              variant="ghost"
                              icon={Pencil}
                              onClick={() => openEditModal(row)}
                            >
                              Edit
                            </Button>
                            {entry.repository ? (
                              <IconButton
                                icon={ExternalLink}
                                label={`Open ${label} on GitHub`}
                                onClick={() =>
                                  app.openExternal(
                                    `https://github.com/${entry.repository}/tree/${entry.sourceBranch || 'master'}`,
                                  )
                                }
                              />
                            ) : null}
                            <IconButton
                              icon={Trash2}
                              tone="danger"
                              label={`Remove ${label}`}
                              onClick={() => removeRow(row.key)}
                            />
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
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

      {modal ? (
        <SubsystemModal
          key={modal.key}
          state={modal}
          live={modal.entry.id ? (liveById.get(modal.entry.id) ?? null) : null}
          onClose={() => setModal(null)}
          onApply={applyModal}
          onRemove={() => removeRow(modal.key)}
          onBrowse={(id) => {
            app.selectSubsystem(id);
            app.navigate('commit-browser');
          }}
          onOpenExternal={(url) => app.openExternal(url)}
        />
      ) : null}
    </div>
  );
}

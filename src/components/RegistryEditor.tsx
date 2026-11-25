import { Fragment, useMemo, useState } from 'react';
import { ChevronDown, Plus, RefreshCw, Save, Search, Trash2 } from 'lucide-react';

import { api, asApiError, type ApiError } from '../api/invoke';
import type { RegistryConfig, RegistrySnapshot, RegistrySubsystem } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import {
  Button,
  Checkbox,
  EmptyState,
  ErrorState,
  IconButton,
  InlineAlert,
  LoadingState,
} from './ui/primitives';
import { cx } from './ui/cx';

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
  value,
  onChange,
}: {
  label: string;
  value: string[];
  onChange: (next: string[]) => void;
}) {
  const [draft, setDraft] = useState(value.join(', '));
  return (
    <label className="block">
      <span className="muted">{label}</span>
      <input
        className="input mt-1"
        value={draft}
        spellCheck={false}
        autoComplete="off"
        onChange={(event) => {
          setDraft(event.target.value);
          onChange(splitList(event.target.value));
        }}
      />
    </label>
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
  const [config, setConfig] = useState<RegistryConfig>(snapshot.config);
  // Keys are issued once per row and never derived from editable fields, so edits and
  // removes can't reattach state to the wrong row.
  const [rows, setRows] = useState<SubsystemRow[]>(() =>
    snapshot.config.subsystems.map((entry) => ({ key: crypto.randomUUID(), entry })),
  );
  // Accordion: one open row keeps six subsystems at six lines.
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [subsystemFilter, setSubsystemFilter] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);

  const mainRepositoryInUse = app.auth.data?.mainRepository ?? config.mainRepository;
  const localPathInEffect = (config.localMainRepositoryPath ?? '').trim();
  const enabledCount = useMemo(() => rows.filter((row) => row.entry.enabled).length, [rows]);
  const filteredRows = useMemo(() => {
    const needle = subsystemFilter.trim().toLowerCase();
    if (!needle) return rows;
    return rows.filter(({ entry }) =>
      `${entry.id} ${entry.name} ${entry.repository} ${entry.sourceBranch}`
        .toLowerCase()
        .includes(needle),
    );
  }, [rows, subsystemFilter]);

  const patchRow = (key: string, patch: Partial<RegistrySubsystem>) => {
    setRows((current) =>
      current.map((row) => (row.key === key ? { ...row, entry: { ...row.entry, ...patch } } : row)),
    );
  };

  const addRow = () => {
    const key = crypto.randomUUID();
    setSubsystemFilter('');
    setRows((current) => [...current, { key, entry: newSubsystem() }]);
    setOpenKey(key);
  };

  const removeRow = (key: string) => {
    setRows((current) => current.filter((row) => row.key !== key));
    setOpenKey((current) => (current === key ? null : current));
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
      <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
        {/* Left column: main system configuration & promotion policy */}
        <section
          aria-label="Main system and policy"
          className="flex min-h-0 w-full shrink-0 flex-col border-b border-line bg-surface lg:w-[440px] lg:border-r lg:border-b-0 xl:w-[480px]"
        >
          <div className="border-b border-line bg-raised px-4 py-2.5">
            <h2 className="truncate text-xs font-semibold text-fg">Main system &amp; policy</h2>
            <p className="truncate text-[11px] text-muted">
              Tool-local configuration — nothing is written into any repository
            </p>
          </div>

          <div className="scroll-area min-h-0 flex-1 divide-y divide-line">
            <div className="space-y-3 p-4">
              <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                Main system
              </h3>
              <label className="block">
                <span className="muted">Main repository (owner/name)</span>
                <input
                  className="input mt-1"
                  value={config.mainRepository}
                  spellCheck={false}
                  autoComplete="off"
                  onChange={(event) =>
                    setConfig((current) => ({ ...current, mainRepository: event.target.value }))
                  }
                />
              </label>
              <label className="block">
                <span className="muted">
                  Local main repository clone (absolute path for local promotion)
                </span>
                <input
                  className="input mono mt-1"
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
                <span className="mt-1 block truncate text-[11px] text-muted">
                  In use: <span className="font-mono">{mainRepositoryInUse || 'unknown'}</span>
                  {' · '}
                  <span className="font-mono">{localPathInEffect || 'managed checkout'}</span>
                </span>
              </label>
              <label className="block">
                <span className="muted">Global files (main-owned)</span>
                <textarea
                  className="input mono mt-1 h-auto min-h-[76px] py-1.5"
                  rows={4}
                  value={(config.globalFiles ?? []).join('\n')}
                  spellCheck={false}
                  autoComplete="off"
                  placeholder={'src/app/layout.tsx\nsrc/components/shared/'}
                  aria-label="Global files (main-owned)"
                  onChange={(event) =>
                    setConfig((current) => ({
                      ...current,
                      globalFiles: splitLines(event.target.value),
                    }))
                  }
                />
              </label>
            </div>

            <div className="space-y-3 p-4">
              <div>
                <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                  Promotion policy
                </h3>
                <p className="mt-0.5 text-[11px] text-muted">
                  Local-first — push + PR is opt-in per promotion.
                </p>
              </div>
              <ListField
                label="Allowed target branches (comma-separated)"
                value={config.promotion.allowedTargetBranches}
                onChange={(next) =>
                  setConfig((current) => ({
                    ...current,
                    promotion: { ...current.promotion, allowedTargetBranches: next },
                  }))
                }
              />
              <ListField
                label="Allowed branch patterns (safe glob, e.g. release/*)"
                value={config.promotion.allowedTargetBranchPatterns}
                onChange={(next) =>
                  setConfig((current) => ({
                    ...current,
                    promotion: { ...current.promotion, allowedTargetBranchPatterns: next },
                  }))
                }
              />
              <label className="flex items-start gap-2 rounded-md border border-line bg-raised px-3 py-2.5">
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
                  <span className="text-xs font-semibold text-fg">
                    Attribute promotions to my GitHub account
                  </span>
                  <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                    Authors promotion commits as your signed-in GitHub account (via a GitHub noreply
                    address). Unchecked authors them as the tool&apos;s bot identity.
                  </span>
                </span>
              </label>
            </div>
          </div>
        </section>

        {/* Right column: registered subsystems table */}
        <section
          aria-label="Registered subsystems"
          className="flex min-h-0 min-w-0 flex-1 flex-col bg-surface"
        >
          <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-raised px-4 py-2.5">
            <div className="min-w-0">
              <h2 className="truncate text-xs font-semibold text-fg">Subsystems ({rows.length})</h2>
              <p className="mono truncate text-[11px] text-muted">{enabledCount} enabled</p>
            </div>
            <div className="flex shrink-0 flex-wrap items-center gap-2">
              <div className="relative w-[220px]">
                <Search
                  className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
                  aria-hidden="true"
                />
                <input
                  type="search"
                  value={subsystemFilter}
                  onChange={(event) => setSubsystemFilter(event.target.value)}
                  placeholder="Filter subsystems…"
                  aria-label="Filter subsystems"
                  className="input pl-8"
                />
              </div>
              <Button size="sm" variant="secondary" icon={Plus} onClick={addRow}>
                Add subsystem
              </Button>
            </div>
          </div>

          <div className="scroll-area min-h-0 flex-1">
            {rows.length === 0 ? (
              <EmptyState
                icon={Plus}
                title="No subsystems registered"
                message="Add a subsystem repository to begin browsing commits and promoting changes."
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
                message="Clear the filter to see all registered subsystems."
                action={
                  <Button size="sm" onClick={() => setSubsystemFilter('')}>
                    Clear filter
                  </Button>
                }
              />
            ) : (
              <table className="w-full border-separate border-spacing-0 text-xs">
                <thead className="sticky top-0 z-10">
                  <tr className="table-head">
                    <th scope="col" className="w-9 border-b border-line py-2 pl-3">
                      <span className="sr-only">Expand</span>
                    </th>
                    <th scope="col" className="w-16 border-b border-line py-2 pr-3 text-left">
                      Enabled
                    </th>
                    <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                      Id
                    </th>
                    <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                      Name
                    </th>
                    <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                      Repository
                    </th>
                    <th scope="col" className="border-b border-line py-2 pr-3 text-left">
                      Source branch
                    </th>
                    <th scope="col" className="w-10 border-b border-line py-2 pr-3 text-right">
                      <span className="sr-only">Remove</span>
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {filteredRows.map((row) => {
                    const { entry } = row;
                    const open = openKey === row.key;
                    const label = entry.id || entry.name || 'new subsystem';
                    return (
                      <Fragment key={row.key}>
                        <tr
                          className={cx(
                            'row-hover border-b border-line',
                            open && 'bg-selection/50',
                          )}
                          onClick={(event) => {
                            if ((event.target as HTMLElement).closest('button,input')) return;
                            setOpenKey(open ? null : row.key);
                          }}
                        >
                          <td className="border-b border-line py-0 pl-3">
                            <button
                              type="button"
                              aria-expanded={open}
                              aria-controls={`subsystem-${row.key}`}
                              aria-label={`${open ? 'Collapse' : 'Expand'} ${label}`}
                              onClick={() => setOpenKey(open ? null : row.key)}
                              className="inline-flex h-[25px] w-[25px] items-center justify-center rounded-md text-muted transition-colors hover:bg-hover hover:text-fg"
                            >
                              <ChevronDown
                                className={cx('h-4 w-4 transition-transform', open && 'rotate-180')}
                                aria-hidden="true"
                              />
                            </button>
                          </td>
                          <td className="h-[29px] border-b border-line py-0 pr-3 align-middle">
                            <Checkbox
                              label={`Enabled: ${label}`}
                              checked={entry.enabled}
                              onChange={(event) =>
                                patchRow(row.key, { enabled: event.target.checked })
                              }
                            />
                          </td>
                          <td className="mono h-[29px] border-b border-line py-0 pr-3 whitespace-nowrap text-fg">
                            {entry.id || <span className="text-muted">—</span>}
                          </td>
                          <td className="h-[29px] border-b border-line py-0 pr-3 whitespace-nowrap text-fg">
                            {entry.name || <span className="text-muted">—</span>}
                          </td>
                          <td className="mono h-[29px] border-b border-line py-0 pr-3 whitespace-nowrap text-fg">
                            {entry.repository || <span className="text-muted">—</span>}
                          </td>
                          <td className="mono h-[29px] border-b border-line py-0 pr-3 whitespace-nowrap text-fg">
                            {entry.sourceBranch || <span className="text-muted">—</span>}
                          </td>
                          <td className="border-b border-line py-0 pr-3 text-right">
                            <IconButton
                              icon={Trash2}
                              tone="danger"
                              label={`Remove ${label}`}
                              onClick={() => removeRow(row.key)}
                            />
                          </td>
                        </tr>
                        {open ? (
                          <tr className="border-b border-line">
                            <td
                              colSpan={7}
                              id={`subsystem-${row.key}`}
                              className="border-b border-line bg-raised px-4 py-3"
                            >
                              <div className="grid gap-3 md:grid-cols-2">
                                <label className="block">
                                  <span className="muted">Id</span>
                                  <input
                                    className="input mt-1"
                                    value={entry.id}
                                    spellCheck={false}
                                    autoComplete="off"
                                    placeholder="hr-management"
                                    onChange={(event) =>
                                      patchRow(row.key, { id: event.target.value })
                                    }
                                  />
                                </label>
                                <label className="block">
                                  <span className="muted">Name</span>
                                  <input
                                    className="input mt-1"
                                    value={entry.name}
                                    spellCheck={false}
                                    autoComplete="off"
                                    placeholder="Human Resource Management"
                                    onChange={(event) =>
                                      patchRow(row.key, { name: event.target.value })
                                    }
                                  />
                                </label>
                                <label className="block">
                                  <span className="muted">Repository (owner/name)</span>
                                  <input
                                    className="input mt-1"
                                    value={entry.repository}
                                    spellCheck={false}
                                    autoComplete="off"
                                    placeholder="ORG/hr-management"
                                    onChange={(event) =>
                                      patchRow(row.key, { repository: event.target.value })
                                    }
                                  />
                                </label>
                                <label className="block">
                                  <span className="muted">Protected source branch</span>
                                  <input
                                    className="input mt-1"
                                    value={entry.sourceBranch}
                                    spellCheck={false}
                                    autoComplete="off"
                                    placeholder="master"
                                    onChange={(event) =>
                                      patchRow(row.key, { sourceBranch: event.target.value })
                                    }
                                  />
                                </label>
                              </div>
                            </td>
                          </tr>
                        ) : null}
                      </Fragment>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        </section>
      </div>

      {error ? (
        <div className="border-t border-line px-4 py-2">
          <InlineAlert tone="danger" title={error.message}>
            {error.remedy ?? 'Check the values and try again.'}
          </InlineAlert>
        </div>
      ) : null}

      <footer className="flex flex-wrap items-center justify-between gap-3 border-t border-line bg-raised px-4 py-2">
        <span
          className="mono min-w-0 flex-1 truncate text-[11px] text-muted"
          title={snapshot.overridePath}
        >
          {snapshot.overridePath}
        </span>
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

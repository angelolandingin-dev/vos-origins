import { Fragment, useState } from 'react';
import { ChevronDown, Plus, RefreshCw, Save, Trash2 } from 'lucide-react';

import { api, asApiError, type ApiError } from '../api/invoke';
import type { RegistryConfig, RegistrySnapshot, RegistrySubsystem } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { Button, ErrorState, InlineAlert, LoadingState, Panel } from './ui/primitives';
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
      <Panel title="Registry">
        <div className="p-3">
          <LoadingState label="Reading the registry…" />
        </div>
      </Panel>
    );
  }

  if (snapshot.error) {
    return (
      <Panel title="Registry">
        <div className="p-3">
          <ErrorState error={snapshot.error} onRetry={() => snapshot.reload()} compact />
        </div>
      </Panel>
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
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);

  const mainRepositoryInUse = app.auth.data?.mainRepository ?? config.mainRepository;
  const localPathInEffect = (config.localMainRepositoryPath ?? '').trim();

  const patchRow = (key: string, patch: Partial<RegistrySubsystem>) => {
    setRows((current) =>
      current.map((row) => (row.key === key ? { ...row, entry: { ...row.entry, ...patch } } : row)),
    );
  };

  const addRow = () => {
    const key = crypto.randomUUID();
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
    <div className="space-y-4">
      <InlineAlert tone="info" title="Tool-local registry — nothing is written into any repository">
        <span className="mono">{snapshot.overridePath}</span>
      </InlineAlert>

      <Panel
        title="Main system"
        actions={
          <Button size="sm" variant="secondary" icon={RefreshCw} onClick={onReload}>
            Reload
          </Button>
        }
      >
        <div className="space-y-3 p-3">
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
                  localMainRepositoryPath: event.target.value.trim() ? event.target.value : null,
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
              className="input mono mt-1 h-auto min-h-[50px] py-1.5"
              rows={2}
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
      </Panel>

      <Panel title="Promotion policy" subtitle="Local-first — push + PR is opt-in per promotion.">
        <div className="space-y-3 p-3">
          <div className="grid gap-3 md:grid-cols-2">
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
          </div>
          <label className="flex items-start gap-2 rounded-md border border-line bg-raised px-3 py-2">
            <input
              type="checkbox"
              className="mt-0.5 h-4 w-4 rounded border-line-strong bg-surface accent-accent"
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
            />
            <span>
              <span className="text-xs font-semibold text-fg">
                Attribute promotions to my GitHub account
              </span>
              <span className="mt-0.5 block text-[11px] text-muted">
                Authors promotion commits as your signed-in GitHub account (via a GitHub noreply
                address). Unchecked authors them as the tool&apos;s bot identity.
              </span>
            </span>
          </label>
        </div>
      </Panel>

      <Panel
        title={`Subsystems (${rows.length})`}
        actions={
          <Button size="sm" variant="secondary" icon={Plus} onClick={addRow}>
            Add subsystem
          </Button>
        }
        bodyClassName="overflow-x-auto"
      >
        {rows.length === 0 ? (
          <div className="p-3">
            <p className="muted">No subsystems registered yet.</p>
          </div>
        ) : (
          <table className="w-full border-separate border-spacing-0 text-xs">
            <thead>
              <tr className="table-head">
                <th scope="col" className="w-10 py-2 pl-3">
                  <span className="sr-only">Expand</span>
                </th>
                <th scope="col" className="py-2 pr-3 text-left">
                  Enabled
                </th>
                <th scope="col" className="py-2 pr-3 text-left">
                  Id
                </th>
                <th scope="col" className="py-2 pr-3 text-left">
                  Name
                </th>
                <th scope="col" className="py-2 pr-3 text-left">
                  Repository
                </th>
                <th scope="col" className="py-2 pr-3 text-left">
                  Source branch
                </th>
                <th scope="col" className="py-2 pr-3 text-right">
                  <span className="sr-only">Actions</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => {
                const { entry } = row;
                const open = openKey === row.key;
                const label = entry.id || entry.name || 'new subsystem';
                return (
                  <Fragment key={row.key}>
                    <tr
                      className="row-hover border-b border-line"
                      onClick={(event) => {
                        if ((event.target as HTMLElement).closest('button,input')) return;
                        setOpenKey(open ? null : row.key);
                      }}
                    >
                      <td className="py-0 pl-3">
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
                      <td className="h-[29px] py-0 align-middle">
                        <input
                          type="checkbox"
                          aria-label={`Enabled: ${label}`}
                          className="h-4 w-4 rounded border-line-strong bg-surface accent-accent"
                          checked={entry.enabled}
                          onChange={(event) => patchRow(row.key, { enabled: event.target.checked })}
                        />
                      </td>
                      <td className="mono h-[29px] py-0 pr-3 whitespace-nowrap text-fg">
                        {entry.id || <span className="text-muted">—</span>}
                      </td>
                      <td className="h-[29px] py-0 pr-3 whitespace-nowrap text-fg">
                        {entry.name || <span className="text-muted">—</span>}
                      </td>
                      <td className="mono h-[29px] py-0 pr-3 whitespace-nowrap text-fg">
                        {entry.repository || <span className="text-muted">—</span>}
                      </td>
                      <td className="mono h-[29px] py-0 pr-3 whitespace-nowrap text-fg">
                        {entry.sourceBranch || <span className="text-muted">—</span>}
                      </td>
                      <td className="py-0 pr-3 text-right">
                        <Button
                          size="sm"
                          variant="danger"
                          icon={Trash2}
                          onClick={() => removeRow(row.key)}
                        >
                          Remove
                        </Button>
                      </td>
                    </tr>
                    {open ? (
                      <tr className="border-b border-line">
                        <td colSpan={7} id={`subsystem-${row.key}`} className="bg-raised px-3 py-3">
                          <div className="grid gap-2 md:grid-cols-2">
                            <label className="block">
                              <span className="muted">Id</span>
                              <input
                                className="input mt-1"
                                value={entry.id}
                                spellCheck={false}
                                autoComplete="off"
                                placeholder="hr-management"
                                onChange={(event) => patchRow(row.key, { id: event.target.value })}
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
      </Panel>

      {error ? (
        <InlineAlert tone="danger" title={error.message}>
          {error.remedy ?? 'Check the values and try again.'}
        </InlineAlert>
      ) : null}

      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" icon={Save} onClick={save} disabled={saving}>
          {saving ? 'Saving…' : 'Save registry'}
        </Button>
        <span className="muted">Validated on save.</span>
      </div>
    </div>
  );
}

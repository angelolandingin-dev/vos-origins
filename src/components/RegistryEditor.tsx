import { useState } from 'react';
import { Plus, RefreshCw, Save, Trash2 } from 'lucide-react';

import { api, asApiError, type ApiError } from '../api/invoke';
import type { RegistryConfig, RegistrySnapshot, RegistrySubsystem } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import { Button, ErrorState, InlineAlert, LoadingState, Panel } from './ui/primitives';

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

/**
 * A comma-separated list field that keeps the raw text while you type.
 *
 * Rendering the parsed array straight back into the box destroys the separator
 * as it is typed - split, filter out the empty entry, join - so a second value
 * can never be entered. The draft holds the text; the config gets the parse.
 */
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

/** Settings → register/configure the main system and its subsystems (spec §4, tool-local). */
export function RegistryEditor() {
  const app = useApp();
  const snapshot = useAsyncData(() => api.getRegistry(), [app.dataVersion]);

  return (
    <Panel
      title="Main system registry"
      subtitle="Owned by the tool — nothing is written into any repository"
      actions={
        <Button size="sm" variant="secondary" icon={RefreshCw} onClick={() => snapshot.reload()}>
          Reload
        </Button>
      }
    >
      <div className="p-3">
        {snapshot.loading && !snapshot.data ? (
          <LoadingState label="Reading the registry…" />
        ) : snapshot.error ? (
          <ErrorState error={snapshot.error} onRetry={() => snapshot.reload()} compact />
        ) : snapshot.data ? (
          <RegistryForm
            key={JSON.stringify(snapshot.data.config)}
            snapshot={snapshot.data}
            onSaved={() => {
              snapshot.reload();
              app.bumpDataVersion();
            }}
          />
        ) : null}
      </div>
    </Panel>
  );
}

function RegistryForm({ snapshot, onSaved }: { snapshot: RegistrySnapshot; onSaved: () => void }) {
  const app = useApp();
  const [config, setConfig] = useState<RegistryConfig>(snapshot.config);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<ApiError | null>(null);

  const mainRepositoryInUse = app.auth.data?.mainRepository ?? config.mainRepository;
  const localPathInEffect = (config.localMainRepositoryPath ?? '').trim();

  const patchSubsystem = (index: number, patch: Partial<RegistrySubsystem>) => {
    setConfig((current) => ({
      ...current,
      subsystems: current.subsystems.map((entry, i) =>
        i === index ? { ...entry, ...patch } : entry,
      ),
    }));
  };

  const save = () => {
    setSaving(true);
    setError(null);
    void api
      .saveRegistry(config)
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

      <div className="grid gap-3 md:grid-cols-2">
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
        <div className="rounded-md border border-line bg-raised px-3 py-2">
          <p className="text-xs font-semibold text-fg">Promotion: local-first</p>
          <p className="mt-1 text-[11px] text-muted">Push + PR is opt-in per promotion.</p>
        </div>
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

      <div className="space-y-2">
        <div className="flex h-[29px] items-center justify-between">
          <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
            Subsystems ({config.subsystems.length})
          </p>
          <Button
            size="sm"
            variant="secondary"
            icon={Plus}
            onClick={() =>
              setConfig((current) => ({
                ...current,
                subsystems: [...current.subsystems, newSubsystem()],
              }))
            }
          >
            Add subsystem
          </Button>
        </div>

        {config.subsystems.length === 0 ? (
          <p className="muted">No subsystems registered yet.</p>
        ) : (
          <ul className="space-y-2">
            {config.subsystems.map((entry, index) => (
              <li key={index} className="rounded-md border border-line bg-raised p-3">
                <div className="grid gap-2 md:grid-cols-2 xl:grid-cols-4">
                  <label className="block">
                    <span className="muted">Id</span>
                    <input
                      className="input mt-1"
                      value={entry.id}
                      spellCheck={false}
                      autoComplete="off"
                      placeholder="hr-management"
                      onChange={(event) => patchSubsystem(index, { id: event.target.value })}
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
                      onChange={(event) => patchSubsystem(index, { name: event.target.value })}
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
                        patchSubsystem(index, { repository: event.target.value })
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
                        patchSubsystem(index, { sourceBranch: event.target.value })
                      }
                    />
                  </label>
                </div>

                <div className="mt-2 flex flex-wrap items-center justify-between gap-2">
                  <label className="flex h-[25px] items-center gap-2 text-xs text-fg">
                    <input
                      type="checkbox"
                      className="h-4 w-4 rounded border-line-strong bg-surface accent-accent"
                      checked={entry.enabled}
                      onChange={(event) => patchSubsystem(index, { enabled: event.target.checked })}
                    />
                    Enabled
                  </label>
                  <Button
                    size="sm"
                    variant="danger"
                    icon={Trash2}
                    onClick={() =>
                      setConfig((current) => ({
                        ...current,
                        subsystems: current.subsystems.filter((_, i) => i !== index),
                      }))
                    }
                  >
                    Remove
                  </Button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>

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

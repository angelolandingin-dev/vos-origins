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
      className="xl:col-span-2"
      title="Main system registry"
      subtitle="Owned by the tool — nothing is written into the main system or subsystem repositories"
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
          message: 'The tool-local registry override has been updated.',
        });
        onSaved();
      })
      .catch((err: unknown) => setError(asApiError(err)))
      .finally(() => setSaving(false));
  };

  return (
    <div className="space-y-4">
      <InlineAlert tone="info" title="Saved to the tool, not the repository">
        <span className="mono">{snapshot.overridePath}</span>
        {snapshot.overrideExists
          ? ' — the saved override is in use.'
          : ' — not created yet; the bundled defaults are in use until you save.'}
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
        <label className="block">
          <span className="muted">Allowed target branches (comma-separated)</span>
          <input
            className="input mt-1"
            value={config.promotion.allowedTargetBranches.join(', ')}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) =>
              setConfig((current) => ({
                ...current,
                promotion: {
                  ...current.promotion,
                  allowedTargetBranches: splitList(event.target.value),
                },
              }))
            }
          />
        </label>
        <label className="block">
          <span className="muted">Allowed branch patterns (safe glob, e.g. release/*)</span>
          <input
            className="input mt-1"
            value={config.promotion.allowedTargetBranchPatterns.join(', ')}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) =>
              setConfig((current) => ({
                ...current,
                promotion: {
                  ...current.promotion,
                  allowedTargetBranchPatterns: splitList(event.target.value),
                },
              }))
            }
          />
        </label>
        <div className="rounded-lg border border-ink-700/60 bg-ink-850/50 px-3 py-2">
          <p className="text-xs font-semibold text-slate-100">Promotion: local-first</p>
          <p className="mt-1 text-2xs text-slate-400">
            Local delivery creates a branch + commit in your clone (you push it yourself). Push + PR
            is an explicit opt-in per promotion.
          </p>
        </div>
        <label className="block md:col-span-2">
          <span className="muted">
            Local main repository clone (absolute path for local promotion)
          </span>
          <input
            className="input mono mt-1"
            value={config.localMainRepositoryPath ?? ''}
            spellCheck={false}
            autoComplete="off"
            placeholder="e.g. C:\repos\main-system (empty = use VOS_TARGET_REPO_DIR / managed checkout)"
            onChange={(event) =>
              setConfig((current) => ({
                ...current,
                localMainRepositoryPath: event.target.value.trim() ? event.target.value : null,
              }))
            }
          />
          <span className="mt-1 block text-2xs text-slate-500">
            Used for local delivery: the integration branch + provenance commit are created here
            (never pushed). Falls back to VOS_TARGET_REPO_DIR, then the tool-managed checkout cache.
          </span>
        </label>
      </div>

      <div className="space-y-2">
        <div className="flex items-center justify-between">
          <p className="text-xs font-semibold uppercase tracking-wide text-slate-400">
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
          <p className="muted">
            No subsystems registered yet. Add one so its commits can be browsed and promoted.
          </p>
        ) : (
          <ul className="space-y-2">
            {config.subsystems.map((entry, index) => (
              <li key={index} className="rounded-lg border border-ink-700/60 bg-ink-850/40 p-3">
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
                  <label className="flex items-center gap-2 text-xs text-slate-300">
                    <input
                      type="checkbox"
                      className="h-4 w-4 rounded border-ink-600 bg-ink-900"
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
        <span className="muted">
          Validated on save: repository slugs, safe branch names, unique subsystem ids.
        </span>
      </div>
    </div>
  );
}

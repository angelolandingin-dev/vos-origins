import { useState } from 'react';
import {
  Database,
  ExternalLink,
  KeyRound,
  Monitor,
  RefreshCw,
  Settings as SettingsIcon,
  ShieldAlert,
  Terminal,
  Trash2,
} from 'lucide-react';

import { api, isTauriRuntime } from '../api/invoke';
import type { CacheStats } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import {
  Button,
  ErrorState,
  InlineAlert,
  KeyValue,
  LoadingState,
  Panel,
} from '../components/ui/primitives';
import { cx } from '../components/ui/cx';
import { formatDateTime } from '../components/ui/format';
import { RegistryEditor } from '../components/RegistryEditor';

/** Settings (spec §3.3). */
export function Settings() {
  const app = useApp();
  const [cacheMessage, setCacheMessage] = useState<string | null>(null);

  const cache = useAsyncData(() => api.getCacheStats(), [app.dataVersion]);
  const status = app.auth.data;

  const clearCache = () => {
    void api
      .clearCache()
      .then((stats: CacheStats) => {
        setCacheMessage(stats.note);
        cache.reload();
      })
      .catch(() => setCacheMessage('Clearing the cache failed.'));
  };

  const diagnostics = [
    `app: Subsystem Integration Manager 0.1.0`,
    `runtime: ${isTauriRuntime() ? 'Tauri v2 (desktop webview)' : 'browser preview (backend unavailable)'}`,
    `platform: ${typeof navigator !== 'undefined' ? navigator.userAgent : 'unknown'}`,
    `data source: ${status?.dataSource ?? 'unknown'}`,
    `main repository: ${status?.mainRepository ?? 'unknown'}`,
    `registry branch: ${status?.registryBranch ?? 'unknown'}`,
    `workflow file: ${status?.workflowFile ?? 'unknown'}`,
    `gh installed: ${status?.ghInstalled ? 'yes' : 'no'}`,
    `gh version: ${status?.ghVersion ?? 'n/a'}`,
    `gh authenticated: ${status?.authenticated ? 'yes' : 'no'}`,
    `token available: ${status?.tokenAvailable ? 'yes' : 'no'}`,
    `git: ${status?.gitInstalled ? (status?.gitVersion ?? 'installed') : 'missing'}`,
    `package manager: ${status?.packageManager ?? 'missing'}`,
    `account: ${status?.accountLogin ?? 'n/a'}`,
    `scopes: ${status?.scopes.join(', ') || 'n/a'}`,
    `target branch in use: ${app.targetBranch}`,
    `selected subsystem: ${app.selectedSubsystemId ?? 'none'}`,
    `registry entries: ${app.subsystems.data?.length ?? 0}`,
    `cache entries: ${cache.data?.entries ?? 0}/${cache.data?.capacity ?? 0}`,
  ].join('\n');

  return (
    <div className="scroll-area h-full min-h-0 space-y-4 p-5">
      <header className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="flex items-center gap-2 text-xl font-semibold tracking-tight text-white">
            <SettingsIcon className="h-5 w-5 text-brand-400" aria-hidden="true" />
            Settings
          </h1>
          <p className="mt-0.5 text-xs text-slate-400">
            Main repository, GitHub CLI, cache and diagnostics
          </p>
        </div>
        <Button variant="secondary" icon={RefreshCw} onClick={app.bumpDataVersion}>
          Refresh everything
        </Button>
      </header>

      {status?.dataSource === 'fixture' ? (
        <InlineAlert tone="warning" title="Fixture data mode is active">
          Registry, commit, workflow and PR records are fictitious sample data served by the Rust
          backend. GitHub CLI detection is real. Unset{' '}
          <span className="mono">VOS_USE_FIXTURES</span> to read live Git history and GitHub data.
        </InlineAlert>
      ) : null}

      <div className="grid gap-4 xl:grid-cols-2">
        <Panel title="Main system" subtitle="Authoritative for the registry and branch policy">
          <div className="px-3 py-2">
            {app.auth.loading && !status ? (
              <LoadingState label="Reading configuration…" />
            ) : app.auth.error ? (
              <ErrorState error={app.auth.error} onRetry={app.refreshAuth} compact />
            ) : (
              <dl className="px-1">
                <KeyValue label="Main repository">
                  <span className="mono">{status?.mainRepository}</span>
                </KeyValue>
                <KeyValue label="Registry branch">
                  <span className="mono">{status?.registryBranch}</span>
                </KeyValue>
                <KeyValue label="Registry entries">{app.subsystems.data?.length ?? 0}</KeyValue>
                <KeyValue label="Data source">
                  <span
                    className={cx(
                      'chip',
                      status?.dataSource === 'fixture'
                        ? 'bg-amber-950/70 text-amber-200'
                        : 'bg-emerald-950/70 text-emerald-300',
                    )}
                  >
                    {status?.dataSource ?? 'unknown'}
                  </span>
                </KeyValue>
              </dl>
            )}
            <div className="mt-2 flex flex-wrap gap-2 px-1">
              <Button
                size="sm"
                variant="secondary"
                icon={ExternalLink}
                onClick={() =>
                  status?.mainRepository &&
                  app.openExternal(`https://github.com/${status.mainRepository}`)
                }
                disabled={!status?.mainRepository}
              >
                Open main repository
              </Button>
            </div>
          </div>
        </Panel>

        <RegistryEditor />

        <Panel
          title="GitHub CLI"
          subtitle="The desktop app uses your existing gh login; credentials are never read by the frontend"
          actions={
            <Button size="sm" variant="secondary" icon={RefreshCw} onClick={app.refreshAuth}>
              Re-check
            </Button>
          }
        >
          <div className="space-y-3 p-3">
            <dl className="rounded-lg border border-ink-700/60 bg-ink-850/50 px-3 py-1">
              <KeyValue label="gh installed">{status?.ghInstalled ? 'yes' : 'no'}</KeyValue>
              <KeyValue label="Version">
                <span className="mono">{status?.ghVersion ?? '—'}</span>
              </KeyValue>
              <KeyValue label="Hostname">
                <span className="mono">{status?.hostname ?? 'github.com'}</span>
              </KeyValue>
              <KeyValue label="Authenticated">{status?.authenticated ? 'yes' : 'no'}</KeyValue>
              <KeyValue label="Account">
                <span className="mono">{status?.accountLogin ?? 'not signed in'}</span>
              </KeyValue>
              <KeyValue label="Token scopes">
                <span className="mono">{status?.scopes.join(', ') || 'not reported'}</span>
              </KeyValue>
              <KeyValue label="Token available">{status?.tokenAvailable ? 'yes' : 'no'}</KeyValue>
              <KeyValue label="git">
                <span className="mono">
                  {status?.gitInstalled ? (status?.gitVersion ?? 'installed') : 'missing'}
                </span>
              </KeyValue>
              <KeyValue label="Package manager">
                <span className="mono">
                  {status?.packageManager
                    ? `${status.packageManager} ${status.packageManagerVersion ?? ''}`.trim()
                    : 'missing'}
                </span>
              </KeyValue>
              <KeyValue label="Checked at">{formatDateTime(status?.checkedAt)}</KeyValue>
            </dl>

            {status?.remedy ? (
              <InlineAlert
                tone={status.authenticated ? 'warning' : 'danger'}
                title={status.message}
              >
                <span className="mono">{status.remedy}</span>
              </InlineAlert>
            ) : (
              <InlineAlert tone="success" title={status?.message ?? 'GitHub CLI is ready'}>
                Reads and promotions use <span className="mono">gh api</span> and{' '}
                <span className="mono">git</span> with explicit argument arrays — never a
                shell-assembled string. The tool opens an integration PR and adds no files to the
                target repositories.
              </InlineAlert>
            )}

            <div className="rounded-lg border border-ink-700/60 bg-ink-925 p-3">
              <p className="flex items-center gap-2 text-2xs font-semibold uppercase tracking-wide text-slate-400">
                <Terminal className="h-3.5 w-3.5" aria-hidden="true" />
                Required setup
              </p>
              <pre className="mono mt-2 whitespace-pre-wrap text-2xs leading-relaxed text-slate-300">
                {`gh auth login --hostname github.com --scopes repo
gh auth status
git --version
npm --version`}
              </pre>
              <p className="mt-2 flex items-start gap-2 text-2xs text-slate-500">
                <KeyRound className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                Promotion needs a token with write access to the main repository (the{' '}
                <span className="mono">repo</span> scope). Without it the request fails with an
                actionable error instead of a silent success.
              </p>
            </div>
          </div>
        </Panel>

        <Panel title="Local cache" subtitle="Disposable and rebuildable — never authoritative">
          <div className="space-y-3 p-3">
            {cache.loading && !cache.data ? (
              <LoadingState label="Reading cache statistics…" />
            ) : (
              <dl className="rounded-lg border border-ink-700/60 bg-ink-850/50 px-3 py-1">
                <KeyValue label="Entries">
                  <span className="mono">
                    {cache.data?.entries ?? 0} / {cache.data?.capacity ?? 0}
                  </span>
                </KeyValue>
                <KeyValue label="Hits / misses">
                  <span className="mono">
                    {cache.data?.hits ?? 0} / {cache.data?.misses ?? 0}
                  </span>
                </KeyValue>
                <KeyValue label="Oldest entry">
                  <span className="mono">{cache.data?.oldestEntryAt ?? '—'}</span>
                </KeyValue>
                <KeyValue label="Rebuildable">{cache.data?.rebuildable ? 'yes' : 'no'}</KeyValue>
              </dl>
            )}

            <p className="text-2xs leading-relaxed text-slate-400">{cache.data?.note}</p>

            {cacheMessage ? (
              <InlineAlert tone="success" title="Cache action">
                {cacheMessage}
              </InlineAlert>
            ) : null}

            <div className="flex flex-wrap gap-2">
              <Button variant="secondary" icon={Trash2} onClick={clearCache}>
                Clear cache
              </Button>
              <Button variant="secondary" icon={RefreshCw} onClick={() => cache.reload()}>
                Re-read statistics
              </Button>
            </div>

            <p className="flex items-start gap-2 text-2xs text-slate-500">
              <Database className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              No SQLite database and no <span className="mono">integrated-commits.json</span> ledger
              exists. Integration state is derived from Git history plus GitHub PRs and runs.
            </p>
          </div>
        </Panel>

        <Panel
          title="Appearance"
          subtitle="Dark desktop dashboard matching the approved design reference"
        >
          <div className="space-y-3 p-3">
            <div className="flex items-center gap-3 rounded-lg border border-ink-700/60 bg-ink-850/50 p-3">
              <Monitor className="h-4 w-4 text-slate-400" aria-hidden="true" />
              <div className="flex-1">
                <p className="text-xs font-semibold text-slate-100">Theme: dark (blue-gray)</p>
                <p className="text-2xs text-slate-400">
                  The window starts dark and uses the OS webview. A light theme is not implemented;
                  the toggle is intentionally absent rather than pretending to work.
                </p>
              </div>
            </div>
            <div className="flex items-center gap-3 rounded-lg border border-ink-700/60 bg-ink-850/50 p-3">
              <ShieldAlert className="h-4 w-4 text-amber-300" aria-hidden="true" />
              <div className="flex-1">
                <p className="text-xs font-semibold text-slate-100">Tauri permissions</p>
                <p className="text-2xs text-slate-400">
                  The window requests core IPC plus the title-bar window controls only. No
                  filesystem, shell or process permission is granted to the frontend.
                </p>
              </div>
            </div>
          </div>
        </Panel>

        <Panel
          className="xl:col-span-2"
          title="Diagnostics"
          subtitle="Copy this when reporting a problem"
          actions={
            <Button
              size="sm"
              variant="secondary"
              icon={RefreshCw}
              onClick={() => {
                void navigator.clipboard?.writeText(diagnostics);
                app.pushNotice({ tone: 'success', title: 'Diagnostics copied to the clipboard' });
              }}
            >
              Copy diagnostics
            </Button>
          }
        >
          <div className="p-3">
            <pre className="mono max-h-[260px] overflow-auto whitespace-pre-wrap rounded-lg border border-ink-700/60 bg-ink-925 p-3 text-2xs leading-relaxed text-slate-300">
              {diagnostics}
            </pre>
          </div>
        </Panel>
      </div>
    </div>
  );
}

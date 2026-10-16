import { useState } from 'react';
import { ExternalLink, KeyRound, RefreshCw, ShieldAlert, Terminal, Trash2 } from 'lucide-react';

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
import { ThemeControl } from '../components/ThemeControl';
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

  const clearCheckouts = () => {
    void api
      .clearCheckouts()
      .then((result) => {
        setCacheMessage(result.message);
        cache.reload();
      })
      .catch(() => setCacheMessage('Deleting the cached clones failed.'));
  };

  const diagnostics = [
    `app: VOS Origins 0.1.0`,
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
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[50px] flex-wrap items-center gap-3 border-b border-line px-5 py-2">
        <h1 className="truncate text-[14px] font-semibold text-fg">Settings</h1>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <Button variant="secondary" icon={RefreshCw} onClick={app.bumpDataVersion}>
            Refresh everything
          </Button>
        </div>
      </header>

      <div className="scroll-area min-h-0 flex-1 space-y-4 p-5">
        {status?.dataSource === 'fixture' ? (
          <InlineAlert tone="warning" title="Fixture data mode">
            Sample data. Unset <span className="mono">VOS_USE_FIXTURES</span> for live data.
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
                  <KeyValue label="Registry entries">{app.subsystems.data?.length ?? 0}</KeyValue>
                  <KeyValue label="Data source">
                    <span
                      className={cx(
                        'meta',
                        status?.dataSource === 'fixture'
                          ? 'text-status-unknown-ink'
                          : 'text-status-applied-ink',
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
            subtitle="Uses your existing gh login"
            actions={
              <Button size="sm" variant="secondary" icon={RefreshCw} onClick={app.refreshAuth}>
                Re-check
              </Button>
            }
          >
            <div className="space-y-3 p-3">
              <dl className="rounded-md border border-line bg-raised px-3 py-1">
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
              ) : null}

              <div className="rounded-md border border-line bg-ground p-3">
                <p className="flex items-center gap-2 text-[11px] font-semibold tracking-wide text-muted uppercase">
                  <Terminal className="h-3.5 w-3.5" aria-hidden="true" />
                  Required setup
                </p>
                <pre className="mono mt-2 whitespace-pre-wrap leading-relaxed text-fg">
                  {`gh auth login --hostname github.com --scopes repo
gh auth status
git --version
npm --version`}
                </pre>
                <p className="mt-2 flex items-start gap-2 text-[11px] text-muted">
                  <KeyRound className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                  Promotion needs a token with write access to the main repository (the{' '}
                  <span className="mono">repo</span> scope).
                </p>
              </div>
            </div>
          </Panel>

          <Panel title="Local cache" subtitle="Disposable and rebuildable — never authoritative">
            <div className="space-y-3 p-3">
              {cache.loading && !cache.data ? (
                <LoadingState label="Reading cache statistics…" />
              ) : (
                <dl className="rounded-md border border-line bg-raised px-3 py-1">
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

              <p className="truncate text-[11px] text-muted">{cache.data?.note}</p>

              {cacheMessage ? (
                <InlineAlert tone="success" title="Cache action">
                  {cacheMessage}
                </InlineAlert>
              ) : null}

              <div className="flex flex-wrap gap-2">
                <Button variant="secondary" icon={Trash2} onClick={clearCache}>
                  Clear cache
                </Button>
                <Button variant="secondary" icon={Trash2} onClick={clearCheckouts}>
                  Clear cached clones
                </Button>
                <Button variant="secondary" icon={RefreshCw} onClick={() => cache.reload()}>
                  Re-read statistics
                </Button>
              </div>
            </div>
          </Panel>

          <Panel title="Appearance" subtitle="Display and theme">
            <div className="space-y-3 p-3">
              <div className="flex items-center gap-3 rounded-md border border-line bg-raised p-3">
                <div className="flex-1">
                  <p className="text-xs font-semibold text-fg">Theme</p>
                </div>
                <ThemeControl />
              </div>
              <div className="flex items-center gap-3 rounded-md border border-line bg-raised p-3">
                <ShieldAlert className="h-4 w-4 shrink-0 text-status-unknown" aria-hidden="true" />
                <div className="flex-1">
                  <p className="text-xs font-semibold text-fg">Tauri permissions</p>
                  <p className="truncate text-[11px] text-muted">
                    Core IPC plus title-bar window controls only.
                  </p>
                </div>
              </div>
            </div>
          </Panel>

          <Panel
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
              <pre className="mono max-h-[260px] overflow-auto rounded-md border border-line bg-ground p-3 leading-relaxed whitespace-pre-wrap text-muted">
                {diagnostics}
              </pre>
            </div>
          </Panel>
        </div>
      </div>
    </div>
  );
}

import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import {
  Download,
  ExternalLink,
  KeyRound,
  RefreshCw,
  ShieldAlert,
  Terminal,
  Trash2,
} from 'lucide-react';

import { api, isTauriRuntime } from '../api/invoke';
import type { CacheStats } from '../api/types';
import { useApp, useAsyncData } from '../app/context';
import type { UpdateInfo } from '../app/updater';
import { currentVersion, findUpdate, installUpdate } from '../app/updater';
import { Button, InlineAlert, KeyValue, LoadingState, Panel } from '../components/ui/primitives';
import { ThemeControl } from '../components/ThemeControl';
import { formatDateTime } from '../components/ui/format';
import { RegistryEditor } from '../components/RegistryEditor';
import { cx } from '../components/ui/cx';

type SettingsGroup =
  'connection' | 'registry' | 'storage' | 'updates' | 'appearance' | 'diagnostics';

const GROUPS: Array<{ id: SettingsGroup; label: string }> = [
  { id: 'connection', label: 'Connection' },
  { id: 'registry', label: 'Registry' },
  { id: 'storage', label: 'Storage' },
  { id: 'updates', label: 'Updates' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'diagnostics', label: 'Diagnostics' },
];

/** Settings (spec §3.3). */
export function Settings() {
  const app = useApp();
  const [cacheMessage, setCacheMessage] = useState<string | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [percent, setPercent] = useState<number | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const railRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void currentVersion().then(setVersion);
  }, []);

  const runCheck = async () => {
    setChecking(true);
    setUpdateError(null);
    const found = await findUpdate();
    setUpdate(found);
    setChecked(true);
    setChecking(false);
  };

  const runInstall = async () => {
    if (!update) return;
    setInstalling(true);
    setPercent(null);
    setUpdateError(null);
    try {
      await installUpdate(update.update, setPercent);
    } catch (error) {
      setInstalling(false);
      setUpdateError(error instanceof Error ? error.message : 'The update could not be installed.');
    }
  };

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
    `app: VOS Origins`,
    `app version: ${version ?? 'unknown'}`,
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

  // Derived, never forced: an unauthenticated tool opens on Diagnostics until the user picks a group.
  const needsAttention = Boolean(status && !status.authenticated);
  const [groupOverride, setGroupOverride] = useState<SettingsGroup | null>(null);
  const group: SettingsGroup = groupOverride ?? (needsAttention ? 'diagnostics' : 'connection');

  // Same roving-focus idiom as the AppShell nav, extended to the vertical axis.
  const onRailKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (
      event.key !== 'ArrowUp' &&
      event.key !== 'ArrowDown' &&
      event.key !== 'ArrowLeft' &&
      event.key !== 'ArrowRight'
    ) {
      return;
    }
    const tabs = Array.from(railRef.current?.querySelectorAll<HTMLElement>('[role="tab"]') ?? []);
    const index = tabs.indexOf(document.activeElement as HTMLElement);
    if (index === -1) return;
    event.preventDefault();
    const delta = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? 1 : -1;
    tabs[(index + delta + tabs.length) % tabs.length]?.focus();
  };

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

      {status?.dataSource === 'fixture' ? (
        <div className="shrink-0 border-b border-line px-5 py-2">
          <InlineAlert tone="warning" title="Fixture data mode">
            Sample data. Unset <span className="mono">VOS_USE_FIXTURES</span> for live data.
          </InlineAlert>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Settings"
          className="w-[250px] shrink-0 overflow-y-auto border-r border-line bg-surface"
        >
          <div
            ref={railRef}
            role="tablist"
            aria-label="Settings groups"
            aria-orientation="vertical"
            onKeyDown={onRailKeyDown}
            className="flex flex-col divide-y divide-line"
          >
            {GROUPS.map((item) => {
              const active = item.id === group;
              return (
                <button
                  key={item.id}
                  type="button"
                  role="tab"
                  id={`settings-tab-${item.id}`}
                  aria-selected={active}
                  aria-controls={`settings-panel-${item.id}`}
                  onClick={() => setGroupOverride(item.id)}
                  className={cx(
                    'relative flex h-[29px] shrink-0 items-center px-3 text-xs whitespace-nowrap transition-colors focus-visible:-outline-offset-4',
                    active
                      ? 'font-semibold text-fg'
                      : 'font-normal text-muted hover:bg-hover hover:text-fg',
                  )}
                >
                  <span className="truncate">{item.label}</span>
                  {active ? (
                    <span
                      aria-hidden="true"
                      className="absolute inset-y-1 left-0 w-[3px] bg-accent"
                    />
                  ) : null}
                </button>
              );
            })}
          </div>
        </nav>

        <div className="scroll-area min-h-0 flex-1 p-5">
          <section
            role="tabpanel"
            id={`settings-panel-${group}`}
            aria-labelledby={`settings-tab-${group}`}
          >
            {group === 'connection' ? (
              <Panel
                title="GitHub CLI"
                subtitle="Uses your existing gh login"
                actions={
                  <Button size="sm" variant="secondary" icon={RefreshCw} onClick={app.refreshAuth}>
                    Re-check
                  </Button>
                }
              >
                <div className="p-3">
                  <dl className="rounded-md border border-line bg-raised px-3 py-1">
                    <KeyValue label="gh installed">{status?.ghInstalled ? 'yes' : 'no'}</KeyValue>
                    <KeyValue label="Version">
                      <span className="mono">{status?.ghVersion ?? '—'}</span>
                    </KeyValue>
                    <KeyValue label="Hostname">
                      <span className="mono">{status?.hostname ?? 'github.com'}</span>
                    </KeyValue>
                    <KeyValue label="Authenticated">
                      {status?.authenticated ? 'yes' : 'no'}
                    </KeyValue>
                    <KeyValue label="Account">
                      <span className="mono">{status?.accountLogin ?? 'not signed in'}</span>
                    </KeyValue>
                    <KeyValue label="Token scopes">
                      <span className="mono">{status?.scopes.join(', ') || 'not reported'}</span>
                    </KeyValue>
                    <KeyValue label="Token available">
                      {status?.tokenAvailable ? 'yes' : 'no'}
                    </KeyValue>
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
                </div>
              </Panel>
            ) : null}

            {group === 'registry' ? <RegistryEditor /> : null}

            {group === 'storage' ? (
              <Panel
                title="Local cache"
                subtitle="Disposable and rebuildable — never authoritative"
              >
                <div className="space-y-3 p-3">
                  {cache.loading && !cache.data ? (
                    <LoadingState label="Reading cache statistics…" />
                  ) : (
                    <dl className="rounded-md border border-line bg-raised px-3 py-1">
                      <KeyValue label="Entries">
                        <span className="mono tabular-nums">
                          {cache.data?.entries ?? 0} / {cache.data?.capacity ?? 0}
                        </span>
                      </KeyValue>
                      <KeyValue label="Hits / misses">
                        <span className="mono tabular-nums">
                          {cache.data?.hits ?? 0} / {cache.data?.misses ?? 0}
                        </span>
                      </KeyValue>
                      <KeyValue label="Oldest entry">
                        <span className="mono">{cache.data?.oldestEntryAt ?? '—'}</span>
                      </KeyValue>
                      <KeyValue label="Rebuildable">
                        {cache.data?.rebuildable ? 'yes' : 'no'}
                      </KeyValue>
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
            ) : null}

            {group === 'updates' ? (
              <Panel
                title="Updates"
                subtitle="Signed releases published on GitHub"
                actions={
                  <Button
                    size="sm"
                    variant="secondary"
                    icon={RefreshCw}
                    onClick={() => void runCheck()}
                    disabled={checking || installing}
                  >
                    {checking ? 'Checking…' : 'Check for updates'}
                  </Button>
                }
              >
                <div className="space-y-3 p-3">
                  <dl className="rounded-md border border-line bg-raised px-3 py-1">
                    <KeyValue label="Installed version">
                      <span className="mono">{version ?? '—'}</span>
                    </KeyValue>
                    <KeyValue label="Channel">
                      <span className="mono">stable</span>
                    </KeyValue>
                  </dl>

                  {update ? (
                    <div className="space-y-2 rounded-md border border-line bg-raised p-3">
                      <div className="flex items-center justify-between gap-3">
                        <p className="text-xs font-semibold text-fg">
                          Version <span className="mono">{update.version}</span> is available
                        </p>
                        <Button
                          size="sm"
                          icon={Download}
                          onClick={() => void runInstall()}
                          disabled={installing}
                        >
                          {installing ? 'Installing…' : 'Install and restart'}
                        </Button>
                      </div>
                      {update.notes ? (
                        <pre className="mono max-h-[180px] overflow-auto rounded-md border border-line bg-ground p-2 leading-relaxed whitespace-pre-wrap text-muted">
                          {update.notes}
                        </pre>
                      ) : null}
                      {installing ? (
                        <div
                          role="progressbar"
                          aria-label="Update download"
                          aria-valuemin={0}
                          aria-valuemax={100}
                          aria-valuenow={percent ?? undefined}
                          className="h-1.5 overflow-hidden rounded-full bg-line"
                        >
                          <div
                            className="h-full bg-accent transition-[width]"
                            style={{ width: `${percent ?? 0}%` }}
                          />
                        </div>
                      ) : (
                        <p className="text-[11px] text-muted">
                          The installer runs on its own and the app reopens on the new version.
                        </p>
                      )}
                    </div>
                  ) : null}

                  {!update && checked && !checking ? (
                    <p className="text-[11px] text-muted">
                      This build is the newest release.
                      {version ? ` Running ${version}.` : ''}
                    </p>
                  ) : null}

                  {updateError ? (
                    <InlineAlert tone="danger" title="Update failed">
                      {updateError}
                    </InlineAlert>
                  ) : null}
                </div>
              </Panel>
            ) : null}

            {group === 'appearance' ? (
              <Panel title="Appearance" subtitle="Display and theme">
                <div className="space-y-3 p-3">
                  <div className="flex items-center gap-3 rounded-md border border-line bg-raised p-3">
                    <div className="flex-1">
                      <p className="text-xs font-semibold text-fg">Theme</p>
                    </div>
                    <ThemeControl />
                  </div>
                  <div className="flex items-center gap-3 rounded-md border border-line bg-raised p-3">
                    <ShieldAlert
                      className="h-4 w-4 shrink-0 text-status-unknown"
                      aria-hidden="true"
                    />
                    <div className="flex-1">
                      <p className="text-xs font-semibold text-fg">Tauri permissions</p>
                      <p className="truncate text-[11px] text-muted">
                        Core IPC, title-bar window controls, and signed updates.
                      </p>
                    </div>
                  </div>
                </div>
              </Panel>
            ) : null}

            {group === 'diagnostics' ? (
              <Panel
                title="Diagnostics"
                subtitle="Copy this when reporting a problem"
                actions={
                  <>
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
                    <Button
                      size="sm"
                      variant="secondary"
                      icon={RefreshCw}
                      onClick={() => {
                        void navigator.clipboard?.writeText(diagnostics);
                        app.pushNotice({
                          tone: 'success',
                          title: 'Diagnostics copied to the clipboard',
                        });
                      }}
                    >
                      Copy diagnostics
                    </Button>
                  </>
                }
              >
                <div className="space-y-3 p-3">
                  <pre className="mono max-h-[260px] overflow-auto rounded-md border border-line bg-ground p-3 leading-relaxed whitespace-pre-wrap text-muted">
                    {diagnostics}
                  </pre>

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
            ) : null}
          </section>
        </div>
      </div>
    </div>
  );
}

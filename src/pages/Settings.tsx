import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import {
  Check,
  Copy,
  Download,
  ExternalLink,
  FolderGit2,
  HardDrive,
  KeyRound,
  Palette,
  Plug,
  RefreshCw,
  ShieldAlert,
  Terminal,
  Trash2,
  type LucideIcon,
} from 'lucide-react';

import { api, isTauriRuntime } from '../api/invoke';
import type { CacheStats } from '../api/types';
import { useApp, useAsyncData, type SettingsGroup } from '../app/context';
import { currentVersion, installUpdate } from '../app/updater';
import { RegistryEditor } from '../components/RegistryEditor';
import { ThemeControl } from '../components/ThemeControl';
import { cx } from '../components/ui/cx';
import { formatDateTime } from '../components/ui/format';
import { Button, InlineAlert, LoadingState } from '../components/ui/primitives';

interface NavGroup {
  heading: string;
  items: Array<{ id: SettingsGroup; label: string; icon: LucideIcon }>;
}

const NAV_GROUPS: NavGroup[] = [
  {
    heading: 'Workspace',
    items: [{ id: 'registry', label: 'Registry', icon: FolderGit2 }],
  },
  {
    heading: 'Environment',
    items: [
      { id: 'connection', label: 'Connection', icon: Plug },
      { id: 'storage', label: 'Storage', icon: HardDrive },
      { id: 'diagnostics', label: 'Diagnostics', icon: Terminal },
    ],
  },
  {
    heading: 'Application',
    items: [
      { id: 'appearance', label: 'Appearance', icon: Palette },
      { id: 'updates', label: 'Updates', icon: Download },
    ],
  },
];

const CLI_PREREQUISITES = `gh auth login --hostname github.com --scopes repo
gh auth status
git --version
npm --version`;

function SectionHeader({
  title,
  subtitle,
  actions,
}: {
  title: string;
  subtitle: string;
  actions?: ReactNode;
}) {
  return (
    <div className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line bg-raised px-5 py-2.5">
      <div className="min-w-0">
        <h2 className="truncate text-xs font-semibold text-fg">{title}</h2>
        <p className="truncate text-[11px] text-muted">{subtitle}</p>
      </div>
      {actions ? <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  );
}

function SettingSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="border-b border-line">
      <div className="border-b border-line/60 bg-raised/40 px-5 py-2">
        <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">{title}</h3>
        {description ? <p className="mt-0.5 text-[11px] text-muted">{description}</p> : null}
      </div>
      <div className="divide-y divide-line">{children}</div>
    </div>
  );
}

function SettingRow({
  label,
  description,
  children,
}: {
  label: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-4 px-5 py-3">
      <div className="min-w-0 flex-1">
        <p className="text-xs font-medium text-fg">{label}</p>
        {description ? (
          <p className="mt-0.5 text-[11px] leading-relaxed text-muted">{description}</p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-2">{children}</div>
    </div>
  );
}

/** Settings (spec §3.3). */
export function Settings() {
  const app = useApp();
  const [cacheMessage, setCacheMessage] = useState<string | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [percent, setPercent] = useState<number | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [copiedCommands, setCopiedCommands] = useState(false);
  const railRef = useRef<HTMLDivElement>(null);

  const update = app.availableUpdate;
  const checking = app.checkingUpdate;
  const checked = app.updateChecked;

  useEffect(() => {
    void currentVersion().then(setVersion);
  }, []);

  const runCheck = async () => {
    setUpdateError(null);
    await app.checkForUpdates();
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
  const group: SettingsGroup = app.settingsGroup ?? (needsAttention ? 'diagnostics' : 'connection');

  const subsystemCount = app.subsystems.data?.length;
  const cacheEntries = cache.data?.entries ?? 0;
  const cacheCapacity = cache.data?.capacity ?? 0;
  const cacheFillPercent = cacheCapacity
    ? Math.min(100, Math.round((cacheEntries / cacheCapacity) * 100))
    : 0;
  const totalLookups = (cache.data?.hits ?? 0) + (cache.data?.misses ?? 0);
  const hitRate = totalLookups > 0 ? Math.round(((cache.data?.hits ?? 0) / totalLookups) * 100) : 0;

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

  const fullHeightPanel = group === 'registry' || group === 'diagnostics';

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex min-h-[48px] flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-2.5">
        <h1 className="truncate text-[14px] font-semibold text-fg">Settings</h1>
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            icon={RefreshCw}
            onClick={app.bumpDataVersion}
            title="Refetch auth status, registry, and cache stats"
          >
            Refresh
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
          className="w-[200px] shrink-0 overflow-y-auto border-r border-line bg-surface px-2 py-3"
        >
          <div
            ref={railRef}
            role="tablist"
            aria-label="Settings groups"
            aria-orientation="vertical"
            onKeyDown={onRailKeyDown}
            className="space-y-4"
          >
            {NAV_GROUPS.map((navGroup) => (
              <div key={navGroup.heading} className="space-y-0.5">
                <p className="px-2.5 pb-1 text-[10px] font-semibold tracking-wider text-disabled uppercase">
                  {navGroup.heading}
                </p>
                {navGroup.items.map((item) => {
                  const active = item.id === group;
                  const Icon = item.icon;
                  const badgeTone =
                    item.id === 'updates' && app.availableUpdate
                      ? 'bg-accent'
                      : item.id === 'diagnostics' && needsAttention
                        ? 'bg-status-unknown'
                        : item.id === 'connection' && status
                          ? status.authenticated
                            ? 'bg-status-applied'
                            : 'bg-status-unknown'
                          : null;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      role="tab"
                      id={`settings-tab-${item.id}`}
                      aria-selected={active}
                      aria-controls={`settings-panel-${item.id}`}
                      onClick={() => app.setSettingsGroup(item.id)}
                      className={cx(
                        'flex h-[29px] w-full shrink-0 items-center gap-2 rounded-md px-2.5 text-xs whitespace-nowrap transition-colors focus-visible:control-focus',
                        active
                          ? 'bg-selection font-semibold text-fg'
                          : 'text-muted hover:bg-hover hover:text-fg',
                      )}
                    >
                      <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate text-left">{item.label}</span>
                      {item.id === 'registry' && subsystemCount != null ? (
                        <span className="mono text-[11px] text-muted tabular-nums">
                          {subsystemCount}
                        </span>
                      ) : badgeTone ? (
                        <span
                          aria-hidden="true"
                          className={cx('h-1.5 w-1.5 shrink-0 rounded-full', badgeTone)}
                        />
                      ) : null}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </nav>

        <div
          className={cx(
            'min-h-0 flex-1 bg-surface',
            fullHeightPanel ? 'flex flex-col' : 'scroll-area',
          )}
        >
          <section
            role="tabpanel"
            id={`settings-panel-${group}`}
            aria-labelledby={`settings-tab-${group}`}
            className={fullHeightPanel ? 'flex min-h-0 flex-1 flex-col' : 'min-h-full'}
          >
            {group === 'connection' ? (
              <>
                <SectionHeader
                  title="GitHub CLI & toolchain"
                  subtitle="Uses your existing gh login and local Git environment"
                  actions={
                    <Button
                      size="sm"
                      variant="secondary"
                      icon={RefreshCw}
                      onClick={app.refreshAuth}
                    >
                      Re-check
                    </Button>
                  }
                />

                <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line bg-surface px-5 py-2.5">
                  <div className="flex min-w-0 items-center gap-2 text-xs">
                    <span
                      aria-hidden="true"
                      className={cx(
                        'h-2 w-2 shrink-0 rounded-full',
                        status?.authenticated ? 'bg-status-applied' : 'bg-status-unknown',
                      )}
                    />
                    <span className="font-semibold text-fg">
                      {status?.authenticated
                        ? `Connected as ${status.accountLogin ?? 'GitHub user'}`
                        : 'GitHub CLI not authenticated'}
                    </span>
                    <span className="mono truncate text-[11px] text-muted">
                      · {status?.hostname ?? 'github.com'}
                    </span>
                  </div>
                  <span className="mono shrink-0 text-[11px] text-muted">
                    Checked {formatDateTime(status?.checkedAt)}
                  </span>
                </div>

                {status?.remedy ? (
                  <div className="border-b border-line px-5 py-3">
                    <InlineAlert
                      tone={status.authenticated ? 'warning' : 'danger'}
                      title={status.message}
                    >
                      <span className="mono">{status.remedy}</span>
                    </InlineAlert>
                  </div>
                ) : null}

                <SettingSection
                  title="GitHub authentication"
                  description="Credentials are read from your local gh CLI session; no token is stored by VOS Origins."
                >
                  <SettingRow label="GitHub CLI" description="Local gh binary used for API calls">
                    <span className="mono text-xs text-fg">
                      {status?.ghInstalled
                        ? `v${status.ghVersion ?? 'installed'}`
                        : 'Not installed'}
                    </span>
                  </SettingRow>
                  <SettingRow label="Account" description="Signed-in GitHub identity">
                    <span className="mono text-xs text-fg">
                      {status?.accountLogin ?? 'Not signed in'}
                    </span>
                  </SettingRow>
                  <SettingRow label="Hostname" description="GitHub host configured in gh">
                    <span className="mono text-xs text-fg">{status?.hostname ?? 'github.com'}</span>
                  </SettingRow>
                  <SettingRow
                    label="Token scopes"
                    description="Promotion and distribution require write access (repo scope)"
                  >
                    <span className="mono text-xs text-muted">
                      {status?.scopes.join(', ') || 'not reported'}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label="Token available"
                    description="Whether gh auth token returns an active credential"
                  >
                    <span className="mono text-xs text-fg">
                      {status?.tokenAvailable ? 'yes' : 'no'}
                    </span>
                  </SettingRow>
                </SettingSection>

                <SettingSection
                  title="Local toolchain"
                  description="Binaries invoked when building commits, diffs, and lockfile updates."
                >
                  <SettingRow
                    label="Git"
                    description="Required for local clones, diffs, and commits"
                  >
                    <span className="mono text-xs text-fg">
                      {status?.gitInstalled ? (status.gitVersion ?? 'installed') : 'Missing'}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label="Package manager"
                    description="Used when regenerating target lockfiles during promotion"
                  >
                    <span className="mono text-xs text-fg">
                      {status?.packageManager
                        ? `${status.packageManager}${status.packageManagerVersion ? ` ${status.packageManagerVersion}` : ''}`
                        : 'Missing'}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label="Main repository"
                    description="Active target repository from your tool-local registry"
                  >
                    <span className="mono text-xs text-fg">{status?.mainRepository ?? '—'}</span>
                  </SettingRow>
                </SettingSection>
              </>
            ) : null}

            {group === 'registry' ? <RegistryEditor /> : null}

            {group === 'storage' ? (
              <>
                <SectionHeader
                  title="Local cache"
                  subtitle="Disposable and rebuildable — Git history and GitHub remain authoritative"
                  actions={
                    <Button
                      variant="secondary"
                      size="sm"
                      icon={RefreshCw}
                      onClick={() => cache.reload()}
                    >
                      Re-read statistics
                    </Button>
                  }
                />

                {cacheMessage ? (
                  <div className="border-b border-line px-5 py-3">
                    <InlineAlert tone="success" title="Cache updated">
                      {cacheMessage}
                    </InlineAlert>
                  </div>
                ) : null}

                {cache.loading && !cache.data ? (
                  <LoadingState label="Reading cache statistics…" />
                ) : (
                  <>
                    <SettingSection
                      title="Usage"
                      description={
                        cache.data?.note ?? 'Short-lived cache for commit listings and file diffs.'
                      }
                    >
                      <SettingRow
                        label="Cache entries"
                        description="In-memory entries currently held before eviction"
                      >
                        <div className="flex items-center gap-3">
                          <div className="h-1.5 w-28 overflow-hidden rounded-full bg-line">
                            <div
                              className="h-full bg-accent transition-[width]"
                              style={{ width: `${cacheFillPercent}%` }}
                            />
                          </div>
                          <span className="mono text-xs text-fg tabular-nums">
                            {cacheEntries} / {cacheCapacity}
                          </span>
                        </div>
                      </SettingRow>
                      <SettingRow
                        label="Session lookups"
                        description="Hits versus misses since the app started"
                      >
                        <span className="mono text-xs text-fg tabular-nums">
                          {cache.data?.hits ?? 0} hits · {cache.data?.misses ?? 0} misses
                          {totalLookups > 0 ? ` (${hitRate}%)` : ''}
                        </span>
                      </SettingRow>
                      <SettingRow
                        label="Oldest entry"
                        description="Age of the earliest entry currently in cache"
                      >
                        <span className="mono text-xs text-fg">
                          {cache.data?.oldestEntryAt ?? '—'}
                        </span>
                      </SettingRow>
                      <SettingRow
                        label="Rebuildable"
                        description="All cached data can be dropped safely without data loss"
                      >
                        <span className="mono text-xs text-fg">
                          {cache.data?.rebuildable ? 'yes' : 'no'}
                        </span>
                      </SettingRow>
                    </SettingSection>

                    <SettingSection
                      title="Maintenance"
                      description="Clearing caches only costs a fresh fetch on the next read."
                    >
                      <SettingRow
                        label="Clear metadata cache"
                        description="Drops cached commit pages, diffs, and API responses."
                      >
                        <Button variant="secondary" size="sm" icon={Trash2} onClick={clearCache}>
                          Clear cache
                        </Button>
                      </SettingRow>
                      <SettingRow
                        label="Clear cached repository clones"
                        description="Deletes tool-managed local clones on disk so the next operation clones cleanly."
                      >
                        <Button
                          variant="secondary"
                          size="sm"
                          icon={Trash2}
                          onClick={clearCheckouts}
                        >
                          Clear cached clones
                        </Button>
                      </SettingRow>
                    </SettingSection>
                  </>
                )}
              </>
            ) : null}

            {group === 'updates' ? (
              <>
                <SectionHeader
                  title="Updates"
                  subtitle="Signed desktop releases published on GitHub"
                  actions={
                    <>
                      <Button
                        size="sm"
                        variant="secondary"
                        icon={RefreshCw}
                        onClick={() => void runCheck()}
                        disabled={checking || installing}
                      >
                        {checking ? 'Checking…' : 'Check for updates'}
                      </Button>
                      {update ? (
                        <Button
                          size="sm"
                          variant="primary"
                          icon={Download}
                          onClick={() => void runInstall()}
                          disabled={installing}
                        >
                          {installing ? 'Installing…' : 'Install and restart'}
                        </Button>
                      ) : null}
                    </>
                  }
                />

                {updateError ? (
                  <div className="border-b border-line px-5 py-3">
                    <InlineAlert tone="danger" title="Update failed">
                      {updateError}
                    </InlineAlert>
                  </div>
                ) : null}

                <SettingSection title="Current installation">
                  <SettingRow
                    label="Installed version"
                    description="Build currently running on this machine"
                  >
                    <span className="mono text-xs font-semibold text-fg">
                      {version ? `v${version}` : '—'}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label="Release channel"
                    description="Signed releases verified before installation"
                  >
                    <span className="mono text-xs text-muted">stable</span>
                  </SettingRow>
                  <SettingRow label="Status" description="Latest release check result">
                    <span className="text-xs text-fg">
                      {checking
                        ? 'Checking GitHub releases…'
                        : update
                          ? `v${update.version} available`
                          : checked
                            ? 'Up to date'
                            : 'Not checked since startup'}
                    </span>
                  </SettingRow>
                </SettingSection>

                {update ? (
                  <div className="space-y-3 p-5">
                    <div className="flex items-center justify-between gap-2">
                      <h3 className="text-xs font-semibold text-fg">
                        Release notes · <span className="mono">v{update.version}</span>
                      </h3>
                      {!installing ? (
                        <span className="text-[11px] text-muted">
                          Restarts automatically after install
                        </span>
                      ) : null}
                    </div>
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
                    ) : null}
                    {update.notes ? (
                      <pre className="mono max-h-[340px] overflow-auto rounded-md border border-line bg-ground p-3.5 leading-relaxed whitespace-pre-wrap text-muted">
                        {update.notes}
                      </pre>
                    ) : (
                      <p className="text-[11px] text-muted">No release notes attached.</p>
                    )}
                  </div>
                ) : null}
              </>
            ) : null}

            {group === 'appearance' ? (
              <>
                <SectionHeader title="Appearance" subtitle="Display theme and desktop runtime" />

                <SettingSection title="Display">
                  <SettingRow
                    label="Theme"
                    description="Follow your operating system appearance or lock to Light or Dark."
                  >
                    <ThemeControl />
                  </SettingRow>
                </SettingSection>

                <SettingSection title="Desktop runtime">
                  <SettingRow
                    label="Runtime"
                    description="Native desktop webview host for local Git and GitHub CLI execution"
                  >
                    <span className="mono text-xs text-fg">
                      {isTauriRuntime() ? 'Tauri v2' : 'Browser preview'}
                    </span>
                  </SettingRow>
                  <SettingRow
                    label="Permissions"
                    description="Core IPC, custom title-bar window controls, and signed GitHub release updates."
                  >
                    <span className="flex items-center gap-1.5 text-[11px] text-muted">
                      <ShieldAlert className="h-3.5 w-3.5 text-status-unknown" aria-hidden="true" />
                      Scoped
                    </span>
                  </SettingRow>
                </SettingSection>
              </>
            ) : null}

            {group === 'diagnostics' ? (
              <>
                <SectionHeader
                  title="Diagnostics"
                  subtitle="Environment snapshot and required local tooling"
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
                        variant="primary"
                        icon={Copy}
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
                />

                {status?.remedy ? (
                  <div className="shrink-0 border-b border-line px-5 py-3">
                    <InlineAlert
                      tone={status.authenticated ? 'warning' : 'danger'}
                      title={status.message}
                    >
                      <span className="mono">{status.remedy}</span>
                    </InlineAlert>
                  </div>
                ) : null}

                <div className="flex min-h-0 flex-1 flex-col lg:flex-row">
                  <div className="flex min-h-0 min-w-0 flex-1 flex-col border-b border-line lg:border-r lg:border-b-0">
                    <div className="border-b border-line bg-raised/40 px-4 py-2">
                      <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                        Environment report
                      </h3>
                    </div>
                    <pre className="mono scroll-area min-h-0 flex-1 bg-ground p-4 text-[11px] leading-relaxed whitespace-pre-wrap text-muted">
                      {diagnostics}
                    </pre>
                  </div>

                  <aside className="flex min-h-0 w-full shrink-0 flex-col bg-surface lg:w-[380px]">
                    <div className="flex items-center justify-between gap-2 border-b border-line bg-raised/40 px-4 py-2">
                      <h3 className="text-[11px] font-semibold tracking-wide text-muted uppercase">
                        Required setup
                      </h3>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={copiedCommands ? Check : Copy}
                        onClick={() => {
                          void navigator.clipboard?.writeText(CLI_PREREQUISITES);
                          setCopiedCommands(true);
                          window.setTimeout(() => setCopiedCommands(false), 1600);
                        }}
                      >
                        {copiedCommands ? 'Copied' : 'Copy'}
                      </Button>
                    </div>
                    <div className="scroll-area min-h-0 flex-1 space-y-3 p-4">
                      <p className="text-[11px] leading-relaxed text-muted">
                        Run these commands in a terminal to verify your local prerequisites:
                      </p>
                      <pre className="mono rounded-md border border-line bg-ground p-3 text-[11px] leading-relaxed whitespace-pre-wrap text-fg">
                        {CLI_PREREQUISITES}
                      </pre>
                      <p className="flex items-start gap-2 text-[11px] leading-relaxed text-muted">
                        <KeyRound className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                        <span>
                          Promotion needs a token with write access to the main repository (the{' '}
                          <span className="mono text-fg">repo</span> scope).
                        </span>
                      </p>
                    </div>
                  </aside>
                </div>
              </>
            ) : null}
          </section>
        </div>
      </div>
    </div>
  );
}

import { useEffect, useRef, useState, type ReactNode } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Building2,
  ChevronDown,
  CircleAlert,
  Copy,
  Maximize2,
  Minus,
  RefreshCw,
  ShieldAlert,
  Square,
  X,
  CircleCheck,
} from 'lucide-react';

import type { AuthStatus } from '../api/types';
import { isTauriRuntime } from '../api/invoke';
import { NAV_ITEMS, type Route } from '../app/navigation';
import type { AsyncResult, Notice } from '../app/context';
import { cx } from './ui/cx';
import { GitHubMark, IconButton, InlineAlert } from './ui/primitives';

/** Persistent desktop shell (spec §3.1). The target branch stays visible because statuses are per (subsystem, SHA, target). */

export interface AppShellProps {
  route: Route;
  onNavigate: (route: Route) => void;
  auth: AsyncResult<AuthStatus>;
  onRefreshAuth: () => void;
  onSwitchAccount: () => void;
  targetBranch: string;
  selectedSubsystemName: string | null;
  notice: Notice | null;
  onDismissNotice: () => void;
  children: ReactNode;
}

function GridMark() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 text-white">
      <rect x="3" y="3" width="8" height="8" rx="2" fill="currentColor" />
      <rect x="13" y="3" width="8" height="8" rx="2" fill="currentColor" fillOpacity="0.75" />
      <rect x="3" y="13" width="8" height="8" rx="2" fill="currentColor" fillOpacity="0.75" />
      <rect x="13" y="13" width="8" height="8" rx="2" fill="currentColor" fillOpacity="0.5" />
    </svg>
  );
}

/**
 * Resize handles for the undecorated window (z-40 stays below the z-50 dialog layer).
 * `ResizeDirection` mirrors the unexported type from `@tauri-apps/api/window`.
 */
type ResizeDirection =
  'East' | 'North' | 'NorthEast' | 'NorthWest' | 'South' | 'SouthEast' | 'SouthWest' | 'West';

const RESIZE_HANDLES: Array<{ direction: ResizeDirection; className: string }> = [
  { direction: 'North', className: 'left-3 right-3 top-0 h-1 cursor-ns-resize' },
  { direction: 'South', className: 'left-3 right-3 bottom-0 h-1 cursor-ns-resize' },
  { direction: 'West', className: 'bottom-3 left-0 top-3 w-1 cursor-ew-resize' },
  { direction: 'East', className: 'bottom-3 right-0 top-3 w-1 cursor-ew-resize' },
  { direction: 'NorthWest', className: 'left-0 top-0 h-3 w-3 cursor-nwse-resize' },
  { direction: 'NorthEast', className: 'right-0 top-0 h-3 w-3 cursor-nesw-resize' },
  { direction: 'SouthWest', className: 'bottom-0 left-0 h-3 w-3 cursor-nesw-resize' },
  { direction: 'SouthEast', className: 'bottom-0 right-0 h-3 w-3 cursor-nwse-resize' },
];

function WindowResizeHandles() {
  return (
    <div className="pointer-events-none fixed inset-0 z-40" aria-hidden="true">
      {RESIZE_HANDLES.map(({ direction, className }) => (
        <div
          key={direction}
          data-tauri-no-drag
          className={`pointer-events-auto absolute ${className}`}
          onMouseDown={(event) => {
            event.preventDefault();
            if (!isTauriRuntime()) return;
            void getCurrentWindow()
              .startResizeDragging(direction)
              .catch(() => undefined);
          }}
        />
      ))}
    </div>
  );
}

function WindowsControls() {
  const [maximized, setMaximized] = useState(false);

  const withWindow = async (action: 'minimize' | 'toggleMaximize' | 'close') => {
    if (!isTauriRuntime()) return;
    try {
      const window_ = getCurrentWindow();
      if (action === 'minimize') await window_.minimize();
      if (action === 'toggleMaximize') {
        await window_.toggleMaximize();
        setMaximized(await window_.isMaximized());
      }
      if (action === 'close') await window_.close();
    } catch {
      /* The window API is only available inside Tauri; ignore in a browser tab. */
    }
  };

  return (
    <div className="flex items-center gap-1" data-tauri-no-drag>
      <IconButton
        icon={Minus}
        label="Minimize window"
        onClick={() => void withWindow('minimize')}
      />
      <IconButton
        icon={maximized ? Square : Maximize2}
        label={maximized ? 'Restore window' : 'Maximize window'}
        onClick={() => void withWindow('toggleMaximize')}
      />
      <IconButton
        icon={X}
        label="Close window"
        tone="danger"
        onClick={() => void withWindow('close')}
      />
    </div>
  );
}

function AccountMenu({
  auth,
  onRefreshAuth,
  onSwitchAccount,
}: {
  auth: AsyncResult<AuthStatus>;
  onRefreshAuth: () => void;
  onSwitchAccount: () => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  const status = auth.data;
  const login = status?.accountLogin;

  return (
    <div ref={ref} className="relative" data-tauri-no-drag>
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="GitHub account menu"
        className="flex items-center gap-2.5 rounded-lg border border-transparent px-2 py-1.5 text-left transition-colors hover:border-ink-600 hover:bg-ink-800/70"
      >
        <span className="flex h-8 w-8 items-center justify-center rounded-full bg-ink-700 text-slate-200">
          {login ? (
            <GitHubMark className="h-4 w-4" />
          ) : (
            <CircleAlert className="h-4 w-4 text-amber-300" />
          )}
        </span>
        <span className="hidden min-w-0 leading-tight sm:block">
          <span className="block truncate text-xs font-semibold text-slate-100">
            {login ?? 'Not signed in'}
          </span>
          <span className="block truncate text-2xs text-slate-400">
            {status?.organization ?? 'GitHub organization'}
          </span>
        </span>
        <ChevronDown className="h-4 w-4 text-slate-400" aria-hidden="true" />
      </button>

      {open ? (
        <div
          role="menu"
          className="absolute right-0 z-40 mt-2 w-80 rounded-xl border border-ink-700 bg-ink-870 p-2 shadow-popover animate-fade-in"
        >
          <div className="px-2 py-1.5">
            <p className="text-2xs uppercase tracking-wide text-slate-500">GitHub CLI identity</p>
            <p className="mt-0.5 text-sm font-semibold text-slate-100">
              {login ?? 'No authenticated account'}
            </p>
            <p className="mono text-2xs text-slate-400">
              {status?.hostname ?? 'github.com'}
              {status?.ghVersion ? ` · gh ${status.ghVersion}` : ''}
            </p>
          </div>

          {status?.message ? (
            <p className="px-2 pb-2 text-2xs leading-relaxed text-slate-400">{status.message}</p>
          ) : null}

          {status?.organization ? (
            <div className="flex items-center gap-2 px-2 py-1.5 text-2xs text-slate-300">
              <Building2 className="h-3.5 w-3.5 text-slate-500" aria-hidden="true" />
              {status.organization}
            </div>
          ) : null}

          {status?.scopes?.length ? (
            <p className="px-2 py-1.5 text-2xs text-slate-400">
              Scopes: {status.scopes.join(', ')}
            </p>
          ) : null}

          <div className="my-1 h-px bg-ink-700/70" />

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              void navigator.clipboard?.writeText(status?.mainRepository ?? '');
              setOpen(false);
            }}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-slate-200 hover:bg-ink-800"
          >
            <Copy className="h-3.5 w-3.5 text-slate-500" aria-hidden="true" />
            Copy main repository slug
          </button>

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              onRefreshAuth();
              setOpen(false);
            }}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-slate-200 hover:bg-ink-800"
          >
            <RefreshCw className="h-3.5 w-3.5 text-slate-500" aria-hidden="true" />
            Re-check GitHub CLI status
          </button>

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              onSwitchAccount();
              setOpen(false);
            }}
            className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs text-slate-200 hover:bg-ink-800"
          >
            <ShieldAlert className="h-3.5 w-3.5 text-slate-500" aria-hidden="true" />
            Switch account
          </button>

          <p className="px-2 pb-1 pt-1 text-2xs leading-relaxed text-slate-500">
            Credentials stay in the GitHub CLI credential store. They are never read into the
            frontend.
          </p>
        </div>
      ) : null}
    </div>
  );
}

export function AppShell({
  route,
  onNavigate,
  auth,
  onRefreshAuth,
  onSwitchAccount,
  targetBranch,
  selectedSubsystemName,
  notice,
  onDismissNotice,
  children,
}: AppShellProps) {
  const status = auth.data;
  const connected = Boolean(status?.authenticated);
  const fixtureMode = status?.dataSource === 'fixture' || (!status && auth.loading);
  const [navExpanded, setNavExpanded] = useState(false);

  return (
    <div className="flex h-full flex-col bg-ink-950 text-slate-200">
      <header
        data-tauri-drag-region
        className="flex h-16 shrink-0 items-center justify-between gap-3 border-b border-ink-700/70 bg-ink-900/95 px-3"
      >
        <div className="flex min-w-0 items-center gap-3" data-tauri-drag-region>
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-brand-600 shadow-[0_6px_18px_-6px_rgba(37,99,235,0.9)]">
            <GridMark />
          </span>
          <div className="min-w-0 leading-tight" data-tauri-drag-region>
            <h1 className="truncate text-sm font-semibold text-white">
              Subsystem Integration Manager
            </h1>
            <p className="truncate text-2xs text-slate-400">Built with Tauri</p>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2 sm:gap-3">
          {fixtureMode ? (
            <span
              className="chip hidden bg-amber-950/70 text-amber-200 ring-1 ring-inset ring-amber-800/70 md:inline-flex"
              title="Registry, commit, workflow and PR data are clearly-marked fictitious samples. Status values are computed by the same branch-scoped algorithm used in live mode."
            >
              Fixture data
            </span>
          ) : null}

          <span
            className="chip hidden max-w-[220px] truncate bg-ink-800 text-slate-300 ring-1 ring-inset ring-ink-600 lg:inline-flex"
            title={`Integration status is shown per (subsystem, source SHA, target branch). Current target: ${targetBranch}`}
          >
            target: {targetBranch}
            {selectedSubsystemName ? ` · ${selectedSubsystemName}` : ''}
          </span>

          <span
            className="chip bg-ink-800/80 text-slate-200 ring-1 ring-inset ring-ink-600/70"
            title={status?.message ?? 'Checking the GitHub CLI…'}
          >
            <span
              aria-hidden="true"
              className={cx(
                'h-2 w-2 rounded-full',
                connected ? 'bg-emerald-400 animate-pulse-dot' : 'bg-amber-400',
              )}
            />
            <span className="hidden sm:inline">
              {connected ? 'GitHub Connected' : 'GitHub not connected'}
            </span>
            <span className="sm:hidden">{connected ? 'Connected' : 'Offline'}</span>
          </span>

          <span className="hidden h-8 w-px bg-ink-700 md:block" aria-hidden="true" />

          <AccountMenu
            auth={auth}
            onRefreshAuth={onRefreshAuth}
            onSwitchAccount={onSwitchAccount}
          />
          <span className="h-8 w-px bg-ink-700" aria-hidden="true" />
          <WindowsControls />
        </div>
      </header>

      {!connected && status && !status.authenticated ? (
        <div className="shrink-0 border-b border-amber-900/50 bg-amber-950/30 px-4 py-2">
          <InlineAlert
            tone="warning"
            title="GitHub CLI is not ready — live GitHub operations are disabled"
            actions={
              <IconButton
                icon={RefreshCw}
                label="Re-check GitHub CLI status"
                onClick={onRefreshAuth}
              />
            }
          >
            <span>{status.message}</span>
            {status.remedy ? (
              <span className="mt-1 block font-mono text-2xs text-amber-200/90">
                {status.remedy}
              </span>
            ) : null}
          </InlineAlert>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1">
        <nav
          aria-label="Primary"
          className={cx(
            'flex shrink-0 flex-col border-r border-ink-700/70 bg-ink-900/80 transition-[width] duration-150',
            navExpanded ? 'w-[236px]' : 'w-[68px] xl:w-[236px]',
          )}
        >
          <div className="flex items-center justify-between px-2 pt-2 xl:hidden">
            <IconButton
              icon={navExpanded ? X : Square}
              label={navExpanded ? 'Collapse navigation' : 'Expand navigation'}
              onClick={() => setNavExpanded((value) => !value)}
            />
          </div>

          <ul className="scroll-area flex-1 space-y-1 p-2">
            {NAV_ITEMS.map((item) => {
              const Icon = item.icon;
              const active = item.id === route;
              const showLabel = navExpanded;
              return (
                <li key={item.id}>
                  <button
                    type="button"
                    onClick={() => {
                      onNavigate(item.id);
                      setNavExpanded(false);
                    }}
                    aria-current={active ? 'page' : undefined}
                    title={item.description}
                    className={cx(
                      'nav-item',
                      active && 'nav-item-active',
                      !showLabel && 'justify-center xl:justify-start',
                    )}
                  >
                    {active ? (
                      <span
                        aria-hidden="true"
                        className="absolute left-0 top-1.5 bottom-1.5 w-[3px] rounded-r-full bg-brand-500"
                      />
                    ) : null}
                    <Icon
                      className={cx(
                        'h-4 w-4 shrink-0',
                        active ? 'text-brand-400' : 'text-slate-400',
                      )}
                      aria-hidden="true"
                    />
                    <span className={cx('truncate', showLabel ? 'block' : 'hidden xl:block')}>
                      {item.label}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>

          <div className="space-y-2 border-t border-ink-700/70 p-2">
            <div
              className={cx(
                'rounded-lg border border-ink-700/70 bg-ink-870 p-2.5',
                navExpanded ? 'block' : 'hidden xl:block',
              )}
            >
              <div className="flex items-center gap-2">
                <span className="flex h-8 w-8 items-center justify-center rounded-full border border-ink-600 bg-ink-800 text-slate-200">
                  <GitHubMark className="h-4 w-4" />
                </span>
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs font-semibold text-slate-100">
                    {status?.accountLogin ?? 'Not signed in'}
                  </p>
                  <p className="truncate text-2xs text-slate-400">
                    {status?.organization ?? 'organization'}
                  </p>
                </div>
                <span
                  className={cx(
                    'chip',
                    connected
                      ? 'bg-emerald-950/70 text-emerald-300 ring-1 ring-inset ring-emerald-800/70'
                      : 'bg-amber-950/70 text-amber-200 ring-1 ring-inset ring-amber-800/70',
                  )}
                >
                  {connected ? (
                    <>
                      <CircleCheck className="h-3 w-3" aria-hidden="true" /> Connected
                    </>
                  ) : (
                    'Not connected'
                  )}
                </span>
              </div>
              <button
                type="button"
                onClick={onSwitchAccount}
                className="mt-2 flex w-full items-center justify-center gap-2 rounded-md border border-ink-600 bg-ink-800/60 px-2 py-1.5 text-xs text-slate-200 hover:bg-ink-750"
                title="The GitHub CLI owns credentials; switching accounts happens in your terminal."
              >
                <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
                Switch Account
              </button>
            </div>
            {!navExpanded ? (
              <div className="flex justify-center xl:hidden">
                <button
                  type="button"
                  onClick={onSwitchAccount}
                  title={`GitHub: ${status?.accountLogin ?? 'not signed in'}`}
                  aria-label={`GitHub account: ${status?.accountLogin ?? 'not signed in'}`}
                  className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-ink-600 bg-ink-800 text-slate-200 hover:bg-ink-750"
                >
                  <GitHubMark className="h-4 w-4" />
                </button>
              </div>
            ) : null}
          </div>
        </nav>

        <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
        </main>
      </div>

      {notice ? (
        <div className="pointer-events-none fixed bottom-4 left-1/2 z-50 w-full max-w-2xl -translate-x-1/2 px-4">
          <div className="pointer-events-auto rounded-xl border border-ink-700 bg-ink-870 p-3 shadow-popover animate-fade-in">
            <InlineAlert
              tone={notice.tone}
              title={notice.title}
              actions={<IconButton icon={X} label="Dismiss message" onClick={onDismissNotice} />}
            >
              {notice.message}
            </InlineAlert>
          </div>
        </div>
      ) : null}

      <WindowResizeHandles />
    </div>
  );
}

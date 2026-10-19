import { useEffect, useRef, useState, type ReactNode } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Building2,
  ChevronDown,
  CircleAlert,
  Copy,
  GitBranch,
  Maximize2,
  Minus,
  RefreshCw,
  ShieldAlert,
  Square,
  TriangleAlert,
  X,
} from 'lucide-react';

import type { AuthStatus } from '../api/types';
import { isTauriRuntime } from '../api/invoke';
import { NAV_ITEMS, type Route } from '../app/navigation';
import type { AsyncResult, Notice } from '../app/context';
import { cx } from './ui/cx';
import { Button, GitHubMark, IconButton } from './ui/primitives';

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

// Flat 12px reduction of the brand hub: gradients, glow and the hub ring read as mud at this size, so solid nodes only.
function BranchMark() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" className="h-3 w-3 text-accent-fg">
      <path
        d="M5.6 8H11M5.6 8C7.8 8 8.4 3.4 10.6 3.3M5.6 8C7.8 8 8.4 12.6 10.6 12.7"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
      <circle cx="3.6" cy="8" r="2.4" fill="currentColor" />
      <circle cx="12.2" cy="3.2" r="1.7" fill="currentColor" />
      <circle cx="12.6" cy="8" r="1.7" fill="currentColor" />
      <circle cx="12.2" cy="12.8" r="1.7" fill="currentColor" />
    </svg>
  );
}

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
    <div className="flex items-center gap-0.5" data-tauri-no-drag>
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
        className="flex items-center gap-2 rounded-md border border-transparent px-1.5 py-1 text-left transition-colors hover:border-line-strong hover:bg-hover"
      >
        <span className="flex h-6 w-6 items-center justify-center rounded-full border border-line bg-raised text-fg">
          {login ? (
            <GitHubMark className="h-3.5 w-3.5" />
          ) : (
            <CircleAlert className="h-3.5 w-3.5 text-status-unknown" />
          )}
        </span>
        <span className="hidden min-w-0 leading-tight sm:block">
          <span className="block truncate text-xs font-semibold text-fg">
            {login ?? 'Not signed in'}
          </span>
          <span className="block truncate text-[11px] text-muted">
            {status?.organization ?? 'GitHub organization'}
          </span>
        </span>
        <ChevronDown className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
      </button>

      {open ? (
        <div
          role="menu"
          className="absolute right-0 z-40 mt-1.5 w-80 rounded-md border border-line bg-overlay p-2 shadow-dialog animate-fade-in"
        >
          <div className="px-2 py-1.5">
            <p className="text-[11px] font-semibold tracking-wide text-muted uppercase">
              GitHub CLI identity
            </p>
            <p className="mt-0.5 truncate text-xs font-semibold text-fg">
              {login ?? 'No authenticated account'}
            </p>
            <p className="mono truncate text-muted">
              {status?.hostname ?? 'github.com'}
              {status?.ghVersion ? ` · gh ${status.ghVersion}` : ''}
            </p>
          </div>

          {status?.message ? (
            <p className="px-2 pb-2 text-[11px] leading-relaxed text-muted">{status.message}</p>
          ) : null}

          {status?.organization ? (
            <div className="flex items-center gap-1.5 px-2 py-1 text-[11px] text-fg">
              <Building2 className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
              <span className="truncate">{status.organization}</span>
            </div>
          ) : null}

          {status?.scopes?.length ? (
            <p className="truncate px-2 py-1 text-[11px] text-muted">
              Scopes: {status.scopes.join(', ')}
            </p>
          ) : null}

          <div className="my-1 h-px bg-line" />

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              void navigator.clipboard?.writeText(status?.mainRepository ?? '');
              setOpen(false);
            }}
            className="flex w-full items-center gap-1.5 rounded-[3px] px-2 py-1.5 text-left text-xs text-fg hover:bg-hover"
          >
            <Copy className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
            Copy main repository slug
          </button>

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              onRefreshAuth();
              setOpen(false);
            }}
            className="flex w-full items-center gap-1.5 rounded-[3px] px-2 py-1.5 text-left text-xs text-fg hover:bg-hover"
          >
            <RefreshCw className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
            Re-check GitHub CLI status
          </button>

          <button
            type="button"
            role="menuitem"
            onClick={() => {
              onSwitchAccount();
              setOpen(false);
            }}
            className="flex w-full items-center gap-1.5 rounded-[3px] px-2 py-1.5 text-left text-xs text-fg hover:bg-hover"
          >
            <ShieldAlert className="h-3.5 w-3.5 text-muted" aria-hidden="true" />
            Switch account
          </button>
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
  const tablistRef = useRef<HTMLDivElement>(null);

  return (
    <div className="flex h-full flex-col bg-ground text-fg">
      <header
        data-tauri-drag-region
        className="flex h-7 shrink-0 items-center justify-between gap-2 border-b border-line bg-surface pr-1 pl-2.5"
      >
        <div className="flex min-w-0 items-center gap-2" data-tauri-drag-region title="VOS Origins">
          <span className="flex h-4 w-4 shrink-0 items-center justify-center rounded-[3px] bg-accent">
            <BranchMark />
          </span>
          <h1 className="truncate text-[11px] font-semibold text-fg">VOS Origins</h1>
        </div>
        <WindowsControls />
      </header>

      <div className="flex h-[50px] shrink-0 items-center justify-between gap-2 border-b border-line bg-surface px-2.5">
        <div className="flex min-w-0 items-center gap-2">
          <div
            className="flex h-[25px] min-w-0 items-center gap-1.5 rounded-md border border-line bg-raised px-2 text-[11px]"
            title={`Target branch: ${targetBranch}${selectedSubsystemName ? ` · ${selectedSubsystemName}` : ''}`}
          >
            <GitBranch className="h-3.5 w-3.5 shrink-0 text-accent" aria-hidden="true" />
            <span className="mono truncate">{targetBranch}</span>
            {selectedSubsystemName ? (
              <span className="truncate text-muted">· {selectedSubsystemName}</span>
            ) : null}
          </div>

          {fixtureMode ? (
            <span
              className="hidden h-[21px] items-center gap-1.5 rounded-[3px] border border-line bg-surface px-2 text-[11px] text-muted md:inline-flex"
              title="Sample data"
            >
              Fixture data
            </span>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <span
            className="inline-flex h-[21px] items-center gap-1.5 rounded-[3px] border border-line bg-raised px-2 text-[11px] text-fg"
            title={status?.message ?? 'Checking the GitHub CLI…'}
          >
            <span
              aria-hidden="true"
              className={cx(
                'h-2 w-2 rounded-full',
                connected ? 'bg-status-applied animate-pulse-dot' : 'bg-status-unknown',
              )}
            />
            <span className="hidden sm:inline">
              {connected ? 'GitHub Connected' : 'GitHub not connected'}
            </span>
            <span className="sm:hidden">{connected ? 'Connected' : 'Offline'}</span>
          </span>

          <span className="hidden h-[25px] w-px bg-line md:block" aria-hidden="true" />

          <AccountMenu
            auth={auth}
            onRefreshAuth={onRefreshAuth}
            onSwitchAccount={onSwitchAccount}
          />
        </div>
      </div>

      {!connected && status && !status.authenticated ? (
        <div
          role="alert"
          className="flex shrink-0 items-center gap-2 border-b border-status-unknown/40 bg-status-unknown-soft px-2.5 py-1.5"
        >
          <TriangleAlert className="h-3.5 w-3.5 shrink-0 text-status-unknown" aria-hidden="true" />
          <p
            className="min-w-0 flex-1 truncate text-[11px] font-medium text-status-unknown-ink"
            title={`${status.message}${status.remedy ? ` — ${status.remedy}` : ''}`}
          >
            GitHub CLI is not ready — live operations disabled
          </p>
          <Button size="sm" variant="secondary" icon={RefreshCw} onClick={onRefreshAuth}>
            Re-check
          </Button>
        </div>
      ) : null}

      <nav aria-label="Primary" className="h-[29px] shrink-0 border-b border-line bg-surface">
        <div
          ref={tablistRef}
          role="tablist"
          aria-label="Primary"
          onKeyDown={(event) => {
            if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
            const tabs = Array.from(
              tablistRef.current?.querySelectorAll<HTMLElement>('[role="tab"]') ?? [],
            );
            const index = tabs.indexOf(document.activeElement as HTMLElement);
            if (index === -1) return;
            event.preventDefault();
            const next =
              event.key === 'ArrowRight'
                ? (index + 1) % tabs.length
                : (index - 1 + tabs.length) % tabs.length;
            tabs[next]?.focus();
          }}
          className="flex h-full overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        >
          {NAV_ITEMS.map((item) => {
            const active = item.id === route;
            return (
              <button
                key={item.id}
                type="button"
                role="tab"
                aria-selected={active}
                title={item.description}
                onClick={() => onNavigate(item.id)}
                className={cx(
                  'relative flex h-full min-w-0 flex-1 items-center justify-center border-r border-line px-2.5 text-xs whitespace-nowrap transition-colors last:border-r-0 focus-visible:-outline-offset-4',
                  active
                    ? 'font-semibold text-fg'
                    : 'font-normal text-muted hover:bg-hover hover:text-fg',
                )}
              >
                <span className="truncate">{item.label}</span>
                {active ? (
                  <span
                    aria-hidden="true"
                    className="absolute inset-x-0 bottom-0 h-[3px] bg-accent"
                  />
                ) : null}
              </button>
            );
          })}
        </div>
      </nav>

      <main className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="min-h-0 flex-1 overflow-hidden">{children}</div>
      </main>

      {notice ? (
        <div className="pointer-events-none fixed right-4 bottom-4 z-50 flex justify-end">
          <div className="pointer-events-auto flex max-w-[20rem] items-center gap-2 rounded-md border border-line bg-surface px-2.5 py-2 text-xs text-fg animate-fade-in">
            <p className="min-w-0 flex-1 truncate" title={notice.title}>
              <span className="font-semibold">{notice.title}</span>
              {notice.message ? <span className="text-muted"> — {notice.message}</span> : null}
            </p>
            <IconButton icon={X} label="Dismiss message" onClick={onDismissNotice} />
          </div>
        </div>
      ) : null}

      <WindowResizeHandles />
    </div>
  );
}

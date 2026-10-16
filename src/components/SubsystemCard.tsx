import {
  AlertTriangle,
  Boxes,
  FileStack,
  Package,
  ShoppingCart,
  Users,
  Wallet,
} from 'lucide-react';
import type { LucideIcon } from 'lucide-react';

import type { ConnectivityState, SubsystemSummary } from '../api/types';
import { cx } from './ui/cx';

/** Subsystem card (spec §3.2). Colour is decoration; the connectivity dot is data and always carries a tooltip. */

const ICONS: Record<string, LucideIcon> = {
  'hr-management': Users,
  'payroll-management': Wallet,
  'inventory-management': Package,
  procurement: ShoppingCart,
  'document-management': FileStack,
};

const TILE_COLORS: Record<string, string> = {
  'hr-management': 'bg-brand-600/90 text-white',
  'payroll-management': 'bg-emerald-600/90 text-white',
  'inventory-management': 'bg-sky-600/90 text-white',
  procurement: 'bg-orange-600/90 text-white',
  'document-management': 'bg-violet-600/90 text-white',
};

const CONNECTIVITY: Record<ConnectivityState, { dot: string; label: string }> = {
  connected: { dot: 'bg-emerald-400', label: 'Connected' },
  degraded: { dot: 'bg-amber-400', label: 'Degraded' },
  unknown: { dot: 'bg-slate-500', label: 'Unknown' },
};

export interface SubsystemCardProps {
  subsystem: SubsystemSummary;
  selected: boolean;
  onSelect: () => void;
  commitCount?: number;
}

export function SubsystemCard({ subsystem, selected, onSelect, commitCount }: SubsystemCardProps) {
  const Icon = ICONS[subsystem.id] ?? Boxes;
  const tile = TILE_COLORS[subsystem.id] ?? 'bg-ink-600 text-white';
  const connectivity = CONNECTIVITY[subsystem.connectivity];

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      aria-label={`${subsystem.name} — ${subsystem.repository} (${connectivity.label})`}
      className={cx(
        'group flex w-full items-center gap-3 rounded-xl border p-3 text-left transition-colors',
        selected
          ? 'border-brand-500 bg-ink-800 ring-1 ring-brand-500/40'
          : 'border-ink-700/70 bg-ink-870 hover:border-ink-600 hover:bg-ink-800/70',
      )}
    >
      <span className={cx('flex h-10 w-10 shrink-0 items-center justify-center rounded-lg', tile)}>
        <Icon className="h-5 w-5" aria-hidden="true" />
      </span>

      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-sm font-semibold text-slate-100">{subsystem.name}</span>
          {!subsystem.enabled ? (
            <span className="chip bg-ink-800 text-slate-400 ring-1 ring-inset ring-ink-600">
              disabled
            </span>
          ) : null}
        </span>
        <span className="mono mt-0.5 block truncate text-slate-400">{subsystem.repository}</span>
        <span className="mt-1 flex items-center gap-2 text-2xs text-slate-500">
          <span className="mono">{subsystem.sourceBranch}</span>
          {typeof commitCount === 'number' ? <span>{commitCount} commits indexed</span> : null}
        </span>
      </span>

      <span
        className={cx('mt-1 h-2.5 w-2.5 shrink-0 rounded-full', connectivity.dot)}
        title={`Repository availability: ${connectivity.label}${subsystem.connectivityMessage ? ` — ${subsystem.connectivityMessage}` : ''}`}
        aria-label={`Repository availability: ${connectivity.label}`}
        role="img"
      />
    </button>
  );
}

/** Compact horizontal variant used when the window is too narrow for the card list. */
export function SubsystemChip({
  subsystem,
  selected,
  onSelect,
}: {
  subsystem: SubsystemSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  const connectivity = CONNECTIVITY[subsystem.connectivity];
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      title={`${subsystem.name} — ${subsystem.repository}`}
      className={cx(
        'flex shrink-0 items-center gap-2 rounded-full border px-3 py-1.5 text-xs transition-colors',
        selected
          ? 'border-brand-500 bg-ink-800 text-white'
          : 'border-ink-700 bg-ink-870 text-slate-300 hover:bg-ink-800',
      )}
    >
      <span className={cx('h-2 w-2 rounded-full', connectivity.dot)} aria-hidden="true" />
      <span className="max-w-[160px] truncate font-medium">{subsystem.name}</span>
    </button>
  );
}

export function SubsystemWarning({ message }: { message: string }) {
  return (
    <p className="flex items-start gap-2 rounded-lg border border-amber-900/60 bg-amber-950/30 px-2.5 py-2 text-2xs text-amber-200">
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      {message}
    </p>
  );
}

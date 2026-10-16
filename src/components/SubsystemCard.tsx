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

/** Subsystem row (spec §3.2). Colour is data only: the connectivity dot always carries a tooltip. */

const ICONS: Record<string, LucideIcon> = {
  'hr-management': Users,
  'payroll-management': Wallet,
  'inventory-management': Package,
  procurement: ShoppingCart,
  'document-management': FileStack,
};

const CONNECTIVITY: Record<ConnectivityState, { dot: string; label: string }> = {
  connected: { dot: 'bg-status-applied', label: 'Connected' },
  degraded: { dot: 'bg-status-unknown', label: 'Degraded' },
  unknown: { dot: 'bg-status-superseded', label: 'Unknown' },
};

export interface SubsystemCardProps {
  subsystem: SubsystemSummary;
  selected: boolean;
  onSelect: () => void;
  commitCount?: number;
}

export function SubsystemCard({ subsystem, selected, onSelect, commitCount }: SubsystemCardProps) {
  const Icon = ICONS[subsystem.id] ?? Boxes;
  const connectivity = CONNECTIVITY[subsystem.connectivity];

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      aria-label={`${subsystem.name} — ${subsystem.repository} (${connectivity.label})`}
      className={cx(
        'group flex h-[50px] w-full items-center gap-2.5 rounded-md border px-2.5 text-left transition-colors focus-visible:control-focus',
        selected ? 'border-accent bg-selection' : 'border-line bg-surface hover:bg-hover',
      )}
    >
      <span className="flex h-4 w-4 shrink-0 items-center justify-center text-muted">
        <Icon className="h-4 w-4" aria-hidden="true" />
      </span>

      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5">
          <span className="truncate text-xs font-semibold text-fg">{subsystem.name}</span>
          {!subsystem.enabled ? <span className="chip text-disabled">disabled</span> : null}
        </span>
        <span className="mono mt-px block truncate text-disabled" title={subsystem.repository}>
          {subsystem.repository}
          {typeof commitCount === 'number' ? ` · ${commitCount}` : null}
        </span>
      </span>

      <span className="flex shrink-0 flex-col items-end gap-0.5">
        <span
          className={cx('h-2.5 w-2.5 shrink-0 rounded-full', connectivity.dot)}
          title={`Repository availability: ${connectivity.label}${subsystem.connectivityMessage ? ` — ${subsystem.connectivityMessage}` : ''}`}
          aria-label={`Repository availability: ${connectivity.label}`}
          role="img"
        />
        {typeof commitCount === 'number' ? (
          <span className="mono text-right text-[11px] text-disabled tabular-nums">
            {commitCount}
          </span>
        ) : null}
      </span>
    </button>
  );
}

/** Single segmented control for the subsystem filter row: one segment per subsystem. */
export function SubsystemSegmentedControl({
  subsystems,
  selectedId,
  onSelect,
}: {
  subsystems: SubsystemSummary[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  return (
    <div
      role="group"
      aria-label="Subsystem filter"
      className="inline-flex h-[25px] shrink-0 items-center divide-x divide-line overflow-hidden rounded-[3px] border border-line bg-surface"
    >
      {subsystems.map((subsystem) => {
        const connectivity = CONNECTIVITY[subsystem.connectivity];
        const selected = subsystem.id === selectedId;
        return (
          <button
            key={subsystem.id}
            type="button"
            onClick={() => onSelect(subsystem.id)}
            aria-pressed={selected}
            title={`${subsystem.name} — ${subsystem.repository}`}
            className={cx(
              'flex h-full items-center gap-2 px-3 text-[11px] transition-colors focus-visible:control-focus',
              selected ? 'bg-raised font-medium text-fg' : 'text-muted hover:bg-hover',
            )}
          >
            <span
              className={cx('h-1.5 w-1.5 shrink-0 rounded-full', connectivity.dot)}
              aria-hidden="true"
            />
            <span className="max-w-[160px] truncate">{subsystem.name}</span>
          </button>
        );
      })}
    </div>
  );
}

export function SubsystemWarning({ message }: { message: string }) {
  return (
    <p className="flex items-start gap-2 rounded-md border border-status-unknown/40 bg-status-unknown-soft px-2.5 py-2 text-[11px] text-status-unknown-ink">
      <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <span className="min-w-0 truncate" title={message}>
        {message}
      </span>
    </p>
  );
}

import { useEffect, useRef, useState } from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { ChevronDown } from 'lucide-react';

import type { SubsystemSummary } from '../api/types';
import { CONNECTIVITY_DOT } from './subsystemConnectivity';
import { SubsystemFilterList } from './SubsystemFilterList';
import { cx } from './ui/cx';

export function SubsystemSelector({
  subsystems,
  selectedId,
  onSelect,
  counts,
}: {
  subsystems: SubsystemSummary[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  counts?: Record<string, number>;
}) {
  const [open, setOpen] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const selected = subsystems.find((subsystem) => subsystem.id === selectedId) ?? null;
  const connectivity = selected ? CONNECTIVITY_DOT[selected.connectivity] : null;

  useEffect(() => {
    if (!open) return;
    const timer = window.setTimeout(() => {
      panelRef.current?.querySelector<HTMLInputElement>('input')?.focus();
    }, 30);
    return () => window.clearTimeout(timer);
  }, [open]);

  return (
    <PopoverPrimitive.Root open={open} onOpenChange={setOpen} modal={false}>
      <PopoverPrimitive.Trigger asChild>
        <button
          type="button"
          aria-label="Select subsystem"
          aria-haspopup="listbox"
          aria-expanded={open}
          title={selected ? `${selected.name} — ${selected.repository}` : 'Select a subsystem'}
          className={cx(
            'inline-flex h-[25px] max-w-full items-center gap-1.5 rounded-md border border-line-strong bg-surface px-2 text-xs text-fg transition-colors',
            'hover:bg-hover focus-visible:border-accent focus-visible:outline-none',
          )}
        >
          {connectivity ? (
            <span
              className={cx('h-1.5 w-1.5 shrink-0 rounded-full', connectivity.dot)}
              title={connectivity.label}
              aria-hidden="true"
            />
          ) : null}
          <span className="min-w-0 max-w-[170px] truncate text-left">
            {selected ? selected.name : <span className="text-muted">Select subsystem…</span>}
          </span>
          {selected ? (
            <span
              className="mono hidden max-w-[150px] shrink truncate text-[11px] text-muted sm:inline"
              title={selected.repository}
            >
              {selected.repository}
            </span>
          ) : null}
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden="true" />
        </button>
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          ref={panelRef}
          side="bottom"
          align="end"
          sideOffset={4}
          collisionPadding={8}
          avoidCollisions
          sticky="partial"
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          className="z-[70] flex max-h-[min(420px,calc(100vh-96px))] w-[320px] max-w-[calc(100vw-16px)] flex-col overflow-hidden rounded-md border border-line bg-overlay animate-fade-in"
        >
          <SubsystemFilterList
            subsystems={subsystems}
            selectedId={selectedId}
            onSelect={(id) => {
              onSelect(id);
              setOpen(false);
            }}
            counts={counts}
            ariaLabel="Filter subsystems"
            className="min-h-0 flex-1"
          />
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
}

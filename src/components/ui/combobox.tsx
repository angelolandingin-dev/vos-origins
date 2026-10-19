import { useEffect, useId, useMemo, useRef, useState } from 'react';
import * as PopoverPrimitive from '@radix-ui/react-popover';
import { Check, ChevronDown, Search } from 'lucide-react';

import { cx } from './cx';

export interface ComboboxOption {
  value: string;
  label: string;
  meta?: string;
  title?: string;
}

function matches(option: ComboboxOption, needle: string): boolean {
  if (!needle) return true;
  const hay = `${option.label} ${option.value}`.toLowerCase();
  return needle
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((part) => hay.includes(part));
}

/**
 * Searchable, filterable, scrollable picker built on a non-modal Radix Popover
 * plus a hand-rolled listbox (ArrowUp/Down/Home/End/Enter/Escape complete).
 *
 * WebView2/Tauri safety: `modal={false}` so the body never gets a stuck
 * `pointer-events: none`; `avoidCollisions` + `sticky="partial"` keeps the
 * 300+ file list usable near the window edge and scrollable inside dialogs.
 */
export function Combobox({
  value,
  options,
  onSelect,
  ariaLabel,
  placeholder = 'Select…',
  searchPlaceholder = 'Filter…',
  emptyMessage = 'No matches.',
  className,
  triggerClassName,
  disabled = false,
  listLabel,
}: {
  value: string | null;
  options: ComboboxOption[];
  onSelect: (value: string) => void;
  ariaLabel: string;
  placeholder?: string;
  searchPlaceholder?: string;
  emptyMessage?: string;
  className?: string;
  triggerClassName?: string;
  disabled?: boolean;
  listLabel?: string;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const listId = useId();
  const searchRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(
    () => options.filter((option) => matches(option, query)),
    [options, query],
  );

  const handleOpenChange = (next: boolean) => {
    if (next) {
      setQuery('');
      setActiveIndex(0);
    }
    setOpen(next);
  };

  useEffect(() => {
    if (!open) return;
    const timer = window.setTimeout(() => searchRef.current?.focus(), 30);
    return () => window.clearTimeout(timer);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const active = listRef.current?.querySelector<HTMLElement>(`[data-index="${activeIndex}"]`);
    active?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex, open]);

  const selected = options.find((option) => option.value === value) ?? null;

  const choose = (next: string) => {
    onSelect(next);
    setOpen(false);
  };

  const onSearchKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActiveIndex((index) => Math.min(filtered.length - 1, index + 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActiveIndex((index) => Math.max(0, index - 1));
    } else if (event.key === 'Home') {
      event.preventDefault();
      setActiveIndex(0);
    } else if (event.key === 'End') {
      event.preventDefault();
      setActiveIndex(Math.max(0, filtered.length - 1));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const next = filtered[activeIndex];
      if (next) choose(next.value);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      setOpen(false);
    }
  };

  return (
    <PopoverPrimitive.Root open={open} onOpenChange={handleOpenChange} modal={false}>
      <PopoverPrimitive.Trigger asChild disabled={disabled}>
        <button
          type="button"
          aria-label={ariaLabel}
          aria-haspopup="listbox"
          aria-expanded={open}
          title={selected ? (selected.title ?? selected.value) : placeholder}
          className={cx(
            'inline-flex h-[25px] max-w-full items-center justify-between gap-1.5 rounded-md border border-line-strong bg-surface px-2 text-xs text-fg transition-colors',
            'hover:bg-hover focus-visible:border-accent focus-visible:outline-none',
            'disabled:cursor-not-allowed disabled:opacity-60',
            triggerClassName,
            className,
          )}
        >
          <span className="mono min-w-0 flex-1 truncate text-left tabular-nums">
            {selected ? selected.label : <span className="text-muted">{placeholder}</span>}
          </span>
          {selected?.meta ? (
            <span className="mono shrink-0 text-[11px] text-muted tabular-nums">
              {selected.meta}
            </span>
          ) : null}
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden="true" />
        </button>
      </PopoverPrimitive.Trigger>
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          side="bottom"
          align="start"
          sideOffset={4}
          collisionPadding={8}
          avoidCollisions
          sticky="partial"
          onOpenAutoFocus={(event) => event.preventDefault()}
          onCloseAutoFocus={(event) => event.preventDefault()}
          className="z-[70] flex w-[var(--radix-popover-trigger-width)] min-w-[320px] max-w-[min(560px,calc(100vw-16px))] flex-col overflow-hidden rounded-md border border-line bg-overlay animate-fade-in"
        >
          <div className="relative shrink-0 border-b border-line">
            <Search
              className="pointer-events-none absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
              aria-hidden="true"
            />
            <input
              ref={searchRef}
              type="search"
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setActiveIndex(0);
              }}
              onKeyDown={onSearchKeyDown}
              placeholder={searchPlaceholder}
              aria-label={`${ariaLabel} filter`}
              aria-controls={listId}
              aria-activedescendant={
                filtered[activeIndex] ? `cb-${listId}-${activeIndex}` : undefined
              }
              spellCheck={false}
              autoComplete="off"
              className="h-[29px] w-full bg-transparent pr-2 pl-8 text-xs text-fg outline-none placeholder:text-disabled"
            />
          </div>
          <div
            ref={listRef}
            id={listId}
            role="listbox"
            aria-label={listLabel ?? ariaLabel}
            className="max-h-[320px] min-h-0 overflow-y-auto overscroll-contain p-1"
            onWheel={(event) => event.stopPropagation()}
          >
            {filtered.length === 0 ? (
              <p className="px-2.5 py-3 text-[11px] text-muted">{emptyMessage}</p>
            ) : (
              filtered.map((option, index) => {
                const active = index === activeIndex;
                const checked = option.value === value;
                return (
                  <div
                    key={option.value}
                    id={`cb-${listId}-${index}`}
                    data-index={index}
                    role="option"
                    aria-selected={checked}
                    tabIndex={-1}
                    title={option.title ?? option.value}
                    onMouseMove={() => setActiveIndex(index)}
                    onClick={() => choose(option.value)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault();
                        choose(option.value);
                      }
                    }}
                    className={cx(
                      'flex min-h-[29px] cursor-pointer items-center gap-2 rounded-[3px] px-2.5 py-1 text-left outline-none select-none',
                      active ? 'bg-selection' : 'hover:bg-hover',
                    )}
                  >
                    <span className="flex w-3.5 shrink-0 justify-center" aria-hidden="true">
                      {checked ? <Check className="h-3.5 w-3.5 text-accent" /> : null}
                    </span>
                    <span
                      className="mono min-w-0 flex-1 truncate text-xs text-fg tabular-nums"
                      title={option.label}
                    >
                      {option.label}
                    </span>
                    {option.meta ? (
                      <span className="mono shrink-0 text-[11px] text-muted tabular-nums">
                        {option.meta}
                      </span>
                    ) : null}
                  </div>
                );
              })
            )}
          </div>
          <p className="mono shrink-0 border-t border-line px-2.5 py-1 text-right text-[11px] text-disabled tabular-nums">
            {filtered.length} of {options.length}
          </p>
        </PopoverPrimitive.Content>
      </PopoverPrimitive.Portal>
    </PopoverPrimitive.Root>
  );
}

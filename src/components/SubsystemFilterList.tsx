import { useMemo, useRef, useState } from 'react';
import { Search } from 'lucide-react';

import type { SubsystemSummary } from '../api/types';
import { CONNECTIVITY_DOT } from './subsystemConnectivity';
import { cx } from './ui/cx';

export function SubsystemFilterList({
  subsystems,
  selectedId,
  onSelect,
  counts,
  ariaLabel = 'Subsystem filter',
  className,
}: {
  subsystems: SubsystemSummary[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  counts?: Record<string, number>;
  ariaLabel?: string;
  className?: string;
}) {
  const [query, setQuery] = useState('');
  const [activeId, setActiveId] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) return subsystems;
    return subsystems.filter(
      (subsystem) =>
        subsystem.name.toLowerCase().includes(needle) ||
        subsystem.repository.toLowerCase().includes(needle) ||
        subsystem.id.toLowerCase().includes(needle),
    );
  }, [subsystems, query]);

  const activeIndex = useMemo(() => {
    const index = filtered.findIndex((entry) => entry.id === (activeId ?? selectedId));
    return index >= 0 ? index : 0;
  }, [filtered, activeId, selectedId]);

  const move = (delta: -1 | 1) => {
    if (filtered.length === 0) return;
    const next = (activeIndex + delta + filtered.length) % filtered.length;
    const entry = filtered[next];
    if (entry) {
      setActiveId(entry.id);
      document.getElementById(`subsystem-filter-row-${entry.id}`)?.focus();
    }
  };

  const chooseActive = () => {
    const entry = filtered[activeIndex] ?? filtered[0];
    if (entry) onSelect(entry.id);
  };

  return (
    <div className={cx('flex min-h-0 flex-col', className)}>
      <div className="relative shrink-0 px-2 pt-2 pb-1.5">
        <Search
          className="pointer-events-none absolute top-1/2 left-5 h-3.5 w-3.5 -translate-y-1/2 text-disabled"
          aria-hidden="true"
        />
        <input
          ref={inputRef}
          type="search"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActiveId(null);
          }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown') {
              event.preventDefault();
              move(1);
            } else if (event.key === 'ArrowUp') {
              event.preventDefault();
              move(-1);
            } else if (event.key === 'Enter') {
              event.preventDefault();
              chooseActive();
            }
          }}
          placeholder="Filter subsystems…"
          aria-label={ariaLabel}
          className="input h-[25px] pl-8 text-xs"
        />
      </div>

      <ul
        aria-label="Subsystems"
        className="scroll-area min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-2"
        onWheel={(event) => event.stopPropagation()}
      >
        {filtered.length === 0 ? (
          <li className="px-1 py-2 text-[11px] text-muted">No subsystems match.</li>
        ) : (
          filtered.map((subsystem, index) => {
            const connectivity = CONNECTIVITY_DOT[subsystem.connectivity];
            const selected = subsystem.id === selectedId;
            const active = index === activeIndex;
            const count = counts?.[subsystem.id];
            return (
              <li key={subsystem.id} className="border-b border-line last:border-b-0">
                <button
                  id={`subsystem-filter-row-${subsystem.id}`}
                  type="button"
                  onClick={() => onSelect(subsystem.id)}
                  onMouseEnter={() => setActiveId(subsystem.id)}
                  onFocus={() => setActiveId(subsystem.id)}
                  onKeyDown={(event) => {
                    if (event.key === 'ArrowDown') {
                      event.preventDefault();
                      move(1);
                    } else if (event.key === 'ArrowUp') {
                      event.preventDefault();
                      move(-1);
                    }
                  }}
                  aria-pressed={selected}
                  aria-current={selected ? 'true' : undefined}
                  title={`${subsystem.name} — ${subsystem.repository}`}
                  className={cx(
                    'flex h-[29px] w-full items-center gap-2 px-1.5 text-left focus-visible:control-focus',
                    selected ? 'bg-selection' : active ? 'bg-hover' : 'hover:bg-hover',
                  )}
                >
                  <span
                    className={cx('h-1.5 w-1.5 shrink-0 rounded-full', connectivity.dot)}
                    title={connectivity.label}
                    aria-label={connectivity.label}
                    role="img"
                  />
                  <span className="min-w-0 flex-1 truncate text-xs text-fg">{subsystem.name}</span>
                  <span
                    className="mono min-w-0 max-w-[45%] shrink truncate text-[11px] text-muted"
                    title={subsystem.repository}
                  >
                    {subsystem.repository}
                  </span>
                  {typeof count === 'number' ? (
                    <span className="mono w-10 shrink-0 text-right text-[11px] text-muted tabular-nums">
                      {count}
                    </span>
                  ) : null}
                </button>
              </li>
            );
          })
        )}
      </ul>
    </div>
  );
}

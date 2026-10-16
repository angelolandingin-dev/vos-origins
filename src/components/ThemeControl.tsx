import { Monitor, Moon, Sun } from 'lucide-react';

import { useTheme, type ThemeMode } from '../app/theme';
import { cx } from './ui/cx';

const OPTIONS: Array<{ value: ThemeMode; label: string; icon: typeof Monitor }> = [
  { value: 'system', label: 'System', icon: Monitor },
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
];

/** Compact theme control (System / Light / Dark). Phase B preserves this. */
export function ThemeControl() {
  const { mode, setMode } = useTheme();

  return (
    <div
      role="radiogroup"
      aria-label="Theme"
      className="inline-flex items-center gap-0.5 rounded-md border border-line-strong bg-surface p-0.5"
    >
      {OPTIONS.map((option) => {
        const Icon = option.icon;
        const selected = mode === option.value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            aria-label={`${option.label} theme`}
            title={`${option.label} theme`}
            onClick={() => setMode(option.value)}
            className={cx(
              'inline-flex h-[21px] items-center gap-1.5 rounded-[3px] px-2 text-[11px] font-medium transition-colors',
              selected ? 'bg-selection text-fg' : 'text-muted hover:bg-hover hover:text-fg',
            )}
          >
            <Icon className="h-3 w-3" aria-hidden="true" />
            {option.label}
          </button>
        );
      })}
    </div>
  );
}

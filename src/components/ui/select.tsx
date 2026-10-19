import * as SelectPrimitive from '@radix-ui/react-select';
import { Check, ChevronDown, ChevronUp } from 'lucide-react';

import { cx } from './cx';

export interface UiSelectOption {
  value: string;
  label: string;
  disabled?: boolean;
  title?: string;
}

/**
 * Ledger-styled Radix Select (listbox popup, keyboard-navigable).
 *
 * WebView2/Tauri safety: Radix Select does not take `modal`, so it never sets
 * `pointer-events: none` on the body. Content uses `position="popper"` with
 * `avoidCollisions` so popups near the window edge flip instead of clipping,
 * and closes restore focus to the trigger via `onCloseAutoFocus`.
 */
export function Select({
  value,
  onValueChange,
  options,
  ariaLabel,
  aria_label,
  placeholder = 'Select…',
  className,
  triggerClassName,
  disabled = false,
  id,
  title,
}: {
  value: string;
  onValueChange: (value: string) => void;
  options: UiSelectOption[];
  ariaLabel?: string;
  aria_label?: string;
  placeholder?: string;
  className?: string;
  triggerClassName?: string;
  disabled?: boolean;
  id?: string;
  title?: string;
}) {
  const label = ariaLabel ?? aria_label;
  return (
    <SelectPrimitive.Root value={value} onValueChange={onValueChange} disabled={disabled}>
      <SelectPrimitive.Trigger
        id={id}
        aria-label={label}
        title={title ?? label}
        className={cx(
          'inline-flex h-[25px] max-w-full items-center justify-between gap-1.5 rounded-md border border-line-strong bg-surface px-2 text-xs text-fg transition-colors',
          'hover:bg-hover focus-visible:border-accent focus-visible:outline-none data-[placeholder]:text-muted',
          'disabled:cursor-not-allowed disabled:opacity-60',
          triggerClassName,
          className,
        )}
      >
        <span className="min-w-0 flex-1 truncate text-left">
          <SelectPrimitive.Value placeholder={placeholder} />
        </span>
        <SelectPrimitive.Icon asChild>
          <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted" aria-hidden="true" />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content
          position="popper"
          side="bottom"
          align="start"
          sideOffset={4}
          collisionPadding={8}
          avoidCollisions
          sticky="partial"
          onCloseAutoFocus={(event) => event.preventDefault()}
          className="z-[70] max-h-[280px] w-[var(--radix-select-trigger-width)] min-w-[4.5rem] overflow-hidden rounded-md border border-line bg-overlay animate-fade-in"
        >
          <SelectPrimitive.ScrollUpButton className="flex h-[25px] items-center justify-center bg-overlay text-muted">
            <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
          </SelectPrimitive.ScrollUpButton>
          {/*
            The viewport is the element to cap, not the trigger or the content
            shell. Radix publishes the space left in the window as
            `--radix-select-content-available-height` on the content (inherited
            here); subtracting the two 25px scroll buttons keeps the whole popup
            inside that space. `overscroll-contain` plus a wheel
            stop-propagation keep a wheel gesture at the end of the list from
            scrolling the surface behind it.
          */}
          <SelectPrimitive.Viewport
            className="overflow-y-auto overscroll-contain p-1"
            style={{
              maxHeight:
                'min(230px, max(0px, calc(var(--radix-select-content-available-height) - 50px)))',
            }}
            onWheel={(event) => event.stopPropagation()}
          >
            {options.map((option) => (
              <SelectPrimitive.Item
                key={option.value}
                value={option.value}
                disabled={option.disabled}
                title={option.title ?? option.label}
                className={cx(
                  'flex h-[29px] w-full cursor-pointer items-center justify-start gap-2 rounded-[3px] px-2.5 text-left text-xs text-fg outline-none select-none',
                  'hover:bg-hover focus:bg-selection data-[state=checked]:bg-selection',
                  'data-[disabled]:cursor-not-allowed data-[disabled]:opacity-60',
                )}
              >
                <span
                  className="flex w-3.5 shrink-0 items-center justify-center"
                  aria-hidden="true"
                >
                  <SelectPrimitive.ItemIndicator asChild>
                    <Check className="h-3.5 w-3.5 shrink-0 text-accent" aria-hidden="true" />
                  </SelectPrimitive.ItemIndicator>
                </span>
                <SelectPrimitive.ItemText asChild>
                  <span className="min-w-0 flex-1 truncate text-left">{option.label}</span>
                </SelectPrimitive.ItemText>
              </SelectPrimitive.Item>
            ))}
          </SelectPrimitive.Viewport>
          <SelectPrimitive.ScrollDownButton className="flex h-[25px] items-center justify-center bg-overlay text-muted">
            <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
          </SelectPrimitive.ScrollDownButton>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}

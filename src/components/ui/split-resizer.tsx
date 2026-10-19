import type { KeyboardEvent, PointerEvent } from 'react';
import { useRef } from 'react';

import { SPLIT_MAX_WIDTH, SPLIT_MIN_WIDTH } from './split-pane';

/**
 * A horizontal splitter handle, shared by the diff splitter (changed-files column
 * against the diff) and the commit-browser rail; each supplies its own width and
 * bounds. There is no drag library in the project, so this mirrors `AppShell`'s
 * pointer-event window handles — pointer capture on the handle itself, no document
 * listeners to clean up.
 */

const KEYBOARD_STEP = 16;

export interface SplitResizerProps {
  /** Current width of the pane the handle sits to the right of. */
  width: number;
  onWidthChange: (width: number) => void;
  /** Accessible name, e.g. "Resize the changed-files column". */
  label: string;
  /** Bounds for the dragged pane; default to the diff splitter's. */
  min?: number;
  max?: number;
}

export function SplitResizer({
  width,
  onWidthChange,
  label,
  min = SPLIT_MIN_WIDTH,
  max = SPLIT_MAX_WIDTH,
}: SplitResizerProps) {
  const origin = useRef({ x: 0, width });
  const clamp = (value: number) => Math.min(max, Math.max(min, value));

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    // A drag must never select text; capture keeps move/up on the handle.
    event.preventDefault();
    origin.current = { x: event.clientX, width };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    onWidthChange(clamp(origin.current.width + (event.clientX - origin.current.x)));
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Home') {
      event.preventDefault();
      onWidthChange(min);
      return;
    }
    if (event.key === 'End') {
      event.preventDefault();
      onWidthChange(max);
      return;
    }
    const step =
      event.key === 'ArrowLeft' ? -KEYBOARD_STEP : event.key === 'ArrowRight' ? KEYBOARD_STEP : 0;
    if (step === 0) return;
    event.preventDefault();
    onWidthChange(clamp(width + step));
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onKeyDown={onKeyDown}
      // The visible 1px hairline is the divider; the ::before widens the grab area to 9px.
      className="relative hidden w-px shrink-0 cursor-col-resize touch-none select-none bg-line before:absolute before:inset-y-0 before:-right-1 before:-left-1 before:content-[''] hover:bg-accent focus-visible:bg-accent md:block"
    />
  );
}

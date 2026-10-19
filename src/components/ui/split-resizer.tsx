import type { KeyboardEvent, PointerEvent } from 'react';
import { useRef } from 'react';

import { SPLIT_MAX_WIDTH, SPLIT_MIN_WIDTH, clampSplitWidth } from './split-pane';

/**
 * The one horizontal splitter in the app, shared by both diff surfaces: the
 * changed-files column against the diff. There is no drag library in the
 * project, so this mirrors `AppShell`'s pointer-event window handles — pointer
 * capture on the handle itself, no document listeners to clean up. Geometry and
 * the `--split-width` contract live in `split-pane.ts`.
 */

const KEYBOARD_STEP = 16;

export interface SplitResizerProps {
  /** Current width of the pane the handle sits to the right of. */
  width: number;
  onWidthChange: (width: number) => void;
  /** Accessible name, e.g. "Resize the changed-files column". */
  label: string;
}

export function SplitResizer({ width, onWidthChange, label }: SplitResizerProps) {
  const origin = useRef({ x: 0, width });

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    // A drag must never select text; capture keeps move/up on the handle.
    event.preventDefault();
    origin.current = { x: event.clientX, width };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    onWidthChange(clampSplitWidth(origin.current.width + (event.clientX - origin.current.x)));
  };

  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Home') {
      event.preventDefault();
      onWidthChange(SPLIT_MIN_WIDTH);
      return;
    }
    if (event.key === 'End') {
      event.preventDefault();
      onWidthChange(SPLIT_MAX_WIDTH);
      return;
    }
    const step =
      event.key === 'ArrowLeft' ? -KEYBOARD_STEP : event.key === 'ArrowRight' ? KEYBOARD_STEP : 0;
    if (step === 0) return;
    event.preventDefault();
    onWidthChange(clampSplitWidth(width + step));
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuenow={width}
      aria-valuemin={SPLIT_MIN_WIDTH}
      aria-valuemax={SPLIT_MAX_WIDTH}
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

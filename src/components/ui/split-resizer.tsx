import type { KeyboardEvent, PointerEvent } from 'react';
import { useRef } from 'react';

import { SPLIT_MAX_WIDTH, SPLIT_MIN_WIDTH } from './split-pane';

const KEYBOARD_STEP = 16;

export interface SplitResizerProps {
  width: number;
  onWidthChange: (width: number) => void;
  label: string;
  min?: number;
  max?: number;
  edge?: 'left' | 'right';
}

export function SplitResizer({
  width,
  onWidthChange,
  label,
  min = SPLIT_MIN_WIDTH,
  max = SPLIT_MAX_WIDTH,
  edge = 'left',
}: SplitResizerProps) {
  const origin = useRef({ x: 0, width });
  const clamp = (value: number) => Math.min(max, Math.max(min, value));

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    origin.current = { x: event.clientX, width };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
    const delta =
      edge === 'right' ? origin.current.x - event.clientX : event.clientX - origin.current.x;
    onWidthChange(clamp(origin.current.width + delta));
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
    const rawStep =
      event.key === 'ArrowLeft' ? -KEYBOARD_STEP : event.key === 'ArrowRight' ? KEYBOARD_STEP : 0;
    const step = edge === 'right' ? -rawStep : rawStep;
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
      className="relative hidden w-px shrink-0 cursor-col-resize touch-none select-none bg-line before:absolute before:inset-y-0 before:-right-1 before:-left-1 before:content-[''] hover:bg-accent focus-visible:bg-accent md:block"
    />
  );
}

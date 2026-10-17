import type { ReactNode } from 'react';
import * as ScrollAreaPrimitive from '@radix-ui/react-scroll-area';

import { cx } from './cx';

/**
 * Ledger-styled Radix ScrollArea: consistent thin scrollbars (10px track,
 * hairline-strong thumb) instead of OS defaults. Viewport carries the scroll.
 */
export function ScrollArea({
  children,
  className,
  viewportClassName,
  orientation = 'vertical',
}: {
  children: ReactNode;
  className?: string;
  viewportClassName?: string;
  orientation?: 'vertical' | 'horizontal' | 'both';
}) {
  const showVertical = orientation === 'vertical' || orientation === 'both';
  const showHorizontal = orientation === 'horizontal' || orientation === 'both';
  return (
    <ScrollAreaPrimitive.Root
      type="auto"
      className={cx('relative min-h-0 overflow-hidden', className)}
    >
      <ScrollAreaPrimitive.Viewport
        className={cx('h-full w-full min-h-0 [&>div]:h-full', viewportClassName)}
      >
        {children}
      </ScrollAreaPrimitive.Viewport>
      {showVertical ? (
        <ScrollAreaPrimitive.Scrollbar
          orientation="vertical"
          className="flex w-[10px] touch-none p-[2px] select-none"
        >
          <ScrollAreaPrimitive.Thumb className="flex-1 rounded-full bg-line-strong" />
        </ScrollAreaPrimitive.Scrollbar>
      ) : null}
      {showHorizontal ? (
        <ScrollAreaPrimitive.Scrollbar
          orientation="horizontal"
          className="flex h-[10px] touch-none p-[2px] select-none"
        >
          <ScrollAreaPrimitive.Thumb className="flex-1 rounded-full bg-line-strong" />
        </ScrollAreaPrimitive.Scrollbar>
      ) : null}
      <ScrollAreaPrimitive.Corner className="bg-transparent" />
    </ScrollAreaPrimitive.Root>
  );
}

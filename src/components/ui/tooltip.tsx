import type { ReactNode } from 'react';
import * as TooltipPrimitive from '@radix-ui/react-tooltip';

export function Tooltip({
  content,
  children,
  side = 'top',
  align = 'center',
  delayDuration = 400,
  disabled = false,
}: {
  content: ReactNode;
  children: ReactNode;
  side?: 'top' | 'right' | 'bottom' | 'left';
  align?: 'start' | 'center' | 'end';
  delayDuration?: number;
  disabled?: boolean;
}) {
  if (disabled || !content) return <>{children}</>;
  return (
    <TooltipPrimitive.Provider delayDuration={delayDuration}>
      <TooltipPrimitive.Root>
        <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
        <TooltipPrimitive.Portal>
          <TooltipPrimitive.Content
            side={side}
            align={align}
            sideOffset={4}
            collisionPadding={8}
            avoidCollisions
            sticky="partial"
            className="z-[80] max-w-[320px] rounded-[3px] border border-line bg-overlay px-1.5 py-1 text-[11px] leading-snug break-words text-fg animate-fade-in"
          >
            {content}
            <TooltipPrimitive.Arrow className="fill-line" width={8} height={4} aria-hidden="true" />
          </TooltipPrimitive.Content>
        </TooltipPrimitive.Portal>
      </TooltipPrimitive.Root>
    </TooltipPrimitive.Provider>
  );
}

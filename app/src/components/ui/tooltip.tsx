import type { ComponentProps, ReactNode } from "react";
import { Tooltip as TooltipPrimitive } from "radix-ui";
import { cn } from "@/lib/utils";

export function TooltipProvider(props: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={450} skipDelayDuration={250} {...props} />;
}

export interface TooltipProps {
  content: ReactNode;
  children: ReactNode;
  side?: "top" | "right" | "bottom" | "left";
  align?: "start" | "center" | "end";
  /** Keyboard shortcut shown after the label. */
  shortcut?: string;
  disabled?: boolean;
}

export function Tooltip({ content, children, side = "top", align = "center", shortcut, disabled }: TooltipProps) {
  if (disabled) return <>{children}</>;
  return (
    <TooltipPrimitive.Root>
      <TooltipPrimitive.Trigger asChild>{children}</TooltipPrimitive.Trigger>
      <TooltipPrimitive.Portal>
        <TooltipPrimitive.Content
          side={side}
          align={align}
          sideOffset={7}
          collisionPadding={8}
          className={cn(
            "z-[70] flex items-center gap-2 rounded-lg bg-tooltip px-2.5 py-1.5 text-xs font-medium text-tooltip-foreground shadow-overlay",
            "origin-(--radix-tooltip-content-transform-origin) animate-in fade-in-0 zoom-in-95 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          )}
        >
          {content}
          {shortcut && (
            <kbd className="rounded bg-white/15 px-1.5 py-px font-mono text-[10.5px] leading-4 text-white/85">
              {shortcut}
            </kbd>
          )}
        </TooltipPrimitive.Content>
      </TooltipPrimitive.Portal>
    </TooltipPrimitive.Root>
  );
}

import { useId, type ReactNode } from "react";
import { ToggleGroup } from "radix-ui";
import { motion } from "motion/react";
import { cn } from "@/lib/utils";
import { transitions } from "@/lib/motion";
import { Tooltip } from "./tooltip";

export interface SegmentedItem<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  /** Show only the icon; the label becomes the tooltip and accessible name. */
  iconOnly?: boolean;
}

export interface SegmentedProps<T extends string> {
  value: T;
  onValueChange: (value: T) => void;
  items: readonly SegmentedItem<T>[];
  "aria-label": string;
  size?: "sm" | "md";
  tone?: "default" | "overlay";
  className?: string;
}

/** A single-choice segmented control (radio-like), with a gliding thumb. */
export function Segmented<T extends string>({
  value,
  onValueChange,
  items,
  size = "md",
  tone = "default",
  className,
  ...props
}: SegmentedProps<T>) {
  const thumbId = useId();
  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      onValueChange={(v) => v && onValueChange(v as T)}
      aria-label={props["aria-label"]}
      className={cn(
        "inline-flex items-center gap-0.5 rounded-full p-1",
        tone === "overlay" ? "video-glass" : "bg-surface-3",
        className,
      )}
    >
      {items.map((item) => {
        const active = item.value === value;
        const button = (
          <ToggleGroup.Item
            key={item.value}
            value={item.value}
            aria-label={item.iconOnly ? item.label : undefined}
            className={cn(
              "relative inline-flex select-none items-center justify-center gap-1.5 rounded-full font-medium transition-colors duration-(--dur-fast)",
              size === "sm" ? "h-7 min-w-7 px-2.5 text-xs" : "h-8 min-w-8 px-3.5 text-[13px]",
              item.iconOnly && (size === "sm" ? "px-1.5" : "px-2"),
              tone === "overlay"
                ? active
                  ? "text-fg"
                  : "text-white/75 hover:text-white"
                : active
                  ? "text-fg"
                  : "text-fg-2 hover:text-fg",
              "[&_svg]:size-4 [&_svg]:shrink-0",
            )}
          >
            {active && (
              <motion.span
                layoutId={thumbId}
                className={cn("absolute inset-0 rounded-full shadow-xs", tone === "overlay" ? "bg-white" : "bg-thumb")}
                transition={transitions.spring}
                aria-hidden
              />
            )}
            <span className={cn("relative z-10 inline-flex items-center gap-1.5", tone === "overlay" && active && "text-[#0f1729]")}>
              {item.icon}
              {!item.iconOnly && item.label}
            </span>
          </ToggleGroup.Item>
        );
        return item.iconOnly ? (
          <Tooltip key={item.value} content={item.label}>
            {button}
          </Tooltip>
        ) : (
          button
        );
      })}
    </ToggleGroup.Root>
  );
}

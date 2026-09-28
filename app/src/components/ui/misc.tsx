// Small display primitives: card, skeleton, badge, chip, progress, kbd, separator.
import type { ComponentProps, ReactNode } from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

export function Card({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      className={cn("rounded-card border border-card-border bg-surface shadow-card", className)}
      {...props}
    />
  );
}

export function Skeleton({ className, ...props }: ComponentProps<"div">) {
  return <div aria-hidden className={cn("skeleton rounded-md", className)} {...props} />;
}

export const badgeVariants = cva(
  "inline-flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-full font-medium leading-none [&_svg]:size-3.5 [&_svg]:shrink-0",
  {
    variants: {
      tone: {
        neutral: "bg-neutral-soft text-neutral",
        brand: "bg-brand-soft text-brand-text",
        success: "bg-success-soft text-success",
        warning: "bg-warning-soft text-warning",
        danger: "bg-danger-soft text-danger",
        live: "bg-live text-white",
        overlay: "video-glass",
        outline: "border border-border-strong text-fg-2",
      },
      size: {
        sm: "h-5 px-2 text-[11px]",
        md: "h-6 px-2.5 text-xs",
        lg: "h-7 px-3 text-[13px]",
      },
    },
    defaultVariants: { tone: "neutral", size: "md" },
  },
);

export function Badge({
  className,
  tone,
  size,
  ...props
}: ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return <span className={cn(badgeVariants({ tone, size }), className)} {...props} />;
}

/** A toggleable filter chip. */
export function Chip({
  selected,
  className,
  children,
  leading,
  count,
  ...props
}: ComponentProps<"button"> & { selected: boolean; leading?: ReactNode; count?: number }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      className={cn(
        "inline-flex h-8 shrink-0 select-none items-center gap-1.5 rounded-full border px-3 text-[13px] font-medium transition-[background-color,border-color,color] duration-(--dur-fast)",
        selected
          ? "border-transparent bg-fg text-background"
          : "border-border bg-surface text-fg-2 hover:border-border-strong hover:text-fg",
        className,
      )}
      {...props}
    >
      {leading}
      {children}
      {count !== undefined && (
        <span className={cn("tabular-nums", selected ? "text-background/70" : "text-fg-3")}>{count}</span>
      )}
    </button>
  );
}

export function Progress({
  value,
  indeterminate,
  tone = "brand",
  className,
  label,
}: {
  /** 0..1 */
  value: number;
  indeterminate?: boolean;
  tone?: "brand" | "success" | "danger" | "neutral";
  className?: string;
  label?: string;
}) {
  const pct = Math.round(Math.min(1, Math.max(0, value)) * 100);
  const fill = {
    brand: "bg-brand",
    success: "bg-success-dot",
    danger: "bg-danger-dot",
    neutral: "bg-neutral-dot",
  }[tone];
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : pct}
      className={cn("relative h-1.5 w-full overflow-hidden rounded-full bg-sunken", className)}
    >
      {indeterminate ? (
        <div className="skeleton absolute inset-0 rounded-full opacity-80" />
      ) : (
        <div
          className={cn("h-full rounded-full transition-[width] duration-300 ease-standard", fill)}
          style={{ width: `${pct}%` }}
        />
      )}
    </div>
  );
}

export function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "inline-flex h-[22px] min-w-[22px] items-center justify-center rounded-md border border-border-strong bg-surface-2 px-1.5 font-mono text-[11px] font-medium text-fg-2 shadow-[inset_0_-1px_0_var(--border-strong)]",
        className,
      )}
      {...props}
    />
  );
}

export function Separator({ className, vertical }: { className?: string; vertical?: boolean }) {
  return (
    <div
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      className={cn(vertical ? "w-px self-stretch" : "h-px w-full", "bg-border", className)}
    />
  );
}

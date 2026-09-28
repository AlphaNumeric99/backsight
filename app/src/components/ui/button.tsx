import type { ComponentProps } from "react";
import { Slot } from "radix-ui";
import { cva, type VariantProps } from "class-variance-authority";
import { LoaderCircle } from "lucide-react";
import { cn } from "@/lib/utils";

export const buttonVariants = cva(
  [
    "relative inline-flex shrink-0 select-none items-center justify-center gap-2 whitespace-nowrap rounded-full font-medium",
    "transition-[background-color,color,box-shadow,opacity,transform] duration-(--dur-fast) ease-standard",
    "active:scale-[0.97] disabled:pointer-events-none disabled:opacity-45 aria-disabled:pointer-events-none aria-disabled:opacity-45",
    "[&_svg]:pointer-events-none [&_svg]:shrink-0",
  ],
  {
    variants: {
      variant: {
        primary: "bg-brand text-brand-contrast shadow-xs hover:bg-brand-hover active:bg-brand-pressed",
        secondary: "bg-surface-3 text-fg hover:bg-surface-3-hover",
        outline: "border border-border-strong bg-surface text-fg shadow-xs hover:bg-surface-2",
        ghost: "text-fg-2 hover:bg-hover hover:text-fg active:bg-press",
        soft: "bg-brand-soft text-brand-text hover:bg-brand-soft-hover",
        danger: "bg-danger-dot text-white shadow-xs hover:bg-danger-hover",
        "danger-ghost": "text-danger hover:bg-danger-soft",
        overlay: "video-glass hover:bg-black/65 active:bg-black/75",
        "overlay-ghost": "text-white/90 hover:bg-white/15 hover:text-white active:bg-white/25",
        link: "h-auto rounded-md px-0 text-brand-text underline-offset-4 hover:underline active:scale-100",
      },
      size: {
        xs: "h-7 px-2.5 text-xs [&_svg]:size-3.5",
        sm: "h-8 px-3 text-[13px] [&_svg]:size-4",
        md: "h-9 px-4 text-sm [&_svg]:size-[18px]",
        lg: "h-11 px-5 text-[15px] [&_svg]:size-5",
        icon: "size-9 [&_svg]:size-[18px]",
        "icon-xs": "size-7 [&_svg]:size-4",
        "icon-sm": "size-8 [&_svg]:size-[17px]",
        "icon-lg": "size-11 [&_svg]:size-5",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export type ButtonProps = ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    asChild?: boolean;
    /** Shows a spinner and blocks clicks while an action runs. */
    loading?: boolean;
  };

export function Button({
  className,
  variant,
  size,
  asChild = false,
  loading = false,
  disabled,
  children,
  type,
  ...props
}: ButtonProps) {
  const Comp = asChild ? Slot.Root : "button";
  return (
    <Comp
      data-slot="button"
      className={cn(buttonVariants({ variant, size }), className)}
      disabled={asChild ? undefined : disabled || loading}
      aria-busy={loading || undefined}
      type={asChild ? undefined : (type ?? "button")}
      {...props}
    >
      {asChild ? (
        children
      ) : (
        <>
          {loading && <LoaderCircle className="animate-spin" aria-hidden />}
          {children}
        </>
      )}
    </Comp>
  );
}

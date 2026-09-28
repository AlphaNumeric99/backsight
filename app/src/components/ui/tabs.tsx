import { createContext, useContext, useId, type ComponentProps, type ReactNode } from "react";
import { Tabs as TabsPrimitive } from "radix-ui";
import { motion } from "motion/react";
import { cn } from "@/lib/utils";
import { transitions } from "@/lib/motion";

type Variant = "segmented" | "pills" | "dark";

const TabsContext = createContext<{ value: string; indicatorId: string; variant: Variant }>({
  value: "",
  indicatorId: "",
  variant: "segmented",
});

export interface TabsProps extends Omit<ComponentProps<typeof TabsPrimitive.Root>, "value" | "onValueChange"> {
  value: string;
  onValueChange: (value: string) => void;
  variant?: Variant;
}

/** Controlled tabs whose active indicator glides between triggers. */
export function Tabs({ value, onValueChange, variant = "segmented", children, ...props }: TabsProps) {
  const indicatorId = useId();
  return (
    <TabsPrimitive.Root value={value} onValueChange={onValueChange} {...props}>
      <TabsContext.Provider value={{ value, indicatorId, variant }}>{children}</TabsContext.Provider>
    </TabsPrimitive.Root>
  );
}

const listClass: Record<Variant, string> = {
  segmented: "inline-flex items-center gap-0.5 rounded-full bg-surface-3 p-1",
  pills: "flex flex-wrap items-center gap-2",
  dark: "inline-flex items-center gap-0.5 rounded-full bg-white/10 p-1",
};

export function TabsList({ className, ...props }: ComponentProps<typeof TabsPrimitive.List>) {
  const { variant } = useContext(TabsContext);
  return <TabsPrimitive.List className={cn(listClass[variant], className)} {...props} />;
}

const triggerClass: Record<Variant, string> = {
  segmented:
    "h-8 px-4 text-[13px] font-medium text-fg-2 hover:text-fg data-[state=active]:text-fg rounded-full",
  pills:
    "h-9 px-4 text-[13px] font-medium rounded-full border border-card-border bg-surface/80 text-fg-2 shadow-xs backdrop-blur hover:text-fg data-[state=active]:border-transparent data-[state=active]:text-background",
  dark: "h-8 px-4 text-[13px] font-medium text-white/70 hover:text-white data-[state=active]:text-fg rounded-full",
};

const indicatorClass: Record<Variant, string> = {
  segmented: "bg-thumb shadow-xs",
  pills: "bg-fg",
  dark: "bg-white",
};

export interface TabsTriggerProps extends ComponentProps<typeof TabsPrimitive.Trigger> {
  value: string;
  children: ReactNode;
}

export function TabsTrigger({ value, className, children, ...props }: TabsTriggerProps) {
  const ctx = useContext(TabsContext);
  const active = ctx.value === value;
  return (
    <TabsPrimitive.Trigger
      value={value}
      className={cn(
        "relative inline-flex select-none items-center justify-center whitespace-nowrap transition-colors duration-(--dur-fast)",
        triggerClass[ctx.variant],
        className,
      )}
      {...props}
    >
      {active && (
        <motion.span
          layoutId={ctx.indicatorId}
          className={cn("absolute inset-0 rounded-full", indicatorClass[ctx.variant])}
          transition={transitions.spring}
          aria-hidden
        />
      )}
      <span className="relative z-10 inline-flex items-center gap-1.5">{children}</span>
    </TabsPrimitive.Trigger>
  );
}

export function TabsContent({ className, ...props }: ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content className={cn("outline-none", className)} {...props} />;
}

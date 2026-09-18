import type { ReactNode } from "react";
import { motion } from "motion/react";
import { cn } from "@/lib/utils";
import { transitions } from "@/lib/motion";

export interface EmptyStateProps {
  art?: ReactNode;
  title: ReactNode;
  body?: ReactNode;
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
  size?: "sm" | "lg";
}

export function EmptyState({ art, title, body, actions, children, className, size = "lg" }: EmptyStateProps) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0, transition: transitions.slow }}
      className={cn(
        "mx-auto flex max-w-md flex-col items-center text-center",
        size === "lg" ? "py-16" : "py-10",
        className,
      )}
    >
      {art && <div className={cn(size === "lg" ? "mb-6" : "mb-4")}>{art}</div>}
      <h2 className={cn("font-semibold tracking-[-0.01em] text-fg", size === "lg" ? "text-xl" : "text-[15px]")}>
        {title}
      </h2>
      {body && (
        <p className={cn("mt-2 text-fg-2", size === "lg" ? "text-[14.5px] leading-relaxed" : "text-[13px]")}>{body}</p>
      )}
      {children}
      {actions && <div className="mt-6 flex flex-wrap items-center justify-center gap-2">{actions}</div>}
    </motion.div>
  );
}

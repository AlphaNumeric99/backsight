import { useId, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "motion/react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { transitions } from "@/lib/motion";

export interface DisclosureProps {
  summary: ReactNode;
  children: ReactNode;
  defaultOpen?: boolean;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  className?: string;
  summaryClassName?: string;
  icon?: ReactNode;
}

/** A button that reveals content below it with a height animation. */
export function Disclosure({
  summary,
  children,
  defaultOpen = false,
  open: controlled,
  onOpenChange,
  className,
  summaryClassName,
  icon,
}: DisclosureProps) {
  const [uncontrolled, setUncontrolled] = useState(defaultOpen);
  /** Clip only while the height animates, so focus rings inside aren't cut off. */
  const [animating, setAnimating] = useState(false);
  const open = controlled ?? uncontrolled;
  const id = useId();
  const toggle = () => {
    const next = !open;
    setAnimating(true);
    setUncontrolled(next);
    onOpenChange?.(next);
  };
  return (
    <div className={className}>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={toggle}
        className={cn(
          "flex w-full items-center gap-2 rounded-lg text-left text-[13px] font-medium text-fg transition-colors hover:text-brand-text",
          summaryClassName,
        )}
      >
        {icon}
        <span className="flex-1">{summary}</span>
        <ChevronDown
          className={cn("size-4 shrink-0 text-fg-3 transition-transform duration-(--dur-base)", open && "rotate-180")}
          aria-hidden
        />
      </button>
      <AnimatePresence initial={false}>
        {open && (
          <motion.div
            id={id}
            key="content"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1, transition: transitions.base }}
            exit={{ height: 0, opacity: 0, transition: transitions.fast }}
            onAnimationStart={() => setAnimating(true)}
            onAnimationComplete={() => setAnimating(false)}
            className={cn(animating && "overflow-hidden")}
          >
            {children}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

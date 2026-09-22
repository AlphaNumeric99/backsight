import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { Card } from "@/components/ui/misc";

export function SettingsSection({
  id,
  title,
  description,
  children,
  className,
}: {
  id: string;
  title: string;
  description?: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section id={`settings-${id}`} aria-labelledby={`settings-${id}-title`} className={cn("scroll-mt-6", className)}>
      <div className="mb-3 px-1">
        <h2 id={`settings-${id}-title`} className="text-[15px] font-semibold tracking-[-0.01em] text-fg">
          {title}
        </h2>
        {description && <p className="mt-0.5 text-[13px] text-fg-2">{description}</p>}
      </div>
      <Card className="divide-y divide-border overflow-hidden">{children}</Card>
    </section>
  );
}

export function SettingRow({
  label,
  help,
  htmlFor,
  children,
  stacked = false,
}: {
  label: ReactNode;
  help?: ReactNode;
  htmlFor?: string;
  children: ReactNode;
  /** Put the control under the label (wide controls). */
  stacked?: boolean;
}) {
  return (
    <div className={cn("px-5 py-4", stacked ? "grid gap-3" : "flex flex-wrap items-center justify-between gap-x-8 gap-y-3")}>
      <div className="min-w-0 max-w-md">
        {htmlFor ? (
          <label htmlFor={htmlFor} className="text-sm font-medium text-fg">
            {label}
          </label>
        ) : (
          <p className="text-sm font-medium text-fg">{label}</p>
        )}
        {help && <p className="mt-0.5 text-[13px] leading-relaxed text-fg-2">{help}</p>}
      </div>
      <div className={cn(stacked ? "" : "shrink-0")}>{children}</div>
    </div>
  );
}

import { useId, useState, type ComponentProps, type ReactNode } from "react";
import { Eye, EyeOff } from "lucide-react";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";

export const inputClass = cn(
  "h-10 w-full min-w-0 rounded-control border border-input bg-surface px-3 text-sm text-fg shadow-xs",
  "placeholder:text-fg-3 transition-[border-color,box-shadow] duration-(--dur-fast)",
  "focus-visible:border-brand focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-brand/15",
  "aria-invalid:border-danger-dot aria-invalid:ring-danger-dot/15 disabled:cursor-not-allowed disabled:opacity-50",
);

export function Input({ className, ...props }: ComponentProps<"input">) {
  return <input data-slot="input" className={cn(inputClass, className)} {...props} />;
}

export function PasswordInput({ className, ...props }: Omit<ComponentProps<"input">, "type">) {
  const [visible, setVisible] = useState(false);
  return (
    <div className="relative">
      <input
        data-slot="input"
        type={visible ? "text" : "password"}
        spellCheck={false}
        autoCapitalize="off"
        className={cn(inputClass, "pr-11", className)}
        {...props}
      />
      <button
        type="button"
        onClick={() => setVisible((v) => !v)}
        aria-label={visible ? strings.common.hide : strings.common.show}
        aria-pressed={visible}
        className="absolute inset-y-1 right-1 grid w-9 place-items-center rounded-lg text-fg-3 transition-colors hover:bg-hover hover:text-fg"
      >
        {visible ? <EyeOff className="size-[18px]" /> : <Eye className="size-[18px]" />}
      </button>
    </div>
  );
}

export interface FieldProps {
  label: ReactNode;
  /** Visible hint under the input. */
  help?: ReactNode;
  error?: ReactNode;
  optional?: boolean;
  className?: string;
  children: (ids: { id: string; describedBy?: string; invalid: boolean }) => ReactNode;
}

/** Label, control, help and error text, wired together for assistive tech. */
export function Field({ label, help, error, optional, className, children }: FieldProps) {
  const id = useId();
  const helpId = help ? `${id}-help` : undefined;
  const errorId = error ? `${id}-error` : undefined;
  const describedBy = [errorId, helpId].filter(Boolean).join(" ") || undefined;
  return (
    <div className={cn("grid gap-1.5", className)}>
      <label htmlFor={id} className="flex items-baseline justify-between text-[13px] font-medium text-fg">
        {label}
        {optional && <span className="text-xs font-normal text-fg-3">{strings.common.optional}</span>}
      </label>
      {children({ id, describedBy, invalid: Boolean(error) })}
      {error && (
        <p id={errorId} className="text-[12.5px] text-danger" role="alert">
          {error}
        </p>
      )}
      {help && (
        <p id={helpId} className="text-[12.5px] leading-relaxed text-fg-2">
          {help}
        </p>
      )}
    </div>
  );
}

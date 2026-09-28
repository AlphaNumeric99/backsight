import { useId } from "react";
import { cn } from "@/lib/utils";
import { strings } from "@/lib/strings";

/** The Backsight glyph: a lens with a look-back arc, on an azure tile. */
export function BrandGlyph({ className }: { className?: string }) {
  const id = useId();
  return (
    <svg viewBox="0 0 32 32" className={cn("size-8 shrink-0", className)} aria-hidden>
      <defs>
        <linearGradient id={id} x1="3" y1="1" x2="29" y2="31" gradientUnits="userSpaceOnUse">
          <stop offset="0" stopColor="#6A93FF" />
          <stop offset="1" stopColor="#2A4FE0" />
        </linearGradient>
      </defs>
      <rect width="32" height="32" rx="9" fill={`url(#${id})`} />
      <path d="M9.7 11.1A7.6 7.6 0 1 1 8.5 17.2" fill="none" stroke="#fff" strokeWidth="2.4" strokeLinecap="round" />
      <path
        d="M8.1 7.9v4.3h4.3"
        fill="none"
        stroke="#fff"
        strokeWidth="2.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="16.3" cy="16.1" r="2.7" fill="#fff" />
    </svg>
  );
}

export function Wordmark({ collapsed = false, className }: { collapsed?: boolean; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-2.5", className)}>
      <BrandGlyph />
      {!collapsed && (
        <span className="text-[16px] font-semibold tracking-[-0.02em] text-fg">{strings.app.name}</span>
      )}
    </span>
  );
}

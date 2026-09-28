import {
  Activity,
  AudioLines,
  Baby,
  BellRing,
  CarFront,
  CircleDot,
  PawPrint,
  PersonStanding,
  ScanLine,
  ShieldAlert,
  SquareDashed,
  type LucideIcon,
} from "lucide-react";
import type { EventType } from "@/ipc";
import { cn } from "@/lib/utils";
import { eventColor, eventLabel } from "@/lib/events";

export const EVENT_ICONS: Record<EventType, LucideIcon> = {
  motion: Activity,
  person: PersonStanding,
  vehicle: CarFront,
  pet: PawPrint,
  baby_cry: Baby,
  sound: AudioLines,
  line_crossing: ScanLine,
  area_intrusion: SquareDashed,
  tamper: ShieldAlert,
  doorbell: BellRing,
  other: CircleDot,
};

const sizes = {
  xs: "size-[18px] [&_svg]:size-[11px]",
  sm: "size-[22px] [&_svg]:size-[13px]",
  md: "size-7 [&_svg]:size-4",
};

/** An event type as a coloured round glyph, like an app icon. */
export function EventGlyph({
  type,
  size = "sm",
  className,
  labelled = true,
}: {
  type: EventType;
  size?: keyof typeof sizes;
  className?: string;
  /** Adds the type's name for assistive tech (off when a visible label sits next to it). */
  labelled?: boolean;
}) {
  const Icon = EVENT_ICONS[type];
  return (
    <span
      role={labelled ? "img" : undefined}
      aria-label={labelled ? eventLabel(type) : undefined}
      aria-hidden={labelled ? undefined : true}
      title={labelled ? eventLabel(type) : undefined}
      className={cn("inline-grid shrink-0 place-items-center rounded-full text-white shadow-[inset_0_0_0_1px_rgb(0_0_0/0.06)]", sizes[size], className)}
      style={{ background: eventColor(type) }}
    >
      <Icon strokeWidth={2.3} />
    </span>
  );
}

export function EventDot({ type, className }: { type: EventType; className?: string }) {
  return (
    <span
      aria-hidden
      className={cn("inline-block size-2 shrink-0 rounded-full", className)}
      style={{ background: eventColor(type) }}
    />
  );
}

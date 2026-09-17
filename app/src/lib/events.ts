import type { EventType } from "@/ipc/api";
import { strings } from "./strings";

/** Display order for filters and legends. */
export const EVENT_TYPES: readonly EventType[] = [
  "person",
  "vehicle",
  "pet",
  "motion",
  "baby_cry",
  "sound",
  "line_crossing",
  "area_intrusion",
  "doorbell",
  "tamper",
  "other",
];

/** CSS custom property holding each event type's colour (see styles/globals.css). */
export const EVENT_COLOR_VAR: Record<EventType, string> = {
  motion: "--ev-motion",
  person: "--ev-person",
  vehicle: "--ev-vehicle",
  pet: "--ev-pet",
  baby_cry: "--ev-baby-cry",
  sound: "--ev-sound",
  line_crossing: "--ev-line-crossing",
  area_intrusion: "--ev-area-intrusion",
  tamper: "--ev-tamper",
  doorbell: "--ev-doorbell",
  other: "--ev-other",
};

export function eventLabel(type: EventType): string {
  return strings.events[type];
}

export function eventColor(type: EventType): string {
  return `var(${EVENT_COLOR_VAR[type]})`;
}

/**
 * The type that best describes an event with several: a person beats plain motion, and so on.
 * Used for the bar colour on the timeline and the lead glyph on cards.
 */
export function primaryEventType(types: readonly EventType[]): EventType {
  if (types.length === 0) return "other";
  let best = types[0];
  for (const t of types) if (EVENT_TYPES.indexOf(t) < EVENT_TYPES.indexOf(best)) best = t;
  return best;
}

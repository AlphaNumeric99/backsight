import type { EventType } from "@/ipc";
import { EVENT_COLOR_VAR, EVENT_TYPES } from "@/lib/events";

/** Resolved colours for canvas drawing, read from the CSS tokens around the canvas. */
export interface TimelinePalette {
  band: string;
  bandStrong: string;
  tick: string;
  tickMajor: string;
  label: string;
  playhead: string;
  now: string;
  hover: string;
  outside: string;
  future: string;
  selection: string;
  surface: string;
  events: Record<EventType, string>;
  font: string;
}

export function readPalette(el: Element): TimelinePalette {
  const cs = getComputedStyle(el);
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  const events = {} as Record<EventType, string>;
  for (const t of EVENT_TYPES) events[t] = v(EVENT_COLOR_VAR[t], "#8a93a3");
  const family = cs.fontFamily || "system-ui, sans-serif";
  return {
    band: v("--tl-band", "rgba(52,97,244,0.15)"),
    bandStrong: v("--tl-band-strong", "rgba(52,97,244,0.3)"),
    tick: v("--tl-tick", "rgba(15,23,41,0.13)"),
    tickMajor: v("--tl-tick-major", "rgba(15,23,41,0.32)"),
    label: v("--tl-label", "#687182"),
    playhead: v("--tl-playhead", "#3461f4"),
    now: v("--tl-now", "#e5484d"),
    hover: v("--tl-hover", "rgba(15,23,41,0.42)"),
    outside: v("--tl-outside", "rgba(15,23,41,0.04)"),
    future: v("--tl-future", "rgba(15,23,41,0.05)"),
    selection: v("--tl-selection", "rgba(52,97,244,0.16)"),
    surface: v("--surface", "#ffffff"),
    events,
    font: `500 11px ${family}`,
  };
}

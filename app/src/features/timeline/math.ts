// Pure timeline math: mapping between time and pixels, zooming around an anchor, panning,
// tick generation aligned to camera-local clock time, and inertia. No DOM here, so it's easy
// to test and cheap to call every frame.

import { DAY, HOUR, MINUTE, SECOND, formatHm } from "@/lib/time";

export interface TimelineView {
  /** Time at the horizontal centre, epoch ms. */
  centerMs: number;
  /** Visible duration across the full width, ms. */
  spanMs: number;
  /** Width in CSS pixels. */
  widthPx: number;
}

export interface Range {
  start: number;
  end: number;
}

/** Zoom limits: from the whole day down to about five minutes across. */
export const MIN_SPAN_MS = 5 * MINUTE;
export const MAX_SPAN_MS = DAY;

export const viewStart = (v: TimelineView) => v.centerMs - v.spanMs / 2;
export const viewEnd = (v: TimelineView) => v.centerMs + v.spanMs / 2;
export const msPerPx = (v: TimelineView) => v.spanMs / Math.max(1, v.widthPx);

export function timeToX(v: TimelineView, t: number): number {
  return ((t - viewStart(v)) / v.spanMs) * v.widthPx;
}

export function xToTime(v: TimelineView, x: number): number {
  return viewStart(v) + (x / Math.max(1, v.widthPx)) * v.spanMs;
}

export const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export interface ZoomLimits {
  minSpan?: number;
  maxSpan?: number;
  /** The centre is kept inside this range (the selected day). */
  bounds?: Range;
}

export function clampView(v: TimelineView, limits: ZoomLimits = {}): TimelineView {
  const spanMs = clamp(v.spanMs, limits.minSpan ?? MIN_SPAN_MS, limits.maxSpan ?? MAX_SPAN_MS);
  const centerMs = limits.bounds ? clamp(v.centerMs, limits.bounds.start, limits.bounds.end) : v.centerMs;
  return spanMs === v.spanMs && centerMs === v.centerMs ? v : { ...v, spanMs, centerMs };
}

/**
 * Zooms by `factor` (> 1 zooms out, < 1 zooms in) keeping the time under `anchorX` at the same
 * pixel, like zooming a map around the cursor. Span and centre are clamped to the limits.
 */
export function zoomAt(v: TimelineView, anchorX: number, factor: number, limits: ZoomLimits = {}): TimelineView {
  const anchorT = xToTime(v, anchorX);
  const spanMs = clamp(v.spanMs * factor, limits.minSpan ?? MIN_SPAN_MS, limits.maxSpan ?? MAX_SPAN_MS);
  const start = anchorT - (anchorX / Math.max(1, v.widthPx)) * spanMs;
  return clampView({ ...v, spanMs, centerMs: start + spanMs / 2 }, limits);
}

/** Zooms keeping time `anchorMs` fixed on screen (e.g. the playhead). */
export function zoomAroundTime(v: TimelineView, anchorMs: number, factor: number, limits: ZoomLimits = {}): TimelineView {
  return zoomAt(v, timeToX(v, anchorMs), factor, limits);
}

/** Moves the view so content follows the pointer by `dx` pixels (drag right → earlier times). */
export function panByPx(v: TimelineView, dx: number, bounds?: Range): TimelineView {
  return clampView({ ...v, centerMs: v.centerMs - dx * msPerPx(v) }, { bounds, minSpan: 0, maxSpan: Infinity });
}

// --- Ticks -----------------------------------------------------------------------------------

/** Every step divides 24 h, so ticks line up with local clock time on any day. */
export const TICK_STEPS = [
  10 * SECOND,
  30 * SECOND,
  MINUTE,
  2 * MINUTE,
  5 * MINUTE,
  10 * MINUTE,
  15 * MINUTE,
  30 * MINUTE,
  HOUR,
  2 * HOUR,
  3 * HOUR,
  6 * HOUR,
  12 * HOUR,
] as const;

export const LABEL_STEPS = TICK_STEPS.filter((s) => s >= MINUTE);

export interface TickOptions {
  /** Minimum gap between minor ticks, px. */
  minMinorPx?: number;
  /** Minimum gap between labels, px. */
  minLabelPx?: number;
}

export function chooseTickSteps(msPerPixel: number, options: TickOptions = {}): { minor: number; label: number } {
  const minMinorPx = options.minMinorPx ?? 9;
  const minLabelPx = options.minLabelPx ?? 76;
  const minor = TICK_STEPS.find((s) => s / msPerPixel >= minMinorPx) ?? TICK_STEPS[TICK_STEPS.length - 1];
  const label =
    LABEL_STEPS.find((s) => s >= minor && s % minor === 0 && s / msPerPixel >= minLabelPx) ??
    LABEL_STEPS[LABEL_STEPS.length - 1];
  return { minor, label };
}

export interface Tick {
  t: number;
  x: number;
  /** Carries a label. */
  major: boolean;
  label?: string;
}

const mod = (a: number, n: number) => ((a % n) + n) % n;

/**
 * Ticks for the visible range, aligned to `originMs` (camera-local midnight of the selected day)
 * and labelled in camera-local time. The end of the day reads "24:00" rather than "00:00".
 */
export function generateTicks(
  v: TimelineView,
  originMs: number,
  offsetMinutes: number,
  options: TickOptions = {},
): { minor: number; label: number; ticks: Tick[] } {
  const { minor, label } = chooseTickSteps(msPerPx(v), options);
  const start = viewStart(v);
  const end = viewEnd(v);
  const first = originMs + Math.ceil((start - originMs) / minor) * minor;
  const ticks: Tick[] = [];
  for (let t = first; t <= end && ticks.length < 2000; t += minor) {
    const rel = t - originMs;
    const major = mod(rel, label) === 0;
    ticks.push({
      t,
      x: timeToX(v, t),
      major,
      label: major ? (rel === DAY ? "24:00" : formatHm(t, offsetMinutes)) : undefined,
    });
  }
  return { minor, label, ticks };
}

// --- Inertia ---------------------------------------------------------------------------------

/** Exponential decay, time constant in ms (≈ iOS scrolling feel). */
export const INERTIA_TIME_CONSTANT_MS = 325;

export function decayVelocity(velocity: number, dtMs: number, timeConstantMs = INERTIA_TIME_CONSTANT_MS): number {
  return velocity * Math.exp(-dtMs / timeConstantMs);
}

/** Total distance an initial velocity (px/ms) will coast. */
export function coastDistance(velocity: number, timeConstantMs = INERTIA_TIME_CONSTANT_MS): number {
  return velocity * timeConstantMs;
}

/** Pointer velocity (px/ms) from recent samples, ignoring anything older than `windowMs`. */
export function releaseVelocity(samples: readonly { x: number; t: number }[], windowMs = 90): number {
  if (samples.length < 2) return 0;
  const last = samples[samples.length - 1];
  let first = last;
  for (let i = samples.length - 2; i >= 0; i--) {
    if (last.t - samples[i].t > windowMs) break;
    first = samples[i];
  }
  const dt = last.t - first.t;
  return dt > 0 ? (last.x - first.x) / dt : 0;
}

// --- Footage ---------------------------------------------------------------------------------

/**
 * Where playback should start for a requested time: inside footage it's the time itself; in a
 * gap it's the start of the next recording; after the last recording it's shortly before its end.
 */
export function snapToFootage(t: number, segments: readonly Range[]): number {
  if (segments.length === 0) return t;
  for (const s of segments) {
    if (t < s.start) return s.start;
    if (t < s.end) return t;
  }
  const last = segments[segments.length - 1];
  return Math.max(last.start, last.end - 5 * SECOND);
}

/** The event under pixel `x`, widening tiny bars to `minPx` so they stay easy to hit. */
export function eventAtX<T extends Range>(v: TimelineView, x: number, events: readonly T[], minPx = 8): T | undefined {
  const t = xToTime(v, x);
  const slack = (minPx / 2) * msPerPx(v);
  let best: T | undefined;
  let bestDist = Infinity;
  for (const e of events) {
    const mid = (e.start + e.end) / 2;
    const half = Math.max((e.end - e.start) / 2, slack);
    const dist = Math.abs(t - mid);
    if (dist <= half && dist < bestDist) {
      best = e;
      bestDist = dist;
    }
  }
  return best;
}

// --- Keyboard --------------------------------------------------------------------------------

export type TimelineAction =
  | { type: "toggle-play" }
  | { type: "seek-by"; ms: number }
  | { type: "speed"; dir: 1 | -1 }
  | { type: "seek-to"; where: "start" | "end" }
  | { type: "live" };

/** Playback shortcuts: Space, ←/→ (±10 s), Shift+←/→ (±1 min), [ ], Home/End, L. */
export function keyToAction(e: {
  key: string;
  shiftKey?: boolean;
  altKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
}): TimelineAction | null {
  if (e.ctrlKey || e.metaKey || e.altKey) return null;
  switch (e.key) {
    case " ":
    case "Spacebar":
      return { type: "toggle-play" };
    case "ArrowLeft":
      return { type: "seek-by", ms: e.shiftKey ? -MINUTE : -10 * SECOND };
    case "ArrowRight":
      return { type: "seek-by", ms: e.shiftKey ? MINUTE : 10 * SECOND };
    case "[":
      return { type: "speed", dir: -1 };
    case "]":
      return { type: "speed", dir: 1 };
    case "Home":
      return { type: "seek-to", where: "start" };
    case "End":
      return { type: "seek-to", where: "end" };
    case "l":
    case "L":
      return { type: "live" };
    default:
      return null;
  }
}

export const SPEEDS = [0.5, 1, 2, 4, 8, 16] as const;

export function stepSpeed(current: number, dir: 1 | -1): number {
  const i = SPEEDS.findIndex((s) => s >= current);
  const index = i === -1 ? SPEEDS.length - 1 : i;
  return SPEEDS[clamp(index + dir, 0, SPEEDS.length - 1)];
}

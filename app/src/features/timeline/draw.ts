// Canvas rendering for the timeline ruler. Called every animation frame while the view moves,
// so it avoids allocation-heavy work: segments and events are pre-parsed, ticks are cheap.

import type { EventType } from "@/ipc";
import { primaryEventType } from "@/lib/events";
import type { ParsedEvent, ParsedSegment } from "@/queries/recordings";
import { generateTicks, timeToX, viewEnd, viewStart, type Range, type TimelineView } from "./math";
import type { TimelinePalette } from "./palette";

export interface TimelineLayout {
  /** Baseline of the hour labels. */
  labelY: number;
  /** Top of the tick marks. */
  tickTop: number;
  bandTop: number;
  bandHeight: number;
}

export const DEFAULT_LAYOUT: TimelineLayout = { labelY: 15, tickTop: 21, bandTop: 36, bandHeight: 26 };

export interface DrawInput {
  width: number;
  height: number;
  dpr: number;
  view: TimelineView;
  day: Range;
  offsetMinutes: number;
  segments: readonly ParsedSegment[];
  events: readonly ParsedEvent[];
  highlight: ReadonlySet<EventType> | null;
  selectedId: string | null;
  hoverId: string | null;
  nowMs: number | null;
  playheadMs: number | null;
  hoverX: number | null;
  palette: TimelinePalette;
  layout?: TimelineLayout;
  /** Clip editor: shade everything outside this range. */
  selection?: Range | null;
}

const hatchCache = new WeakMap<CanvasRenderingContext2D, { color: string; dpr: number; pattern: CanvasPattern | null }>();

function hatch(ctx: CanvasRenderingContext2D, color: string, dpr: number): CanvasPattern | null {
  const cached = hatchCache.get(ctx);
  if (cached && cached.color === color && cached.dpr === dpr) return cached.pattern;
  const size = Math.round(8 * dpr);
  const tile = document.createElement("canvas");
  tile.width = size;
  tile.height = size;
  const t = tile.getContext("2d");
  let pattern: CanvasPattern | null = null;
  if (t) {
    t.strokeStyle = color;
    t.lineWidth = Math.max(1, dpr);
    t.beginPath();
    t.moveTo(0, size);
    t.lineTo(size, 0);
    t.moveTo(-size / 2, size / 2);
    t.lineTo(size / 2, -size / 2);
    t.moveTo(size / 2, size * 1.5);
    t.lineTo(size * 1.5, size / 2);
    t.stroke();
    pattern = ctx.createPattern(tile, "repeat");
    pattern?.setTransform(new DOMMatrix().scale(1 / dpr));
  }
  hatchCache.set(ctx, { color, dpr, pattern });
  return pattern;
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  const radius = Math.max(0, Math.min(r, w / 2, h / 2));
  ctx.beginPath();
  if (typeof ctx.roundRect === "function") ctx.roundRect(x, y, w, h, radius);
  else ctx.rect(x, y, w, h);
}

/** Snaps a line position to the device pixel grid so 1px lines stay crisp. */
const crisp = (x: number, dpr: number) => (Math.round(x * dpr) + 0.5) / dpr;

export function drawTimeline(ctx: CanvasRenderingContext2D, input: DrawInput): void {
  const { width, height, dpr, view, day, palette } = input;
  const layout = input.layout ?? DEFAULT_LAYOUT;
  const { bandTop, bandHeight } = layout;
  const start = viewStart(view);
  const end = viewEnd(view);
  const x = (t: number) => timeToX(view, t);

  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, height);

  // Outside the selected day.
  ctx.fillStyle = palette.outside;
  if (start < day.start) ctx.fillRect(0, 0, Math.max(0, x(day.start)), height);
  if (end > day.end) {
    const x0 = Math.max(0, x(day.end));
    ctx.fillRect(x0, 0, width - x0, height);
  }

  // The future (today only): hatched, since there's no footage yet.
  if (input.nowMs !== null && input.nowMs < end && input.nowMs < day.end) {
    const x0 = Math.max(0, x(input.nowMs));
    const x1 = Math.min(width, x(day.end));
    if (x1 > x0) {
      const pattern = hatch(ctx, palette.future, dpr);
      ctx.fillStyle = pattern ?? palette.future;
      ctx.fillRect(x0, bandTop, x1 - x0, bandHeight);
    }
  }

  // Recorded footage: a soft band; detection-only recordings are a little stronger.
  for (const s of input.segments) {
    if (s.end < start || s.start > end) continue;
    const x0 = Math.max(-4, x(s.start));
    const x1 = Math.min(width + 4, x(s.end));
    const w = Math.max(1.5, x1 - x0);
    ctx.fillStyle = s.kind === "detection" ? palette.bandStrong : palette.band;
    roundRect(ctx, x0, bandTop, w, bandHeight, 5);
    ctx.fill();
  }

  // Detection events: bars in the event colour.
  const inset = 3;
  let selectedRect: [number, number, number, number] | null = null;
  for (const e of input.events) {
    if (e.end < start || e.start > end) continue;
    const type = primaryEventType(e.types);
    const x0 = x(e.start);
    const w = Math.max(3, x(e.end) - x0);
    const dim = input.highlight !== null && input.highlight.size > 0 && !e.types.some((t) => input.highlight!.has(t));
    ctx.globalAlpha = dim ? 0.18 : e.id === input.hoverId ? 1 : 0.92;
    ctx.fillStyle = palette.events[type];
    roundRect(ctx, x0, bandTop + inset, w, bandHeight - inset * 2, 2.5);
    ctx.fill();
    if (e.id === input.selectedId) selectedRect = [x0, bandTop, w, bandHeight];
  }
  ctx.globalAlpha = 1;

  if (selectedRect) {
    const [x0, y0, w, h] = selectedRect;
    ctx.strokeStyle = palette.playhead;
    ctx.lineWidth = 2;
    roundRect(ctx, x0 - 2, y0 - 1, w + 4, h + 2, 4);
    ctx.stroke();
  }

  // Clip selection: shade outside the chosen range.
  if (input.selection) {
    const sx0 = x(input.selection.start);
    const sx1 = x(input.selection.end);
    ctx.fillStyle = "rgba(0,0,0,0.38)";
    if (sx0 > 0) ctx.fillRect(0, 0, Math.min(width, sx0), height);
    if (sx1 < width) ctx.fillRect(Math.max(0, sx1), 0, width - Math.max(0, sx1), height);
  }

  // Ruler.
  const { ticks } = generateTicks(view, day.start, input.offsetMinutes);
  ctx.beginPath();
  for (const tick of ticks) {
    if (tick.major) continue;
    const tx = crisp(tick.x, dpr);
    ctx.moveTo(tx, layout.tickTop + 4);
    ctx.lineTo(tx, layout.tickTop + 9);
  }
  ctx.strokeStyle = palette.tick;
  ctx.lineWidth = 1;
  ctx.stroke();

  ctx.beginPath();
  for (const tick of ticks) {
    if (!tick.major) continue;
    const tx = crisp(tick.x, dpr);
    ctx.moveTo(tx, layout.tickTop);
    ctx.lineTo(tx, layout.tickTop + 11);
  }
  ctx.strokeStyle = palette.tickMajor;
  ctx.stroke();

  ctx.font = palette.font;
  ctx.textAlign = "center";
  ctx.textBaseline = "alphabetic";
  ctx.fillStyle = palette.label;
  for (const tick of ticks) if (tick.label) ctx.fillText(tick.label, tick.x, layout.labelY);

  // "Now" marker for today.
  if (input.nowMs !== null && input.nowMs >= start && input.nowMs <= end) {
    const nx = crisp(x(input.nowMs), dpr);
    ctx.strokeStyle = palette.now;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(nx, bandTop - 5);
    ctx.lineTo(nx, bandTop + bandHeight + 5);
    ctx.stroke();
    ctx.fillStyle = palette.now;
    ctx.beginPath();
    ctx.arc(nx, bandTop - 6, 3, 0, Math.PI * 2);
    ctx.fill();
  }

  // Hover guide.
  if (input.hoverX !== null) {
    const hx = crisp(input.hoverX, dpr);
    ctx.strokeStyle = palette.hover;
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.moveTo(hx, layout.tickTop);
    ctx.lineTo(hx, height);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  // Playhead.
  if (input.playheadMs !== null) {
    const px = x(input.playheadMs);
    if (px >= -2 && px <= width + 2) {
      ctx.strokeStyle = palette.playhead;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(px, 0);
      ctx.lineTo(px, height);
      ctx.stroke();
      ctx.fillStyle = palette.playhead;
      ctx.beginPath();
      ctx.moveTo(px - 5, height);
      ctx.lineTo(px + 5, height);
      ctx.lineTo(px, height - 6);
      ctx.closePath();
      ctx.fill();
    }
  }
}

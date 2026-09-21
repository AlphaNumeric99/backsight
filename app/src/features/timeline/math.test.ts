import { describe, expect, it } from "vitest";
import { DAY, HOUR, MINUTE, SECOND, dayStartMs } from "@/lib/time";
import {
  MAX_SPAN_MS,
  MIN_SPAN_MS,
  chooseTickSteps,
  coastDistance,
  decayVelocity,
  eventAtX,
  generateTicks,
  keyToAction,
  msPerPx,
  panByPx,
  releaseVelocity,
  snapToFootage,
  stepSpeed,
  timeToX,
  viewEnd,
  viewStart,
  xToTime,
  zoomAroundTime,
  zoomAt,
  type TimelineView,
} from "./math";

const DAY0 = dayStartMs("2026-09-29", 120); // camera-local midnight at UTC+02:00
const view = (over: Partial<TimelineView> = {}): TimelineView => ({
  centerMs: DAY0 + 12 * HOUR,
  spanMs: 2 * HOUR,
  widthPx: 1200,
  ...over,
});

describe("time ↔ x", () => {
  it("maps the view edges to 0 and the width", () => {
    const v = view();
    expect(timeToX(v, viewStart(v))).toBe(0);
    expect(timeToX(v, viewEnd(v))).toBe(1200);
    expect(timeToX(v, v.centerMs)).toBe(600);
  });

  it("round-trips", () => {
    const v = view({ spanMs: 37 * MINUTE, widthPx: 977 });
    for (const x of [0, 1, 250.5, 488, 976]) expect(timeToX(v, xToTime(v, x))).toBeCloseTo(x, 6);
    for (const t of [v.centerMs - 10 * MINUTE, v.centerMs, v.centerMs + 3 * SECOND]) {
      expect(xToTime(v, timeToX(v, t))).toBeCloseTo(t, 3);
    }
  });

  it("reports resolution", () => {
    expect(msPerPx(view())).toBe((2 * HOUR) / 1200);
  });
});

describe("zoom anchoring", () => {
  it("keeps the time under the cursor at the same pixel", () => {
    const v = view();
    for (const anchorX of [0, 173, 600, 1111, 1200]) {
      const before = xToTime(v, anchorX);
      for (const factor of [0.5, 0.8, 1.25, 2]) {
        const z = zoomAt(v, anchorX, factor);
        expect(z.spanMs).toBeCloseTo(v.spanMs * factor, 3);
        expect(xToTime(z, anchorX)).toBeCloseTo(before, 3);
      }
    }
  });

  it("zooming at the centre keeps the centre", () => {
    const z = zoomAt(view(), 600, 0.5);
    expect(z.centerMs).toBeCloseTo(view().centerMs, 6);
  });

  it("clamps between five minutes and a whole day", () => {
    expect(zoomAt(view(), 300, 1e-6).spanMs).toBe(MIN_SPAN_MS);
    expect(zoomAt(view(), 300, 1e6).spanMs).toBe(MAX_SPAN_MS);
    expect(MIN_SPAN_MS).toBe(5 * MINUTE);
    expect(MAX_SPAN_MS).toBe(DAY);
  });

  it("anchors on a time, e.g. the playhead", () => {
    const v = view();
    const playhead = v.centerMs - 20 * MINUTE;
    const x = timeToX(v, playhead);
    const z = zoomAroundTime(v, playhead, 0.25);
    expect(timeToX(z, playhead)).toBeCloseTo(x, 6);
  });

  it("keeps the centre inside the day", () => {
    const bounds = { start: DAY0, end: DAY0 + DAY };
    const v = view({ centerMs: DAY0 + 10 * MINUTE, spanMs: 30 * MINUTE });
    const z = zoomAt(v, 0, 8, { bounds });
    expect(z.centerMs).toBeGreaterThanOrEqual(bounds.start);
  });
});

describe("panning", () => {
  it("drags content with the pointer", () => {
    const v = view();
    const p = panByPx(v, 120);
    expect(p.centerMs).toBeCloseTo(v.centerMs - 120 * msPerPx(v), 6);
    expect(p.spanMs).toBe(v.spanMs);
  });

  it("stops at the day's edges", () => {
    const bounds = { start: DAY0, end: DAY0 + DAY };
    expect(panByPx(view(), 1e9, bounds).centerMs).toBe(bounds.start);
    expect(panByPx(view(), -1e9, bounds).centerMs).toBe(bounds.end);
  });
});

describe("tick generation", () => {
  it("chooses finer steps as you zoom in", () => {
    const wide = chooseTickSteps(DAY / 1200);
    const narrow = chooseTickSteps((5 * MINUTE) / 1200);
    expect(wide.minor).toBeGreaterThan(narrow.minor);
    expect(wide.label).toBeGreaterThan(narrow.label);
    expect(wide.label % wide.minor).toBe(0);
    expect(narrow.label % narrow.minor).toBe(0);
  });

  it("labels hours in camera-local time for the whole day", () => {
    const v: TimelineView = { centerMs: DAY0 + 12 * HOUR, spanMs: DAY, widthPx: 1200 };
    const { ticks, label } = generateTicks(v, DAY0, 120);
    expect(label).toBe(2 * HOUR);
    const labels = ticks.filter((t) => t.major).map((t) => t.label);
    expect(labels[0]).toBe("00:00");
    expect(labels).toContain("12:00");
    expect(labels.at(-1)).toBe("24:00");
    // Aligned: every labelled tick sits on a whole local hour.
    for (const t of ticks.filter((t) => t.major)) expect((t.t - DAY0) % HOUR).toBe(0);
  });

  it("aligns to local midnight with half-hour offsets", () => {
    const origin = dayStartMs("2026-09-29", 330); // UTC+05:30
    const v: TimelineView = { centerMs: origin + 9 * HOUR + 7 * MINUTE, spanMs: 30 * MINUTE, widthPx: 1000 };
    const { ticks, minor } = generateTicks(v, origin, 330);
    for (const t of ticks) expect((t.t - origin) % minor).toBe(0);
    expect(ticks.find((t) => t.major)?.label).toMatch(/^0[89]:\d\d$/);
  });

  it("keeps ticks inside the view, evenly spaced and at readable density", () => {
    for (const span of [5 * MINUTE, 17 * MINUTE, 2 * HOUR, 9 * HOUR, DAY]) {
      const v = view({ spanMs: span, widthPx: 960 });
      const { ticks, minor } = generateTicks(v, DAY0, 120);
      expect(ticks.length).toBeGreaterThan(3);
      expect(ticks.length).toBeLessThan(200);
      for (const t of ticks) {
        expect(t.x).toBeGreaterThanOrEqual(-0.001);
        expect(t.x).toBeLessThanOrEqual(960.001);
      }
      for (let i = 1; i < ticks.length; i++) expect(ticks[i].t - ticks[i - 1].t).toBe(minor);
      const labelled = ticks.filter((t) => t.major);
      for (let i = 1; i < labelled.length; i++) expect(labelled[i].x - labelled[i - 1].x).toBeGreaterThanOrEqual(76 - 1e-6);
    }
  });

  it("uses one-minute labels at the closest zoom", () => {
    const { label, minor } = generateTicks(view({ spanMs: 5 * MINUTE, widthPx: 1200 }), DAY0, 120);
    expect(label).toBe(MINUTE);
    expect(minor).toBeLessThanOrEqual(30 * SECOND);
  });
});

describe("inertia", () => {
  it("decays exponentially and coasts a bounded distance", () => {
    expect(decayVelocity(1, 0)).toBe(1);
    expect(decayVelocity(1, 325)).toBeCloseTo(Math.exp(-1), 6);
    expect(decayVelocity(-2, 100)).toBeLessThan(0);
    let v = 1.2;
    let travelled = 0;
    for (let i = 0; i < 400; i++) {
      travelled += v * 16;
      v = decayVelocity(v, 16);
    }
    // A per-frame sum slightly overshoots the continuous integral.
    expect(Math.abs(travelled - coastDistance(1.2)) / coastDistance(1.2)).toBeLessThan(0.03);
  });

  it("measures release velocity from recent samples only", () => {
    const samples = [
      { x: 0, t: 0 },
      { x: 500, t: 10 }, // stale burst
      { x: 510, t: 200 },
      { x: 530, t: 220 },
      { x: 560, t: 240 },
    ];
    expect(releaseVelocity(samples)).toBeCloseTo(50 / 40, 6);
    expect(releaseVelocity([{ x: 1, t: 1 }])).toBe(0);
  });
});

describe("footage helpers", () => {
  const segs = [
    { start: 100, end: 200 },
    { start: 300, end: 400_000 },
  ];
  it("snaps seeks out of gaps", () => {
    expect(snapToFootage(150, segs)).toBe(150);
    expect(snapToFootage(250, segs)).toBe(300);
    expect(snapToFootage(10, segs)).toBe(100);
    expect(snapToFootage(1e9, segs)).toBe(400_000 - 5 * SECOND);
    expect(snapToFootage(42, [])).toBe(42);
  });

  it("hit-tests events, widening tiny bars", () => {
    const v: TimelineView = { centerMs: 1000 * SECOND, spanMs: 1000 * SECOND, widthPx: 1000 };
    const e = { start: 1000 * SECOND, end: 1000 * SECOND + 100 }; // far thinner than a pixel
    expect(eventAtX(v, 500, [e])).toBe(e);
    expect(eventAtX(v, 503, [e])).toBe(e);
    expect(eventAtX(v, 520, [e])).toBeUndefined();
  });
});

describe("keyboard", () => {
  it("maps the playback shortcuts", () => {
    expect(keyToAction({ key: " " })).toEqual({ type: "toggle-play" });
    expect(keyToAction({ key: "ArrowLeft" })).toEqual({ type: "seek-by", ms: -10 * SECOND });
    expect(keyToAction({ key: "ArrowRight" })).toEqual({ type: "seek-by", ms: 10 * SECOND });
    expect(keyToAction({ key: "ArrowLeft", shiftKey: true })).toEqual({ type: "seek-by", ms: -MINUTE });
    expect(keyToAction({ key: "ArrowRight", shiftKey: true })).toEqual({ type: "seek-by", ms: MINUTE });
    expect(keyToAction({ key: "[" })).toEqual({ type: "speed", dir: -1 });
    expect(keyToAction({ key: "]" })).toEqual({ type: "speed", dir: 1 });
    expect(keyToAction({ key: "Home" })).toEqual({ type: "seek-to", where: "start" });
    expect(keyToAction({ key: "End" })).toEqual({ type: "seek-to", where: "end" });
    expect(keyToAction({ key: "l" })).toEqual({ type: "live" });
    expect(keyToAction({ key: "L", shiftKey: true })).toEqual({ type: "live" });
  });

  it("ignores modified keys and others", () => {
    expect(keyToAction({ key: "ArrowLeft", ctrlKey: true })).toBeNull();
    expect(keyToAction({ key: "l", metaKey: true })).toBeNull();
    expect(keyToAction({ key: "x" })).toBeNull();
  });

  it("steps through the supported speeds", () => {
    expect(stepSpeed(1, 1)).toBe(2);
    expect(stepSpeed(1, -1)).toBe(0.5);
    expect(stepSpeed(0.5, -1)).toBe(0.5);
    expect(stepSpeed(16, 1)).toBe(16);
    expect(stepSpeed(3, 1)).toBe(8);
  });
});

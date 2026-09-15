import { describe, expect, it } from "vitest";
import { LIVE_PLAYOUT, PLAYBACK_PLAYOUT, PlayoutClock, SlidingExtreme } from "./clock";

const T0 = 1_790_000_000_000_000; // µs since the epoch
const FRAME_US = 1_000_000 / 15;
const FRAME_MS = 1000 / 15;
const ts = (i: number) => T0 + Math.round(i * FRAME_US);

/** Feeds frames 0..n-1 arriving at `arrival(i)`; returns the last frame index + 1. */
function feed(clock: PlayoutClock, from: number, to: number, arrival: (i: number) => number): number[] {
  const lateness: number[] = [];
  for (let i = from; i < to; i++) lateness.push(clock.observe(ts(i), arrival(i)));
  return lateness;
}

describe("SlidingExtreme", () => {
  it("tracks the minimum over a window", () => {
    const min = new SlidingExtreme("min", 1000);
    expect(min.value).toBeNaN();
    min.push(0, 5);
    min.push(100, 3);
    min.push(200, 4);
    expect(min.value).toBe(3);
    min.push(1150, 9); // the 3 (t=100) expired
    expect(min.value).toBe(4);
    min.push(1300, 10); // the 4 (t=200) expired
    expect(min.value).toBe(9);
    min.clear();
    expect(min.value).toBeNaN();
  });

  it("tracks the maximum and keeps the newest sample", () => {
    const max = new SlidingExtreme("max", 100);
    max.push(0, 1);
    max.push(10, 7);
    max.push(20, 2);
    expect(max.value).toBe(7);
    max.push(5000, 0);
    expect(max.value).toBe(0);
    for (let t = 5000; t < 20000; t += 10) max.push(t, t % 7);
    expect(max.value).toBe(6);
  });
});

describe("PlayoutClock", () => {
  it("is not started before the first frame", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    expect(clock.started).toBe(false);
    expect(clock.mediaTimeUs(1000)).toBeNaN();
  });

  it("shows the first frame the minimum delay after it arrives", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    expect(clock.observe(ts(0), 1000)).toBe(-150);
    expect(clock.dueAtMs(ts(0))).toBe(1150);
    expect(clock.mediaTimeUs(1150)).toBe(ts(0));
  });

  it("keeps a steady stream at the minimum delay", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    const late = feed(clock, 0, 150, (i) => 1000 + i * FRAME_MS);
    for (const l of late) expect(l).toBeCloseTo(-150, 3);
    expect(clock.delayMs).toBe(150);
    for (const i of [10, 75, 149]) {
      expect(clock.mediaTimeUs(1000 + i * FRAME_MS + 150)).toBeCloseTo(ts(i), -1);
    }
  });

  it("raises the delay with jitter, up to the maximum", () => {
    const moderate = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(moderate, 0, 60, (i) => 1000 + i * FRAME_MS + (i % 2 ? 200 : 0));
    expect(moderate.delayMs).toBeCloseTo(230, 2); // 200 ms jitter + 30 ms margin

    const heavy = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(heavy, 0, 60, (i) => 1000 + i * FRAME_MS + (i % 3 ? 0 : 900));
    expect(heavy.delayMs).toBe(300);
  });

  it("lowers the delay again once the jitter leaves the window", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(clock, 0, 30, (i) => 1000 + i * FRAME_MS + (i % 2 ? 250 : 0));
    expect(clock.delayMs).toBeCloseTo(280, 2);
    feed(clock, 30, 30 + 11 * 15, (i) => 1000 + i * FRAME_MS);
    expect(clock.delayMs).toBe(150);
  });

  it("slews gently toward a larger delay", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(clock, 0, 30, (i) => 1000 + i * FRAME_MS);
    let now = 1000 + 30 * FRAME_MS;
    const before = clock.mediaTimeUs(now);
    // One frame arrives 250 ms late: the target rises from 150 to 280 ms.
    clock.observe(ts(30), now + 250);
    expect(clock.delayMs).toBeCloseTo(280, 2);
    for (let step = 0; step < 60; step++) {
      now += 1000 / 60;
      clock.mediaTimeUs(now);
    }
    // 1 s of wall time moved the media clock by 1 s minus the 5 % slew.
    expect(clock.mediaTimeUs(now) - before).toBeCloseTo(950_000, -3);
  });

  it("jumps when the offset changes a lot", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(clock, 0, 10, (i) => 1000 + i * FRAME_MS);
    const now = 1000 + 10 * FRAME_MS;
    const before = clock.mediaTimeUs(now);
    // The stream suddenly arrives 2 s "earlier" (e.g. the camera clock stepped): snap forward.
    clock.observe(ts(10) + 2_000_000, now);
    expect(clock.mediaTimeUs(now + 1) - before).toBeCloseTo(2_001_000, -3);
  });

  it("scales stream time by the playback speed", () => {
    const clock = new PlayoutClock({ speed: 4, ...PLAYBACK_PLAYOUT });
    // At 4×, frames 66.7 ms of media apart arrive 16.7 ms apart.
    const late = feed(clock, 0, 60, (i) => 5000 + (i * FRAME_MS) / 4);
    for (const l of late) expect(l).toBeCloseTo(-500, 3);
    const a = clock.mediaTimeUs(5600);
    const b = clock.mediaTimeUs(5700);
    expect(b - a).toBeCloseTo(400_000, 3);
    expect(clock.dueAtMs(ts(40))).toBeCloseTo(5000 + (40 * FRAME_MS) / 4 + 500, 3);
  });

  it("plays out a burst after a rebuffer in playback instead of skipping it", () => {
    // Frames 0-29 arrive on time; the player runs dry and re-anchors; then frames 30-74 (3 s)
    // arrive at once.
    const burstAt = 1000 + 75 * FRAME_MS;
    const playback = new PlayoutClock({ speed: 1, ...PLAYBACK_PLAYOUT });
    feed(playback, 0, 30, (i) => 1000 + i * FRAME_MS);
    playback.reanchor();
    feed(playback, 30, 75, () => burstAt);
    // Frame 30 is due half a second after the burst and the rest follow at the normal pace.
    expect(playback.dueAtMs(ts(30))).toBeCloseTo(burstAt + 500, 3);
    expect(playback.dueAtMs(ts(74))).toBeCloseTo(burstAt + 500 + 44 * FRAME_MS, 0);
    expect(playback.mediaTimeUs(burstAt + 500)).toBeCloseTo(ts(30), -1);

    // An adaptive clock would jump to the newest frame of the burst, skipping 3 s.
    const adaptive = new PlayoutClock({ speed: 1, ...PLAYBACK_PLAYOUT, adaptive: true });
    feed(adaptive, 0, 30, (i) => 1000 + i * FRAME_MS);
    adaptive.reanchor();
    feed(adaptive, 30, 75, () => burstAt);
    expect(adaptive.mediaTimeUs(burstAt + 500)).toBeCloseTo(ts(74), -1);
  });

  it("keeps live at the live edge when late frames arrive in a burst", () => {
    const burstAt = 1000 + 75 * FRAME_MS;
    const live = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(live, 0, 75, (i) => (i < 30 ? 1000 + i * FRAME_MS : burstAt));
    // The burst counts as jitter (the delay goes to its 300 ms cap); the clock stays at the
    // live edge minus that delay, so most of the late frames are simply skipped.
    expect(live.delayMs).toBe(300);
    expect(live.mediaTimeUs(burstAt)).toBeCloseTo(ts(75) - 300_000, -2);
  });

  it("re-anchors after a stall", () => {
    const clock = new PlayoutClock({ speed: 1, ...PLAYBACK_PLAYOUT });
    feed(clock, 0, 30, (i) => 1000 + i * FRAME_MS);
    clock.reanchor();
    expect(clock.started).toBe(false);
    expect(clock.mediaTimeUs(9000)).toBeNaN();
    // The stream resumes 5 s late; playout restarts half a second after the next frame.
    expect(clock.observe(ts(30), 1000 + 30 * FRAME_MS + 5000)).toBe(-500);
    expect(clock.mediaTimeUs(1000 + 30 * FRAME_MS + 5500)).toBeCloseTo(ts(30), -1);
  });

  it("starts over on reset", () => {
    const clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
    feed(clock, 0, 5, (i) => i * FRAME_MS);
    clock.reset();
    expect(clock.observe(T0 + 60_000_000, 100)).toBe(-150);
    expect(clock.mediaTimeUs(250)).toBe(T0 + 60_000_000);
  });

  it("rejects a non-positive speed", () => {
    expect(() => new PlayoutClock({ speed: 0, ...LIVE_PLAYOUT })).toThrow(RangeError);
  });
});

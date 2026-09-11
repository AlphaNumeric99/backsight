import { describe, expect, it } from "vitest";
import { AudioSync } from "./audioSync";
import type { PcmPlayerStatus } from "./pcm";

const T0 = 1_790_000_000_000_000;

function report(overrides: Partial<PcmPlayerStatus>): PcmPlayerStatus {
  return {
    playing: true,
    positionUs: T0,
    bufferStartUs: T0,
    bufferEndUs: T0 + 300_000,
    starving: false,
    ...overrides,
  };
}

describe("AudioSync", () => {
  it("starts playback at the wanted time plus the output latency", () => {
    const sync = new AudioSync();
    sync.latencyUs = 40_000;
    const cmd = sync.onReport(report({ playing: false }), 1000, T0 + 100_000);
    expect(cmd).toEqual({ type: "sync", positionUs: T0 + 140_000 });
    expect(sync.isMaster).toBe(false);
  });

  it("waits while the buffer does not reach the wanted time", () => {
    const sync = new AudioSync();
    expect(sync.onReport(report({ playing: false, bufferEndUs: T0 + 10_000 }), 1000, T0 + 100_000)).toBeUndefined();
    expect(
      sync.onReport(report({ playing: false, bufferStartUs: NaN, bufferEndUs: NaN }), 1000, T0),
    ).toBeUndefined();
  });

  it("follows the audio clock and slows audio that is ahead", () => {
    const sync = new AudioSync();
    // Audio is 50 ms ahead of the playout clock.
    const cmd = sync.onReport(report({ positionUs: T0 + 50_000 }), 1000, T0);
    expect(cmd?.type).toBe("rate");
    expect((cmd as { rate: number }).rate).toBeCloseTo(0.99, 6);
    expect(sync.isMaster).toBe(true);
    expect(sync.errorUs).toBe(50_000);
    expect(sync.clockUs(1000)).toBe(T0 + 50_000);
    expect(sync.clockUs(1100)).toBe(T0 + 150_000);
  });

  it("speeds up audio that is behind, within the limit", () => {
    const sync = new AudioSync();
    const cmd = sync.onReport(report({ positionUs: T0 - 140_000 }), 1000, T0);
    expect(cmd).toEqual({ type: "rate", rate: 1.02 });
  });

  it("leaves a small error alone", () => {
    const sync = new AudioSync();
    expect(sync.onReport(report({ positionUs: T0 + 3_000 }), 1000, T0)).toBeUndefined();
    expect(sync.isMaster).toBe(true);
  });

  it("returns to the normal rate once in sync", () => {
    const sync = new AudioSync();
    sync.onReport(report({ positionUs: T0 + 50_000 }), 1000, T0);
    expect(sync.onReport(report({ positionUs: T0 + 1_000 }), 1025, T0)).toEqual({ type: "rate", rate: 1 });
  });

  it("jumps when the error is large", () => {
    const sync = new AudioSync();
    const cmd = sync.onReport(report({ positionUs: T0 + 400_000, bufferEndUs: T0 + 900_000 }), 1000, T0 + 100_000);
    expect(cmd).toEqual({ type: "sync", positionUs: T0 + 100_000 });
    expect(sync.isMaster).toBe(false);
    expect(sync.clockUs(1000)).toBeNaN();
  });

  it("flushes audio that is all older than the wanted time", () => {
    const sync = new AudioSync();
    const cmd = sync.onReport(report({ positionUs: T0, bufferEndUs: T0 + 100_000 }), 1000, T0 + 2_000_000);
    expect(cmd).toEqual({ type: "flush" });
  });

  it("keeps audio that is far ahead and waits", () => {
    const sync = new AudioSync();
    const ahead = report({ positionUs: T0 + 3_000_000, bufferStartUs: T0 + 3_000_000, bufferEndUs: T0 + 3_300_000 });
    expect(sync.onReport(ahead, 1000, T0)).toBeUndefined();
  });

  it("is not a clock while starving, stale or without a wanted time", () => {
    const sync = new AudioSync();
    sync.onReport(report({ starving: true }), 1000, T0);
    expect(sync.clockUs(1000)).toBeNaN();

    sync.onReport(report({}), 2000, T0);
    expect(sync.clockUs(2000)).toBe(T0);
    expect(sync.clockUs(2300)).toBeNaN(); // no report for 300 ms

    sync.onReport(report({}), 3000, NaN);
    expect(sync.isMaster).toBe(false);
  });

  it("forgets everything on reset", () => {
    const sync = new AudioSync();
    sync.onReport(report({ positionUs: T0 + 50_000 }), 1000, T0);
    sync.reset();
    expect(sync.isMaster).toBe(false);
    expect(sync.errorUs).toBeNaN();
    expect(sync.clockUs(1000)).toBeNaN();
  });
});

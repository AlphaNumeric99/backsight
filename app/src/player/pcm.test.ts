import { describe, expect, it } from "vitest";
import { PcmPlayer, s16ToFloat32 } from "./pcm";

const T0 = 1_790_000_000_000_000;

function ramp(n: number, start = 0, step = 0.001): Float32Array {
  return Float32Array.from({ length: n }, (_, i) => start + i * step);
}

function render(player: PcmPlayer, frames: number, channels = 1): Float32Array[] {
  const out = Array.from({ length: channels }, () => new Float32Array(frames).fill(NaN));
  player.render(out);
  return out;
}

describe("s16ToFloat32", () => {
  it("scales to [-1, 1)", () => {
    expect(Array.from(s16ToFloat32(Int16Array.from([0, 16384, -16384, -32768, 32767])))).toEqual([
      0,
      0.5,
      -0.5,
      -1,
      32767 / 32768,
    ]);
  });
});

describe("PcmPlayer", () => {
  it("is silent until synced", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(256, 0.1));
    expect(Array.from(render(p, 4)[0])).toEqual([0, 0, 0, 0]);
    expect(p.status()).toMatchObject({ playing: false, bufferStartUs: T0, bufferEndUs: T0 + 32_000 });
  });

  it("plays samples in order at the same rate", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    const input = ramp(256);
    p.write(T0, input);
    p.sync(T0);
    const out = render(p, 128)[0];
    expect(out).toEqual(input.subarray(0, 128));
    expect(p.status().positionUs).toBe(T0 + 16_000);
  });

  it("starts at the synced position", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(800));
    p.sync(T0 + 50_000); // sample 400
    expect(render(p, 1)[0][0]).toBeCloseTo(0.4, 6);
  });

  it("resamples 8 kHz to 48 kHz by interpolation", () => {
    const p = new PcmPlayer(48000);
    p.configure(8000, 1);
    p.write(T0, ramp(100));
    p.sync(T0);
    const out = render(p, 48)[0];
    for (let j = 0; j < 48; j++) expect(out[j]).toBeCloseTo((j / 6) * 0.001, 6);
    expect(p.status().positionUs).toBeCloseTo(T0 + 1000, 3); // 1 ms consumed
  });

  it("plays faster or slower with the rate", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(8000));
    p.sync(T0);
    p.setRate(1.02);
    render(p, 1000);
    expect(p.status().positionUs).toBeCloseTo(T0 + 127_500, 0); // 1020 samples
    p.setRate(5);
    render(p, 10);
    expect(p.status().positionUs).toBeCloseTo(T0 + 127_500 + 2500, 0); // clamped to 2×
  });

  it("appends packets with small timestamp jitter back to back", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(160));
    p.write(T0 + 20_000 + 4_000, ramp(160)); // 4 ms late: within tolerance
    expect(p.status().bufferEndUs).toBe(T0 + 40_000);
  });

  it("fills a gap with silence", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(160, 0.5, 0)); // 20 ms of 0.5
    p.write(T0 + 60_000, ramp(160, 0.25, 0)); // 40 ms gap
    expect(p.status().bufferEndUs).toBe(T0 + 80_000);
    p.sync(T0 + 10_000);
    const out = render(p, 480)[0];
    expect(out[0]).toBeCloseTo(0.5, 6);
    expect(out[79]).toBeCloseTo(0.5, 6); // last sample of the first packet
    expect(out[80]).toBe(0);
    expect(out[399]).toBe(0);
    expect(out[400]).toBeCloseTo(0.25, 6);
  });

  it("trims an overlap", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(320, 0.5, 0)); // 40 ms
    p.write(T0, ramp(160, 0.9, 0)); // complete repeat: ignored
    p.write(T0 + 0, ramp(480, 0.25, 0)); // 20 ms beyond the end are new
    expect(p.status().bufferEndUs).toBe(T0 + 60_000);
    p.sync(T0 + 30_000);
    const out = render(p, 160)[0];
    expect(out[0]).toBeCloseTo(0.5, 6);
    expect(out[80]).toBeCloseTo(0.25, 6);
  });

  it("restarts on a large timestamp jump", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(160));
    p.sync(T0);
    p.write(T0 + 5_000_000, ramp(160));
    expect(p.status()).toMatchObject({ playing: false, bufferStartUs: T0 + 5_000_000, bufferEndUs: T0 + 5_020_000 });
  });

  it("holds its position while starving", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0, ramp(100, 0.1, 0));
    p.sync(T0);
    const out = render(p, 128)[0];
    expect(out[98]).toBeCloseTo(0.1, 6);
    expect(out[127]).toBe(0);
    const status = p.status();
    expect(status.starving).toBe(true);
    expect(status.positionUs).toBe(T0 + 99 * 125); // waits at the last frame
    p.write(T0 + 12_500, ramp(100, 0.2, 0));
    render(p, 10);
    expect(p.status().starving).toBe(false);
  });

  it("plays silence before the buffered audio when synced early", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1);
    p.write(T0 + 10_000, ramp(160, 0.3, 0));
    p.sync(T0); // 10 ms (80 samples) before the first sample
    const out = render(p, 100)[0];
    expect(out[79]).toBe(0);
    expect(out[80]).toBeCloseTo(0.3, 6);
    expect(p.status().positionUs).toBe(T0 + 12_500);
  });

  it("handles interleaved stereo and upmixes mono", () => {
    const stereo = new PcmPlayer(8000);
    stereo.configure(8000, 2);
    stereo.write(T0, Float32Array.from([0.1, -0.1, 0.2, -0.2, 0.3, -0.3]));
    stereo.sync(T0);
    const [l, r] = render(stereo, 2, 2);
    expect(Array.from(l)).toEqual([0.1, 0.2].map(Math.fround));
    expect(Array.from(r)).toEqual([-0.1, -0.2].map(Math.fround));

    const mono = new PcmPlayer(8000);
    mono.configure(8000, 1);
    mono.write(T0, Float32Array.from([0.5, 0.5, 0.5]));
    mono.sync(T0);
    const [a, b] = render(mono, 2, 2);
    expect(Array.from(a)).toEqual([0.5, 0.5]);
    expect(Array.from(b)).toEqual([0.5, 0.5]);
  });

  it("keeps only the newest audio when overfilled", () => {
    const p = new PcmPlayer(8000);
    p.configure(8000, 1, 0.1); // 800 samples
    p.write(T0, ramp(400, 0.1, 0));
    p.sync(T0);
    p.write(T0 + 50_000, ramp(800, 0.2, 0)); // 400 unread samples are overwritten
    expect(p.status().positionUs).toBe(T0 + 50_000);
    expect(render(p, 1)[0][0]).toBeCloseTo(0.2, 6);
  });

  it("ignores writes before it is configured and empties on flush", () => {
    const p = new PcmPlayer(8000);
    p.write(T0, ramp(10));
    expect(p.status().bufferEndUs).toBeNaN();
    p.configure(8000, 1);
    p.write(T0, ramp(10));
    p.sync(T0);
    p.flush();
    expect(p.status()).toMatchObject({ playing: false, positionUs: NaN });
  });
});

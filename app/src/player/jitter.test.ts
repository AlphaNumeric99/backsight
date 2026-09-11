import { describe, expect, it } from "vitest";
import { JitterBuffer, type EncodedFrame } from "./jitter";

const FRAME = 66_667; // µs at 15 fps

interface TestDecoded {
  tsUs: number;
  closed: boolean;
  close(): void;
}

function decoded(tsUs: number): TestDecoded {
  return {
    tsUs,
    closed: false,
    close() {
      this.closed = true;
    },
  };
}

/** Frames 0..n-1, a keyframe every `gop` frames. */
function frames(n: number, gop = 30, bytes = 1000): EncodedFrame[] {
  return Array.from({ length: n }, (_, i) => ({ tsUs: i * FRAME, keyframe: i % gop === 0, byteLength: bytes }));
}

function buffer(speed = 1, options = {}) {
  return new JitterBuffer<EncodedFrame, TestDecoded>({ speed, ...options });
}

describe("JitterBuffer", () => {
  it("drops delta frames until the first keyframe", () => {
    const jb = buffer();
    const [key, ...deltas] = frames(3, 2);
    expect(jb.push(deltas[0])).toBe(false);
    expect(jb.waitingForKeyframe).toBe(true);
    expect(jb.push(frames(3, 2)[2])).toBe(true); // index 2 is a keyframe (gop 2)
    expect(jb.push(key)).toBe(true);
    expect(jb.dropped).toBe(1);
  });

  it("lets the first frames through before the clock starts", () => {
    const jb = buffer(1, { maxDecodeAhead: 4 });
    for (const f of frames(10)) jb.push(f);
    expect(jb.takeDecodable(NaN, 0).map((f) => f.tsUs)).toEqual([0, FRAME, 2 * FRAME, 3 * FRAME]);
  });

  it("decodes only frames within the lead time", () => {
    const jb = buffer(1, { decodeLeadMs: 150, maxDecodeAhead: 100 });
    for (const f of frames(10)) jb.push(f);
    // Clock at frame 1: frames up to 1 + 150 ms (frame 3) are decodable.
    expect(jb.takeDecodable(FRAME, 0).map((f) => f.tsUs / FRAME)).toEqual([0, 1, 2, 3]);
    expect(jb.takeDecodable(FRAME, 0)).toEqual([]);
    expect(jb.encoded).toHaveLength(6);
  });

  it("bounds frames decoded ahead, counting the decoder queue", () => {
    const jb = buffer(1, { maxDecodeAhead: 5, decodeLeadMs: 10_000 });
    for (const f of frames(20)) jb.push(f);
    jb.addDecoded(decoded(0));
    jb.addDecoded(decoded(FRAME));
    expect(jb.takeDecodable(0, 2)).toHaveLength(1);
    expect(jb.takeDecodable(0, 3)).toHaveLength(0);
  });

  it("shows the newest due frame and drops the ones it supersedes", () => {
    const jb = buffer();
    const out = [0, 1, 2, 3].map((i) => decoded(i * FRAME));
    for (const d of out) jb.addDecoded(d);
    expect(jb.takeDue(NaN)).toBeUndefined();
    expect(jb.takeDue(-1)).toBeUndefined();
    expect(jb.takeDue(2 * FRAME + 10)).toBe(out[2]);
    expect(out.map((d) => d.closed)).toEqual([true, true, false, false]);
    expect(jb.dropped).toBe(2);
    expect(jb.takeDue(2 * FRAME + 20)).toBeUndefined();
    expect(jb.decoded).toEqual([out[3]]);
  });

  it("counts superseded frames as skipped in fast playback", () => {
    const jb = buffer(8);
    for (let i = 0; i < 4; i++) jb.addDecoded(decoded(i * FRAME));
    jb.takeDue(3 * FRAME);
    expect(jb.skipped).toBe(3);
    expect(jb.dropped).toBe(0);
  });

  it("keeps decoded frames in presentation order", () => {
    const jb = buffer();
    for (const t of [0, 2, 1, 3]) jb.addDecoded(decoded(t * FRAME));
    expect(jb.decoded.map((d) => d.tsUs / FRAME)).toEqual([0, 1, 2, 3]);
  });

  it("gives the first picture regardless of the clock", () => {
    const jb = buffer();
    const first = decoded(5 * FRAME);
    jb.addDecoded(first);
    expect(jb.takeFirst()).toBe(first);
    expect(jb.takeFirst()).toBeUndefined();
  });

  it("restarts at the next keyframe", () => {
    const jb = buffer();
    for (const f of frames(40)) jb.push(f); // keyframes at 0 and 30
    jb.takeDecodable(NaN, 0); // frames 0..5 go to the decoder
    jb.requireKeyframe();
    expect(jb.encoded[0].tsUs).toBe(30 * FRAME);
    expect(jb.dropped).toBe(24);

    jb.requireKeyframe(); // frame 30 is a keyframe: nothing to drop
    expect(jb.encoded[0].tsUs).toBe(30 * FRAME);
  });

  it("waits for a new keyframe when none is queued", () => {
    const jb = buffer();
    for (const f of frames(10)) jb.push(f);
    jb.takeDecodable(NaN, 0);
    jb.requireKeyframe();
    expect(jb.encoded).toEqual([]);
    expect(jb.waitingForKeyframe).toBe(true);
    expect(jb.push({ tsUs: 10 * FRAME, keyframe: false, byteLength: 1 })).toBe(false);
    expect(jb.takeDecodable(NaN, 0)).toEqual([]);
  });

  it("drops the oldest group of pictures beyond the queue span", () => {
    const jb = buffer(1, { maxQueueMs: 3000 });
    for (const f of frames(61)) jb.push(f); // 4 s: keyframes at 0, 30, 60
    expect(jb.encoded[0].tsUs).toBe(30 * FRAME);
    expect(jb.dropped).toBe(30);
  });

  it("drops the oldest group of pictures beyond the byte limit", () => {
    const jb = buffer(1, { maxQueueBytes: 25_000 });
    for (const f of frames(40, 10)) jb.push(f);
    const kept = jb.encoded.map((f) => f.tsUs / FRAME);
    expect(kept[0] % 10).toBe(0);
    expect(kept.length * 1000).toBeLessThanOrEqual(25_000);
    expect(kept[kept.length - 1]).toBe(39);
  });

  it("waits for a keyframe when a single group of pictures overflows", () => {
    const jb = buffer(1, { maxQueueMs: 1000 });
    for (const f of frames(20, 1000)) jb.push(f); // only frame 0 is a keyframe
    expect(jb.encoded).toEqual([]);
    expect(jb.waitingForKeyframe).toBe(true);
  });

  it("skips to a due keyframe when decoding falls far behind", () => {
    const jb = buffer(1, { catchUpMs: 1000, maxDecodeAhead: 100 });
    for (const f of frames(90)) jb.push(f); // keyframes at 0, 30, 60
    // The clock is at frame 70 (4.7 s) while frame 0 is next: jump to the keyframe at 60.
    const out = jb.takeDecodable(70 * FRAME, 0);
    expect(out[0].tsUs).toBe(60 * FRAME);
    expect(out[0].keyframe).toBe(true);
    expect(jb.dropped).toBe(60);
  });

  it("counts catch-ups", () => {
    const jb = buffer(16, { catchUpMs: 100, maxDecodeAhead: 1000 });
    for (const f of frames(90)) jb.push(f);
    jb.takeDecodable(70 * FRAME, 0);
    expect(jb.catchUps).toBe(1);
  });

  it("releases superseded frames as soon as a newer due one is decoded", () => {
    const jb = buffer(16);
    const [a, b, c] = [0, 1, 2].map((i) => decoded(i * FRAME));
    jb.addDecoded(a);
    jb.addDecoded(b, NaN); // no clock: keep
    expect(jb.decoded).toHaveLength(2);
    jb.addDecoded(c, 2 * FRAME); // due now: a and b can never be shown
    expect([a.closed, b.closed, c.closed]).toEqual([true, true, false]);
    expect(jb.decoded).toEqual([c]);
    expect(jb.skipped).toBe(2);
    jb.addDecoded(decoded(3 * FRAME), 2 * FRAME); // not due yet: c stays
    expect(jb.decoded).toHaveLength(2);
  });

  it("decodes more frames ahead at high speed", () => {
    const at = (speed: number) => {
      const jb = buffer(speed, { decodeLeadMs: 10_000 });
      for (const f of frames(100)) jb.push(f);
      return jb.takeDecodable(0, 0).length;
    };
    expect([at(1), at(4), at(16)]).toEqual([6, 8, 32]);
  });

  it("keeps only keyframes in keyframe-only mode", () => {
    const jb = buffer(16);
    jb.keyframesOnly = true;
    for (const f of frames(90)) jb.push(f);
    expect(jb.encoded.map((f) => f.tsUs / FRAME)).toEqual([0, 30, 60]);
    expect(jb.skipped).toBe(87);
  });

  it("does not skip ahead for a small delay", () => {
    const jb = buffer(1, { catchUpMs: 1000, maxDecodeAhead: 100 });
    for (const f of frames(40)) jb.push(f);
    expect(jb.takeDecodable(10 * FRAME, 0)[0].tsUs).toBe(0);
  });

  it("clears everything and closes decoded frames", () => {
    const jb = buffer();
    for (const f of frames(5)) jb.push(f);
    const d = decoded(0);
    jb.addDecoded(d);
    expect(jb.newestTsUs).toBe(4 * FRAME);
    jb.clear();
    expect(d.closed).toBe(true);
    expect(jb.isEmpty).toBe(true);
    expect(jb.newestTsUs).toBeNaN();
    expect(jb.waitingForKeyframe).toBe(true);
  });
});

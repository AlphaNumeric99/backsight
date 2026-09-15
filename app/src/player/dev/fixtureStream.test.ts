import { describe, expect, it } from "vitest";
import { encodeBatch, type Packet } from "../wire";
import { FixturePacer, parseFixture, type Fixture } from "./fixtureStream";

const EPOCH = 1_790_000_000_000_000;
const FRAME = 100_000; // 10 fps keeps the numbers round

/** 1 s of video (10 frames, keyframe first) and 8 kHz audio in 100 ms packets. */
function fixtureBytes(withAudio = true): Uint8Array {
  const packets: Packet[] = [
    {
      kind: "videoConfig",
      timestampUs: 0,
      discontinuity: false,
      config: { codec: "avc1.4D401F", codedWidth: 1280, codedHeight: 720 },
      description: new Uint8Array([1, 2, 3]),
    },
  ];
  if (withAudio) {
    packets.push({
      kind: "audioConfig",
      timestampUs: 0,
      discontinuity: false,
      config: { sampleRate: 8000, channels: 1, format: "s16le" },
    });
  }
  for (let i = 0; i < 10; i++) {
    packets.push({ kind: "videoFrame", timestampUs: i * FRAME, discontinuity: false, keyframe: i === 0, data: new Uint8Array([i]) });
    if (withAudio) packets.push({ kind: "audioPcm", timestampUs: i * FRAME, discontinuity: false, samples: new Int16Array(800) });
  }
  return encodeBatch(packets);
}

function drain(pacer: FixturePacer, count: number) {
  return Array.from({ length: count }, () => pacer.next()!);
}

describe("parseFixture", () => {
  it("splits configs from media and measures one pass", () => {
    const fixture = parseFixture(fixtureBytes());
    expect(fixture.videoConfig.config.codec).toBe("avc1.4D401F");
    expect(fixture.audioConfig?.config.sampleRate).toBe(8000);
    expect(fixture.media).toHaveLength(20);
    expect(fixture.durationUs).toBe(1_000_000);
  });

  it("measures video-only fixtures from the frame rate", () => {
    expect(parseFixture(fixtureBytes(false)).durationUs).toBe(1_000_000);
  });

  it("rejects files that are not fixtures", () => {
    expect(() => parseFixture(new Uint8Array([1, 2, 3]))).toThrow(/invalid fixture/);
    expect(() => parseFixture(encodeBatch([{ kind: "endOfStream", timestampUs: 0, discontinuity: false }]))).toThrow(
      /no videoConfig/,
    );
  });
});

describe("FixturePacer", () => {
  const fixture: Fixture = parseFixture(fixtureBytes());
  const live = { startEpochUs: EPOCH, speed: 1, loop: true, audio: true, discontinuity: false };

  it("sends the configs first, stamped with the start time", () => {
    const [video, audio, first] = drain(new FixturePacer(fixture, live), 3);
    expect(video).toMatchObject({ dueMs: 0, packet: { kind: "videoConfig", timestampUs: EPOCH, discontinuity: false } });
    expect(audio).toMatchObject({ dueMs: 0, packet: { kind: "audioConfig", timestampUs: EPOCH } });
    expect(first).toMatchObject({ dueMs: 0, packet: { kind: "videoFrame", keyframe: true, timestampUs: EPOCH } });
  });

  it("flags a discontinuity on the first packet only", () => {
    const packets = drain(new FixturePacer(fixture, { ...live, discontinuity: true }), 6);
    expect(packets.map((p) => p.packet.discontinuity)).toEqual([true, false, false, false, false, false]);
  });

  it("paces packets in real time with continuous timestamps across loops", () => {
    const pacer = new FixturePacer(fixture, live);
    const packets = drain(pacer, 2 + 20 * 3).slice(2);
    const video = packets.filter((p) => p.packet.kind === "videoFrame");
    expect(video.map((p) => p.dueMs)).toEqual(Array.from({ length: 30 }, (_, i) => i * 100));
    expect(video.map((p) => p.packet.timestampUs - EPOCH)).toEqual(Array.from({ length: 30 }, (_, i) => i * FRAME));
    // Each pass starts with the fixture's keyframe.
    expect(video.filter((p) => p.packet.kind === "videoFrame" && p.packet.keyframe).map((p) => p.dueMs)).toEqual([
      0, 1000, 2000,
    ]);
    expect(pacer.peekDueMs()).toBe(3000);
  });

  it("scales the send times by the speed but not the timestamps", () => {
    const pacer = new FixturePacer(fixture, { ...live, speed: 4, audio: false, discontinuity: true });
    const packets = drain(pacer, 1 + 10);
    expect(packets.some((p) => p.packet.kind === "audioConfig" || p.packet.kind === "audioPcm")).toBe(false);
    const frames = packets.slice(1);
    expect(frames.map((p) => p.dueMs)).toEqual(Array.from({ length: 10 }, (_, i) => i * 25));
    expect(frames.map((p) => p.packet.timestampUs - EPOCH)).toEqual(Array.from({ length: 10 }, (_, i) => i * FRAME));
  });

  it("ends with a status and end of stream when not looping", () => {
    const pacer = new FixturePacer(fixture, { ...live, loop: false, speed: 2 });
    const packets = drain(pacer, 2 + 20 + 2);
    expect(packets.slice(-2)).toMatchObject([
      { dueMs: 500, packet: { kind: "status", status: { state: "ended" }, timestampUs: EPOCH + 1_000_000 } },
      { dueMs: 500, packet: { kind: "endOfStream" } },
    ]);
    expect(pacer.next()).toBeUndefined();
    expect(pacer.peekDueMs()).toBe(Infinity);
  });

  it("batches due packets in groups of at most three", () => {
    const pacer = new FixturePacer(fixture, live);
    // At 0 ms: two configs, frame 0 and PCM 0 are due.
    expect(pacer.takeDue(0).map((b) => b.map((p) => p.kind))).toEqual([
      ["videoConfig", "audioConfig", "videoFrame"],
      ["audioPcm"],
    ]);
    expect(pacer.takeDue(50)).toEqual([]);
    expect(pacer.takeDue(100).map((b) => b.length)).toEqual([2]);
    // A late timer releases everything due, still at most three per batch.
    expect(pacer.takeDue(450).map((b) => b.length)).toEqual([3, 3]); // 200, 300, 400 ms
  });

  it("rejects a non-positive speed", () => {
    expect(() => new FixturePacer(fixture, { ...live, speed: 0 })).toThrow(RangeError);
  });
});

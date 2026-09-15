// Turns a recorded fixture (.bspk, see scripts/make-player-fixture.mjs) into a paced packet
// stream, the way the backend streams a camera: re-stamped with wall-clock times and released at
// the playback speed. Pure logic (the caller owns timers), used by the mock backend.

import {
  decodeBatch,
  type AudioConfigPacket,
  type AudioPcmPacket,
  type Packet,
  type VideoConfigPacket,
  type VideoFramePacket,
} from "../wire";

export interface Fixture {
  videoConfig: VideoConfigPacket;
  audioConfig?: AudioConfigPacket;
  /** Frames and PCM in stream order, timestamps relative to the fixture's start. */
  media: (VideoFramePacket | AudioPcmPacket)[];
  /** Length of one pass, µs: the loop period. */
  durationUs: number;
}

/** Parses a fixture file. Throws if it isn't one (fixtures are ours). */
export function parseFixture(bytes: ArrayBuffer | ArrayBufferView): Fixture {
  const { packets, errors } = decodeBatch(bytes);
  if (errors.length) throw new Error(`invalid fixture: ${errors[0].message}`);
  const videoConfig = packets.find((p): p is VideoConfigPacket => p.kind === "videoConfig");
  if (!videoConfig) throw new Error("invalid fixture: no videoConfig");
  const audioConfig = packets.find((p): p is AudioConfigPacket => p.kind === "audioConfig");
  const media = packets.filter(
    (p): p is VideoFramePacket | AudioPcmPacket => p.kind === "videoFrame" || p.kind === "audioPcm",
  );
  const frames = media.filter((p) => p.kind === "videoFrame");
  if (frames.length < 2 || !frames[0].keyframe) throw new Error("invalid fixture: needs a keyframe and 2+ frames");

  const lastFrame = frames[frames.length - 1].timestampUs;
  let end = lastFrame + (lastFrame - frames[frames.length - 2].timestampUs);
  if (audioConfig) {
    const { sampleRate, channels } = audioConfig.config;
    for (const p of media) {
      if (p.kind === "audioPcm") end = Math.max(end, p.timestampUs + (p.samples.length / channels / sampleRate) * 1e6);
    }
  }
  return { videoConfig, audioConfig, media, durationUs: Math.round(end / 1000) * 1000 };
}

export interface PacerOptions {
  /** Epoch time (µs) given to the fixture's start. */
  startEpochUs: number;
  /** Media seconds per wall-clock second. */
  speed: number;
  /** Play the fixture over and over (otherwise end with Status "ended" and EndOfStream). */
  loop: boolean;
  /** Include the audio track. */
  audio: boolean;
  /** Flag the first packet as a discontinuity (the backend does after every seek). */
  discontinuity: boolean;
}

export interface PacedPacket {
  /** When to send it, wall-clock ms after the stream opened. */
  dueMs: number;
  packet: Packet;
}

export class FixturePacer {
  private readonly queue: PacedPacket[] = [];
  private pass = 0;
  private index = 0;
  private finished = false;

  constructor(
    private readonly fixture: Fixture,
    private readonly options: PacerOptions,
  ) {
    if (!(options.speed > 0)) throw new RangeError("speed must be positive");
    const { startEpochUs, discontinuity, audio } = options;
    this.queue.push({ dueMs: 0, packet: { ...fixture.videoConfig, timestampUs: startEpochUs, discontinuity } });
    if (audio && fixture.audioConfig) {
      this.queue.push({ dueMs: 0, packet: { ...fixture.audioConfig, timestampUs: startEpochUs, discontinuity: false } });
    }
  }

  /** When the next packet is due (ms after opening); Infinity once the stream has ended. */
  peekDueMs(): number {
    this.fill();
    return this.queue.length ? this.queue[0].dueMs : Infinity;
  }

  next(): PacedPacket | undefined {
    this.fill();
    return this.queue.shift();
  }

  /** The packets due by `elapsedMs`, grouped into batches of at most `maxPerBatch`. */
  takeDue(elapsedMs: number, maxPerBatch = 3): Packet[][] {
    const batches: Packet[][] = [];
    while (this.peekDueMs() <= elapsedMs) {
      const last = batches[batches.length - 1];
      const packet = (this.next() as PacedPacket).packet;
      if (last && last.length < maxPerBatch) last.push(packet);
      else batches.push([packet]);
    }
    return batches;
  }

  private fill(): void {
    if (this.queue.length || this.finished) return;
    const { media, durationUs } = this.fixture;
    const { startEpochUs, speed, loop, audio } = this.options;
    while (this.queue.length === 0) {
      if (this.index >= media.length) {
        if (!loop) {
          const endUs = durationUs;
          const dueMs = endUs / 1000 / speed;
          const timestampUs = startEpochUs + endUs;
          this.queue.push({ dueMs, packet: { kind: "status", timestampUs, discontinuity: false, status: { state: "ended" } } });
          this.queue.push({ dueMs, packet: { kind: "endOfStream", timestampUs, discontinuity: false } });
          this.finished = true;
          return;
        }
        this.pass++;
        this.index = 0;
      }
      const packet = media[this.index++];
      if (packet.kind === "audioPcm" && !audio) continue;
      const streamUs = this.pass * durationUs + packet.timestampUs;
      this.queue.push({
        dueMs: streamUs / 1000 / speed,
        packet: { ...packet, timestampUs: startEpochUs + streamUs, discontinuity: false },
      });
    }
  }
}

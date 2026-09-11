// The jitter buffer: encoded frames wait here until they are close to their presentation time,
// then go to the decoder; decoded frames wait until the clock reaches them. Pure logic over
// abstract frames, so the policies can be unit-tested without WebCodecs.
//
// Encoded frames are buffered rather than decoded frames because decoded frames hold decoder
// (often GPU) memory: a hardware decoder stalls when the app keeps too many of its outputs.

export interface EncodedFrame {
  tsUs: number;
  keyframe: boolean;
  byteLength: number;
}

export interface DecodedFrame {
  tsUs: number;
  close(): void;
}

export interface JitterBufferOptions {
  /** Media seconds per wall-clock second. */
  speed: number;
  /**
   * At most this many frames decoded, or being decoded, ahead of the one on screen. Defaults to
   * 6, more at high speeds, where many frames must be decoded per display refresh.
   */
  maxDecodeAhead?: number;
  /** Frames are handed to the decoder this long (wall-clock ms) before they are due. */
  decodeLeadMs?: number;
  /** When the next frame to decode is this late (wall ms), skip ahead to a keyframe. */
  catchUpMs?: number;
  /** Encoded queue limits; beyond them the oldest group of pictures is dropped. */
  maxQueueMs?: number;
  maxQueueBytes?: number;
}

export class JitterBuffer<E extends EncodedFrame, D extends DecodedFrame> {
  readonly encoded: E[] = [];
  readonly decoded: D[] = [];
  /** Frames discarded before display: late, undecodable, or over the queue limits. */
  dropped = 0;
  /** Frames not shown because a newer one was due in the same refresh (fast playback). */
  skipped = 0;
  /** Times the decoder fell so far behind that frames were skipped to a keyframe. */
  catchUps = 0;
  /**
   * Decode keyframes only, for fast playback on machines that can't decode every frame in
   * time: the picture updates at the keyframe rate instead of stalling and jumping.
   */
  keyframesOnly = false;

  private readonly speed: number;
  private readonly maxDecodeAhead: number;
  private readonly leadUs: number;
  private readonly catchUpUs: number;
  private readonly maxQueueUs: number;
  private readonly maxQueueBytes: number;
  private queuedBytes = 0;
  private needKeyframe = true;

  constructor(options: JitterBufferOptions) {
    this.speed = options.speed;
    this.maxDecodeAhead = options.maxDecodeAhead ?? Math.max(6, Math.ceil(2 * options.speed));
    this.leadUs = (options.decodeLeadMs ?? 250) * 1000 * options.speed;
    this.catchUpUs = (options.catchUpMs ?? 1000) * 1000 * options.speed;
    this.maxQueueUs = (options.maxQueueMs ?? 4000) * 1000 * Math.max(1, options.speed);
    this.maxQueueBytes = options.maxQueueBytes ?? 32 * 1024 * 1024;
  }

  /** True while delta frames are being discarded until the next keyframe. */
  get waitingForKeyframe(): boolean {
    return this.needKeyframe;
  }

  /** Timestamp of the newest frame buffered (encoded or decoded), NaN when empty. */
  get newestTsUs(): number {
    if (this.encoded.length) return this.encoded[this.encoded.length - 1].tsUs;
    if (this.decoded.length) return this.decoded[this.decoded.length - 1].tsUs;
    return NaN;
  }

  get isEmpty(): boolean {
    return this.encoded.length === 0 && this.decoded.length === 0;
  }

  /** Queues an encoded frame. Returns false if it was dropped (waiting for a keyframe). */
  push(frame: E): boolean {
    if (this.keyframesOnly && !frame.keyframe) {
      this.skipped++;
      return false;
    }
    if (this.needKeyframe) {
      if (!frame.keyframe) {
        this.dropped++;
        return false;
      }
      this.needKeyframe = false;
    }
    this.encoded.push(frame);
    this.queuedBytes += frame.byteLength;
    while (
      this.encoded.length > 1 &&
      (this.queuedBytes > this.maxQueueBytes ||
        this.encoded[this.encoded.length - 1].tsUs - this.encoded[0].tsUs > this.maxQueueUs)
    ) {
      this.dropOldestGop();
    }
    return true;
  }

  /**
   * Returns the encoded frames to decode now, in order. `nowUs` is the presentation clock (NaN
   * before it starts, which lets the first frames through for a quick first picture);
   * `inFlight` is the decoder's queue size.
   */
  takeDecodable(nowUs: number, inFlight: number): E[] {
    if (this.needKeyframe) return [];
    if (Number.isFinite(nowUs)) this.catchUp(nowUs);
    const horizon = Number.isFinite(nowUs) ? nowUs + this.leadUs : Infinity;
    const out: E[] = [];
    while (
      this.encoded.length > 0 &&
      this.decoded.length + inFlight + out.length < this.maxDecodeAhead &&
      this.encoded[0].tsUs <= horizon
    ) {
      const frame = this.encoded.shift() as E;
      this.queuedBytes -= frame.byteLength;
      out.push(frame);
    }
    return out;
  }

  /**
   * Adds a decoder output, keeping presentation order. If the clock (`nowUs`) has already
   * passed it, the older frames can never be shown and are released at once, which frees the
   * decoder to catch up without waiting for the next display refresh.
   */
  addDecoded(frame: D, nowUs = NaN): void {
    let i = this.decoded.length;
    while (i > 0 && this.decoded[i - 1].tsUs > frame.tsUs) i--;
    this.decoded.splice(i, 0, frame);
    if (frame.tsUs <= nowUs && i > 0) this.release(i);
  }

  /**
   * Returns the frame to show at `nowUs` — the newest one due — and closes the older ones.
   * Returns undefined when nothing new is due.
   */
  takeDue(nowUs: number): D | undefined {
    if (!Number.isFinite(nowUs)) return undefined;
    let due = -1;
    while (due + 1 < this.decoded.length && this.decoded[due + 1].tsUs <= nowUs) due++;
    if (due < 0) return undefined;
    this.release(due);
    return this.decoded.shift();
  }

  /** Takes the oldest decoded frame regardless of the clock (the first picture after a start). */
  takeFirst(): D | undefined {
    return this.decoded.shift();
  }

  /** The decoder must restart: drop frames up to the next keyframe. */
  requireKeyframe(): void {
    const key = this.encoded.findIndex((f) => f.keyframe);
    if (key < 0) {
      this.dropEncoded(this.encoded.length);
      this.needKeyframe = true;
    } else {
      this.dropEncoded(key);
    }
  }

  /** Drops the encoded frames and waits for a keyframe (new codec parameters); keeps decoded ones. */
  discardEncoded(): void {
    this.dropEncoded(this.encoded.length);
    this.needKeyframe = true;
  }

  /** Drops everything, e.g. on a discontinuity. Closes decoded frames. */
  clear(): void {
    this.encoded.length = 0;
    this.queuedBytes = 0;
    for (const frame of this.decoded) frame.close();
    this.decoded.length = 0;
    this.needKeyframe = true;
  }

  /** Closes the `count` oldest decoded frames, superseded before they were shown. */
  private release(count: number): void {
    for (const frame of this.decoded.splice(0, count)) {
      frame.close();
      if (this.speed > 1) this.skipped++;
      else this.dropped++;
    }
  }

  /** Skips to the newest due keyframe when the decoder has fallen far behind the clock. */
  private catchUp(nowUs: number): void {
    if (this.encoded.length < 2 || nowUs - this.encoded[0].tsUs <= this.catchUpUs) return;
    for (let i = this.encoded.length - 1; i > 0; i--) {
      if (this.encoded[i].keyframe && this.encoded[i].tsUs <= nowUs) {
        this.dropEncoded(i);
        this.catchUps++;
        return;
      }
    }
  }

  private dropOldestGop(): void {
    const next = this.encoded.findIndex((f, i) => i > 0 && f.keyframe);
    if (next > 0) {
      this.dropEncoded(next);
    } else {
      this.dropEncoded(this.encoded.length);
      this.needKeyframe = true;
    }
  }

  private dropEncoded(count: number): void {
    for (const frame of this.encoded.splice(0, count)) {
      this.queuedBytes -= frame.byteLength;
      this.dropped++;
    }
  }
}

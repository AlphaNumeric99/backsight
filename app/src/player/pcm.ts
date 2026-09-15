// PCM helpers and the audio playout buffer used by the AudioWorklet. Pure logic, no Web Audio
// types, so it can be unit-tested.

/** Converts signed 16-bit samples to floats in [-1, 1). */
export function s16ToFloat32(samples: Int16Array): Float32Array<ArrayBuffer> {
  const out = new Float32Array(samples.length);
  for (let i = 0; i < samples.length; i++) out[i] = samples[i] / 32768;
  return out;
}

export interface PcmPlayerStatus {
  /** Whether playback was started by `sync` (otherwise the output is silent). */
  playing: boolean;
  /** Media time of the next sample to be rendered, µs since the epoch; NaN before any data. */
  positionUs: number;
  /** Media time range held in the buffer; NaN when empty. */
  bufferStartUs: number;
  bufferEndUs: number;
  /** The last render ran out of data. */
  starving: boolean;
}

/**
 * A ring buffer of interleaved samples tagged with media time, read at a variable rate.
 *
 * Writes are treated as one continuous signal: small timestamp jitter is ignored, gaps are filled
 * with silence and overlaps are trimmed, and a jump of more than a second restarts the buffer.
 * Reads interpolate linearly, which covers both drift correction (`setRate`, a few percent) and
 * resampling when the AudioContext could not be created at the source rate.
 */
export class PcmPlayer {
  private srcRate = 0;
  private channels = 1;
  private capacity = 0;
  private ring = new Float32Array(0);
  /** Media time of frame 0; NaN until the first write after a reset. */
  private baseUs = NaN;
  /** Absolute index of the next frame to write. */
  private writeFrame = 0;
  /** Absolute, fractional position of the next frame to read. */
  private readPos = 0;
  private rate = 1;
  private playing = false;
  private starving = false;

  /** Writes within this many µs of the expected time are appended as-is. */
  static readonly toleranceUs = 30_000;
  /** Timestamp jumps beyond this restart the buffer. */
  static readonly resetUs = 1_000_000;

  constructor(private readonly outRate: number) {}

  get configured(): boolean {
    return this.srcRate > 0;
  }

  /** Sets the source format and empties the buffer. */
  configure(sampleRate: number, channels: number, capacitySeconds = 4): void {
    this.srcRate = sampleRate;
    this.channels = Math.max(1, Math.floor(channels));
    this.capacity = Math.max(1, Math.ceil(sampleRate * capacitySeconds));
    this.ring = new Float32Array(this.capacity * this.channels);
    this.flush();
  }

  /** Empties the buffer and stops until the next `sync`. */
  flush(): void {
    this.baseUs = NaN;
    this.writeFrame = 0;
    this.readPos = 0;
    this.playing = false;
    this.starving = false;
  }

  /** Appends interleaved samples whose first sample is at `tsUs`. */
  write(tsUs: number, interleaved: Float32Array): void {
    if (!this.configured) return;
    let frames = Math.floor(interleaved.length / this.channels);
    let data = interleaved;
    if (frames === 0) return;
    if (Number.isNaN(this.baseUs)) this.baseUs = tsUs;

    const deltaUs = tsUs - this.frameTimeUs(this.writeFrame);
    if (Math.abs(deltaUs) > PcmPlayer.resetUs) {
      // A different part of the stream: start over from this packet.
      this.flush();
      this.baseUs = tsUs;
    } else if (deltaUs > PcmPlayer.toleranceUs) {
      this.append(null, Math.round((deltaUs * this.srcRate) / 1e6));
    } else if (deltaUs < -PcmPlayer.toleranceUs) {
      const overlap = Math.round((-deltaUs * this.srcRate) / 1e6);
      if (overlap >= frames) return;
      data = data.subarray(overlap * this.channels);
      frames -= overlap;
    }
    this.append(data, frames);
  }

  /** Starts (or moves) playback so the next rendered sample is the one at `positionUs`. */
  sync(positionUs: number): void {
    if (Number.isNaN(this.baseUs)) return;
    this.readPos = ((positionUs - this.baseUs) * this.srcRate) / 1e6;
    this.playing = true;
  }

  /** Playback rate relative to real time, for drift correction; clamped to [0.5, 2]. */
  setRate(rate: number): void {
    this.rate = Math.min(2, Math.max(0.5, rate));
  }

  /** Fills `output` (one array per output channel, all the same length). */
  render(output: Float32Array[]): void {
    const length = output.length ? output[0].length : 0;
    this.starving = false;
    if (!this.playing || !this.configured) {
      for (const channel of output) channel.fill(0);
      return;
    }
    const step = (this.srcRate / this.outRate) * this.rate;
    const oldest = this.writeFrame - this.capacity;
    for (let i = 0; i < length; i++) {
      const pos = this.readPos;
      if (pos < 0 || pos < oldest) {
        // Before the buffered audio (just synced ahead of it): silence, but keep time moving.
        for (const channel of output) channel[i] = 0;
        this.readPos += step;
        continue;
      }
      const frame = Math.floor(pos);
      if (frame + 1 >= this.writeFrame) {
        // Out of data: silence, and hold the position until more arrives.
        for (const channel of output) channel[i] = 0;
        this.starving = true;
        continue;
      }
      const frac = pos - frame;
      const a = (frame % this.capacity) * this.channels;
      const b = ((frame + 1) % this.capacity) * this.channels;
      for (let c = 0; c < output.length; c++) {
        const src = Math.min(c, this.channels - 1);
        const s0 = this.ring[a + src];
        output[c][i] = s0 + (this.ring[b + src] - s0) * frac;
      }
      this.readPos += step;
    }
  }

  status(): PcmPlayerStatus {
    const empty = Number.isNaN(this.baseUs);
    const start = Math.max(0, this.writeFrame - this.capacity, Math.floor(this.readPos));
    return {
      playing: this.playing,
      positionUs: empty ? NaN : this.frameTimeUs(this.readPos),
      bufferStartUs: empty || start >= this.writeFrame ? NaN : this.frameTimeUs(start),
      bufferEndUs: empty || start >= this.writeFrame ? NaN : this.frameTimeUs(this.writeFrame),
      starving: this.starving,
    };
  }

  private frameTimeUs(frame: number): number {
    return this.baseUs + (frame * 1e6) / this.srcRate;
  }

  /** Appends `frames` frames of `data`, or of silence when `data` is null. */
  private append(data: Float32Array | null, frames: number): void {
    if (frames <= 0) return;
    // Beyond the capacity only the newest samples matter.
    const skip = Math.max(0, frames - this.capacity);
    this.writeFrame += skip;
    for (let f = skip; f < frames; f++) {
      const at = (this.writeFrame % this.capacity) * this.channels;
      for (let c = 0; c < this.channels; c++) this.ring[at + c] = data ? data[f * this.channels + c] : 0;
      this.writeFrame++;
    }
    // Unread samples that were just overwritten are lost: move the reader past them.
    const oldest = this.writeFrame - this.capacity;
    if (this.playing && this.readPos < oldest && this.readPos >= 0) this.readPos = oldest;
  }
}

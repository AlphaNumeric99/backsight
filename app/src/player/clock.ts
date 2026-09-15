// The playout clock: maps packet timestamps to local wall-clock time, with an adaptive delay that
// absorbs network and pipeline jitter. Pure logic (time is passed in), so it can be unit-tested.
//
// Frames reach the player at `arrival_i` with media timestamps `ts_i`. The camera's clock and
// ours differ by an unknown offset, so only relative delays can be measured:
//
//   stream time   σ_i = (ts_i − ts_0) / speed          (wall-clock ms since the first frame)
//   delay         d_i = arrival_i − σ_i                (unknown constant + transport jitter)
//   base delay    d_min = min d_i over a sliding window (the least-delayed recent frame)
//   jitter        j_i = d_i − d_min
//   target delay  D = clamp(max j over the window + margin, minDelay, maxDelay)
//
// Frame i is shown at σ_i + d_min + D. The applied offset (d_min + D) slews by at most a few
// percent of elapsed time so video speeds up or slows down imperceptibly rather than jumping;
// changes larger than `snapMs` are applied at once. That keeps live view close to the live edge.
//
// Playback is not adaptive: the first frame anchors the offset (arrival + delay) and it stays,
// so a burst of late data after a stall plays out in full instead of being skipped. Only a
// rebuffer (`reanchor`) moves it.

export interface PlayoutOptions {
  /** Media seconds per wall-clock second. */
  speed: number;
  /** Bounds of the adaptive delay between arrival and display, ms. */
  minDelayMs: number;
  maxDelayMs: number;
  /** Delay on top of the measured jitter, ms. */
  marginMs?: number;
  /** How long arrival statistics are remembered, ms. */
  windowMs?: number;
  /** Largest change of the playout offset per elapsed ms (0.05 = 5 %). */
  slewRate?: number;
  /** Offset changes larger than this are applied immediately, ms. */
  snapMs?: number;
  /** Follow the arrival statistics (live), or keep the first anchor (playback). Default true. */
  adaptive?: boolean;
}

/** Live view: low latency, adapts between 150 and 300 ms. */
export const LIVE_PLAYOUT = { minDelayMs: 150, maxDelayMs: 300, adaptive: true } as const;
/** Playback from the SD card: a steady half-second buffer. */
export const PLAYBACK_PLAYOUT = { minDelayMs: 500, maxDelayMs: 500, adaptive: false } as const;

/** Sliding-window minimum or maximum of timestamped samples (a monotonic deque). */
export class SlidingExtreme {
  private readonly times: number[] = [];
  private readonly values: number[] = [];
  private head = 0;

  constructor(
    private readonly kind: "min" | "max",
    private readonly windowMs: number,
  ) {}

  push(timeMs: number, value: number): void {
    while (this.values.length > this.head) {
      const last = this.values[this.values.length - 1];
      if (this.kind === "min" ? last < value : last > value) break;
      this.values.pop();
      this.times.pop();
    }
    this.times.push(timeMs);
    this.values.push(value);
    // Expire old samples; the newest one always stays.
    while (this.head < this.times.length - 1 && this.times[this.head] < timeMs - this.windowMs) this.head++;
    if (this.head > 32 && this.head * 2 > this.times.length) {
      this.times.splice(0, this.head);
      this.values.splice(0, this.head);
      this.head = 0;
    }
  }

  /** NaN when empty. */
  get value(): number {
    return this.head < this.values.length ? this.values[this.head] : NaN;
  }

  clear(): void {
    this.times.length = 0;
    this.values.length = 0;
    this.head = 0;
  }
}

export class PlayoutClock {
  readonly speed: number;
  private readonly minDelayMs: number;
  private readonly maxDelayMs: number;
  private readonly marginMs: number;
  private readonly slewRate: number;
  private readonly snapMs: number;
  private readonly adaptive: boolean;
  private readonly delays: SlidingExtreme;
  private readonly jitter: SlidingExtreme;
  private baseUs = NaN;
  private offsetMs = NaN;
  private lastNowMs = NaN;

  constructor(options: PlayoutOptions) {
    if (!(options.speed > 0)) throw new RangeError("speed must be positive");
    this.speed = options.speed;
    this.minDelayMs = options.minDelayMs;
    this.maxDelayMs = Math.max(options.minDelayMs, options.maxDelayMs);
    this.marginMs = options.marginMs ?? 30;
    this.slewRate = options.slewRate ?? 0.05;
    this.snapMs = options.snapMs ?? 750;
    this.adaptive = options.adaptive ?? true;
    const windowMs = options.windowMs ?? 10_000;
    this.delays = new SlidingExtreme("min", windowMs);
    this.jitter = new SlidingExtreme("max", windowMs);
  }

  /** True once a frame has anchored the clock. */
  get started(): boolean {
    return !Number.isNaN(this.offsetMs);
  }

  /** The current target delay between arrival and display, ms. */
  get delayMs(): number {
    if (!this.adaptive) return this.minDelayMs;
    const peak = this.jitter.value;
    const wanted = (Number.isNaN(peak) ? 0 : peak) + this.marginMs;
    return Math.min(this.maxDelayMs, Math.max(this.minDelayMs, wanted));
  }

  /**
   * Records the arrival of a video frame and returns how late it came relative to its playout
   * time, in ms (negative when early, as it should be).
   */
  observe(tsUs: number, arrivalMs: number): number {
    if (Number.isNaN(this.baseUs)) this.baseUs = tsUs;
    const sigma = this.streamMs(tsUs);
    const delay = arrivalMs - sigma;
    if (this.adaptive || !this.started) {
      this.delays.push(arrivalMs, delay);
      this.jitter.push(arrivalMs, delay - this.delays.value);
    }
    if (!this.started) {
      this.offsetMs = this.desiredOffsetMs();
      this.lastNowMs = arrivalMs;
    }
    return arrivalMs - (sigma + this.offsetMs);
  }

  /**
   * The media time to present at `nowMs`, in µs since the epoch; NaN until the first frame.
   * `nowMs` must not decrease between calls (it drives the slewing).
   */
  mediaTimeUs(nowMs: number): number {
    if (!this.started) return NaN;
    const elapsed = nowMs > this.lastNowMs ? nowMs - this.lastNowMs : 0;
    this.lastNowMs = Math.max(this.lastNowMs, nowMs);
    const diff = this.adaptive ? this.desiredOffsetMs() - this.offsetMs : 0;
    if (Math.abs(diff) > this.snapMs) {
      this.offsetMs += diff;
    } else {
      const step = this.slewRate * elapsed;
      this.offsetMs += Math.max(-step, Math.min(step, diff));
    }
    return this.baseUs + (nowMs - this.offsetMs) * 1000 * this.speed;
  }

  /** Wall-clock time (ms) at which a frame with timestamp `tsUs` is due; NaN until started. */
  dueAtMs(tsUs: number): number {
    return this.streamMs(tsUs) + this.offsetMs;
  }

  /** Forgets the arrival history, so the next frame re-anchors playout (after a stall). */
  reanchor(): void {
    this.delays.clear();
    this.jitter.clear();
    this.offsetMs = NaN;
    this.lastNowMs = NaN;
  }

  /** Starts over, as for a new stream. */
  reset(): void {
    this.reanchor();
    this.baseUs = NaN;
  }

  private desiredOffsetMs(): number {
    return this.delays.value + this.delayMs;
  }

  private streamMs(tsUs: number): number {
    return (tsUs - this.baseUs) / 1000 / this.speed;
  }
}

// Keeps audio in step with the playout clock, and turns the audio output into a clock that video
// can follow. Runs in the media worker; the AudioWorklet reports its position a few dozen times a
// second and obeys `sync` / `rate` / `flush` commands. Pure logic, unit-tested.
//
// The playout clock says which media time should be on screen now ("wanted"). The audio output
// reports which media time is audible now. The difference is corrected gently by playing audio a
// little faster or slower (at most ±2 %), or by jumping when it exceeds `resyncUs`. While audio is
// close enough, video uses the audio position as its clock, so sound and picture stay in sync
// even while the audio converges on the target.

import type { PcmPlayerStatus } from "./pcm";

export type AudioCommand =
  | { type: "sync"; positionUs: number }
  | { type: "rate"; rate: number }
  | { type: "flush" };

export interface AudioSyncOptions {
  /** Errors beyond this are fixed by jumping instead of changing the rate, µs. */
  resyncUs?: number;
  /** Rate change per ms of error. */
  gainPerMs?: number;
  /** Largest rate change, as a fraction. */
  maxRateChange?: number;
  /** Errors below this are left alone, µs. */
  deadbandUs?: number;
  /** Reports older than this make the audio clock unusable, ms. */
  staleMs?: number;
}

export class AudioSync {
  /** Output latency (worklet to speaker), µs. */
  latencyUs = 0;
  private readonly resyncUs: number;
  private readonly gainPerMs: number;
  private readonly maxRateChange: number;
  private readonly deadbandUs: number;
  private readonly staleMs: number;
  private rate = 1;
  private master = false;
  /** Audible media time minus local time (µs), low-pass filtered; NaN when unknown. */
  private offsetUs = NaN;
  private lastReportMs = NaN;
  private lastErrorUs = NaN;

  constructor(options: AudioSyncOptions = {}) {
    this.resyncUs = options.resyncUs ?? 150_000;
    this.gainPerMs = options.gainPerMs ?? 0.0002;
    this.maxRateChange = options.maxRateChange ?? 0.02;
    this.deadbandUs = options.deadbandUs ?? 5_000;
    this.staleMs = options.staleMs ?? 250;
  }

  /** Audible minus wanted media time at the last report, µs (positive: audio is ahead). */
  get errorUs(): number {
    return this.lastErrorUs;
  }

  /**
   * Handles a worklet report received at `nowMs`. `wantedUs` is the playout clock at `nowMs`
   * (NaN when it isn't running). Returns a command for the worklet, if any.
   */
  onReport(report: PcmPlayerStatus, nowMs: number, wantedUs: number): AudioCommand | undefined {
    this.lastReportMs = nowMs;
    if (!Number.isFinite(wantedUs)) {
      this.demote();
      return undefined;
    }
    // The worklet renders ahead of the speaker by the output latency.
    const targetUs = wantedUs + this.latencyUs;
    const hasData = Number.isFinite(report.bufferStartUs) && Number.isFinite(report.bufferEndUs);
    // Enough buffered to start at the target? Starting up to 1 s before the buffer is fine: the
    // worklet plays silence until it reaches the first sample.
    const canStartAt = hasData && targetUs >= report.bufferStartUs - 1_000_000 && targetUs < report.bufferEndUs - 20_000;

    if (!report.playing) {
      this.demote();
      return canStartAt ? this.startAt(targetUs) : undefined;
    }

    const audibleUs = report.positionUs - this.latencyUs;
    const errorUs = audibleUs - wantedUs;
    this.lastErrorUs = errorUs;
    if (Math.abs(errorUs) > this.resyncUs) {
      this.demote();
      if (canStartAt) return this.startAt(targetUs);
      // All the audio we hold is older than the target: drop it and wait for fresh data. (If it
      // is all far newer, keep it: the clock will get there.)
      if (!hasData || targetUs >= report.bufferEndUs - 20_000) return { type: "flush" };
      return undefined;
    }
    if (report.starving) {
      this.demote();
      return undefined;
    }

    const filtered = audibleUs - nowMs * 1000;
    this.offsetUs = Number.isNaN(this.offsetUs) ? filtered : this.offsetUs + (filtered - this.offsetUs) * 0.2;
    this.master = true;

    let rate = 1;
    if (Math.abs(errorUs) > this.deadbandUs) {
      const change = (errorUs / 1000) * this.gainPerMs;
      rate = 1 - Math.max(-this.maxRateChange, Math.min(this.maxRateChange, change));
    }
    if (Math.abs(rate - this.rate) >= 0.001 || (rate === 1 && this.rate !== 1)) {
      this.rate = rate;
      return { type: "rate", rate };
    }
    return undefined;
  }

  /** The audible media time at `nowMs` (µs), or NaN when video should use the playout clock. */
  clockUs(nowMs: number): number {
    if (!this.master || Number.isNaN(this.offsetUs) || !(nowMs - this.lastReportMs <= this.staleMs)) return NaN;
    return nowMs * 1000 + this.offsetUs;
  }

  /** Whether video currently follows the audio clock. */
  get isMaster(): boolean {
    return this.master;
  }

  reset(): void {
    this.demote();
    this.rate = 1;
    this.lastReportMs = NaN;
    this.lastErrorUs = NaN;
  }

  private startAt(targetUs: number): AudioCommand {
    this.rate = 1;
    return { type: "sync", positionUs: targetUs };
  }

  private demote(): void {
    this.master = false;
    this.offsetUs = NaN;
  }
}

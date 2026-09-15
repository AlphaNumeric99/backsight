// Rolling counters for the player statistics. Pure logic, unit-tested.

/** Sum and count of samples over a trailing time window. */
export class WindowCounter {
  private readonly times: number[] = [];
  private readonly values: number[] = [];
  private total = 0;
  private firstMs = NaN;

  constructor(private readonly windowMs: number) {}

  add(nowMs: number, value = 1): void {
    if (Number.isNaN(this.firstMs)) this.firstMs = nowMs;
    this.times.push(nowMs);
    this.values.push(value);
    this.total += value;
    this.expire(nowMs);
  }

  sum(nowMs: number): number {
    this.expire(nowMs);
    return this.total;
  }

  count(nowMs: number): number {
    this.expire(nowMs);
    return this.times.length;
  }

  /** Sum per second. Before a full window has passed, divides by the time covered so far. */
  perSecond(nowMs: number): number {
    const sum = this.sum(nowMs);
    if (Number.isNaN(this.firstMs)) return 0;
    const span = Math.min(this.windowMs, nowMs - this.firstMs);
    return span > 0 ? (sum * 1000) / span : 0;
  }

  /** Mean of the samples in the window; NaN when there are none. */
  mean(nowMs: number): number {
    const n = this.count(nowMs);
    return n ? this.total / n : NaN;
  }

  clear(): void {
    this.times.length = 0;
    this.values.length = 0;
    this.total = 0;
    this.firstMs = NaN;
  }

  private expire(nowMs: number): void {
    let n = 0;
    while (n < this.times.length && this.times[n] <= nowMs - this.windowMs) {
      this.total -= this.values[n];
      n++;
    }
    if (n) {
      this.times.splice(0, n);
      this.values.splice(0, n);
      if (this.times.length === 0) this.total = 0; // drop accumulated rounding
    }
  }
}

// Small deterministic random number generator for fixtures: the same seed always produces the
// same recordings, events and thumbnails, so the mock is stable across reloads and in tests.

/** FNV-1a, 32-bit. */
export function hashString(value: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < value.length; i++) {
    h ^= value.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
}

/** mulberry32: fast, good enough for fixtures, and trivially seedable. */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export class Rng {
  private readonly source: () => number;

  constructor(seed: string | number) {
    this.source = mulberry32(typeof seed === "number" ? seed : hashString(seed));
  }

  /** [0, 1) */
  next(): number {
    return this.source();
  }

  /** [min, max) */
  range(min: number, max: number): number {
    return min + (max - min) * this.next();
  }

  /** Integer in [min, max] */
  int(min: number, max: number): number {
    return Math.floor(this.range(min, max + 1));
  }

  chance(p: number): boolean {
    return this.next() < p;
  }

  pick<T>(items: readonly T[]): T {
    return items[Math.floor(this.next() * items.length)];
  }

  /** Picks by relative weight. */
  weighted<T>(entries: readonly (readonly [T, number])[]): T {
    const total = entries.reduce((sum, [, w]) => sum + w, 0);
    let r = this.next() * total;
    for (const [value, w] of entries) {
      r -= w;
      if (r < 0) return value;
    }
    return entries[entries.length - 1][0];
  }

  /** Log-uniform in [min, max): most values small, a few large. */
  logRange(min: number, max: number): number {
    return Math.exp(this.range(Math.log(min), Math.log(max)));
  }
}

import { describe, expect, it } from "vitest";
import { fitRect } from "./fit";
import { WindowCounter } from "./stats";

describe("WindowCounter", () => {
  it("sums over a trailing window", () => {
    const c = new WindowCounter(1000);
    c.add(0, 5);
    c.add(500, 5);
    c.add(900, 5);
    expect(c.sum(900)).toBe(15);
    expect(c.sum(1000)).toBe(10); // the sample at 0 expired
    expect(c.count(1600)).toBe(1); // (600, 1600]
    expect(c.count(1900)).toBe(0);
    expect(c.mean(1900)).toBeNaN();
  });

  it("computes a rate over the time covered so far", () => {
    const c = new WindowCounter(2000);
    expect(c.perSecond(0)).toBe(0);
    c.add(0, 1000);
    c.add(500, 1000);
    expect(c.perSecond(500)).toBe(4000); // 2000 in 0.5 s
    for (let t = 1000; t <= 10_000; t += 500) c.add(t, 1000);
    expect(c.perSecond(10_000)).toBe(2000); // 4 samples in the window (8500, 10000] / 2 s
  });

  it("averages samples", () => {
    const c = new WindowCounter(1000);
    c.add(0, 100);
    c.add(10, 200);
    expect(c.mean(10)).toBe(150);
    c.clear();
    expect(c.sum(10)).toBe(0);
  });
});

describe("fitRect", () => {
  it("letterboxes with contain", () => {
    expect(fitRect(1280, 720, 400, 400, "contain")).toEqual({
      sx: 0,
      sy: 0,
      sw: 1280,
      sh: 720,
      dx: 0,
      dy: 87,
      dw: 400,
      dh: 225,
    });
    expect(fitRect(1280, 720, 1000, 300, "contain")).toMatchObject({ dx: 233, dy: 0, dw: 533, dh: 300 });
  });

  it("crops evenly with cover", () => {
    const r = fitRect(1280, 720, 400, 400, "cover");
    expect(r).toMatchObject({ sy: 0, sh: 720, sw: 720, sx: 280, dx: 0, dy: 0, dw: 400, dh: 400 });
  });

  it("draws nothing for empty sizes", () => {
    expect(fitRect(0, 720, 400, 400, "contain").dw).toBe(0);
    expect(fitRect(1280, 720, 400, 0, "cover").dh).toBe(0);
  });
});

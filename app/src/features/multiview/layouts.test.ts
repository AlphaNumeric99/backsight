import { describe, expect, it } from "vitest";
import { LAYOUTS, LAYOUT_ORDER, fitGrid, formatPage, moveInOrder, normalizeOrder, pageCount, pageSlice, tileQuality } from "./layouts";

describe("multi-view layouts", () => {
  it("covers 1, 2, 4, 1+5, 9 and 16", () => {
    expect(LAYOUT_ORDER.map((l) => LAYOUTS[l].tiles)).toEqual([1, 2, 4, 6, 9, 16]);
  });

  it("pages cameras", () => {
    const ids = ["a", "b", "c", "d", "e", "f", "g"];
    expect(pageCount(7, LAYOUTS["4"])).toBe(2);
    expect(pageCount(0, LAYOUTS["4"])).toBe(1);
    expect(pageCount(7, LAYOUTS["16"])).toBe(1);
    expect(pageSlice(ids, LAYOUTS["4"], 1)).toEqual(["e", "f", "g"]);
    expect(pageSlice(ids, LAYOUTS["1+5"], 0)).toHaveLength(6);
    expect(formatPage(2)).toBe("02");
  });

  it("normalises a saved order against the current cameras", () => {
    const cams = [{ id: "a" }, { id: "b" }, { id: "c" }];
    expect(normalizeOrder(["c", "gone", "a", "c"], cams)).toEqual(["c", "a", "b"]);
    expect(normalizeOrder([], cams)).toEqual(["a", "b", "c"]);
  });

  it("moves a camera to another's position", () => {
    expect(moveInOrder(["a", "b", "c", "d"], "a", "c")).toEqual(["b", "c", "a", "d"]);
    expect(moveInOrder(["a", "b", "c", "d"], "d", "b")).toEqual(["a", "d", "b", "c"]);
    expect(moveInOrder(["a", "b"], "a", "zzz")).toEqual(["a", "b"]);
  });

  it("fits 16:9 tiles into the container", () => {
    const wide = fitGrid({ width: 2000, height: 600 }, LAYOUTS["4"], 0);
    expect(wide.height).toBe(600);
    expect(wide.width).toBe(Math.floor((600 / 2) * (16 / 9) * 2));
    const tall = fitGrid({ width: 800, height: 2000 }, LAYOUTS["4"], 0);
    expect(tall.width).toBe(800);
    expect(tall.height).toBe(450);
    const withGap = fitGrid({ width: 1003, height: 5000 }, LAYOUTS["4"], 3);
    expect(withGap.width).toBe(1003);
    // (1003 - 3) / 2 = 500 wide tiles → 281.25 high each, plus one gap.
    expect(withGap.height).toBe(Math.floor(281.25 * 2 + 3));
  });

  it("uses HD only for big tiles", () => {
    expect(tileQuality(LAYOUTS["1"], 0)).toBe("hd");
    expect(tileQuality(LAYOUTS["2"], 1)).toBe("hd");
    expect(tileQuality(LAYOUTS["4"], 0)).toBe("sd");
    expect(tileQuality(LAYOUTS["1+5"], 0)).toBe("hd");
    expect(tileQuality(LAYOUTS["1+5"], 3)).toBe("sd");
    expect(tileQuality(LAYOUTS["16"], 5)).toBe("sd");
  });
});

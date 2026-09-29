import { describe, expect, it } from "vitest";
import { storageSummary } from "./storage-usage";

/** Sizes show in decimal units, like SD card labels. */
const GB = 1e9;

describe("storageSummary", () => {
  it("tells unknown apart from no card", () => {
    expect(storageSummary(undefined, "connecting").text).toBe("Checking…");
    expect(storageSummary(undefined, "online").text).toBe("Checking…");
    expect(storageSummary(undefined, "offline").text).toBe("Unknown");
    expect(storageSummary({ present: false, status: "none", totalBytes: 0, freeBytes: 0 }, "online").text).toBe(
      "No SD card",
    );
  });

  it("summarises usage, and a full loop-recording card as normal", () => {
    const card = { present: true, status: "normal" as const, totalBytes: 128 * GB, freeBytes: 32 * GB };
    expect(storageSummary(card)).toMatchObject({ text: "96 GB of 128 GB", tone: "normal" });
    const loop = { ...card, status: "full" as const, freeBytes: 0, loopRecording: true };
    expect(storageSummary(loop)).toMatchObject({ text: "128 GB · loop recording", tone: "normal" });
  });
});

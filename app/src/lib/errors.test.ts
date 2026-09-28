import { describe, expect, it } from "vitest";
import { describeError, isApiError, shouldRetry, toApiError } from "./errors";
import { renderExportName, sanitizeFileName } from "./export-name";
import { formatBitrate, formatBytes, formatEta } from "./format";

describe("API error copy", () => {
  const now = Date.parse("2026-09-29T10:00:00Z");

  it("explains a locked camera with the time left and when it unlocks", () => {
    const copy = describeError(
      { code: "camera_locked", message: "x", retryAt: "2026-09-29T10:23:00Z" },
      { now, offsetMinutes: 0 },
    );
    expect(copy.title).toBe("Camera is locked");
    expect(copy.body).toContain("in 23 min");
    expect(copy.body).toContain("10:23");
  });

  it("guides a rejected password and warns about lockouts", () => {
    const copy = describeError({ code: "auth_failed", message: "rejected" });
    expect(copy.title).toBe("Password not accepted");
    expect(copy.body).toMatch(/TP-Link account/);
    expect(copy.hint).toMatch(/10 failed attempts/);
  });

  it("says where to turn on Third-Party Compatibility", () => {
    const copy = describeError({ code: "third_party_compat_off", message: "" });
    expect(copy.body).toContain("Me → Third-Party Services");
  });

  it("passes the backend's message through for input and internal errors", () => {
    expect(describeError({ code: "invalid_input", message: "The clip must end after it starts." }).body).toBe(
      "The clip must end after it starts.",
    );
    expect(describeError({ code: "internal", message: "" }).body).toBe("An unexpected error occurred.");
  });

  it("has copy for every code", () => {
    for (const code of [
      "playback_busy",
      "stream_limit",
      "offline",
      "privacy_mode",
      "unsupported",
      "not_found",
    ] as const) {
      const copy = describeError({ code, message: "" });
      expect(copy.title.length).toBeGreaterThan(3);
      expect(copy.body.length).toBeGreaterThan(3);
    }
  });

  it("normalises unknown rejections", () => {
    expect(isApiError({ code: "offline", message: "" })).toBe(true);
    expect(isApiError({ code: "nope" })).toBe(false);
    expect(toApiError(new Error("boom"))).toEqual({ code: "internal", message: "boom" });
    expect(toApiError("x").code).toBe("internal");
  });

  it("retries only transient errors", () => {
    expect(shouldRetry(0, { code: "offline", message: "" })).toBe(true);
    expect(shouldRetry(0, { code: "auth_failed", message: "" })).toBe(false);
    expect(shouldRetry(2, { code: "internal", message: "" })).toBe(false);
  });
});

describe("export file names", () => {
  const start = Date.parse("2026-09-29T12:03:27Z");
  const end = start + 6 * 60_000 + 4000;

  it("renders the template in camera-local time", () => {
    expect(renderExportName("{camera} {date} {start}-{end}", { camera: "Front Door", start, end, offsetMinutes: 120 })).toBe(
      "Front Door 2026-09-29 14.03.27-14.09.31.mp4",
    );
  });

  it("keeps unknown tokens and strips characters files can't have", () => {
    expect(renderExportName("{camera}:{nope}", { camera: "A/B", start, end, offsetMinutes: 0 })).toBe("A-B-{nope}.mp4");
    expect(sanitizeFileName(' ..clip*?".. ')).toBe("clip---");
  });

  it("falls back to the camera and date when the template is blank", () => {
    expect(renderExportName("  ", { camera: "Garage", start, end, offsetMinutes: 0 })).toBe("Garage 2026-09-29.mp4");
  });
});

describe("formatting", () => {
  it("formats sizes, bitrates and ETAs", () => {
    expect(formatBytes(81_900_000_000)).toMatch(/^81\.9 GB$/);
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBitrate(2_400_000)).toMatch(/^2\.4 Mbps$/);
    expect(formatBitrate(512_000)).toBe("512 kbps");
    expect(formatEta(12)).toBe("12 s");
    expect(formatEta(185)).toBe("about 3 min");
    expect(formatEta(4200)).toBe("about 1 h 10 min");
  });
});

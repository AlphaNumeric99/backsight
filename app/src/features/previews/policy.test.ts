import { describe, expect, it } from "vitest";
import type { Camera } from "@/ipc";
import { isPreviewDue, PREVIEW_RETRY_MS, PREVIEW_STALE_MS } from "./policy";

const NOW = Date.parse("2026-09-29T12:00:00Z");

function camera(patch: Partial<Camera> = {}): Camera {
  return {
    id: "c1",
    name: "Front Door",
    brand: "tapo",
    host: "192.168.1.20",
    groupIds: [],
    favorite: false,
    hasCameraAccount: false,
    status: { state: "online" },
    ...patch,
  };
}

describe("isPreviewDue", () => {
  it("wants a preview for online cameras without one", () => {
    expect(isPreviewDue(camera(), NOW)).toBe(true);
  });

  it("refreshes stale previews only", () => {
    const fresh = new Date(NOW - PREVIEW_STALE_MS + 1000).toISOString();
    const stale = new Date(NOW - PREVIEW_STALE_MS).toISOString();
    expect(isPreviewDue(camera({ snapshotAt: fresh }), NOW)).toBe(false);
    expect(isPreviewDue(camera({ snapshotAt: stale }), NOW)).toBe(true);
  });

  it("leaves cameras that can't stream alone", () => {
    for (const state of ["offline", "privacy", "locked", "auth_failed", "unsupported", "connecting"] as const) {
      expect(isPreviewDue(camera({ status: { state } }), NOW)).toBe(false);
    }
  });

  it("waits after an attempt before trying again", () => {
    expect(isPreviewDue(camera(), NOW, NOW - PREVIEW_RETRY_MS + 1000)).toBe(false);
    expect(isPreviewDue(camera(), NOW, NOW - PREVIEW_RETRY_MS)).toBe(true);
  });
});

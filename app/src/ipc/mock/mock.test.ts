import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppEvent, ExportJob } from "../api";
import { addDays, DAY, dayRange, localDateOf, MINUTE, SECOND } from "@/lib/time";
import { createMockData } from "./data";
import { MOCK_CAMERAS } from "./fixtures";
import { generateDayIndex } from "./recordings";

const NOW = Date.parse("2026-09-29T12:30:00Z");
const OFFSET = 120; // UTC+02:00
const ISO_UTC = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d{3})?Z$/;

function memoryStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => void map.set(k, v),
  };
}

function makeApi(extra: Parameters<typeof createMockData>[0] = {}) {
  return createMockData({
    latency: false,
    now: () => NOW,
    storage: null,
    scenario: "default",
    utcOffsetMinutes: OFFSET,
    timeZone: "Europe/Berlin",
    discoverMs: 0,
    seedExports: false,
    ...extra,
  });
}

describe("mock cameras", () => {
  it("has five online cameras across models, one offline and one locked", async () => {
    const cams = await makeApi().listCameras();
    expect(cams).toHaveLength(7);
    const online = cams.filter((c) => c.status.state === "online");
    expect(online).toHaveLength(5);
    expect(new Set(online.map((c) => c.model))).toEqual(new Set(["C200", "C210", "C220", "C320WS", "C520WS"]));
    expect(cams.filter((c) => c.status.state === "offline")).toHaveLength(1);
    const locked = cams.find((c) => c.status.state === "locked")!;
    expect(Date.parse(locked.status.lockedUntil!) - NOW).toBe(23 * MINUTE);
    expect(cams.every((c) => c.utcOffsetMinutes === OFFSET)).toBe(true);
  });

  it("has Indoor and Outdoor groups", async () => {
    expect((await makeApi().listGroups()).map((g) => g.name)).toEqual(["Indoor", "Outdoor"]);
  });

  it("starts empty in the empty scenario", async () => {
    const api = makeApi({ scenario: "empty" });
    expect(await api.listCameras()).toEqual([]);
    expect(await api.listExports()).toEqual([]);
  });
});

describe("day index generator", () => {
  const cam = MOCK_CAMERAS[0]; // continuous recording
  const date = addDays(localDateOf(NOW, OFFSET), -3);

  it("is deterministic", () => {
    expect(generateDayIndex(cam, date, OFFSET, NOW)).toEqual(generateDayIndex(cam, date, OFFSET, NOW));
  });

  it("produces 20–80 sorted events inside the day and inside recorded footage", () => {
    const index = generateDayIndex(cam, date, OFFSET, NOW);
    const { start, end } = dayRange(date, OFFSET);
    expect(index.events.length).toBeGreaterThanOrEqual(20);
    expect(index.events.length).toBeLessThanOrEqual(80);
    let prev = -Infinity;
    for (const e of index.events) {
      expect(e.start).toMatch(ISO_UTC);
      const s = Date.parse(e.start);
      const t = Date.parse(e.end);
      expect(s).toBeGreaterThanOrEqual(prev);
      expect(t).toBeGreaterThan(s);
      expect(s).toBeGreaterThanOrEqual(start);
      expect(t).toBeLessThanOrEqual(end);
      expect(
        index.segments.some((seg) => Date.parse(seg.start) <= s && Date.parse(seg.end) >= t),
      ).toBe(true);
      expect(e.thumbnailUrl).toMatch(/^data:image\/svg\+xml,/);
      expect(e.types.length).toBeGreaterThan(0);
      prev = s;
    }
  });

  it("has continuous segments with gaps", () => {
    const index = generateDayIndex(cam, date, OFFSET, NOW);
    expect(index.segments.length).toBeGreaterThan(3);
    expect(index.segments.every((s) => s.kind === "continuous")).toBe(true);
    for (let i = 1; i < index.segments.length; i++) {
      expect(Date.parse(index.segments[i].start)).toBeGreaterThan(Date.parse(index.segments[i - 1].end));
    }
  });

  it("uses detection segments for detection-only cameras", () => {
    const nursery = MOCK_CAMERAS.find((c) => c.storage?.recordingMode === "detection")!;
    const index = generateDayIndex(nursery, date, OFFSET, NOW);
    expect(index.segments.length).toBeGreaterThan(0);
    expect(index.segments.every((s) => s.kind === "detection")).toBe(true);
  });

  it("cuts today off at the current time, as a stable prefix of the full day", () => {
    const today = localDateOf(NOW, OFFSET);
    const early = generateDayIndex(cam, today, OFFSET, NOW - 2 * 60 * MINUTE);
    const later = generateDayIndex(cam, today, OFFSET, NOW);
    for (const s of later.segments) expect(Date.parse(s.end)).toBeLessThanOrEqual(NOW);
    expect(later.events.slice(0, early.events.length).map((e) => e.id)).toEqual(early.events.map((e) => e.id));
  });

  it("has nothing for future days", () => {
    const index = generateDayIndex(cam, addDays(localDateOf(NOW, OFFSET), 1), OFFSET, NOW);
    expect(index.segments).toEqual([]);
    expect(index.events).toEqual([]);
  });
});

describe("days with recordings", () => {
  it("covers the current and previous month and nothing earlier", async () => {
    const api = makeApi();
    const current = await api.getDaysWithRecordings("cam-front-door", "2026-09");
    const previous = await api.getDaysWithRecordings("cam-front-door", "2026-08");
    const older = await api.getDaysWithRecordings("cam-front-door", "2026-07");
    expect(current.length).toBeGreaterThan(20);
    expect(current.every((d) => d <= "2026-09-29")).toBe(true);
    expect(current).toContain("2026-09-29");
    expect(previous.length).toBeGreaterThan(25);
    expect(older).toEqual([]);
  });

  it("rejects offline and locked cameras with their error codes", async () => {
    const api = makeApi();
    await expect(api.getDaysWithRecordings("cam-backyard", "2026-09")).rejects.toMatchObject({ code: "offline" });
    await expect(api.getDayIndex("cam-side-gate", "2026-09-29")).rejects.toMatchObject({
      code: "camera_locked",
      retryAt: expect.stringMatching(ISO_UTC),
    });
    await expect(api.getCamera("nope")).rejects.toMatchObject({ code: "not_found" });
  });
});

describe("adding cameras", () => {
  it("maps the magic passwords to errors", async () => {
    const api = makeApi();
    const base = { host: "192.168.1.60" };
    await expect(api.addCamera({ ...base, cloudPassword: "wrong" })).rejects.toMatchObject({ code: "auth_failed" });
    await expect(api.addCamera({ host: "192.168.1.61", cloudPassword: "compat" })).rejects.toMatchObject({
      code: "third_party_compat_off",
    });
    const locked = api.addCamera({ ...base, cloudPassword: "locked" });
    await expect(locked).rejects.toMatchObject({ code: "camera_locked" });
    const err = await locked.catch((e) => e);
    expect(Date.parse(err.retryAt) - NOW).toBe(23 * MINUTE);
    // Still locked, even with the right password.
    await expect(api.addCamera({ ...base, cloudPassword: "hunter2" })).rejects.toMatchObject({ code: "camera_locked" });
  });

  it("adds a discovered camera and announces the change", async () => {
    const api = makeApi();
    const events: AppEvent[] = [];
    api.subscribe((e) => events.push(e));
    const devices = await api.discover();
    const kitchen = devices.find((d) => d.name === "Kitchen")!;
    expect(kitchen.alreadyAdded).toBe(false);
    expect(devices.find((d) => d.host === "192.168.1.41")?.alreadyAdded).toBe(true);
    const cam = await api.addCamera({ host: kitchen.host, cloudPassword: "correct horse", groupIds: ["indoor"] });
    expect(cam).toMatchObject({ name: "Kitchen", model: "C210", groupIds: ["indoor"] });
    expect(events).toContainEqual({ type: "cameras-changed" });
    expect((await api.listCameras()).some((c) => c.id === cam.id)).toBe(true);
    await expect(api.addCamera({ host: kitchen.host, cloudPassword: "x" })).rejects.toMatchObject({
      code: "invalid_input",
    });
  });
});

describe("exports", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("queues, runs and finishes a job, emitting progress", async () => {
    let t = NOW;
    const api = makeApi({ now: () => t });
    const events: ExportJob[] = [];
    api.subscribe((e) => {
      if (e.type === "export-progress") events.push(e.job);
    });
    const start = NOW - DAY;
    const job = await api.startExport({
      cameraId: "cam-front-door",
      start: new Date(start).toISOString(),
      end: new Date(start + 60 * SECOND).toISOString(),
    });
    expect(job.state).toBe("queued");
    expect(job.outputPath).toMatch(/Front Door .*\.mp4$/);

    for (let i = 0; i < 40; i++) {
      t += 250;
      await vi.advanceTimersByTimeAsync(250);
    }
    const states = events.map((e) => e.state);
    expect(states[0]).toBe("queued");
    expect(states).toContain("running");
    expect(states.at(-1)).toBe("done");
    const progress = events.filter((e) => e.state === "running").map((e) => e.progress);
    expect(progress).toEqual([...progress].sort((a, b) => a - b));
    const done = (await api.listExports()).find((j) => j.id === job.id)!;
    expect(done.progress).toBe(1);
    expect(done.bytesWritten).toBeGreaterThan(0);
    await expect(api.revealExport(job.id)).resolves.toBeUndefined();
  });

  it("cancels a job", async () => {
    const api = makeApi();
    const start = NOW - DAY;
    const job = await api.startExport({
      cameraId: "cam-front-door",
      start: new Date(start).toISOString(),
      end: new Date(start + 10 * MINUTE).toISOString(),
    });
    await api.cancelExport(job.id);
    expect((await api.listExports())[0].state).toBe("cancelled");
    await expect(api.revealExport(job.id)).rejects.toMatchObject({ code: "invalid_input" });
  });

  it("rejects clips that end in the future", async () => {
    const api = makeApi();
    await expect(
      api.startExport({
        cameraId: "cam-front-door",
        start: new Date(NOW - MINUTE).toISOString(),
        end: new Date(NOW + MINUTE).toISOString(),
      }),
    ).rejects.toMatchObject({ code: "invalid_input" });
  });
});

describe("previews", () => {
  afterEach(() => {
    Reflect.deleteProperty(URL, "createObjectURL");
    Reflect.deleteProperty(URL, "revokeObjectURL");
  });

  it("show the captured frame and announce it", async () => {
    Object.defineProperty(URL, "createObjectURL", { value: () => "blob:preview-1", configurable: true });
    Object.defineProperty(URL, "revokeObjectURL", { value: () => {}, configurable: true });
    const api = makeApi();
    const events: AppEvent[] = [];
    api.subscribe((e) => events.push(e));
    const cam = (await api.listCameras()).find((c) => !c.snapshotUrl)!;

    await api.savePreview(cam.id, new Blob([new Uint8Array([0xff, 0xd8, 0xff])], { type: "image/jpeg" }));

    const after = await api.getCamera(cam.id);
    expect(after.snapshotUrl).toBe("blob:preview-1");
    expect(Date.parse(after.snapshotAt!)).toBe(NOW);
    expect(events).toContainEqual({
      type: "camera-preview",
      cameraId: cam.id,
      snapshotUrl: "blob:preview-1",
      snapshotAt: after.snapshotAt,
    });
  });

  it("keep a drawn scene but record the capture time", async () => {
    const api = makeApi();
    const cam = (await api.listCameras()).find((c) => c.snapshotUrl?.startsWith("data:"))!;
    await api.savePreview(cam.id, new Blob([new Uint8Array([0xff, 0xd8, 0xff])]));
    const after = await api.getCamera(cam.id);
    expect(after.snapshotUrl).toBe(cam.snapshotUrl);
    expect(after.snapshotAt).toMatch(ISO_UTC);
  });

  it("reject an empty image", async () => {
    const api = makeApi();
    const [cam] = await api.listCameras();
    await expect(api.savePreview(cam.id, new Blob([]))).rejects.toMatchObject({ code: "invalid_input" });
  });
});

describe("settings", () => {
  it("persists to storage", async () => {
    const storage = memoryStorage();
    const a = makeApi({ storage });
    await a.updateSettings({ theme: "dark", multiviewLayout: "9" });
    const b = makeApi({ storage });
    expect(await b.getSettings()).toMatchObject({ theme: "dark", multiviewLayout: "9" });
  });

  it("validates patches", async () => {
    const api = makeApi();
    await expect(api.updateSettings({ exportNameTemplate: " " })).rejects.toMatchObject({ code: "invalid_input" });
    await expect(api.updateSettings({ cacheLimitMb: 1 })).rejects.toMatchObject({ code: "invalid_input" });
  });
});

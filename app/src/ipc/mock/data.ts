// In-memory implementation of `BacksightApi` (everything except `openStream`) for browser
// development and tests: seven cameras, deterministic SD-card recordings and events, a
// simulated export queue, settings in localStorage, realistic latency and error cases.
//
// Handy for manual testing:
// - `?mock=empty` in the URL starts with no cameras and no downloads.
// - Adding a camera with the password `wrong`, `locked` or `compat` fails with
//   `auth_failed`, `camera_locked` or `third_party_compat_off`; a host ending in `.99` is offline.

import type {
  AddCameraRequest,
  ApiError,
  ApiErrorCode,
  AppEvent,
  BacksightApi,
  Camera,
  CameraGroup,
  CameraId,
  CameraStatus,
  DiscoveredDevice,
  ExportJob,
  LocalDate,
  Month,
  MultiviewLayout,
  Settings,
  UpdateCameraRequest,
} from "../api";
import { renderExportName } from "@/lib/export-name";
import { HOUR, isLocalDate, localOffsetMinutes, localParts, MINUTE, SECOND, toIso } from "@/lib/time";
import { MockExportQueue } from "./exports";
import { MOCK_CAMERAS, MOCK_GROUPS, MOCK_NEW_DEVICES, newCameraFixture, type MockCameraFixture } from "./fixtures";
import { daysWithRecordings, generateDayIndex, hasFootageBetween } from "./recordings";
import { isNightHour, snapshotDataUrl } from "./thumbnails";

type KeyValueStorage = Pick<Storage, "getItem" | "setItem">;

export interface MockDataOptions {
  /** Artificial latency range in ms, or `false` for none. Default 150–400 (none under test). */
  latency?: false | readonly [number, number];
  now?: () => number;
  /** Where settings persist. Default `localStorage`; `null` keeps them in memory. */
  storage?: KeyValueStorage | null;
  /** "empty" starts with no cameras and no downloads. Default: `?mock=` in the URL. */
  scenario?: "default" | "empty";
  /** The cameras' UTC offset. Default: the viewer's own offset. */
  utcOffsetMinutes?: number;
  timeZone?: string;
  /** How long a LAN scan takes. Default 2.4 s. */
  discoverMs?: number;
  /** Seeds finished and running export jobs. Default true (false under test). */
  seedExports?: boolean;
}

const SETTINGS_KEY = "backsight.mock.settings.v1";
const LAYOUTS: MultiviewLayout[] = ["1", "2", "4", "1+5", "9", "16"];

const isTestEnv = typeof import.meta !== "undefined" && import.meta.env?.MODE === "test";

function apiError(code: ApiErrorCode, message: string, extra: Partial<ApiError> = {}): ApiError {
  return { code, message, ...extra };
}

function clone<T>(value: T): T {
  return typeof structuredClone === "function" ? structuredClone(value) : JSON.parse(JSON.stringify(value));
}

function scenarioFromUrl(): "default" | "empty" {
  try {
    return new URLSearchParams(window.location.search).get("mock") === "empty" ? "empty" : "default";
  } catch {
    return "default";
  }
}

function safeLocalStorage(): KeyValueStorage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

type Platform = "windows" | "mac" | "linux";

function detectPlatform(): Platform {
  const ua = typeof navigator === "undefined" ? "" : navigator.userAgent;
  if (/Windows/i.test(ua)) return "windows";
  if (/Mac/i.test(ua)) return "mac";
  return "linux";
}

function joinPath(platform: Platform, ...parts: string[]): string {
  const sep = platform === "windows" ? "\\" : "/";
  return parts.map((p, i) => (i === 0 ? p.replace(/[\\/]+$/, "") : p.replace(/^[\\/]+|[\\/]+$/g, ""))).join(sep);
}

function defaultSettings(platform: Platform): Settings {
  const exportDir =
    platform === "windows"
      ? "C:\\Users\\you\\Videos\\Backsight"
      : platform === "mac"
        ? "/Users/you/Movies/Backsight"
        : "/home/you/Videos/Backsight";
  return {
    theme: "system",
    exportDir,
    exportNameTemplate: "{camera} {date} {start}-{end}",
    cacheLimitMb: 2048,
    defaultLiveQuality: "hd",
    multiviewLayout: "4",
    multiviewOrder: [],
  };
}

function loadSettings(storage: KeyValueStorage | null, defaults: Settings): Settings {
  if (!storage) return defaults;
  try {
    const raw = storage.getItem(SETTINGS_KEY);
    if (!raw) return defaults;
    const parsed = JSON.parse(raw) as Partial<Settings>;
    return { ...defaults, ...parsed };
  } catch {
    return defaults;
  }
}

export function createMockData(options: MockDataOptions = {}): Omit<BacksightApi, "openStream"> {
  const now = options.now ?? (() => Date.now());
  const latency = options.latency === undefined ? (isTestEnv ? false : ([150, 400] as const)) : options.latency;
  const storage = options.storage === undefined ? safeLocalStorage() : options.storage;
  const scenario = options.scenario ?? scenarioFromUrl();
  const offset = options.utcOffsetMinutes ?? localOffsetMinutes(now());
  const timeZone =
    options.timeZone ?? (typeof Intl !== "undefined" ? Intl.DateTimeFormat().resolvedOptions().timeZone : undefined);
  const discoverMs = options.discoverMs ?? (isTestEnv ? 0 : 2400);
  const platform = detectPlatform();

  const listeners = new Set<(event: AppEvent) => void>();
  const emit = (event: AppEvent) => {
    for (const listener of [...listeners]) {
      try {
        listener(event);
      } catch (err) {
        console.error("[mock] listener failed", err);
      }
    }
  };

  const wait = (): Promise<void> =>
    latency
      ? new Promise((resolve) => setTimeout(resolve, latency[0] + Math.random() * (latency[1] - latency[0])))
      : Promise.resolve();

  // --- State ----------------------------------------------------------------------------------

  const fixtures = new Map<CameraId, MockCameraFixture>();
  let cameras: Camera[] = [];
  let groups: CameraGroup[] = scenario === "empty" ? [] : clone(MOCK_GROUPS);
  let settings: Settings = loadSettings(storage, defaultSettings(platform));
  /** Hosts locked out by failed logins during this session, with when they unlock. */
  const lockouts = new Map<string, number>();
  let nextId = 1;

  const toCamera = (f: MockCameraFixture, t0: number): Camera => {
    const status: CameraStatus = { state: f.state };
    if (f.state === "offline") status.lastSeen = toIso(t0 - (f.lastSeenMinutesAgo ?? 60) * MINUTE);
    else status.lastSeen = toIso(t0);
    if (f.state === "locked") {
      status.lockedUntil = toIso(t0 + (f.lockedForMinutes ?? 30) * MINUTE);
      status.message = "Too many failed login attempts.";
    }
    const night = isNightHour(localParts(t0, offset).hour);
    return {
      id: f.id,
      name: f.name,
      host: f.host,
      model: f.model,
      firmware: f.firmware,
      mac: f.mac,
      groupIds: [...f.groupIds],
      favorite: f.favorite,
      hasCameraAccount: f.hasCameraAccount,
      status,
      storage: f.storage
        ? {
            present: f.storage.present,
            status: f.storage.status,
            totalBytes: f.storage.totalBytes,
            freeBytes: Math.round(f.storage.totalBytes * (1 - f.storage.usedFraction)),
            recordingMode: f.storage.recordingMode,
          }
        : undefined,
      videoCodec: f.videoCodec,
      utcOffsetMinutes: offset,
      timeZone,
      snapshotUrl: f.hasSnapshot ? snapshotDataUrl(f.scene, night) : undefined,
      snapshotAt: f.hasSnapshot ? toIso(t0) : undefined,
    };
  };

  if (scenario === "default") {
    const t0 = now();
    for (const f of MOCK_CAMERAS) {
      fixtures.set(f.id, f);
      cameras.push(toCamera(f, t0));
    }
  }

  const queue = new MockExportQueue({ emit, now });

  const findCamera = (id: CameraId): Camera => {
    const cam = cameras.find((c) => c.id === id);
    if (!cam) throw apiError("not_found", "No camera with that id.");
    return cam;
  };

  /** Locked cameras unlock on their own once `lockedUntil` passes. */
  const refreshStates = () => {
    const t = now();
    for (const cam of cameras) {
      if (cam.status.state === "locked" && cam.status.lockedUntil && Date.parse(cam.status.lockedUntil) <= t) {
        cam.status = { state: "online", lastSeen: toIso(t) };
        emit({ type: "camera-status", cameraId: cam.id, status: clone(cam.status) });
      }
    }
  };

  const assertReachable = (cam: Camera) => {
    switch (cam.status.state) {
      case "offline":
        throw apiError("offline", `${cam.name} is offline.`);
      case "locked":
        throw apiError("camera_locked", "Too many failed login attempts.", { retryAt: cam.status.lockedUntil });
      case "privacy":
        throw apiError("privacy_mode", "Privacy mode is on.");
      case "auth_failed":
        throw apiError("auth_failed", "The camera rejected the saved password.");
      case "unsupported":
        throw apiError("unsupported", "This camera isn't supported yet.");
      default:
        break;
    }
  };

  /** Simulates the camera checking a TP-Link account password. */
  const checkPassword = (host: string, password: string) => {
    const lockedUntil = lockouts.get(host);
    if (lockedUntil && lockedUntil > now()) {
      throw apiError("camera_locked", "Too many failed login attempts.", { retryAt: toIso(lockedUntil) });
    }
    switch (password) {
      case "wrong":
        throw apiError("auth_failed", "The camera rejected the password.");
      case "locked": {
        const until = now() + 23 * MINUTE;
        lockouts.set(host, until);
        throw apiError("camera_locked", "Too many failed login attempts.", { retryAt: toIso(until) });
      }
      case "compat":
        throw apiError("third_party_compat_off", "Third-Party Compatibility is turned off on the camera.");
      default:
        break;
    }
  };

  const persistSettings = () => {
    try {
      storage?.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      // Quota or privacy mode: settings still apply for this session.
    }
  };

  const outputPathFor = (cam: Camera, start: number, end: number) =>
    joinPath(
      platform,
      settings.exportDir,
      renderExportName(settings.exportNameTemplate, {
        camera: cam.name,
        start,
        end,
        offsetMinutes: cam.utcOffsetMinutes ?? offset,
      }),
    );

  // Download history, plus one job that is exporting right now.
  if (scenario === "default" && (options.seedExports ?? !isTestEnv)) {
    const t = now();
    const byId = (id: string) => cameras.find((c) => c.id === id)!;
    const historical = (
      id: string,
      cameraId: string,
      startAgo: number,
      length: number,
      extra: Partial<ExportJob>,
    ): ExportJob => {
      const cam = byId(cameraId);
      const start = Math.floor((t - startAgo) / SECOND) * SECOND;
      const end = start + length;
      return {
        id,
        cameraId,
        cameraName: cam.name,
        start: toIso(start),
        end: toIso(end),
        state: "done",
        progress: 1,
        bytesWritten: Math.round((length / 1000) * 275_000),
        outputPath: outputPathFor(cam, start, end),
        createdAt: toIso(t - startAgo + 20 * MINUTE),
        ...extra,
      };
    };
    queue.seed([
      historical("export-seed-1", "cam-front-door", 26 * HOUR + 12 * MINUTE, 2 * MINUTE + 35 * SECOND, {}),
      historical("export-seed-2", "cam-living-room", 50 * HOUR + 3 * MINUTE, 6 * MINUTE + 4 * SECOND, {}),
      historical("export-seed-3", "cam-driveway", 74 * HOUR + 40 * MINUTE, 11 * MINUTE, {
        state: "failed",
        progress: 0.37,
        bytesWritten: Math.round(0.37 * 660 * 275_000),
        outputPath: undefined,
        error: "The camera closed the connection.",
      }),
    ]);
    const garage = byId("cam-garage");
    const start = Math.floor((t - 3 * HOUR) / MINUTE) * MINUTE;
    queue.enqueue({
      cameraId: garage.id,
      cameraName: garage.name,
      start: toIso(start),
      end: toIso(start + 12 * MINUTE),
      outputPath: outputPathFor(garage, start, start + 12 * MINUTE),
    });
  }

  // --- API ------------------------------------------------------------------------------------

  return {
    async listCameras() {
      await wait();
      refreshStates();
      return clone(cameras);
    },

    async getCamera(id) {
      await wait();
      refreshStates();
      return clone(findCamera(id));
    },

    async discover(timeoutMs = 3000) {
      await new Promise((resolve) => setTimeout(resolve, Math.min(timeoutMs, discoverMs)));
      const added = new Set(cameras.map((c) => c.host));
      const devices: DiscoveredDevice[] = [
        ...MOCK_CAMERAS.map((f) => ({
          host: f.host,
          mac: f.mac,
          model: f.model,
          name: f.name,
          firmware: f.firmware,
          kind: "camera" as const,
          loginScheme: "secure" as const,
          alreadyAdded: added.has(f.host),
        })),
        ...MOCK_NEW_DEVICES.map((d) => ({ ...d, alreadyAdded: added.has(d.host) })),
      ];
      const ip = (h: string) => h.split(".").reduce((n, part) => n * 256 + Number(part), 0);
      return devices.sort((a, b) => Number(a.alreadyAdded) - Number(b.alreadyAdded) || ip(a.host) - ip(b.host));
    },

    async addCamera(req: AddCameraRequest) {
      await wait();
      const host = req.host.trim();
      if (!host) throw apiError("invalid_input", "Enter the camera's IP address.");
      if (!req.cloudPassword) throw apiError("invalid_input", "Enter the TP-Link account password.");
      if (cameras.some((c) => c.host === host)) throw apiError("invalid_input", "This camera is already added.");
      if (/\.99$/.test(host)) throw apiError("offline", `No response from ${host}.`);
      checkPassword(host, req.cloudPassword);

      const known =
        MOCK_CAMERAS.find((c) => c.host === host) ?? MOCK_NEW_DEVICES.find((d) => d.host === host);
      const kind = MOCK_NEW_DEVICES.find((d) => d.host === host)?.kind;
      if (kind === "hub") throw apiError("unsupported", "Hubs don't have a camera to view.");
      const name = req.name?.trim() || known?.name || `Camera ${host}`;
      const id = `cam-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "new"}-${nextId++}`;
      const fixture = newCameraFixture({
        id,
        host,
        name,
        model: known?.model,
        firmware: known?.firmware,
        mac: known?.mac,
      });
      fixture.groupIds = (req.groupIds ?? []).filter((g) => groups.some((x) => x.id === g));
      fixture.hasCameraAccount = Boolean(req.cameraAccount?.username);
      fixtures.set(id, fixture);
      const camera = toCamera(fixture, now());
      cameras.push(camera);
      emit({ type: "cameras-changed" });
      return clone(camera);
    },

    async updateCamera(id, req: UpdateCameraRequest) {
      await wait();
      const cam = findCamera(id);
      if (req.name !== undefined) {
        const name = req.name.trim();
        if (!name) throw apiError("invalid_input", "Enter a name for the camera.");
        cam.name = name;
      }
      if (req.groupIds !== undefined) cam.groupIds = req.groupIds.filter((g) => groups.some((x) => x.id === g));
      if (req.favorite !== undefined) cam.favorite = req.favorite;
      if (req.cameraAccount !== undefined) cam.hasCameraAccount = req.cameraAccount !== null;
      if (req.cloudPassword !== undefined) {
        if (!req.cloudPassword) throw apiError("invalid_input", "Enter the TP-Link account password.");
        checkPassword(cam.host, req.cloudPassword);
        if (cam.status.state === "auth_failed") {
          cam.status = { state: "online", lastSeen: toIso(now()) };
          emit({ type: "camera-status", cameraId: cam.id, status: clone(cam.status) });
        }
      }
      emit({ type: "cameras-changed" });
      return clone(cam);
    },

    async removeCamera(id) {
      await wait();
      findCamera(id);
      cameras = cameras.filter((c) => c.id !== id);
      fixtures.delete(id);
      if (settings.multiviewOrder.includes(id)) {
        settings = { ...settings, multiviewOrder: settings.multiviewOrder.filter((c) => c !== id) };
        persistSettings();
      }
      emit({ type: "cameras-changed" });
    },

    async listGroups() {
      await wait();
      return clone(groups);
    },

    async saveGroups(next: CameraGroup[]) {
      await wait();
      const ids = new Set<string>();
      for (const g of next) {
        if (!g.id || !g.name.trim()) throw apiError("invalid_input", "Every group needs a name.");
        if (ids.has(g.id)) throw apiError("invalid_input", "Group ids must be unique.");
        ids.add(g.id);
      }
      groups = next.map((g) => ({ id: g.id, name: g.name.trim() }));
      for (const cam of cameras) cam.groupIds = cam.groupIds.filter((g) => ids.has(g));
      emit({ type: "cameras-changed" });
    },

    async getDaysWithRecordings(cameraId, month: Month) {
      await wait();
      const cam = findCamera(cameraId);
      if (!/^\d{4}-\d{2}$/.test(month)) throw apiError("invalid_input", "Months look like 2026-09.");
      assertReachable(cam);
      return daysWithRecordings(fixtures.get(cameraId)!, month, cam.utcOffsetMinutes ?? offset, now());
    },

    async getDayIndex(cameraId, date: LocalDate) {
      await wait();
      const cam = findCamera(cameraId);
      if (!isLocalDate(date)) throw apiError("invalid_input", "Dates look like 2026-09-29.");
      assertReachable(cam);
      return generateDayIndex(fixtures.get(cameraId)!, date, cam.utcOffsetMinutes ?? offset, now());
    },

    async startExport(req) {
      await wait();
      const cam = findCamera(req.cameraId);
      const start = Date.parse(req.start);
      const end = Date.parse(req.end);
      if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) {
        throw apiError("invalid_input", "The clip must end after it starts.");
      }
      if (end > now()) throw apiError("invalid_input", "The clip can't end in the future.");
      assertReachable(cam);
      // The last few seconds count too: the camera is still writing them to the card.
      const empty = !hasFootageBetween(
        fixtures.get(cam.id)!,
        start,
        end,
        cam.utcOffsetMinutes ?? offset,
        now() + 20 * SECOND,
      );
      return queue.enqueue(
        {
          cameraId: cam.id,
          cameraName: cam.name,
          start: toIso(start),
          end: toIso(end),
          outputPath: req.outputPath ?? outputPathFor(cam, start, end),
        },
        { failMessage: empty ? "There are no recordings in the selected range." : undefined },
      );
    },

    async listExports() {
      await wait();
      return queue.list().sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt));
    },

    async cancelExport(id) {
      await wait();
      if (!queue.cancel(id)) throw apiError("not_found", "No export with that id.");
    },

    async revealExport(id) {
      await wait();
      const job = queue.get(id);
      if (!job) throw apiError("not_found", "No export with that id.");
      if (job.state !== "done") throw apiError("invalid_input", "The file isn't ready yet.");
      console.info(`[mock] reveal ${job.outputPath}`);
    },

    async saveSnapshot(cameraId, png) {
      await wait();
      const cam = findCamera(cameraId);
      if (!(png instanceof Blob) || png.size === 0) throw apiError("invalid_input", "The snapshot is empty.");
      const p = localParts(now(), cam.utcOffsetMinutes ?? offset);
      const pad = (n: number) => String(n).padStart(2, "0");
      const stamp = `${p.year}-${pad(p.month)}-${pad(p.day)} ${pad(p.hour)}.${pad(p.minute)}.${pad(p.second)}`;
      return joinPath(platform, settings.exportDir, "Snapshots", `${cam.name} ${stamp}.png`);
    },

    async savePreview(cameraId, jpeg) {
      await wait();
      const cam = findCamera(cameraId);
      if (!(jpeg instanceof Blob) || jpeg.size === 0) throw apiError("invalid_input", "The preview is empty.");
      // Cameras with a drawn scene keep it (the fixture streams are test patterns); the others
      // show the captured frame.
      if (!cam.snapshotUrl?.startsWith("data:") && typeof URL.createObjectURL === "function") {
        if (cam.snapshotUrl?.startsWith("blob:")) URL.revokeObjectURL(cam.snapshotUrl);
        cam.snapshotUrl = URL.createObjectURL(jpeg);
      }
      cam.snapshotAt = toIso(now());
      if (cam.snapshotUrl) {
        emit({ type: "camera-preview", cameraId, snapshotUrl: cam.snapshotUrl, snapshotAt: cam.snapshotAt });
      }
    },

    async getSettings() {
      await wait();
      return clone(settings);
    },

    async updateSettings(patch) {
      await wait();
      const next = { ...settings, ...patch };
      if (!["system", "light", "dark"].includes(next.theme)) throw apiError("invalid_input", "Unknown theme.");
      if (!next.exportDir.trim()) throw apiError("invalid_input", "Choose an export folder.");
      if (!next.exportNameTemplate.trim()) throw apiError("invalid_input", "The file name can't be empty.");
      if (!Number.isFinite(next.cacheLimitMb) || next.cacheLimitMb < 64 || next.cacheLimitMb > 65_536) {
        throw apiError("invalid_input", "The cache limit must be between 64 MB and 64 GB.");
      }
      if (next.defaultLiveQuality !== "hd" && next.defaultLiveQuality !== "sd") {
        throw apiError("invalid_input", "Unknown quality.");
      }
      if (!LAYOUTS.includes(next.multiviewLayout)) throw apiError("invalid_input", "Unknown layout.");
      next.multiviewOrder = [...new Set(next.multiviewOrder)].filter((id) => typeof id === "string");
      settings = next;
      persistSettings();
      return clone(settings);
    },

    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
  };
}

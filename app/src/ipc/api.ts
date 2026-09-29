// The contract between the UI and the Rust backend.
//
// Every screen talks to `BacksightApi` only. There are two implementations:
// `mock/` (browser development, tests) and `tauri.ts` (the real backend).
// Keep this file free of implementation details.

export type CameraId = string;
/** RFC 3339 timestamp in UTC, e.g. "2026-09-29T08:15:00Z". */
export type IsoDateTime = string;
/** Calendar date in the camera's local time zone, "YYYY-MM-DD". */
export type LocalDate = string;
/** Calendar month, "YYYY-MM". */
export type Month = string;

export type CameraState =
  | "online"
  | "connecting"
  | "offline"
  /** Privacy mode is on: the camera refuses to stream or record. */
  | "privacy"
  /** Too many failed logins; the camera refuses logins until `lockedUntil`. */
  | "locked"
  /** The stored password was rejected. */
  | "auth_failed"
  /** Firmware or model we can't talk to yet. */
  | "unsupported";

export interface CameraStatus {
  state: CameraState;
  message?: string;
  lockedUntil?: IsoDateTime;
  lastSeen?: IsoDateTime;
}

export type VideoCodecName = "h264" | "h265";

export interface StorageInfo {
  present: boolean;
  status: "normal" | "unformatted" | "full" | "error" | "none";
  totalBytes: number;
  freeBytes: number;
  recordingMode?: "continuous" | "detection" | "off";
  /** The card overwrites its oldest footage when full, so a full card is normal. */
  loopRecording?: boolean;
}

export interface Camera {
  id: CameraId;
  name: string;
  host: string;
  model?: string;
  firmware?: string;
  mac?: string;
  groupIds: string[];
  favorite: boolean;
  /** Whether an RTSP "Camera Account" is stored (enables RTSP live view). */
  hasCameraAccount: boolean;
  status: CameraStatus;
  storage?: StorageInfo;
  videoCodec?: VideoCodecName;
  /** The camera clock's offset from UTC, in minutes. */
  utcOffsetMinutes?: number;
  /** IANA zone reported by the camera, if any. */
  timeZone?: string;
  /** URL of the camera's preview: the most recent frame the player captured, if any. */
  snapshotUrl?: string;
  /** When the preview was captured. */
  snapshotAt?: IsoDateTime;
}

export interface CameraGroup {
  id: string;
  name: string;
}

export type DeviceKind = "camera" | "doorbell" | "hub" | "other";
export type LoginScheme = "legacy" | "secure" | "tpap" | "unknown";

export interface DiscoveredDevice {
  host: string;
  mac?: string;
  model?: string;
  name?: string;
  firmware?: string;
  kind: DeviceKind;
  loginScheme?: LoginScheme;
  alreadyAdded: boolean;
}

export interface CameraAccount {
  username: string;
  password: string;
}

export interface AddCameraRequest {
  host: string;
  name?: string;
  /** The owner's TP-Link account password. Stored in the OS keychain, never in app data. */
  cloudPassword: string;
  cameraAccount?: CameraAccount;
  groupIds?: string[];
}

export interface UpdateCameraRequest {
  name?: string;
  groupIds?: string[];
  favorite?: boolean;
  cloudPassword?: string;
  /** `null` removes the stored camera account. */
  cameraAccount?: CameraAccount | null;
}

export type RecordingKind = "continuous" | "detection";

export interface RecordingSegment {
  start: IsoDateTime;
  end: IsoDateTime;
  kind: RecordingKind;
}

export type EventType =
  | "motion"
  | "person"
  | "vehicle"
  | "pet"
  | "baby_cry"
  | "line_crossing"
  | "area_intrusion"
  | "tamper"
  | "sound"
  | "doorbell"
  | "other";

export interface DetectionEvent {
  id: string;
  start: IsoDateTime;
  end: IsoDateTime;
  types: EventType[];
  thumbnailUrl?: string;
}

export interface DayIndex {
  cameraId: CameraId;
  date: LocalDate;
  segments: RecordingSegment[];
  events: DetectionEvent[];
}

export type StreamQuality = "hd" | "sd";

export type StreamRequest =
  | { kind: "live"; cameraId: CameraId; quality: StreamQuality }
  | {
      kind: "playback";
      cameraId: CameraId;
      start: IsoDateTime;
      /** 1 = real time. Supported: 0.5, 1, 2, 4, 8, 16. */
      speed: number;
    };

export interface StreamHandle {
  id: string;
  close(): Promise<void>;
}

/**
 * Receives one batch of media packets (see docs/ARCHITECTURE.md, "Wire format").
 * The buffer is owned by the listener and may be transferred to a Worker.
 */
export type BatchListener = (batch: ArrayBuffer) => void;

export interface ExportRequest {
  cameraId: CameraId;
  start: IsoDateTime;
  end: IsoDateTime;
  /** Defaults to the export folder from settings. */
  outputPath?: string;
}

export type ExportState = "queued" | "running" | "paused" | "done" | "failed" | "cancelled";

export interface ExportJob {
  id: string;
  cameraId: CameraId;
  cameraName: string;
  start: IsoDateTime;
  end: IsoDateTime;
  state: ExportState;
  /** 0..1 */
  progress: number;
  bytesWritten: number;
  etaSeconds?: number;
  outputPath?: string;
  error?: string;
  createdAt: IsoDateTime;
}

export type MultiviewLayout = "1" | "2" | "4" | "1+5" | "9" | "16";

export interface Settings {
  theme: "system" | "light" | "dark";
  exportDir: string;
  /** Tokens: {camera}, {date}, {start}, {end}. */
  exportNameTemplate: string;
  cacheLimitMb: number;
  defaultLiveQuality: StreamQuality;
  multiviewLayout: MultiviewLayout;
  /** Camera ids in multi-view order. */
  multiviewOrder: CameraId[];
}

export type AppEvent =
  | { type: "camera-status"; cameraId: CameraId; status: CameraStatus }
  | { type: "cameras-changed" }
  /** What the status poll reads besides the state; an absent field is unknown. */
  | { type: "camera-info"; cameraId: CameraId; storage?: StorageInfo; utcOffsetMinutes?: number }
  | { type: "camera-preview"; cameraId: CameraId; snapshotUrl: string; snapshotAt: IsoDateTime }
  | { type: "export-progress"; job: ExportJob };

export type ApiErrorCode =
  | "camera_locked"
  | "auth_failed"
  | "third_party_compat_off"
  | "playback_busy"
  | "stream_limit"
  | "offline"
  | "privacy_mode"
  | "unsupported"
  | "not_found"
  | "invalid_input"
  | "internal";

/** Shape of every rejected promise from `BacksightApi`. */
export interface ApiError {
  code: ApiErrorCode;
  message: string;
  /** For `camera_locked`: when logins are accepted again. */
  retryAt?: IsoDateTime;
}

export interface BacksightApi {
  listCameras(): Promise<Camera[]>;
  getCamera(id: CameraId): Promise<Camera>;
  /** Scans the LAN for Tapo devices. */
  discover(timeoutMs?: number): Promise<DiscoveredDevice[]>;
  /** Logs in once to verify the credentials, then saves the camera. */
  addCamera(req: AddCameraRequest): Promise<Camera>;
  updateCamera(id: CameraId, req: UpdateCameraRequest): Promise<Camera>;
  removeCamera(id: CameraId): Promise<void>;

  listGroups(): Promise<CameraGroup[]>;
  saveGroups(groups: CameraGroup[]): Promise<void>;

  /** Dates in `month` that have any recording. */
  getDaysWithRecordings(cameraId: CameraId, month: Month): Promise<LocalDate[]>;
  getDayIndex(cameraId: CameraId, date: LocalDate): Promise<DayIndex>;

  openStream(req: StreamRequest, onBatch: BatchListener): Promise<StreamHandle>;

  startExport(req: ExportRequest): Promise<ExportJob>;
  listExports(): Promise<ExportJob[]>;
  cancelExport(id: string): Promise<void>;
  /** Shows the exported file in the OS file manager. */
  revealExport(id: string): Promise<void>;

  /** Saves a PNG snapshot; resolves to the saved file path. */
  saveSnapshot(cameraId: CameraId, png: Blob): Promise<string>;
  /** Stores a JPEG of the camera's current picture as its preview (followed by `camera-preview`). */
  savePreview(cameraId: CameraId, jpeg: Blob): Promise<void>;

  getSettings(): Promise<Settings>;
  updateSettings(patch: Partial<Settings>): Promise<Settings>;

  /** Subscribes to backend events; returns an unsubscribe function. */
  subscribe(listener: (event: AppEvent) => void): () => void;
}

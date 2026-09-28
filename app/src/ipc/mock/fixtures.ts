import type { CameraGroup, CameraState, EventType, StorageInfo, VideoCodecName } from "../api";

export type SceneKind = "porch" | "driveway" | "living" | "nursery" | "garage" | "yard" | "gate";

/** Relative event activity per camera-local hour, 0–23. */
type HourlyActivity = readonly number[];

const OUTDOOR_ACTIVITY: HourlyActivity = [
  0.25, 0.15, 0.12, 0.12, 0.18, 0.35, 0.9, 1.6, 1.9, 1.2, 0.9, 0.9, 1.2, 1.0, 0.9, 1.1, 1.5, 2.0,
  1.9, 1.5, 1.0, 0.7, 0.5, 0.35,
];
const INDOOR_ACTIVITY: HourlyActivity = [
  0.08, 0.05, 0.05, 0.05, 0.05, 0.1, 0.5, 1.4, 1.6, 0.8, 0.5, 0.5, 0.9, 0.7, 0.5, 0.6, 0.9, 1.4,
  1.9, 2.0, 1.8, 1.3, 0.7, 0.25,
];
const NURSERY_ACTIVITY: HourlyActivity = [
  0.6, 0.4, 0.9, 0.5, 0.4, 0.9, 1.3, 1.2, 0.7, 0.6, 0.8, 0.6, 1.0, 1.3, 0.9, 0.6, 0.6, 0.8, 1.0,
  1.4, 1.2, 0.8, 0.6, 0.7,
];

export interface MockCameraFixture {
  id: string;
  name: string;
  host: string;
  model: string;
  firmware: string;
  mac: string;
  groupIds: string[];
  favorite: boolean;
  hasCameraAccount: boolean;
  videoCodec: VideoCodecName;
  state: CameraState;
  /** Minutes since the camera was last seen, for offline cameras. */
  lastSeenMinutesAgo?: number;
  /** Minutes until logins are accepted again, for locked cameras. */
  lockedForMinutes?: number;
  storage?: Omit<StorageInfo, "freeBytes"> & { usedFraction: number };
  scene: SceneKind;
  /** Whether a cached snapshot exists. */
  hasSnapshot: boolean;
  activity: HourlyActivity;
  eventWeights: readonly (readonly [EventType, number])[];
}

const GB = 1_000_000_000;

export const MOCK_GROUPS: CameraGroup[] = [
  { id: "indoor", name: "Indoor" },
  { id: "outdoor", name: "Outdoor" },
];

/**
 * Seven cameras: five online across the C200, C210, C220, C320WS and C520WS, one offline and
 * one locked out after failed logins.
 */
export const MOCK_CAMERAS: MockCameraFixture[] = [
  {
    id: "cam-front-door",
    name: "Front Door",
    host: "192.168.1.41",
    model: "C520WS",
    firmware: "1.2.8 Build 240517",
    mac: "5C:62:8B:4A:17:E2",
    groupIds: ["outdoor"],
    favorite: true,
    hasCameraAccount: true,
    videoCodec: "h264",
    state: "online",
    storage: { present: true, status: "normal", totalBytes: 128 * GB, usedFraction: 0.64, recordingMode: "continuous" },
    scene: "porch",
    hasSnapshot: true,
    activity: OUTDOOR_ACTIVITY,
    eventWeights: [
      ["person", 5],
      ["motion", 3.5],
      ["pet", 0.8],
      ["line_crossing", 1],
      ["sound", 0.6],
    ],
  },
  {
    id: "cam-driveway",
    name: "Driveway",
    host: "192.168.1.42",
    model: "C320WS",
    firmware: "1.3.2 Build 240221",
    mac: "5C:62:8B:4A:22:9C",
    groupIds: ["outdoor"],
    favorite: false,
    hasCameraAccount: false,
    videoCodec: "h264",
    state: "online",
    storage: { present: true, status: "normal", totalBytes: 64 * GB, usedFraction: 0.91, recordingMode: "continuous" },
    scene: "driveway",
    hasSnapshot: true,
    activity: OUTDOOR_ACTIVITY,
    eventWeights: [
      ["vehicle", 4.5],
      ["person", 3],
      ["motion", 3],
      ["line_crossing", 1.2],
      ["area_intrusion", 1],
    ],
  },
  {
    id: "cam-living-room",
    name: "Living Room",
    host: "192.168.1.51",
    model: "C210",
    firmware: "1.4.3 Build 231012",
    mac: "3C:52:A1:0E:6B:14",
    groupIds: ["indoor"],
    favorite: true,
    hasCameraAccount: true,
    videoCodec: "h264",
    state: "online",
    storage: { present: true, status: "normal", totalBytes: 128 * GB, usedFraction: 0.38, recordingMode: "continuous" },
    scene: "living",
    hasSnapshot: true,
    activity: INDOOR_ACTIVITY,
    eventWeights: [
      ["motion", 4],
      ["person", 3],
      ["pet", 3],
      ["sound", 1.4],
    ],
  },
  {
    id: "cam-nursery",
    name: "Nursery",
    host: "192.168.1.52",
    model: "C200",
    firmware: "1.3.14 Build 240304",
    mac: "3C:52:A1:0E:7F:30",
    groupIds: ["indoor"],
    favorite: true,
    hasCameraAccount: false,
    videoCodec: "h264",
    state: "online",
    storage: { present: true, status: "normal", totalBytes: 32 * GB, usedFraction: 0.22, recordingMode: "detection" },
    scene: "nursery",
    hasSnapshot: true,
    activity: NURSERY_ACTIVITY,
    eventWeights: [
      ["baby_cry", 4],
      ["motion", 3],
      ["sound", 2],
      ["person", 1.2],
    ],
  },
  {
    id: "cam-garage",
    name: "Garage",
    host: "192.168.1.53",
    model: "C220",
    firmware: "1.1.9 Build 240612",
    mac: "A8:42:A1:31:C4:08",
    groupIds: ["indoor"],
    favorite: false,
    hasCameraAccount: false,
    videoCodec: "h265",
    state: "online",
    storage: { present: true, status: "normal", totalBytes: 256 * GB, usedFraction: 0.17, recordingMode: "continuous" },
    scene: "garage",
    hasSnapshot: true,
    activity: INDOOR_ACTIVITY,
    eventWeights: [
      ["motion", 3],
      ["person", 2.2],
      ["vehicle", 2],
      ["sound", 1],
      ["tamper", 0.25],
    ],
  },
  {
    id: "cam-backyard",
    name: "Backyard",
    host: "192.168.1.43",
    model: "C320WS",
    firmware: "1.3.2 Build 240221",
    mac: "5C:62:8B:4A:31:05",
    groupIds: ["outdoor"],
    favorite: false,
    hasCameraAccount: false,
    videoCodec: "h264",
    state: "offline",
    lastSeenMinutesAgo: 133,
    storage: { present: true, status: "normal", totalBytes: 64 * GB, usedFraction: 0.55, recordingMode: "continuous" },
    scene: "yard",
    hasSnapshot: false,
    activity: OUTDOOR_ACTIVITY,
    eventWeights: [
      ["pet", 2],
      ["motion", 3],
      ["person", 2],
      ["area_intrusion", 1],
    ],
  },
  {
    id: "cam-side-gate",
    name: "Side Gate",
    host: "192.168.1.44",
    model: "C520WS",
    firmware: "1.2.8 Build 240517",
    mac: "5C:62:8B:4A:3A:71",
    groupIds: ["outdoor"],
    favorite: false,
    hasCameraAccount: false,
    videoCodec: "h264",
    state: "locked",
    lockedForMinutes: 23,
    storage: { present: true, status: "normal", totalBytes: 128 * GB, usedFraction: 0.47, recordingMode: "continuous" },
    scene: "gate",
    hasSnapshot: true,
    activity: OUTDOOR_ACTIVITY,
    eventWeights: [
      ["person", 3],
      ["motion", 3],
      ["line_crossing", 2],
      ["tamper", 0.3],
    ],
  },
];

/** A profile for a camera added at runtime, so it gets recordings and events too. */
export function newCameraFixture(device: {
  id: string;
  host: string;
  name: string;
  model?: string;
  firmware?: string;
  mac?: string;
}): MockCameraFixture {
  const known = MOCK_CAMERAS.find((c) => c.host === device.host);
  if (known) return { ...known, id: device.id, name: device.name, state: "online", hasSnapshot: true, favorite: false };
  const model = device.model ?? "C210";
  const outdoor = /^C(3|5)/.test(model);
  return {
    id: device.id,
    name: device.name,
    host: device.host,
    model,
    firmware: device.firmware ?? "1.0.0",
    mac: device.mac ?? "00:00:00:00:00:00",
    groupIds: [],
    favorite: false,
    hasCameraAccount: false,
    videoCodec: "h264",
    state: "online",
    storage: { present: true, status: "normal", totalBytes: 64 * GB, usedFraction: 0.04, recordingMode: "continuous" },
    scene: outdoor ? "yard" : "living",
    hasSnapshot: true,
    activity: outdoor ? OUTDOOR_ACTIVITY : INDOOR_ACTIVITY,
    eventWeights: outdoor
      ? [
          ["person", 3],
          ["motion", 3],
          ["vehicle", 1],
        ]
      : [
          ["motion", 3],
          ["person", 2],
          ["pet", 1.5],
        ],
  };
}

/** Devices the LAN scan finds that aren't added yet. */
export const MOCK_NEW_DEVICES = [
  {
    host: "192.168.1.60",
    mac: "3C:52:A1:0F:02:5D",
    model: "C210",
    name: "Kitchen",
    firmware: "1.4.3 Build 231012",
    kind: "camera" as const,
    loginScheme: "secure" as const,
  },
  {
    host: "192.168.1.61",
    mac: "A8:42:A1:33:18:C2",
    model: "C225",
    name: "Hallway",
    firmware: "1.0.12 Build 240430",
    kind: "camera" as const,
    loginScheme: "tpap" as const,
  },
  {
    host: "192.168.1.62",
    mac: "B0:19:21:7E:44:A0",
    model: "D230",
    name: "Doorbell",
    firmware: "1.1.6 Build 240318",
    kind: "doorbell" as const,
    loginScheme: "secure" as const,
  },
  {
    host: "192.168.1.70",
    mac: "B0:19:21:12:9B:01",
    model: "H200",
    name: "Tapo Hub",
    firmware: "1.3.0 Build 240109",
    kind: "hub" as const,
    loginScheme: "legacy" as const,
  },
];

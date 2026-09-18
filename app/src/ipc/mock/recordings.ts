// Deterministic SD-card contents for mock cameras. A day is generated in full from a seed of
// (camera, date); "today" is the same day cut off at the current time, so recordings and events
// already shown never move as time passes.

import type { DayIndex, DetectionEvent, EventType, LocalDate, Month, RecordingSegment } from "../api";
import {
  addMonths,
  dayRange,
  daysInMonth,
  formatLocalDate,
  localParts,
  MINUTE,
  monthOf,
  SECOND,
  toIso,
  todayIn,
} from "@/lib/time";
import type { MockCameraFixture } from "./fixtures";
import { Rng } from "./prng";
import { eventThumbnailDataUrl, isNightHour } from "./thumbnails";

interface Span {
  start: number;
  end: number;
}

interface GeneratedEvent extends Span {
  types: EventType[];
}

interface GeneratedDay {
  segments: (Span & { kind: RecordingSegment["kind"] })[];
  events: GeneratedEvent[];
}

/**
 * The SD card holds footage from the first day of the previous month until today, minus a
 * couple of days per month when the camera was unplugged.
 */
export function hasRecordingsOn(camera: MockCameraFixture, date: LocalDate, offsetMinutes: number, now: number): boolean {
  const today = todayIn(offsetMinutes, now);
  if (date > today) return false;
  const retentionStart = `${addMonths(monthOf(today), -1)}-01`;
  if (date < retentionStart) return false;
  if (date === today) return true;
  return !missingDays(camera, monthOf(date)).has(date);
}

function missingDays(camera: MockCameraFixture, month: Month): Set<LocalDate> {
  const rng = new Rng(`${camera.id}|missing|${month}`);
  const [y, m] = month.split("-").map(Number);
  const count = rng.int(1, 2);
  const out = new Set<LocalDate>();
  for (let i = 0; i < count; i++) out.add(formatLocalDate(y, m, rng.int(1, daysInMonth(month))));
  return out;
}

export function daysWithRecordings(
  camera: MockCameraFixture,
  month: Month,
  offsetMinutes: number,
  now: number,
): LocalDate[] {
  const [y, m] = month.split("-").map(Number);
  const out: LocalDate[] = [];
  for (let d = 1; d <= daysInMonth(month); d++) {
    const date = formatLocalDate(y, m, d);
    if (hasRecordingsOn(camera, date, offsetMinutes, now)) out.push(date);
  }
  return out;
}

function generateFullDay(camera: MockCameraFixture, date: LocalDate, offsetMinutes: number): GeneratedDay {
  const rng = new Rng(`${camera.id}|${date}`);
  const { start: dayStart, end: dayEnd } = dayRange(date, offsetMinutes);
  const continuous = camera.storage?.recordingMode === "continuous";

  // Continuous footage: long stretches separated by short gaps (reboots, Wi-Fi drops) and the
  // occasional long one (power cut).
  const recorded: Span[] = [];
  if (continuous) {
    let t = dayStart;
    while (t < dayEnd) {
      const length = rng.weighted([
        [rng.range(20, 60), 2],
        [rng.range(60, 180), 5],
        [rng.range(180, 320), 1],
      ] as const) * MINUTE;
      const end = Math.min(dayEnd, t + length);
      recorded.push({ start: t, end });
      const gap = rng.weighted([
        [rng.range(4, 90) * SECOND, 70],
        [rng.range(2, 12) * MINUTE, 24],
        [rng.range(20, 75) * MINUTE, 6],
      ] as const);
      t = end + gap;
    }
  }

  // Detection events, spread over the day by the camera's hourly activity.
  const target = rng.int(20, 80);
  const hourWeights = camera.activity.map((w, hour) => [hour, w] as const);
  const candidates: GeneratedEvent[] = [];
  let attempts = 0;
  while (candidates.length < target && attempts < target * 8) {
    attempts++;
    const hour = rng.weighted(hourWeights);
    const start = dayStart + hour * 60 * MINUTE + Math.floor(rng.range(0, 60 * MINUTE) / SECOND) * SECOND;
    const duration = Math.round(rng.logRange(6, 150)) * SECOND;
    let end = start + duration;
    if (continuous) {
      const seg = recorded.find((s) => start >= s.start && start < s.end);
      if (!seg || seg.end - start < 5 * SECOND) continue;
      end = Math.min(end, seg.end);
    } else {
      end = Math.min(end, dayEnd);
    }
    const primary = rng.weighted(camera.eventWeights);
    const types: EventType[] = [primary];
    if ((primary === "person" || primary === "vehicle" || primary === "pet") && rng.chance(0.3)) types.push("motion");
    else if (primary !== "motion" && rng.chance(0.08)) {
      const extra = rng.weighted(camera.eventWeights);
      if (!types.includes(extra)) types.push(extra);
    }
    candidates.push({ start, end, types });
  }

  // Keep events apart so each is a distinct bar and card.
  candidates.sort((a, b) => a.start - b.start);
  const events: GeneratedEvent[] = [];
  for (const e of candidates) {
    const prev = events[events.length - 1];
    if (prev && e.start < prev.end + 8 * SECOND) continue;
    events.push(e);
  }

  const segments: GeneratedDay["segments"] = continuous
    ? recorded.map((s) => ({ ...s, kind: "continuous" as const }))
    : mergeSpans(events.map((e) => ({ start: e.start - 3 * SECOND, end: e.end + 5 * SECOND }))).map((s) => ({
        start: Math.max(dayStart, s.start),
        end: Math.min(dayEnd, s.end),
        kind: "detection" as const,
      }));

  return { segments, events };
}

function mergeSpans(spans: Span[]): Span[] {
  const sorted = [...spans].sort((a, b) => a.start - b.start);
  const out: Span[] = [];
  for (const s of sorted) {
    const last = out[out.length - 1];
    if (last && s.start <= last.end) last.end = Math.max(last.end, s.end);
    else out.push({ ...s });
  }
  return out;
}

const fullDayCache = new Map<string, GeneratedDay>();

function fullDay(camera: MockCameraFixture, date: LocalDate, offsetMinutes: number): GeneratedDay {
  const key = `${camera.id}|${date}|${offsetMinutes}`;
  let day = fullDayCache.get(key);
  if (!day) {
    day = generateFullDay(camera, date, offsetMinutes);
    fullDayCache.set(key, day);
    if (fullDayCache.size > 200) fullDayCache.delete(fullDayCache.keys().next().value!);
  }
  return day;
}

const thumbnailCache = new Map<string, string>();

function thumbnailFor(camera: MockCameraFixture, event: GeneratedEvent, offsetMinutes: number, id: string): string {
  let url = thumbnailCache.get(id);
  if (!url) {
    const night = isNightHour(localParts(event.start, offsetMinutes).hour);
    url = eventThumbnailDataUrl(camera.scene, event.types[0], night, id);
    thumbnailCache.set(id, url);
    if (thumbnailCache.size > 2000) thumbnailCache.delete(thumbnailCache.keys().next().value!);
  }
  return url;
}

export function generateDayIndex(
  camera: MockCameraFixture,
  date: LocalDate,
  offsetMinutes: number,
  now: number,
): DayIndex {
  if (!hasRecordingsOn(camera, date, offsetMinutes, now)) {
    return { cameraId: camera.id, date, segments: [], events: [] };
  }
  const day = fullDay(camera, date, offsetMinutes);
  // Footage reaches the card a few seconds after it happens.
  const limit = now - 15 * SECOND;

  const segments: RecordingSegment[] = [];
  for (const s of day.segments) {
    if (s.start >= limit) break;
    segments.push({ start: toIso(s.start), end: toIso(Math.min(s.end, limit)), kind: s.kind });
  }

  const events: DetectionEvent[] = [];
  day.events.forEach((e, i) => {
    if (e.end > limit) return;
    const id = `${camera.id}:${date}:${i}`;
    events.push({
      id,
      start: toIso(e.start),
      end: toIso(e.end),
      types: [...e.types],
      thumbnailUrl: thumbnailFor(camera, e, offsetMinutes, id),
    });
  });

  return { cameraId: camera.id, date, segments, events };
}

/** Whether any footage exists in [start, end), used to fail exports of empty ranges. */
export function hasFootageBetween(
  camera: MockCameraFixture,
  start: number,
  end: number,
  offsetMinutes: number,
  now: number,
): boolean {
  let date = todayIn(offsetMinutes, start);
  for (let i = 0; i < 3; i++) {
    const index = generateDayIndex(camera, date, offsetMinutes, now);
    if (index.segments.some((s) => Date.parse(s.start) < end && Date.parse(s.end) > start)) return true;
    const next = dayRange(date, offsetMinutes).end;
    if (next >= end) break;
    date = todayIn(offsetMinutes, next);
  }
  return false;
}

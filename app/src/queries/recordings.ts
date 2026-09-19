import { useQuery } from "@tanstack/react-query";
import { api, type CameraId, type DayIndex, type EventType, type LocalDate, type Month, type RecordingKind } from "@/ipc";
import { queryKeys } from "./keys";

export interface ParsedSegment {
  start: number;
  end: number;
  kind: RecordingKind;
}

export interface ParsedEvent {
  id: string;
  start: number;
  end: number;
  types: EventType[];
  thumbnailUrl?: string;
}

export interface ParsedDayIndex {
  date: LocalDate;
  segments: ParsedSegment[];
  events: ParsedEvent[];
}

/** ISO strings → epoch ms once, so the timeline never parses dates while drawing. */
export function parseDayIndex(index: DayIndex): ParsedDayIndex {
  return {
    date: index.date,
    segments: index.segments
      .map((s) => ({ start: Date.parse(s.start), end: Date.parse(s.end), kind: s.kind }))
      .sort((a, b) => a.start - b.start),
    events: index.events
      .map((e) => ({
        id: e.id,
        start: Date.parse(e.start),
        end: Date.parse(e.end),
        types: e.types,
        thumbnailUrl: e.thumbnailUrl,
      }))
      .sort((a, b) => a.start - b.start),
  };
}

const toSet = (days: LocalDate[]) => new Set(days);

export function useDaysWithRecordings(cameraId: CameraId, month: Month, enabled = true) {
  return useQuery({
    queryKey: queryKeys.days(cameraId, month),
    queryFn: () => api.getDaysWithRecordings(cameraId, month),
    select: toSet,
    staleTime: 5 * 60_000,
    enabled,
  });
}

export function useDayIndex(cameraId: CameraId, date: LocalDate, options: { enabled?: boolean; isToday?: boolean } = {}) {
  const { enabled = true, isToday = false } = options;
  return useQuery({
    queryKey: queryKeys.dayIndex(cameraId, date),
    queryFn: () => api.getDayIndex(cameraId, date),
    select: parseDayIndex,
    // Today keeps growing; past days are immutable.
    staleTime: isToday ? 30_000 : 30 * 60_000,
    refetchInterval: isToday ? 60_000 : false,
    enabled,
  });
}

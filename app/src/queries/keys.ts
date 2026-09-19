import type { CameraId, LocalDate, Month } from "@/ipc";

export const queryKeys = {
  cameras: () => ["cameras"] as const,
  camera: (id: CameraId) => ["cameras", id] as const,
  groups: () => ["groups"] as const,
  days: (id: CameraId, month: Month) => ["recordings", id, "days", month] as const,
  dayIndex: (id: CameraId, date: LocalDate) => ["recordings", id, "day", date] as const,
  exports: () => ["exports"] as const,
  settings: () => ["settings"] as const,
};

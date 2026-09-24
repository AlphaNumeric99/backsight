import { describe, expect, it } from "vitest";
import {
  DAY,
  HOUR,
  MINUTE,
  addDays,
  addMonths,
  dayRange,
  dayStartMs,
  daysInMonth,
  formatClockDuration,
  formatHms,
  formatStamp,
  formatUtcOffset,
  isLocalDate,
  localDateOf,
  minutesUntil,
  monthGrid,
  toIso,
} from "./time";

describe("camera-local time", () => {
  // 2026-09-29 22:30 UTC
  const t = Date.parse("2026-09-29T22:30:00Z");

  it("finds the local date on either side of midnight", () => {
    expect(localDateOf(t, 0)).toBe("2026-09-29");
    expect(localDateOf(t, 120)).toBe("2026-09-30"); // 00:30 next day at UTC+2
    expect(localDateOf(t, -300)).toBe("2026-09-29"); // 17:30 at UTC-5
    expect(localDateOf(t, 90)).toBe("2026-09-30");
  });

  it("computes local midnight in UTC", () => {
    expect(dayStartMs("2026-09-30", 120)).toBe(Date.parse("2026-09-29T22:00:00Z"));
    expect(dayStartMs("2026-09-30", -330)).toBe(Date.parse("2026-09-30T05:30:00Z"));
    const r = dayRange("2026-03-29", 60);
    expect(r.end - r.start).toBe(DAY);
  });

  it("formats wall-clock time in the camera's zone, 24-hour", () => {
    expect(formatHms(t, 120)).toBe("00:30:00");
    expect(formatStamp(t + 7 * MINUTE + 5000, -300)).toBe("2026-09-29 17:37:05");
  });

  it("does calendar arithmetic across months and years", () => {
    expect(addDays("2026-09-30", 1)).toBe("2026-10-01");
    expect(addDays("2026-01-01", -1)).toBe("2025-12-31");
    expect(addMonths("2026-01", -1)).toBe("2025-12");
    expect(addMonths("2026-11", 3)).toBe("2027-02");
    expect(daysInMonth("2028-02")).toBe(29);
  });

  it("validates dates", () => {
    expect(isLocalDate("2026-09-29")).toBe(true);
    expect(isLocalDate("2026-02-30")).toBe(false);
    expect(isLocalDate("29/09/2026")).toBe(false);
    expect(isLocalDate(undefined)).toBe(false);
  });

  it("lays out a month grid starting on Monday or Sunday", () => {
    const monday = monthGrid("2026-09", 1); // 1 Sep 2026 is a Tuesday
    expect(monday[0]).toEqual([null, "2026-09-01", "2026-09-02", "2026-09-03", "2026-09-04", "2026-09-05", "2026-09-06"]);
    expect(monday.every((w) => w.length === 7)).toBe(true);
    const sunday = monthGrid("2026-09", 0);
    expect(sunday[0][2]).toBe("2026-09-01");
    expect(sunday.flat().filter(Boolean)).toHaveLength(30);
  });

  it("formats durations as clocks", () => {
    expect(formatClockDuration(42_000)).toBe("00:42");
    expect(formatClockDuration(6 * MINUTE + 4000)).toBe("06:04");
    expect(formatClockDuration(HOUR + 2 * MINUTE + 3000)).toBe("1:02:03");
    expect(formatClockDuration(-5)).toBe("00:00");
  });

  it("formats offsets and ISO strings", () => {
    expect(formatUtcOffset(330)).toBe("UTC+05:30");
    expect(formatUtcOffset(-300)).toBe("UTC−05:00");
    expect(toIso(Date.parse("2026-09-29T08:15:00Z"))).toBe("2026-09-29T08:15:00Z");
    expect(toIso(Date.parse("2026-09-29T08:15:00.250Z"))).toBe("2026-09-29T08:15:00.250Z");
  });

  it("counts down to a time to the nearest minute, never early", () => {
    const now = Date.parse("2026-09-29T10:00:00Z");
    expect(minutesUntil("2026-09-29T10:23:00Z", now)).toBe(23);
    expect(minutesUntil("2026-09-29T10:23:00Z", now + 400)).toBe(23);
    expect(minutesUntil("2026-09-29T10:00:20Z", now)).toBe(1);
    expect(minutesUntil("2026-09-29T09:59:00Z", now)).toBe(0);
  });
});

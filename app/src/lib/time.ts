// Time helpers. The backend speaks UTC (ISO strings); screens show camera-local time, which is
// UTC shifted by the camera's `utcOffsetMinutes`. A "shifted" epoch (utc + offset) lets us read
// local wall-clock fields with the getUTC* accessors, independent of the viewer's time zone.

import type { IsoDateTime, LocalDate, Month } from "@/ipc/api";

export const SECOND = 1000;
export const MINUTE = 60 * SECOND;
export const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;

/** The viewer's current UTC offset in minutes; the fallback when a camera doesn't report one. */
export function localOffsetMinutes(at: number = Date.now()): number {
  return -new Date(at).getTimezoneOffset();
}

export function parseIso(iso: IsoDateTime): number {
  return Date.parse(iso);
}

/** RFC 3339 in UTC, without a fractional part when it is zero: "2026-09-29T08:15:00Z". */
export function toIso(ms: number): IsoDateTime {
  const s = new Date(ms).toISOString();
  return s.endsWith(".000Z") ? `${s.slice(0, -5)}Z` : s;
}

export interface LocalParts {
  year: number;
  /** 1–12 */
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
  /** 0 = Sunday */
  weekday: number;
}

export function localParts(ms: number, offsetMinutes: number): LocalParts {
  const d = new Date(ms + offsetMinutes * MINUTE);
  return {
    year: d.getUTCFullYear(),
    month: d.getUTCMonth() + 1,
    day: d.getUTCDate(),
    hour: d.getUTCHours(),
    minute: d.getUTCMinutes(),
    second: d.getUTCSeconds(),
    weekday: d.getUTCDay(),
  };
}

const pad2 = (n: number) => String(n).padStart(2, "0");

export function formatLocalDate(year: number, month: number, day: number): LocalDate {
  return `${year}-${pad2(month)}-${pad2(day)}`;
}

/** The camera-local calendar date of an instant. */
export function localDateOf(ms: number, offsetMinutes: number): LocalDate {
  const p = localParts(ms, offsetMinutes);
  return formatLocalDate(p.year, p.month, p.day);
}

export function parseLocalDate(date: LocalDate): { year: number; month: number; day: number } {
  const [y, m, d] = date.split("-").map(Number);
  return { year: y, month: m, day: d };
}

export function isLocalDate(value: unknown): value is LocalDate {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const { year, month, day } = parseLocalDate(value);
  const d = new Date(Date.UTC(year, month - 1, day));
  return d.getUTCFullYear() === year && d.getUTCMonth() === month - 1 && d.getUTCDate() === day;
}

/** UTC epoch ms of camera-local midnight at the start of `date`. */
export function dayStartMs(date: LocalDate, offsetMinutes: number): number {
  const { year, month, day } = parseLocalDate(date);
  return Date.UTC(year, month - 1, day) - offsetMinutes * MINUTE;
}

/** [start, end) of a camera-local day in UTC epoch ms. Always 24 h: a fixed offset has no DST. */
export function dayRange(date: LocalDate, offsetMinutes: number): { start: number; end: number } {
  const start = dayStartMs(date, offsetMinutes);
  return { start, end: start + DAY };
}

export function addDays(date: LocalDate, n: number): LocalDate {
  const { year, month, day } = parseLocalDate(date);
  const d = new Date(Date.UTC(year, month - 1, day + n));
  return formatLocalDate(d.getUTCFullYear(), d.getUTCMonth() + 1, d.getUTCDate());
}

export function todayIn(offsetMinutes: number, now: number = Date.now()): LocalDate {
  return localDateOf(now, offsetMinutes);
}

export function monthOf(date: LocalDate): Month {
  return date.slice(0, 7);
}

export function addMonths(month: Month, n: number): Month {
  const [y, m] = month.split("-").map(Number);
  const d = new Date(Date.UTC(y, m - 1 + n, 1));
  return `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}`;
}

export function daysInMonth(month: Month): number {
  const [y, m] = month.split("-").map(Number);
  return new Date(Date.UTC(y, m, 0)).getUTCDate();
}

/**
 * Weeks of a month for a calendar grid. Each week has 7 cells; cells outside the month are
 * `null`. `weekStartsOn` is 0 for Sunday, 1 for Monday.
 */
export function monthGrid(month: Month, weekStartsOn: 0 | 1 = 1): (LocalDate | null)[][] {
  const [y, m] = month.split("-").map(Number);
  const first = new Date(Date.UTC(y, m - 1, 1)).getUTCDay();
  const lead = (first - weekStartsOn + 7) % 7;
  const total = daysInMonth(month);
  const cells: (LocalDate | null)[] = [];
  for (let i = 0; i < lead; i++) cells.push(null);
  for (let d = 1; d <= total; d++) cells.push(formatLocalDate(y, m, d));
  while (cells.length % 7 !== 0) cells.push(null);
  const weeks: (LocalDate | null)[][] = [];
  for (let i = 0; i < cells.length; i += 7) weeks.push(cells.slice(i, i + 7));
  return weeks;
}

/** First day of the week for the viewer's locale (0 = Sunday, 1 = Monday). */
export function localeWeekStart(): 0 | 1 {
  try {
    const locale = new Intl.Locale(navigator.language) as Intl.Locale & {
      getWeekInfo?: () => { firstDay: number };
      weekInfo?: { firstDay: number };
    };
    const info = locale.getWeekInfo?.() ?? locale.weekInfo;
    if (info) return info.firstDay === 7 ? 0 : 1;
  } catch {
    // Older engines: fall through.
  }
  return 1;
}

// ---------------------------------------------------------------------------------------------
// Formatting. Wall-clock times use a 24-hour clock everywhere, like camera on-screen displays.

/** "14:03" */
export function formatHm(ms: number, offsetMinutes: number): string {
  const p = localParts(ms, offsetMinutes);
  return `${pad2(p.hour)}:${pad2(p.minute)}`;
}

/** "14:03:27" */
export function formatHms(ms: number, offsetMinutes: number): string {
  const p = localParts(ms, offsetMinutes);
  return `${pad2(p.hour)}:${pad2(p.minute)}:${pad2(p.second)}`;
}

/** "2026-09-29 14:03:27", like a camera's on-screen timestamp. */
export function formatStamp(ms: number, offsetMinutes: number): string {
  const p = localParts(ms, offsetMinutes);
  return `${formatLocalDate(p.year, p.month, p.day)} ${pad2(p.hour)}:${pad2(p.minute)}:${pad2(p.second)}`;
}

const dateFormatters = new Map<string, Intl.DateTimeFormat>();
function dateFormatter(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = JSON.stringify(options);
  let f = dateFormatters.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, { ...options, timeZone: "UTC" });
    dateFormatters.set(key, f);
  }
  return f;
}

/** A local date in the viewer's language, e.g. "Tue, 29 Sep". */
export function formatLocalDateLabel(
  date: LocalDate,
  options: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short" },
): string {
  const { year, month, day } = parseLocalDate(date);
  return dateFormatter(options).format(Date.UTC(year, month - 1, day));
}

/** "September 2026" */
export function formatMonthLabel(month: Month): string {
  const [y, m] = month.split("-").map(Number);
  return dateFormatter({ month: "long", year: "numeric" }).format(Date.UTC(y, m - 1, 1));
}

/** Narrow weekday names starting at `weekStartsOn`, e.g. ["M", "T", …]. */
export function weekdayLabels(weekStartsOn: 0 | 1, style: "narrow" | "short" = "narrow"): string[] {
  const f = dateFormatter({ weekday: style });
  // 2023-01-01 was a Sunday.
  return Array.from({ length: 7 }, (_, i) => f.format(Date.UTC(2023, 0, 1 + ((i + weekStartsOn) % 7))));
}

/**
 * A duration as a clock: "00:42", "06:04", "1:02:03". Minutes are always two digits so the
 * readout doesn't jump as it grows.
 */
export function formatClockDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / SECOND));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}:${pad2(m)}:${pad2(s)}` : `${pad2(m)}:${pad2(s)}`;
}

/** "UTC+02:00", "UTC−05:30" */
export function formatUtcOffset(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? "−" : "+";
  const abs = Math.abs(offsetMinutes);
  return `UTC${sign}${pad2(Math.floor(abs / 60))}:${pad2(abs % 60)}`;
}

/**
 * Minutes from `now` until `iso`, to the nearest minute: 0 once it has passed, and at least 1
 * while it hasn't, so a countdown never says "now" early.
 */
export function minutesUntil(iso: IsoDateTime, now: number = Date.now()): number {
  const mins = (parseIso(iso) - now) / MINUTE;
  return mins <= 0 ? 0 : Math.max(1, Math.round(mins));
}

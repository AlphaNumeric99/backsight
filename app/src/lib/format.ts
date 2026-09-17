import { strings } from "./strings";
import { DAY, HOUR, MINUTE } from "./time";

const numberFormats = new Map<number, Intl.NumberFormat>();
function num(maxFractionDigits: number): Intl.NumberFormat {
  let f = numberFormats.get(maxFractionDigits);
  if (!f) {
    f = new Intl.NumberFormat(undefined, { maximumFractionDigits: maxFractionDigits });
    numberFormats.set(maxFractionDigits, f);
  }
  return f;
}

/** Decimal units, like storage vendors and camera apps: "61.2 GB", "812 MB". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  const digits = value >= 100 || unit === 0 ? 0 : 1;
  return `${num(digits).format(value)} ${units[unit]}`;
}

/** "2.4 Mbps", "512 kbps" */
export function formatBitrate(bitsPerSecond: number): string {
  if (!Number.isFinite(bitsPerSecond) || bitsPerSecond <= 0) return "0 kbps";
  if (bitsPerSecond >= 1_000_000) return `${num(1).format(bitsPerSecond / 1_000_000)} Mbps`;
  return `${num(0).format(bitsPerSecond / 1000)} kbps`;
}

/** "42%" */
export function formatPercent(fraction: number): string {
  return `${Math.round(Math.min(1, Math.max(0, fraction)) * 100)}%`;
}

/** Remaining time for a job: "12 s", "about 3 min", "about 1 h 10 min". */
export function formatEta(seconds: number): string {
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))} s`;
  const mins = Math.round(seconds / 60);
  if (mins < 60) return `about ${mins} min`;
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return m === 0 ? `about ${h} h` : `about ${h} h ${m} min`;
}

/** "just now", "5 min ago", "2 h ago", "yesterday", "3 days ago" */
export function formatRelative(then: number, now: number = Date.now()): string {
  const diff = Math.max(0, now - then);
  if (diff < MINUTE) return strings.relative.justNow;
  if (diff < HOUR) return strings.relative.minutes(Math.floor(diff / MINUTE));
  if (diff < DAY) return strings.relative.hours(Math.floor(diff / HOUR));
  const days = Math.floor(diff / DAY);
  return days === 1 ? strings.relative.yesterday : strings.relative.days(days);
}

/** "1920 × 1080" */
export function formatResolution(width?: number, height?: number): string | undefined {
  return width && height ? `${width} × ${height}` : undefined;
}

/** A friendly name for a WebCodecs codec string: "avc1.64001F" → "H.264", "hvc1…" → "H.265". */
export function formatCodec(codec?: string): string | undefined {
  if (!codec) return undefined;
  if (/^(avc1|avc3)/i.test(codec)) return `H.264 (${codec})`;
  if (/^(hvc1|hev1)/i.test(codec)) return `H.265 (${codec})`;
  return codec;
}

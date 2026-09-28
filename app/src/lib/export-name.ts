import { localParts } from "./time";

export const EXPORT_NAME_TOKENS = ["camera", "date", "start", "end"] as const;
export type ExportNameToken = (typeof EXPORT_NAME_TOKENS)[number];

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Characters that aren't allowed in file names on Windows, macOS or Linux. */
const INVALID = /[\\/:*?"<>|\u0000-\u001f]/g;

export function sanitizeFileName(name: string): string {
  return name.replace(INVALID, "-").replace(/\s+/g, " ").replace(/^[\s.]+|[\s.]+$/g, "");
}

/**
 * Renders the export file name template from settings, e.g.
 * "{camera} {date} {start}-{end}" → "Front Door 2026-09-29 14.03.27-14.09.31.mp4".
 * Times are camera-local; colons become dots so the name is valid everywhere.
 */
export function renderExportName(
  template: string,
  values: { camera: string; start: number; end: number; offsetMinutes: number },
): string {
  const s = localParts(values.start, values.offsetMinutes);
  const e = localParts(values.end, values.offsetMinutes);
  const tokens: Record<ExportNameToken, string> = {
    camera: values.camera,
    date: `${s.year}-${pad2(s.month)}-${pad2(s.day)}`,
    start: `${pad2(s.hour)}.${pad2(s.minute)}.${pad2(s.second)}`,
    end: `${pad2(e.hour)}.${pad2(e.minute)}.${pad2(e.second)}`,
  };
  const body = template.replace(/\{(\w+)\}/g, (match, key: string) =>
    key in tokens ? tokens[key as ExportNameToken] : match,
  );
  const clean = sanitizeFileName(body) || sanitizeFileName(`${tokens.camera} ${tokens.date}`);
  return `${clean}.mp4`;
}

import type { Camera } from "@/ipc";
import type { SnapshotOptions } from "@/player/types";

/** How previews are encoded: small JPEGs, plenty for a card or a poster. */
export const PREVIEW_SNAPSHOT: SnapshotOptions = { type: "image/jpeg", quality: 0.82, maxWidth: 640 };

/** The Cameras page refreshes previews older than this. */
export const PREVIEW_STALE_MS = 10 * 60_000;
/** After an attempt, a camera isn't tried again for this long. */
export const PREVIEW_RETRY_MS = 10 * 60_000;

/** Whether the Cameras page should grab a fresh preview for `camera`. */
export function isPreviewDue(camera: Camera, now: number, lastAttemptAt?: number): boolean {
  if (camera.status.state !== "online") return false;
  if (lastAttemptAt !== undefined && now - lastAttemptAt < PREVIEW_RETRY_MS) return false;
  const at = camera.snapshotAt ? Date.parse(camera.snapshotAt) : NaN;
  return !Number.isFinite(at) || now - at >= PREVIEW_STALE_MS;
}

import { useEffect, useRef, useState } from "react";
import { api, type Camera, type CameraId } from "@/ipc";
import { capturePreview } from "./capture";
import { isPreviewDue } from "./policy";

/** How often the page looks for missing or stale previews. */
const CHECK_EVERY_MS = 60_000;
/** A short wait after the page opens, so its own requests go first. */
const START_DELAY_MS = 1500;

/** When each camera was last tried, across visits to the page. */
const lastAttempt = new Map<CameraId, number>();

/**
 * Keeps the Cameras page's previews fresh: for online cameras whose preview is missing or
 * stale, grabs a frame from the live stream, one camera at a time, while the page is visible.
 * Returns the camera being captured, if any.
 */
export function usePreviewRefresh(cameras: readonly Camera[]): CameraId | null {
  const [capturing, setCapturing] = useState<CameraId | null>(null);
  const camerasRef = useRef(cameras);
  const kickRef = useRef<() => void>(() => {});

  useEffect(() => {
    camerasRef.current = cameras;
  }, [cameras]);

  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let running = false;
    const schedule = (ms: number) => {
      clearTimeout(timer);
      timer = setTimeout(() => void run(), ms);
    };
    const run = async () => {
      if (running) return;
      running = true;
      try {
        while (!abort.signal.aborted && document.visibilityState === "visible") {
          const now = Date.now();
          const camera = camerasRef.current.find((c) => isPreviewDue(c, now, lastAttempt.get(c.id)));
          if (!camera) break;
          lastAttempt.set(camera.id, now);
          setCapturing(camera.id);
          try {
            await api.savePreview(camera.id, await capturePreview(camera.id, abort.signal));
          } catch (error) {
            if (!abort.signal.aborted) console.debug(`Backsight: no preview from ${camera.name}`, error);
          }
        }
      } finally {
        running = false;
        if (!abort.signal.aborted) {
          setCapturing(null);
          schedule(CHECK_EVERY_MS);
        }
      }
    };
    kickRef.current = () => {
      if (!running) schedule(START_DELAY_MS);
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") kickRef.current();
    };
    document.addEventListener("visibilitychange", onVisibility);
    schedule(START_DELAY_MS);
    return () => {
      abort.abort();
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
      kickRef.current = () => {};
    };
  }, []);

  // A camera that is added or comes online without a preview gets one soon, not at the next check.
  const waiting = cameras
    .filter((c) => c.status.state === "online" && !c.snapshotAt)
    .map((c) => c.id)
    .join();
  useEffect(() => {
    if (waiting) kickRef.current();
  }, [waiting]);

  return capturing;
}

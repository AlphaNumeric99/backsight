import { useEffect, type RefObject } from "react";
import { api, type CameraId } from "@/ipc";
import type { VideoSurfaceHandle } from "@/player/types";
import { PREVIEW_SNAPSHOT } from "./policy";

/** The first capture waits this long after the picture appears. */
const FIRST_CAPTURE_MS = 2000;

/**
 * While a live player shows `cameraId`, saves its picture as the camera's preview: shortly
 * after the stream starts, then every `intervalMs`, so the camera's card shows what was last
 * seen.
 */
export function useLivePreview(
  cameraId: CameraId,
  surface: RefObject<VideoSurfaceHandle | null>,
  playing: boolean,
  intervalMs = 60_000,
): void {
  useEffect(() => {
    if (!playing) return;
    let cancelled = false;
    const capture = async () => {
      try {
        const jpeg = await surface.current?.snapshot(PREVIEW_SNAPSHOT);
        if (jpeg && !cancelled) await api.savePreview(cameraId, jpeg);
      } catch (error) {
        console.debug("Backsight: couldn't save a preview", error);
      }
    };
    const first = setTimeout(() => void capture(), FIRST_CAPTURE_MS);
    const every = setInterval(() => void capture(), intervalMs);
    return () => {
      cancelled = true;
      clearTimeout(first);
      clearInterval(every);
    };
  }, [cameraId, surface, playing, intervalMs]);
}

import { useCallback, useState, type RefObject } from "react";
import { toast } from "sonner";
import { api, type Camera } from "@/ipc";
import { describeError, isApiError } from "@/lib/errors";
import { strings } from "@/lib/strings";
import type { VideoSurfaceHandle } from "@/player/types";

/** Captures the current frame via the player and saves it through the backend. */
export function useSnapshot(camera: Camera, videoRef: RefObject<VideoSurfaceHandle | null>) {
  const [busy, setBusy] = useState(false);
  const take = useCallback(async () => {
    const surface = videoRef.current;
    if (!surface || busy) return;
    setBusy(true);
    try {
      const png = await surface.snapshot();
      const path = await api.saveSnapshot(camera.id, png);
      toast.success(strings.live.snapshotSaved, { description: path });
    } catch (err) {
      const description = isApiError(err)
        ? describeError(err).body
        : err instanceof Error
          ? err.message
          : undefined;
      toast.error(strings.live.snapshotFailed, { description });
    } finally {
      setBusy(false);
    }
  }, [camera.id, videoRef, busy]);
  return { take, busy };
}

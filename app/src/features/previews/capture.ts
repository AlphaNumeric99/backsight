import type { CameraId } from "@/ipc";
import { PlayerController } from "@/player/controller";
import { PREVIEW_SNAPSHOT } from "./policy";

/** How long to wait for a picture before giving up. */
const CAPTURE_TIMEOUT_MS = 15_000;
/** Let the picture settle after the first frame, so the grab isn't the stream's very first. */
const SETTLE_MS = 600;

/**
 * Grabs one frame from a camera's live SD stream without showing it, as a preview JPEG. The
 * player runs in an off-screen element and is torn down as soon as the frame is captured.
 */
export function capturePreview(cameraId: CameraId, signal?: AbortSignal): Promise<Blob> {
  return new Promise<Blob>((resolve, reject) => {
    const host = document.createElement("div");
    host.setAttribute("aria-hidden", "true");
    // Off screen but laid out, so the player behaves as it does for a visible tile.
    Object.assign(host.style, {
      position: "fixed",
      left: "-10000px",
      top: "0",
      width: "320px",
      height: "180px",
      pointerEvents: "none",
    });
    document.body.appendChild(host);

    let controller: PlayerController | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    let settle: ReturnType<typeof setTimeout> | undefined;
    let done = false;
    const finish = (outcome: () => void) => {
      if (done) return;
      done = true;
      clearTimeout(timeout);
      clearTimeout(settle);
      signal?.removeEventListener("abort", onAbort);
      controller?.dispose();
      host.remove();
      outcome();
    };
    const fail = (error: unknown) => finish(() => reject(error instanceof Error ? error : new Error(String(error))));
    const onAbort = () => finish(() => reject(new DOMException("The preview capture was cancelled", "AbortError")));

    if (signal?.aborted) return onAbort();
    signal?.addEventListener("abort", onAbort);
    timeout = setTimeout(() => fail(new Error("The camera sent no picture in time")), CAPTURE_TIMEOUT_MS);

    try {
      controller = new PlayerController(
        host,
        {
          onState: (state) => {
            if (state.kind === "error") fail(new Error(state.message));
            else if (state.kind === "ended") fail(new Error("The stream ended before a picture arrived"));
          },
          onStats: () => {},
          onTime: () => {},
          onFirstFrame: () => {
            settle = setTimeout(() => {
              controller?.snapshot(PREVIEW_SNAPSHOT).then((jpeg) => finish(() => resolve(jpeg)), fail);
            }, SETTLE_MS);
          },
        },
        { fit: "contain", muted: true },
      );
      controller.setSource({ kind: "live", cameraId, quality: "sd" }, true);
    } catch (error) {
      // No player here (e.g. no Worker or OffscreenCanvas support).
      fail(error);
    }
  });
}

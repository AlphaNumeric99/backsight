import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState, type RefObject } from "react";
import type { StreamRequest } from "../ipc/api";
import { PlayerController, sourceKey } from "./controller";
import type { PlayerState, PlayerStats, VideoSurfaceHandle } from "./types";

export interface VideoSurfaceProps {
  /** What to play. `null` shows the idle poster. Changing it restarts the stream. */
  source: StreamRequest | null;
  /**
   * Defaults to true, as multi-view needs; pass false to hear the camera. Audio plays only at 1×.
   * The browser may hold audio back until the user has interacted with the page.
   */
  muted?: boolean;
  fit?: "contain" | "cover";
  /** Shown while idle or connecting. */
  posterUrl?: string;
  className?: string;
  onState?: (state: PlayerState) => void;
  onStats?: (stats: PlayerStats) => void;
  /** Current presentation time, epoch milliseconds (UTC), about 10× per second. */
  onTime?: (epochMs: number) => void;
}

/** How long a tile may be out of view before its stream is closed. */
const PAUSE_DELAY_MS = 1000;

/**
 * Renders a camera stream: WebCodecs decoding and drawing run in a worker, audio in an
 * AudioWorklet. The stream is `source` compared by value, and it is closed while the element is
 * scrolled out of view or the window is hidden (paused playback resumes where it stopped).
 */
export const VideoSurface = forwardRef<VideoSurfaceHandle, VideoSurfaceProps>(function VideoSurface(
  { source, muted = true, fit = "contain", posterUrl, className, onState, onStats, onTime },
  ref,
) {
  const hostRef = useRef<HTMLDivElement>(null);
  const canvasHostRef = useRef<HTMLDivElement>(null);
  const controllerRef = useRef<PlayerController | null>(null);
  const key = sourceKey(source);
  const latest = useRef({ key, source, muted, fit, onState, onStats, onTime });
  useLayoutEffect(() => {
    latest.current = { key, source, muted, fit, onState, onStats, onTime };
  });

  const [status, setStatus] = useState<PlayerState["kind"]>("idle");
  const [frameKey, setFrameKey] = useState<string | null>(null);
  const active = useActive(hostRef);

  useEffect(() => {
    const host = canvasHostRef.current;
    if (!host) return;
    const controller = new PlayerController(
      host,
      {
        onState: (state) => {
          setStatus(state.kind);
          latest.current.onState?.(state);
        },
        onStats: (stats) => latest.current.onStats?.(stats),
        onTime: (epochMs) => latest.current.onTime?.(epochMs),
        onFirstFrame: () => setFrameKey(latest.current.key),
      },
      { fit: latest.current.fit, muted: latest.current.muted },
    );
    controllerRef.current = controller;
    return () => {
      controllerRef.current = null;
      controller.dispose();
    };
  }, []);

  useEffect(() => {
    controllerRef.current?.setSource(latest.current.source, active);
  }, [key, active]);

  useEffect(() => {
    controllerRef.current?.setFit(fit);
  }, [fit]);

  useEffect(() => {
    controllerRef.current?.setMuted(muted);
  }, [muted]);

  useImperativeHandle(
    ref,
    () => ({
      snapshot: (options) =>
        controllerRef.current?.snapshot(options) ?? Promise.reject(new Error("The player is not mounted")),
    }),
    [],
  );

  const showPoster = !!posterUrl && (status === "idle" || status === "connecting" || frameKey !== key);
  return (
    <div
      ref={hostRef}
      className={className}
      style={{ position: "relative", overflow: "hidden", width: "100%", height: "100%", background: "#0b0d12" }}
    >
      <div ref={canvasHostRef} style={{ position: "absolute", inset: 0 }} />
      {showPoster && (
        <div
          aria-hidden
          style={{
            position: "absolute",
            inset: 0,
            pointerEvents: "none",
            background: `center / ${fit} no-repeat url(${JSON.stringify(posterUrl)})`,
          }}
        />
      )}
    </div>
  );
});

/**
 * Whether the element is on screen and the page is visible. Turning true is immediate; turning
 * false waits `PAUSE_DELAY_MS`, so brief scrolls or window switches don't reconnect the camera.
 */
function useActive(ref: RefObject<HTMLElement | null>): boolean {
  const [intersecting, setIntersecting] = useState(() => typeof IntersectionObserver === "undefined");
  const [pageVisible, setPageVisible] = useState(() => document.visibilityState !== "hidden");

  useEffect(() => {
    const onChange = () => setPageVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", onChange);
    return () => document.removeEventListener("visibilitychange", onChange);
  }, []);

  useEffect(() => {
    const element = ref.current;
    if (!element || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((entries) => {
      const entry = entries[entries.length - 1];
      if (entry) setIntersecting(entry.isIntersecting);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);

  const visible = intersecting && pageVisible;
  const [lingering, setLingering] = useState(visible);
  useEffect(() => {
    if (visible) {
      setLingering(true);
      return;
    }
    const timer = setTimeout(() => setLingering(false), PAUSE_DELAY_MS);
    return () => clearTimeout(timer);
  }, [visible]);
  return visible || lingering;
}

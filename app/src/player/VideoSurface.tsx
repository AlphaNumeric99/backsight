import { forwardRef, useImperativeHandle } from "react";
import type { StreamRequest } from "../ipc/api";
import type { PlayerState, PlayerStats, VideoSurfaceHandle } from "./types";

export interface VideoSurfaceProps {
  /** What to play. `null` shows the idle poster. Changing it restarts the stream. */
  source: StreamRequest | null;
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

/**
 * Renders a camera stream. Placeholder until the player engine lands: it draws an
 * empty frame so screens can be built against the final props.
 */
export const VideoSurface = forwardRef<VideoSurfaceHandle, VideoSurfaceProps>(
  function VideoSurface({ className, posterUrl }, ref) {
    useImperativeHandle(ref, () => ({
      snapshot: () => Promise.reject(new Error("player not implemented")),
    }));
    return (
      <div
        className={className}
        style={{
          background: posterUrl ? `center / cover url(${posterUrl})` : "#0b0d12",
          width: "100%",
          height: "100%",
        }}
      />
    );
  },
);

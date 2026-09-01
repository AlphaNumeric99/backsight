// Public types of the player. Screens depend on these; the implementation lives in the
// rest of `player/`.

export type PlayerState =
  | { kind: "idle" }
  | { kind: "connecting" }
  | { kind: "buffering" }
  | { kind: "playing" }
  | { kind: "ended" }
  | {
      kind: "error";
      /** `codec_unsupported` when the webview can't decode the camera's codec. */
      code: "codec_unsupported" | "stream_failed" | "decode_failed" | string;
      message: string;
    };

export interface PlayerStats {
  /** e.g. "avc1.64001F" */
  codec?: string;
  width?: number;
  height?: number;
  fps: number;
  /** Incoming media bitrate in bits per second, averaged over ~2 s. */
  bitrate: number;
  droppedFrames: number;
  decodeQueue: number;
  /** Estimated end-to-end buffering delay in milliseconds. */
  latencyMs?: number;
}

export interface VideoSurfaceHandle {
  /** Captures the current frame as a PNG. */
  snapshot(): Promise<Blob>;
}

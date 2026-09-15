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
      /**
       * `codec_unsupported` when the webview can't decode the camera's codec (see `codec`),
       * `stream_failed` / `decode_failed` for other player failures. Errors from the backend
       * (`openStream` rejections, `Status` packets) pass their `ApiErrorCode` through, e.g.
       * `stream_limit` or `playback_busy`.
       */
      code: "codec_unsupported" | "stream_failed" | "decode_failed" | string;
      message: string;
      /** For `codec_unsupported`: the codec string, e.g. "hvc1.1.6.L93.B0". */
      codec?: string;
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
  /** Media buffered ahead of the picture on screen, in wall-clock milliseconds. */
  bufferMs?: number;
  /** The jitter buffer's current delay target, ms (adapts between 150 and 300 ms live). */
  targetDelayMs?: number;
  /** Mean time from handing a frame to the decoder to getting the picture back, ms. */
  decodeMs?: number;
  /** While audio plays: the audible position minus the playout clock, ms (A/V sync error). */
  audioOffsetMs?: number;
  /** What paces the video: the audio output (unmuted at 1×) or the wall clock. */
  clock?: "audio" | "wall";
}

export interface VideoSurfaceHandle {
  /** Captures the current frame as a PNG. */
  snapshot(): Promise<Blob>;
}

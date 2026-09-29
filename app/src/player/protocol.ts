// Messages between the main thread, the media worker and the audio worklet.
//
//   main ──WorkerCommand / batch ArrayBuffer──▶ media worker ──PCM, commands──▶ audio worklet
//   main ◀────────────── WorkerEvent ────────── media worker ◀─────reports───── audio worklet
//
// Batches are posted as bare ArrayBuffers (transferred) and belong to the session of the latest
// `start`. Events carry the session they belong to, so the main thread can ignore stale ones.

import type { Fit } from "./fit";
import type { PcmPlayerStatus } from "./pcm";
import type { PlayerState, PlayerStats, SnapshotOptions } from "./types";

export type StreamMode = "live" | "playback";

export type WorkerCommand =
  | { type: "init"; canvas: OffscreenCanvas; width: number; height: number; fit: Fit }
  /** A new stream: resets decoding and the clock. `clear` blanks the canvas at once. */
  | { type: "start"; session: number; mode: StreamMode; speed: number; clear: boolean }
  /** The stream was closed (source removed or tile hidden). Keeps the last frame unless `clear`. */
  | { type: "stop"; session: number; clear: boolean }
  /** Canvas size in device pixels. */
  | { type: "resize"; width: number; height: number }
  | { type: "fit"; fit: Fit }
  /** Whether to play audio (the player is unmuted); the worker adds the 1× condition. */
  | { type: "audio"; enabled: boolean }
  /** A port to a new audio worklet, replacing any previous one. */
  | { type: "audioPort"; port: MessagePort; latencyMs: number }
  | { type: "audioLatency"; latencyMs: number }
  | { type: "snapshot"; id: number; options?: SnapshotOptions };

export type WorkerEvent =
  | { type: "state"; session: number; state: PlayerState }
  | { type: "stats"; session: number; stats: PlayerStats }
  | { type: "time"; session: number; epochMs: number }
  /** The first frame of the session is on screen. */
  | { type: "frame"; session: number }
  /** The stream's audio format; the main thread sets up an AudioContext for it. */
  | { type: "audioConfig"; session: number; sampleRate: number; channels: number }
  | { type: "snapshot"; id: number; blob?: Blob; error?: string }
  | { type: "log"; session: number; level: "info" | "warn"; message: string };

/** Media worker → audio worklet. */
export type WorkletCommand =
  | { type: "config"; sampleRate: number; channels: number }
  /** Interleaved float samples; the first one is at `tsUs`. */
  | { type: "pcm"; tsUs: number; data: Float32Array<ArrayBuffer> }
  | { type: "sync"; positionUs: number }
  | { type: "rate"; rate: number }
  | { type: "flush" };

/** Audio worklet → media worker, about 40 times a second while configured. */
export type WorkletReport = { type: "report" } & PcmPlayerStatus;

/** Main thread → audio worklet (through `AudioWorkletNode.port`). */
export type WorkletSetup = { type: "port"; port: MessagePort };

export const WORKLET_PROCESSOR = "backsight-pcm";

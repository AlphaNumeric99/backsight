// Main-thread side of one player: creates the canvas and the media worker, opens and closes
// streams through `BacksightApi`, forwards batches to the worker and turns worker events into
// callbacks. VideoSurface wraps it for React.

import { api as defaultApi } from "../ipc";
import type { ApiError, BacksightApi, StreamHandle, StreamRequest } from "../ipc/api";
import { AudioOutput } from "./audioOutput";
import type { Fit } from "./fit";
import type { WorkerCommand, WorkerEvent } from "./protocol";
import type { PlayerState, PlayerStats } from "./types";

export interface PlayerEvents {
  onState(state: PlayerState): void;
  onStats(stats: PlayerStats): void;
  onTime(epochMs: number): void;
  /** The first frame of a stream is on screen. */
  onFirstFrame(): void;
}

export interface PlayerOptions {
  fit: Fit;
  muted: boolean;
  /** Defaults to the app's `api`. */
  api?: BacksightApi;
}

/** A value-based identity for a stream request: equal keys mean the same stream. */
export function sourceKey(source: StreamRequest | null): string {
  if (!source) return "";
  if (source.kind === "live") return `live|${source.cameraId}|${source.quality}`;
  const start = Date.parse(source.start);
  return `playback|${source.cameraId}|${Number.isNaN(start) ? source.start : start}|${source.speed}`;
}

interface OpenStream {
  session: number;
  closed: boolean;
  handle?: StreamHandle;
}

interface PendingSnapshot {
  resolve(blob: Blob): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

const SNAPSHOT_TIMEOUT_MS = 5000;

export class PlayerController {
  private readonly api: BacksightApi;
  private readonly canvas: HTMLCanvasElement;
  private readonly worker: Worker;
  private readonly resizeObserver: ResizeObserver;
  private readonly audio: AudioOutput;
  private session = 0;
  private key = "";
  private source: StreamRequest | null = null;
  private running = false;
  private stream: OpenStream | undefined;
  private state: PlayerState = { kind: "idle" };
  /** The last presented time of the current source, to resume playback where it was paused. */
  private resumeAtMs = NaN;
  private readonly snapshots = new Map<number, PendingSnapshot>();
  private snapshotId = 0;
  private size = { width: 0, height: 0 };

  constructor(
    host: HTMLElement,
    private readonly events: PlayerEvents,
    options: PlayerOptions,
  ) {
    this.api = options.api ?? defaultApi;
    // A fresh canvas per controller: a canvas can hand its control to a worker only once, and
    // StrictMode mounts twice.
    this.canvas = document.createElement("canvas");
    Object.assign(this.canvas.style, { position: "absolute", inset: "0", width: "100%", height: "100%", display: "block" });
    host.appendChild(this.canvas);
    this.size = cssSizeInDevicePixels(this.canvas);
    const offscreen = this.canvas.transferControlToOffscreen();

    this.worker = new Worker(new URL("./media.worker.ts", import.meta.url), { type: "module", name: "backsight-media" });
    this.worker.onmessage = (event: MessageEvent<WorkerEvent>) => this.onEvent(event.data);
    this.worker.onerror = (event) => {
      event.preventDefault();
      this.setState({ kind: "error", code: "stream_failed", message: `The media worker failed: ${event.message}` });
    };
    this.post({ type: "init", canvas: offscreen, ...this.size, fit: options.fit }, [offscreen]);

    this.resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      const box = entry.devicePixelContentBoxSize?.[0];
      const size = box
        ? { width: box.inlineSize, height: box.blockSize }
        : cssSizeInDevicePixels(this.canvas);
      if (size.width === this.size.width && size.height === this.size.height) return;
      this.size = size;
      this.post({ type: "resize", ...size });
    });
    try {
      this.resizeObserver.observe(this.canvas, { box: "device-pixel-content-box" });
    } catch {
      this.resizeObserver.observe(this.canvas);
    }

    this.audio = new AudioOutput((command, transfer) => this.post(command, transfer));
    this.audio.setMuted(options.muted);
  }

  /**
   * Plays `source` while `active` (the tile is visible). Opening happens only when the source's
   * value changes or the player becomes active again; pausing closes the stream but keeps the
   * picture, and a paused playback resumes where it stopped.
   */
  setSource(source: StreamRequest | null, active: boolean): void {
    const key = sourceKey(source);
    const changed = key !== this.key;
    const newCamera = changed && source?.cameraId !== this.source?.cameraId;
    if (changed) this.resumeAtMs = NaN;
    this.key = key;
    this.source = source;
    const run = source !== null && active;
    if (!changed && run === this.running) return;
    this.running = run;

    this.closeStream(source === null || newCamera);
    if (!source) {
      this.setState({ kind: "idle" });
    } else if (run) {
      this.openStream(source, newCamera);
    }
  }

  setFit(fit: Fit): void {
    this.post({ type: "fit", fit });
  }

  setMuted(muted: boolean): void {
    this.audio.setMuted(muted);
  }

  snapshot(): Promise<Blob> {
    return new Promise((resolve, reject) => {
      const id = ++this.snapshotId;
      const timer = setTimeout(() => {
        this.snapshots.delete(id);
        reject(new Error("The snapshot timed out"));
      }, SNAPSHOT_TIMEOUT_MS);
      this.snapshots.set(id, { resolve, reject, timer });
      this.post({ type: "snapshot", id });
    });
  }

  dispose(): void {
    this.closeStream(true);
    this.resizeObserver.disconnect();
    this.audio.dispose();
    this.worker.terminate();
    this.canvas.remove();
    for (const pending of this.snapshots.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error("The player was unmounted"));
    }
    this.snapshots.clear();
  }

  private openStream(source: StreamRequest, clear: boolean): void {
    const session = ++this.session;
    const request: StreamRequest =
      source.kind === "playback" && Number.isFinite(this.resumeAtMs)
        ? { ...source, start: new Date(this.resumeAtMs).toISOString() }
        : source;
    const stream: OpenStream = { session, closed: false };
    this.stream = stream;
    this.post({
      type: "start",
      session,
      mode: request.kind,
      speed: request.kind === "playback" ? request.speed : 1,
      clear,
    });
    this.setState({ kind: "connecting" });

    const onBatch = (batch: ArrayBuffer) => {
      if (!stream.closed) this.worker.postMessage(batch, [batch]);
    };
    this.api.openStream(request, onBatch).then(
      (handle) => {
        if (stream.closed) void handle.close().catch(() => {});
        else stream.handle = handle;
      },
      (error: unknown) => {
        if (stream.closed) return;
        const apiError = (typeof error === "object" && error !== null ? error : {}) as Partial<ApiError>;
        this.closeStream(false);
        this.setState({
          kind: "error",
          code: apiError.code ?? "stream_failed",
          message: apiError.message ?? String(error),
        });
      },
    );
  }

  private closeStream(clear: boolean): void {
    const stream = this.stream;
    if (!stream) {
      if (clear) this.post({ type: "stop", session: this.session, clear: true });
      return;
    }
    this.stream = undefined;
    stream.closed = true;
    this.post({ type: "stop", session: stream.session, clear });
    void stream.handle?.close().catch(() => {});
  }

  private onEvent(event: WorkerEvent): void {
    if (event.type === "snapshot") {
      const pending = this.snapshots.get(event.id);
      if (!pending) return;
      this.snapshots.delete(event.id);
      clearTimeout(pending.timer);
      if (event.blob) pending.resolve(event.blob);
      else pending.reject(new Error(event.error ?? "The snapshot failed"));
      return;
    }
    // Ignore events from earlier or paused streams.
    if (!this.stream || event.session !== this.stream.session) return;
    switch (event.type) {
      case "state":
        // The stream ended, or failed for good (unsupported codec, decoder or backend failure):
        // release it, as it counts against the camera's viewer limit. Changing the source
        // (or the tile becoming visible again) opens a new one.
        if (event.state.kind === "error" || event.state.kind === "ended") this.closeStream(false);
        this.setState(event.state);
        break;
      case "stats":
        this.events.onStats(event.stats);
        break;
      case "time":
        this.resumeAtMs = event.epochMs;
        this.events.onTime(event.epochMs);
        break;
      case "frame":
        this.events.onFirstFrame();
        break;
      case "audioConfig":
        this.audio.setFormat(event.sampleRate, event.channels);
        break;
      case "log":
        if (event.level === "warn") console.warn(`Backsight player: ${event.message}`);
        else console.debug(`Backsight player: ${event.message}`);
        break;
    }
  }

  private setState(state: PlayerState): void {
    const same =
      state.kind === this.state.kind &&
      (state.kind !== "error" ||
        (this.state.kind === "error" && state.code === this.state.code && state.message === this.state.message));
    if (same) return;
    this.state = state;
    this.events.onState(state);
  }

  private post(command: WorkerCommand, transfer: Transferable[] = []): void {
    this.worker.postMessage(command, transfer);
  }
}

function cssSizeInDevicePixels(element: HTMLElement): { width: number; height: number } {
  const rect = element.getBoundingClientRect();
  const ratio = window.devicePixelRatio || 1;
  return { width: Math.round(rect.width * ratio), height: Math.round(rect.height * ratio) };
}

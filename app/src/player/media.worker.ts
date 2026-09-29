// The media worker: one per VideoSurface. It receives wire-format batches (transferred from the
// main thread), runs the playout clock and jitter buffer, decodes video with WebCodecs, draws on
// the transferred OffscreenCanvas and feeds PCM to the audio worklet over a MessagePort.
//
// Nothing here blocks on audio: video follows the audio clock only while the worklet reports a
// position close to the playout clock, and the wall clock otherwise.

import { AudioSync, type AudioCommand } from "./audioSync";
import { LIVE_PLAYOUT, PLAYBACK_PLAYOUT, PlayoutClock } from "./clock";
import { fitRect, type Fit } from "./fit";
import { JitterBuffer } from "./jitter";
import { s16ToFloat32 } from "./pcm";
import type { StreamMode, WorkerCommand, WorkerEvent, WorkletCommand, WorkletReport } from "./protocol";
import { WindowCounter } from "./stats";
import type { PlayerState, PlayerStats, SnapshotOptions } from "./types";
import { decodeBatch, type AudioConfigInfo, type Packet, type StreamStatus, type VideoConfigPacket } from "./wire";

interface WorkerScope {
  postMessage(message: WorkerEvent, transfer?: Transferable[]): void;
  onmessage: ((event: MessageEvent<WorkerCommand | ArrayBuffer>) => void) | null;
  requestAnimationFrame?: (callback: (time: number) => void) => number;
  cancelAnimationFrame?: (handle: number) => void;
}

const scope = self as unknown as WorkerScope;

/** No new picture for this long means the stream stalled. */
const STALL_MS = 1500;
const TIME_INTERVAL_MS = 100;
const STATS_INTERVAL_MS = 1000;
/** This many decoder failures within the window is fatal. */
const MAX_DECODER_ERRORS = 3;
const DECODER_ERROR_WINDOW_MS = 30_000;
/** Frames arriving after their playout time for this long re-anchor the clock. */
const LATE_STREAK_MS = 1000;
/** Timestamp jumps treated as an unflagged discontinuity: forward (wall ms at the current speed), back (µs). */
const JUMP_FORWARD_MS = 3000;
const JUMP_BACK_US = 1_000_000;

interface QueuedFrame {
  tsUs: number;
  keyframe: boolean;
  byteLength: number;
  data: Uint8Array;
  arrivalMs: number;
}

interface ReadyFrame {
  tsUs: number;
  frame: VideoFrame;
  arrivalMs: number;
  close(): void;
}

/** Draws the current frame to fit the canvas, and keeps it for redraws and snapshots. */
class Renderer {
  private readonly ctx: OffscreenCanvasRenderingContext2D;
  private current: VideoFrame | undefined;

  constructor(
    private readonly canvas: OffscreenCanvas,
    private fit: Fit,
  ) {
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("the 2D canvas context is unavailable");
    this.ctx = ctx;
  }

  get hasFrame(): boolean {
    return this.current !== undefined;
  }

  resize(width: number, height: number): void {
    if (width === this.canvas.width && height === this.canvas.height) return;
    this.canvas.width = width;
    this.canvas.height = height;
    this.draw();
  }

  setFit(fit: Fit): void {
    this.fit = fit;
    this.draw();
  }

  /** Shows `frame`, taking ownership of it; the previous one is closed. */
  show(frame: VideoFrame): void {
    this.current?.close();
    this.current = frame;
    this.draw();
  }

  clear(): void {
    this.current?.close();
    this.current = undefined;
    this.draw();
  }

  /** The current frame: PNG at its natural size unless `options` say otherwise. */
  async snapshot(options: SnapshotOptions = {}): Promise<Blob> {
    const frame = this.current;
    if (!frame) throw new Error("no frame to capture yet");
    const scale = options.maxWidth ? Math.min(1, options.maxWidth / frame.displayWidth) : 1;
    const width = Math.max(1, Math.round(frame.displayWidth * scale));
    const height = Math.max(1, Math.round(frame.displayHeight * scale));
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("the 2D canvas context is unavailable");
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(frame, 0, 0, width, height);
    return canvas.convertToBlob({ type: options.type ?? "image/png", quality: options.quality });
  }

  private draw(): void {
    const { width, height } = this.canvas;
    this.ctx.clearRect(0, 0, width, height);
    const frame = this.current;
    if (!frame || width === 0 || height === 0) return;
    const r = fitRect(frame.displayWidth, frame.displayHeight, width, height, this.fit);
    this.ctx.imageSmoothingQuality = "high";
    this.ctx.drawImage(frame, r.sx, r.sy, r.sw, r.sh, r.dx, r.dy, r.dw, r.dh);
  }
}

async function supportedConfig(config: VideoDecoderConfig): Promise<VideoDecoderConfig | null> {
  if (typeof VideoDecoder === "undefined") return null;
  const preferences: HardwareAcceleration[] =
    config.hardwareAcceleration === "no-preference" ? ["no-preference"] : ["prefer-hardware", "no-preference"];
  for (const hardwareAcceleration of preferences) {
    const candidate = { ...config, hardwareAcceleration };
    try {
      if ((await VideoDecoder.isConfigSupported(candidate)).supported) return candidate;
    } catch {
      // An invalid config (e.g. a malformed codec string) is simply unsupported.
    }
  }
  return null;
}

function sameVideoConfig(a: VideoConfigPacket, b: VideoConfigPacket): boolean {
  if (a.config.codec !== b.config.codec) return false;
  if (a.config.codedWidth !== b.config.codedWidth || a.config.codedHeight !== b.config.codedHeight) return false;
  if (a.description.length !== b.description.length) return false;
  return a.description.every((byte, i) => byte === b.description[i]);
}

function sameState(a: PlayerState, b: PlayerState): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "error" && b.kind === "error") return a.code === b.code && a.message === b.message;
  return true;
}

function describeError(error: unknown): string {
  return error instanceof Error ? error.message || error.name : String(error);
}

class MediaEngine {
  private renderer: Renderer | undefined;
  private session = 0;
  private mode: StreamMode = "live";
  private speed = 1;
  private clock = new PlayoutClock({ speed: 1, ...LIVE_PLAYOUT });
  private buffer = new JitterBuffer<QueuedFrame, ReadyFrame>({ speed: 1 });
  private state: PlayerState = { kind: "idle" };
  private fatal: PlayerState | undefined;
  private receivedAny = false;
  private loopScheduled = false;
  private loopTimer: ReturnType<typeof setTimeout> | undefined;
  private animationFrame: number | undefined;
  private animationFramesWork = true;

  // Video decoding
  private config: VideoConfigPacket | undefined;
  private decoderConfig: VideoDecoderConfig | undefined;
  private configState: "none" | "checking" | "ready" | "unsupported" = "none";
  private configGen = 0;
  private decoder: VideoDecoder | undefined;
  private decoderGen = 0;
  private decoderErrors: number[] = [];
  private hardwareAcceleration: HardwareAcceleration = "prefer-hardware";
  /** Timestamps given to the decoder are relative to this (µs), to keep them small. */
  private tsBase = NaN;
  /** Frames given to the decoder, by decoder timestamp: arrival and hand-off times (ms). */
  private readonly pending = new Map<number, { arrivalMs: number; sentMs: number }>();
  private lastVideoTsUs = NaN;
  private frameIntervalUs = 66_667;
  private lateSinceMs = NaN;
  private ended = false;
  private flushed = false;
  private flushing = false;

  // Presentation
  private shownSinceReset = false;
  private firstShownTsUs = NaN;
  private lastShownMs = NaN;
  private lastShownTsUs = NaN;
  private announcedFrame = false;
  private rebuffering = false;
  private backendBuffering = false;
  private width: number | undefined;
  private height: number | undefined;

  // Audio
  private audioEnabled = false;
  private audioPort: MessagePort | undefined;
  private audioConfig: AudioConfigInfo | undefined;
  private readonly audioSync = new AudioSync();
  private clockSource: "audio" | "wall" = "wall";

  // Statistics
  private readonly bytes = new WindowCounter(2000);
  private readonly shown = new WindowCounter(1000);
  private readonly latency = new WindowCounter(1000);
  private readonly decodeTime = new WindowCounter(1000);
  private droppedOther = 0;
  private lastStatsMs = 0;
  private lastTimeMs = 0;
  private lastTimeSent = NaN;
  private logBudget = 20;

  handle(message: WorkerCommand | ArrayBuffer): void {
    if (message instanceof ArrayBuffer) {
      this.onBatch(message);
      return;
    }
    switch (message.type) {
      case "init":
        this.renderer = new Renderer(message.canvas, message.fit);
        this.renderer.resize(message.width, message.height);
        break;
      case "start":
        this.start(message.session, message.mode, message.speed, message.clear);
        break;
      case "stop":
        if (message.session === this.session) this.stop(message.clear);
        break;
      case "resize":
        this.renderer?.resize(message.width, message.height);
        break;
      case "fit":
        this.renderer?.setFit(message.fit);
        break;
      case "audio":
        this.setAudioEnabled(message.enabled);
        break;
      case "audioPort":
        this.attachAudioPort(message.port, message.latencyMs);
        break;
      case "audioLatency":
        this.audioSync.latencyUs = message.latencyMs * 1000;
        break;
      case "snapshot":
        void this.snapshot(message.id, message.options);
        break;
    }
  }

  // --- Sessions -------------------------------------------------------------------------------

  private start(session: number, mode: StreamMode, speed: number, clear: boolean): void {
    this.stop(clear);
    this.session = session;
    this.mode = mode;
    this.speed = speed > 0 ? speed : 1;
    this.clock = new PlayoutClock({ speed: this.speed, ...(mode === "live" ? LIVE_PLAYOUT : PLAYBACK_PLAYOUT) });
    this.buffer = new JitterBuffer<QueuedFrame, ReadyFrame>({ speed: this.speed });
    this.state = { kind: "idle" };
    this.setState({ kind: "connecting" });
    const now = performance.now();
    this.lastStatsMs = now;
    this.lastTimeMs = now;
    this.scheduleLoop();
  }

  /** Ends the session and releases the decoder. Keeps the picture unless `clear`. */
  private stop(clear: boolean): void {
    this.resetMedia();
    this.closeDecoder();
    this.session = 0;
    this.config = undefined;
    this.decoderConfig = undefined;
    this.configState = "none";
    this.configGen++;
    this.decoderErrors = [];
    this.hardwareAcceleration = "prefer-hardware";
    this.fatal = undefined;
    this.receivedAny = false;
    this.announcedFrame = false;
    this.backendBuffering = false;
    this.audioConfig = undefined;
    this.bytes.clear();
    this.shown.clear();
    this.latency.clear();
    this.decodeTime.clear();
    this.droppedOther = 0;
    this.lastTimeSent = NaN;
    this.lastShownTsUs = NaN;
    this.lastShownMs = NaN;
    this.logBudget = 20;
    if (clear) {
      this.renderer?.clear();
      this.width = undefined;
      this.height = undefined;
    }
  }

  /** Drops queued media and restarts the clock, e.g. on a discontinuity. Keeps the codec config. */
  private resetMedia(): void {
    this.buffer.clear();
    this.closeDecoder();
    this.clock.reset();
    this.tsBase = NaN;
    this.pending.clear();
    this.lastVideoTsUs = NaN;
    this.lateSinceMs = NaN;
    this.shownSinceReset = false;
    this.firstShownTsUs = NaN;
    this.ended = false;
    this.flushed = false;
    this.flushing = false;
    this.rebuffering = false;
    this.sendAudio({ type: "flush" });
    this.audioSync.reset();
  }

  // --- Input ----------------------------------------------------------------------------------

  private onBatch(batch: ArrayBuffer): void {
    if (!this.session) return;
    const arrivalMs = performance.now();
    this.bytes.add(arrivalMs, batch.byteLength);
    const { packets, errors } = decodeBatch(batch);
    for (const error of errors) this.log("warn", `wire: ${error.code} at byte ${error.offset}: ${error.message}`);
    for (const packet of packets) {
      try {
        this.onPacket(packet, arrivalMs);
      } catch (error) {
        this.log("warn", `failed to handle ${packet.kind}: ${describeError(error)}`);
      }
    }
  }

  private onPacket(packet: Packet, arrivalMs: number): void {
    this.receivedAny = true;
    if (packet.discontinuity) this.resetMedia();
    switch (packet.kind) {
      case "videoConfig":
        this.onVideoConfig(packet);
        break;
      case "videoFrame":
        this.onVideoFrame(packet.timestampUs, packet.keyframe, packet.data, arrivalMs);
        break;
      case "audioConfig":
        this.onAudioConfig(packet.config);
        break;
      case "audioPcm":
        this.onAudioPcm(packet.timestampUs, packet.samples);
        break;
      case "status":
        this.onStatus(packet.status);
        break;
      case "endOfStream":
        this.ended = true;
        break;
    }
  }

  private onVideoConfig(packet: VideoConfigPacket): void {
    if (this.config && sameVideoConfig(this.config, packet)) return;
    this.config = packet;
    // Frames still queued belong to the old parameters; the new ones start at a keyframe.
    this.buffer.discardEncoded();
    this.closeDecoder();
    this.decoderConfig = undefined;
    this.configState = "checking";
    const gen = ++this.configGen;
    const { codec, codedWidth, codedHeight } = packet.config;
    const config: VideoDecoderConfig = {
      codec,
      codedWidth,
      codedHeight,
      hardwareAcceleration: this.hardwareAcceleration,
      optimizeForLatency: true,
    };
    if (packet.description.length > 0) config.description = packet.description;
    void supportedConfig(config).then((supported) => {
      if (gen !== this.configGen) return;
      if (!supported) {
        this.configState = "unsupported";
        this.buffer.clear();
        const webcodecs = typeof VideoDecoder !== "undefined";
        this.fail({
          kind: "error",
          code: "codec_unsupported",
          codec,
          message: webcodecs
            ? `This system can't decode the camera's video (${codec}).`
            : "Video decoding (WebCodecs) is not available in this webview.",
        });
        return;
      }
      this.decoderConfig = supported;
      this.configState = "ready";
      if (this.fatal?.kind === "error" && this.fatal.code === "codec_unsupported") this.fatal = undefined;
    });
  }

  private onVideoFrame(tsUs: number, keyframe: boolean, data: Uint8Array, arrivalMs: number): void {
    if (this.configState === "unsupported" || this.fatal) return;
    if (!this.config) {
      this.droppedOther++;
      return;
    }
    if (!Number.isNaN(this.lastVideoTsUs)) {
      const deltaUs = tsUs - this.lastVideoTsUs;
      if (deltaUs / this.speed > JUMP_FORWARD_MS * 1000 || deltaUs < -JUMP_BACK_US) {
        this.log("info", `timestamps jumped by ${Math.round(deltaUs / 1000)} ms: restarting the clock`);
        this.resetMedia();
      } else if (deltaUs > 0) {
        this.frameIntervalUs = Math.min(1_000_000, Math.max(5_000, deltaUs));
      }
    }
    this.lastVideoTsUs = tsUs;
    this.backendBuffering = false;
    if (Number.isNaN(this.tsBase)) this.tsBase = tsUs;

    if (this.rebuffering) {
      this.rebuffering = false; // the clock was re-anchored; this frame restarts it
    }
    const lateMs = this.clock.observe(tsUs, arrivalMs);
    // Live only: playback handles lateness by rebuffering, without skipping content.
    if (lateMs > 0 && this.mode === "live") {
      if (Number.isNaN(this.lateSinceMs)) {
        this.lateSinceMs = arrivalMs;
      } else if (arrivalMs - this.lateSinceMs > LATE_STREAK_MS) {
        this.log("info", "frames keep arriving late: re-anchoring the clock");
        this.clock.reanchor();
        this.clock.observe(tsUs, arrivalMs);
        this.lateSinceMs = NaN;
      }
    } else {
      this.lateSinceMs = NaN;
    }
    this.buffer.push({ tsUs, keyframe, byteLength: data.byteLength, data, arrivalMs });
  }

  private onStatus(status: StreamStatus): void {
    switch (status.state) {
      case "error":
        this.fail({
          kind: "error",
          code: status.code ?? "stream_failed",
          message: status.message ?? "The stream failed.",
        });
        break;
      case "ended":
        this.ended = true;
        break;
      case "buffering":
        this.backendBuffering = true;
        break;
      case "playing":
        this.backendBuffering = false;
        break;
    }
  }

  private fail(state: PlayerState): void {
    this.fatal = state;
    this.buffer.clear();
    this.closeDecoder();
    this.sendAudio({ type: "flush" });
    this.setState(state);
  }

  // --- Decoder --------------------------------------------------------------------------------

  private ensureDecoder(): VideoDecoder | undefined {
    if (this.configState !== "ready" || !this.decoderConfig) return undefined;
    if (this.decoder && this.decoder.state !== "closed") return this.decoder;
    const gen = ++this.decoderGen;
    const decoder = new VideoDecoder({
      output: (frame) => this.onDecoded(frame, gen),
      error: (error) => this.onDecoderError(error, gen),
    });
    try {
      decoder.configure(this.decoderConfig);
    } catch (error) {
      this.fail({ kind: "error", code: "decode_failed", message: `Could not configure the decoder: ${describeError(error)}` });
      return undefined;
    }
    this.decoder = decoder;
    // A fresh decoder has no reference pictures.
    this.buffer.requireKeyframe();
    return decoder;
  }

  private closeDecoder(): void {
    this.decoderGen++;
    const decoder = this.decoder;
    this.decoder = undefined;
    if (decoder && decoder.state !== "closed") {
      try {
        decoder.close();
      } catch {
        // already closed
      }
    }
  }

  private onDecoded(frame: VideoFrame, gen: number): void {
    if (gen !== this.decoderGen || !this.session) {
      frame.close();
      return;
    }
    const now = performance.now();
    const pending = this.pending.get(frame.timestamp);
    this.pending.delete(frame.timestamp);
    if (pending) this.decodeTime.add(now, now - pending.sentMs);
    // Until the first picture is up, keep every frame (the first one is shown at once).
    const nowUs = this.shownSinceReset ? this.presentationTimeUs(now) : NaN;
    this.buffer.addDecoded(
      { tsUs: frame.timestamp + this.tsBase, frame, arrivalMs: pending?.arrivalMs ?? NaN, close: () => frame.close() },
      nowUs,
    );
    // Keep the decoder busy between display refreshes (fast playback needs many frames each).
    this.feed(nowUs);
  }

  private onDecoderError(error: DOMException, gen: number): void {
    if (gen !== this.decoderGen) return;
    this.decoder = undefined;
    this.decoderGen++;
    const now = performance.now();
    this.decoderErrors = this.decoderErrors.filter((t) => now - t < DECODER_ERROR_WINDOW_MS);
    this.decoderErrors.push(now);
    this.log("warn", `decoder error: ${describeError(error)}`);
    if (this.decoderErrors.length >= MAX_DECODER_ERRORS) {
      this.fail({ kind: "error", code: "decode_failed", message: `Video decoding failed: ${describeError(error)}` });
      return;
    }
    if (this.decoderErrors.length >= 2 && this.hardwareAcceleration !== "no-preference" && this.decoderConfig) {
      // Hardware decoding keeps failing: let the browser pick (usually software).
      this.hardwareAcceleration = "no-preference";
      this.decoderConfig = { ...this.decoderConfig, hardwareAcceleration: "no-preference" };
    }
    // The next feed creates a new decoder, starting at a keyframe.
  }

  private feed(nowUs: number): void {
    if (this.fatal) return;
    const decoder = this.ensureDecoder();
    if (!decoder || decoder.state !== "configured") return;
    // Until the first picture is up, decode regardless of the clock: it is shown at once.
    const clockUs = this.shownSinceReset ? nowUs : NaN;
    for (const frame of this.buffer.takeDecodable(clockUs, decoder.decodeQueueSize)) {
      const timestamp = frame.tsUs - this.tsBase;
      try {
        decoder.decode(
          new EncodedVideoChunk({ type: frame.keyframe ? "key" : "delta", timestamp, data: frame.data }),
        );
      } catch (error) {
        this.log("warn", `decode() rejected a frame: ${describeError(error)}`);
        this.droppedOther++;
        this.buffer.requireKeyframe();
        break;
      }
      this.pending.set(timestamp, { arrivalMs: frame.arrivalMs, sentMs: performance.now() });
    }
    if (this.pending.size > 256) {
      // Outputs that never came (decoder reset): forget the oldest.
      for (const key of [...this.pending.keys()].slice(0, 128)) this.pending.delete(key);
    }
  }

  private maybeFlush(): void {
    if (!this.ended || this.flushing || this.flushed) return;
    if (this.buffer.encoded.length > 0) return;
    const decoder = this.decoder;
    if (!decoder || decoder.state !== "configured") {
      this.flushed = true;
      return;
    }
    // The end of the stream: make the decoder output what it still holds.
    this.flushing = true;
    const gen = this.decoderGen;
    decoder.flush().then(
      () => gen === this.decoderGen && (this.flushed = true),
      () => gen === this.decoderGen && (this.flushed = true),
    );
  }

  // --- Presentation loop ----------------------------------------------------------------------

  /**
   * Runs `step` on animation frames (in step with the display), with a timer as a fallback:
   * windows that are occluded, minimized or in a background tab can stop animation frames
   * while still counting as visible, and decoding, audio and the clock must keep going.
   */
  private scheduleLoop(): void {
    if (this.loopScheduled || !this.session) return;
    this.loopScheduled = true;
    const tick = (fromAnimationFrame: boolean) => {
      if (!this.loopScheduled) return;
      this.loopScheduled = false;
      this.animationFramesWork = fromAnimationFrame;
      if (this.animationFrame !== undefined) scope.cancelAnimationFrame?.(this.animationFrame);
      this.animationFrame = undefined;
      clearTimeout(this.loopTimer);
      try {
        if (this.session) this.step(performance.now());
      } catch (error) {
        this.log("warn", `player loop: ${describeError(error)}`);
      }
      this.scheduleLoop();
    };
    this.loopTimer = setTimeout(() => tick(false), this.animationFramesWork ? 50 : 16);
    if (typeof scope.requestAnimationFrame === "function") {
      try {
        this.animationFrame = scope.requestAnimationFrame(() => tick(true));
      } catch {
        // No animation frames in this worker: the timer drives the loop.
      }
    }
  }

  /** The media time to show at `now`: the audio clock when it leads, else the playout clock. */
  private presentationTimeUs(now: number): number {
    const wallUs = this.clock.mediaTimeUs(now);
    this.clockSource = "wall";
    if (this.audioActive && Number.isFinite(wallUs)) {
      const audioUs = this.audioSync.clockUs(now);
      if (Number.isFinite(audioUs)) {
        this.clockSource = "audio";
        return audioUs;
      }
    }
    return wallUs;
  }

  private step(now: number): void {
    const nowUs = this.presentationTimeUs(now);
    this.feed(nowUs);
    if (this.buffer.catchUps >= 2 && this.speed >= 4 && !this.buffer.keyframesOnly) {
      this.buffer.keyframesOnly = true;
      this.log("info", `decoding can't keep up at ${this.speed}×: showing keyframes only`);
    }
    const frame = this.shownSinceReset ? this.buffer.takeDue(nowUs) : this.buffer.takeFirst();
    if (frame) this.present(frame, now);
    this.maybeFlush();
    this.updateState(now, nowUs);

    if (now - this.lastTimeMs >= TIME_INTERVAL_MS) {
      this.lastTimeMs = now;
      if (Number.isFinite(this.lastShownTsUs) && this.lastShownTsUs !== this.lastTimeSent) {
        this.lastTimeSent = this.lastShownTsUs;
        this.post({ type: "time", session: this.session, epochMs: Math.round(this.lastShownTsUs / 1000) });
      }
    }
    if (now - this.lastStatsMs >= STATS_INTERVAL_MS) {
      this.lastStatsMs = now;
      this.post({ type: "stats", session: this.session, stats: this.stats(now, nowUs) });
    }
  }

  private present(frame: ReadyFrame, now: number): void {
    if (!this.renderer) {
      frame.close();
      return;
    }
    this.renderer.show(frame.frame);
    this.lastShownMs = now;
    this.lastShownTsUs = frame.tsUs;
    this.width = frame.frame.displayWidth;
    this.height = frame.frame.displayHeight;
    this.shown.add(now);
    if (Number.isFinite(frame.arrivalMs)) this.latency.add(now, now - frame.arrivalMs);
    if (!this.shownSinceReset) {
      this.shownSinceReset = true;
      this.firstShownTsUs = frame.tsUs;
      // Report the new position right away (e.g. after a seek), not at the next interval.
      this.lastTimeMs = -Infinity;
    }
    if (!this.announcedFrame) {
      this.announcedFrame = true;
      this.post({ type: "frame", session: this.session });
    }
  }

  private updateState(now: number, nowUs: number): void {
    if (this.fatal) return;
    const inFlight = this.decoder?.decodeQueueSize ?? 0;
    if (this.ended) {
      if (this.buffer.isEmpty && inFlight === 0 && this.flushed) {
        this.setState({ kind: "ended" });
        return;
      }
    } else if (
      this.mode === "playback" &&
      !this.rebuffering &&
      this.shownSinceReset &&
      this.buffer.isEmpty &&
      inFlight === 0 &&
      Number.isFinite(nowUs) &&
      nowUs > this.lastVideoTsUs + 2 * this.frameIntervalUs
    ) {
      // Played everything we have: pause the clock until more arrives, then buffer again.
      this.rebuffering = true;
      this.clock.reanchor();
      this.audioSync.reset();
      this.sendAudio({ type: "flush" });
    }

    if (!this.receivedAny) {
      this.setState({ kind: "connecting" });
      return;
    }
    const running = this.clock.started && Number.isFinite(nowUs) && nowUs >= this.firstShownTsUs;
    const stallMs = Math.max(STALL_MS, (3 * this.frameIntervalUs) / 1000 / this.speed);
    const stalled = !(now - this.lastShownMs <= stallMs);
    const buffering = !this.shownSinceReset || !running || this.rebuffering || this.backendBuffering || stalled;
    this.setState({ kind: buffering && !this.ended ? "buffering" : "playing" });
  }

  private setState(state: PlayerState): void {
    if (sameState(this.state, state)) return;
    this.state = state;
    if (this.session) this.post({ type: "state", session: this.session, state });
  }

  private stats(now: number, nowUs: number): PlayerStats {
    const newest = this.buffer.newestTsUs;
    const latency = this.latency.mean(now);
    const decodeMs = this.decodeTime.mean(now);
    return {
      codec: this.config?.config.codec,
      width: this.width,
      height: this.height,
      fps: Math.round(this.shown.perSecond(now) * 10) / 10,
      bitrate: Math.round(this.bytes.perSecond(now) * 8),
      droppedFrames: this.buffer.dropped + this.droppedOther,
      decodeQueue: this.decoder?.decodeQueueSize ?? 0,
      latencyMs: Number.isFinite(latency) ? Math.round(latency) : undefined,
      bufferMs:
        Number.isFinite(newest) && Number.isFinite(nowUs)
          ? Math.max(0, Math.round((newest - nowUs) / 1000 / this.speed))
          : 0,
      targetDelayMs: Math.round(this.clock.delayMs),
      decodeMs: Number.isFinite(decodeMs) ? Math.round(decodeMs * 10) / 10 : undefined,
      audioOffsetMs:
        this.audioActive && Number.isFinite(this.audioSync.errorUs) ? Math.round(this.audioSync.errorUs / 1000) : undefined,
      clock: this.clockSource,
    };
  }

  // --- Audio ----------------------------------------------------------------------------------

  private get audioActive(): boolean {
    return this.audioEnabled && this.audioPort !== undefined && this.speed === 1 && this.audioConfig !== undefined && !this.fatal;
  }

  private onAudioConfig(config: AudioConfigInfo): void {
    if (config.format !== "s16le") {
      this.log("warn", `unsupported audio format ${config.format}`);
      this.audioConfig = undefined;
      return;
    }
    const current = this.audioConfig;
    if (current && current.sampleRate === config.sampleRate && current.channels === config.channels) return;
    this.audioConfig = config;
    this.audioSync.reset();
    this.sendAudio({ type: "config", sampleRate: config.sampleRate, channels: config.channels });
    this.post({ type: "audioConfig", session: this.session, sampleRate: config.sampleRate, channels: config.channels });
  }

  private onAudioPcm(tsUs: number, samples: Int16Array): void {
    if (!this.audioActive || this.audioConfig === undefined) return;
    if (samples.length % this.audioConfig.channels !== 0) return;
    const data = s16ToFloat32(samples);
    this.sendAudio({ type: "pcm", tsUs, data }, [data.buffer]);
  }

  private setAudioEnabled(enabled: boolean): void {
    if (enabled === this.audioEnabled) return;
    this.audioEnabled = enabled;
    if (!enabled) {
      this.sendAudio({ type: "flush" });
      this.audioSync.reset();
    }
  }

  private attachAudioPort(port: MessagePort, latencyMs: number): void {
    this.audioPort?.close();
    this.audioPort = port;
    this.audioSync.reset();
    this.audioSync.latencyUs = latencyMs * 1000;
    port.onmessage = (event: MessageEvent<WorkletReport>) => this.onWorkletReport(event.data);
    if (this.audioConfig) {
      this.sendAudio({ type: "config", sampleRate: this.audioConfig.sampleRate, channels: this.audioConfig.channels });
    }
  }

  private onWorkletReport(report: WorkletReport): void {
    if (!this.audioActive || report?.type !== "report") return;
    const now = performance.now();
    const command: AudioCommand | undefined = this.audioSync.onReport(report, now, this.clock.mediaTimeUs(now));
    if (!command) return;
    if (command.type !== "rate") {
      const error = this.audioSync.errorUs;
      this.log("info", `audio ${command.type}${Number.isFinite(error) ? ` (off by ${Math.round(error / 1000)} ms)` : ""}`);
    }
    this.sendAudio(command);
  }

  private sendAudio(command: WorkletCommand, transfer: Transferable[] = []): void {
    this.audioPort?.postMessage(command, transfer);
  }

  // --- Misc -----------------------------------------------------------------------------------

  private async snapshot(id: number, options?: SnapshotOptions): Promise<void> {
    try {
      if (!this.renderer) throw new Error("the player is not ready");
      this.post({ type: "snapshot", id, blob: await this.renderer.snapshot(options) });
    } catch (error) {
      this.post({ type: "snapshot", id, error: describeError(error) });
    }
  }

  private log(level: "info" | "warn", message: string): void {
    if (this.logBudget <= 0) return;
    this.logBudget--;
    this.post({ type: "log", session: this.session, level, message });
  }

  private post(event: WorkerEvent): void {
    scope.postMessage(event);
  }
}

const engine = new MediaEngine();
scope.onmessage = (event) => engine.handle(event.data);

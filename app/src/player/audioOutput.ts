// Main-thread side of the audio path: owns the AudioContext and the worklet node, and connects the
// worklet to the media worker with a MessageChannel. Created lazily, the first time the player is
// unmuted with an audio stream, so muted tiles in a multi-view cost nothing.
//
// Autoplay: a context created before any user gesture starts suspended. We try to resume it right
// away and again on the next pointer or key event. Video never waits for it: until the worklet
// runs and reports its position, the media worker keeps the wall clock.

import type { WorkerCommand } from "./protocol";
import { WORKLET_PROCESSOR } from "./protocol";
import workletUrl from "./audio.worklet.ts?worker&url";

const GESTURES = ["pointerdown", "keydown", "touchend"] as const;
const LATENCY_POLL_MS = 2000;

export class AudioOutput {
  private muted = true;
  private format: { sampleRate: number; channels: number } | undefined;
  private context: AudioContext | undefined;
  private contextKey = "";
  private node: AudioWorkletNode | undefined;
  private latencyTimer: ReturnType<typeof setInterval> | undefined;
  private disposed = false;
  private readonly resumeOnGesture = () => this.resume();

  constructor(private readonly post: (command: WorkerCommand, transfer?: Transferable[]) => void) {}

  /** The AudioContext's state, for diagnostics. */
  get contextState(): AudioContextState | "none" {
    return this.context?.state ?? "none";
  }

  setMuted(muted: boolean): void {
    this.muted = muted;
    this.post({ type: "audio", enabled: !muted });
    if (muted) {
      void this.context?.suspend().catch(() => {});
    } else {
      this.ensure();
    }
  }

  /** The stream's audio format, reported by the media worker. */
  setFormat(sampleRate: number, channels: number): void {
    this.format = { sampleRate, channels };
    if (!this.muted) this.ensure();
  }

  dispose(): void {
    this.disposed = true;
    this.teardown();
  }

  private ensure(): void {
    if (this.disposed || this.muted || !this.format) return;
    const key = `${this.format.sampleRate}/${this.format.channels}`;
    if (this.context && this.contextKey === key) {
      this.resume();
      return;
    }
    this.teardown();
    this.contextKey = key;
    this.create(this.format.sampleRate, this.format.channels).catch((error) => {
      console.warn("Backsight player: audio output unavailable", error);
    });
  }

  private async create(sampleRate: number, channels: number): Promise<void> {
    let context: AudioContext;
    try {
      // Chromium runs contexts at 8 kHz fine and resamples to the device itself.
      context = new AudioContext({ sampleRate, latencyHint: "interactive" });
    } catch {
      // Unsupported rate: run at the device rate; the worklet resamples.
      context = new AudioContext({ latencyHint: "interactive" });
    }
    this.context = context;
    for (const type of GESTURES) window.addEventListener(type, this.resumeOnGesture, { capture: true, passive: true });
    this.resume();

    await context.audioWorklet.addModule(workletUrl);
    if (this.context !== context || this.disposed) return;
    const node = new AudioWorkletNode(context, WORKLET_PROCESSOR, {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [channels],
    });
    node.connect(context.destination);
    this.node = node;

    const channel = new MessageChannel();
    node.port.postMessage({ type: "port", port: channel.port1 }, [channel.port1]);
    this.post({ type: "audioPort", port: channel.port2, latencyMs: this.latencyMs() }, [channel.port2]);
    this.latencyTimer = setInterval(() => {
      this.post({ type: "audioLatency", latencyMs: this.latencyMs() });
    }, LATENCY_POLL_MS);
  }

  private resume(): void {
    const context = this.context;
    if (!context || this.muted || context.state === "running" || context.state === "closed") return;
    void context.resume().catch(() => {});
  }

  private latencyMs(): number {
    const context = this.context;
    if (!context) return 0;
    return ((context.baseLatency || 0) + (context.outputLatency || 0)) * 1000;
  }

  private teardown(): void {
    for (const type of GESTURES) window.removeEventListener(type, this.resumeOnGesture, { capture: true });
    clearInterval(this.latencyTimer);
    this.latencyTimer = undefined;
    this.node?.disconnect();
    this.node = undefined;
    void this.context?.close().catch(() => {});
    this.context = undefined;
    this.contextKey = "";
  }
}

// The audio output: an AudioWorkletProcessor that plays PCM sent by the media worker over a
// MessagePort (no SharedArrayBuffer, so no COOP/COEP requirement). The buffering, resampling and
// drift correction live in `PcmPlayer`; the media worker decides when to start, jump or adjust
// the rate, based on the position reports sent from here.
//
// Loaded with `audioWorklet.addModule()` from a `?worker&url` import, so Vite bundles it.

import { PcmPlayer } from "./pcm";
import { WORKLET_PROCESSOR, type WorkletCommand, type WorkletReport, type WorkletSetup } from "./protocol";

// AudioWorkletGlobalScope, which the DOM typings don't describe.
declare const sampleRate: number;
declare const currentTime: number;
declare function registerProcessor(name: string, processor: new () => AudioWorkletProcessor): void;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}

/** Seconds between position reports. */
const REPORT_INTERVAL = 0.025;

class PcmProcessor extends AudioWorkletProcessor {
  private readonly player = new PcmPlayer(sampleRate);
  private media: MessagePort | undefined;
  private lastReport = -Infinity;

  constructor() {
    super();
    this.port.onmessage = (event: MessageEvent<WorkletSetup>) => {
      if (event.data?.type !== "port") return;
      this.media?.close();
      this.media = event.data.port;
      this.media.onmessage = (message: MessageEvent<WorkletCommand>) => this.command(message.data);
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    const output = outputs[0];
    if (output && output.length > 0) this.player.render(output);
    if (this.media && this.player.configured && currentTime - this.lastReport >= REPORT_INTERVAL) {
      this.lastReport = currentTime;
      const report: WorkletReport = { type: "report", ...this.player.status() };
      this.media.postMessage(report);
    }
    return true;
  }

  private command(command: WorkletCommand): void {
    switch (command.type) {
      case "config":
        this.player.configure(command.sampleRate, command.channels);
        break;
      case "pcm":
        this.player.write(command.tsUs, command.data);
        break;
      case "sync":
        this.player.sync(command.positionUs);
        break;
      case "rate":
        this.player.setRate(command.rate);
        break;
      case "flush":
        this.player.flush();
        break;
    }
  }
}

registerProcessor(WORKLET_PROCESSOR, PcmProcessor);

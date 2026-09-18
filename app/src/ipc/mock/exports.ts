// Simulated export queue: one job runs at a time at ~10× real time, emitting `export-progress`
// events as it goes, like the real exporter.

import type { AppEvent, ExportJob } from "../api";
import { toIso } from "@/lib/time";

/** Encoded footage size used for byte counts: roughly a 2.2 Mbps HD stream. */
const BYTES_PER_SECOND_OF_FOOTAGE = 275_000;
/** Export speed relative to real time. */
const SPEEDUP = 10;
const MIN_EXPORT_MS = 3_000;
const MAX_EXPORT_MS = 90_000;

export interface ExportQueueOptions {
  emit: (event: AppEvent) => void;
  now: () => number;
  tickMs?: number;
}

interface Runtime {
  /** Wall-clock export duration for this job. */
  durationMs: number;
  totalBytes: number;
  /** Set when the job will fail partway, with the message to report. */
  failAt?: { progress: number; message: string };
}

export class MockExportQueue {
  private jobs: ExportJob[] = [];
  private runtime = new Map<string, Runtime>();
  private timer: ReturnType<typeof setInterval> | null = null;
  private lastTick = 0;
  private seq = 0;
  private readonly tickMs: number;

  constructor(private readonly options: ExportQueueOptions) {
    this.tickMs = options.tickMs ?? 250;
  }

  list(): ExportJob[] {
    return this.jobs.map((j) => ({ ...j }));
  }

  get(id: string): ExportJob | undefined {
    return this.jobs.find((j) => j.id === id);
  }

  /** Adds historical jobs without emitting events. */
  seed(jobs: ExportJob[]): void {
    this.jobs.push(...jobs.map((j) => ({ ...j })));
  }

  enqueue(
    job: Omit<ExportJob, "id" | "state" | "progress" | "bytesWritten" | "createdAt" | "etaSeconds">,
    options: { failMessage?: string } = {},
  ): ExportJob {
    const clipMs = Math.max(1000, Date.parse(job.end) - Date.parse(job.start));
    const created: ExportJob = {
      ...job,
      id: `export-${Date.now().toString(36)}-${(this.seq++).toString(36)}`,
      state: "queued",
      progress: 0,
      bytesWritten: 0,
      createdAt: toIso(this.options.now()),
    };
    const durationMs = Math.min(MAX_EXPORT_MS, Math.max(MIN_EXPORT_MS, clipMs / SPEEDUP));
    this.runtime.set(created.id, {
      durationMs,
      totalBytes: Math.round((clipMs / 1000) * BYTES_PER_SECOND_OF_FOOTAGE),
      failAt: options.failMessage ? { progress: 0.08, message: options.failMessage } : undefined,
    });
    created.etaSeconds = Math.round(durationMs / 1000);
    this.jobs.push(created);
    this.emit(created);
    this.ensureTimer();
    return { ...created };
  }

  cancel(id: string): boolean {
    const job = this.get(id);
    if (!job) return false;
    if (job.state === "queued" || job.state === "running" || job.state === "paused") {
      job.state = "cancelled";
      job.etaSeconds = undefined;
      this.emit(job);
    }
    return true;
  }

  dispose(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  private emit(job: ExportJob): void {
    this.options.emit({ type: "export-progress", job: { ...job } });
  }

  private ensureTimer(): void {
    if (this.timer) return;
    this.lastTick = this.options.now();
    this.timer = setInterval(() => this.tick(), this.tickMs);
  }

  private tick(): void {
    const now = this.options.now();
    const dt = Math.max(0, now - this.lastTick);
    this.lastTick = now;

    let running = this.jobs.find((j) => j.state === "running");
    if (!running) {
      running = this.jobs.find((j) => j.state === "queued");
      if (running) {
        running.state = "running";
        this.emit(running);
        return;
      }
    }
    if (!running) {
      this.dispose();
      return;
    }

    const rt = this.runtime.get(running.id);
    if (!rt) {
      running.state = "failed";
      running.error = "The export was interrupted.";
      this.emit(running);
      return;
    }

    running.progress = Math.min(1, running.progress + dt / rt.durationMs);
    running.bytesWritten = Math.round(running.progress * rt.totalBytes);
    running.etaSeconds = Math.max(0, Math.round(((1 - running.progress) * rt.durationMs) / 1000));

    if (rt.failAt && running.progress >= rt.failAt.progress) {
      running.state = "failed";
      running.error = rt.failAt.message;
      running.etaSeconds = undefined;
    } else if (running.progress >= 1) {
      running.state = "done";
      running.progress = 1;
      running.bytesWritten = rt.totalBytes;
      running.etaSeconds = undefined;
    }
    this.emit(running);
  }
}

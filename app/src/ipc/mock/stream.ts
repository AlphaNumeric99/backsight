import type { ApiError, BatchListener, StreamHandle, StreamRequest } from "../api";
import { FixturePacer, parseFixture, type Fixture } from "../../player/dev/fixtureStream";
import { encodeBatch, type Packet } from "../../player/wire";

// Streams a recorded fixture (public/fixtures/*.bspk) in the wire format, paced like the real
// backend: live loops forever with wall-clock timestamps; playback starts at `req.start` with a
// discontinuity and runs at `req.speed` (audio only at 1×). Batches carry 1-3 packets.
//
// Mock cameras pick their fixture by id: an id containing "h265" or "hevc" streams H.265 (which
// webviews without HEVC support can't decode); "unsupported" streams it with a codec string no
// decoder accepts, to show the codec_unsupported error anywhere; "short" plays the fixture once
// and then ends.

const FIXTURE_H264 = "testsrc2-720p15-h264.bspk";
const FIXTURE_H265 = "testsrc2-360p15-h265.bspk";
/** An HEVC codec string with a profile that does not exist (Chromium rejects it). */
const UNSUPPORTED_CODEC = "hvc1.99.0.L63.90";

/** Simulated network trouble, for the player harness. */
export const mockNetwork = {
  /** Each batch is delayed by up to this much (order is kept, as over TCP). */
  jitterMs: 0,
  /** Chance that a batch has some payload bytes overwritten with garbage. */
  corruptRate: 0,
  /** Nothing is delivered before this time (`performance.now()`); the backlog follows at once. */
  stalledUntil: 0,
};

/** Open mock streams, for the harness (and to spot leaks). */
export const mockStreamCounters = { open: 0, opened: 0 };

const fixtures = new Map<string, Promise<Fixture>>();

function loadFixture(name: string): Promise<Fixture> {
  let fixture = fixtures.get(name);
  if (!fixture) {
    fixture = fetch(`${import.meta.env.BASE_URL}fixtures/${name}`)
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.arrayBuffer();
      })
      .then(parseFixture);
    fixtures.set(name, fixture);
    fixture.catch(() => fixtures.delete(name));
  }
  return fixture;
}

function reject(code: ApiError["code"], message: string): never {
  throw { code, message } satisfies ApiError;
}

export async function openMockStream(req: StreamRequest, onBatch: BatchListener): Promise<StreamHandle> {
  const id = req.cameraId.toLowerCase();
  const unsupported = id.includes("unsupported");
  const name = unsupported || /h265|hevc/.test(id) ? FIXTURE_H265 : FIXTURE_H264;
  let startEpochUs = Date.now() * 1000;
  if (req.kind === "playback") {
    const start = Date.parse(req.start);
    if (Number.isNaN(start)) reject("invalid_input", `invalid playback start: ${req.start}`);
    if (!(req.speed > 0)) reject("invalid_input", `invalid playback speed: ${req.speed}`);
    startEpochUs = start * 1000;
  }

  let fixture: Fixture;
  try {
    fixture = await loadFixture(name);
  } catch (error) {
    reject("internal", `mock stream: can't load ${name}: ${error instanceof Error ? error.message : error}`);
  }

  if (unsupported) {
    const { videoConfig } = fixture;
    fixture = { ...fixture, videoConfig: { ...videoConfig, config: { ...videoConfig.config, codec: UNSUPPORTED_CODEC } } };
  }
  const speed = req.kind === "playback" ? req.speed : 1;
  const pacer = new FixturePacer(fixture, {
    startEpochUs,
    speed,
    loop: !id.includes("short"),
    audio: speed === 1,
    discontinuity: req.kind === "playback",
  });

  const openedAt = performance.now();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let closed = false;
  mockStreamCounters.open++;
  mockStreamCounters.opened++;

  // Batches go out in order through one queue, like over TCP: jitter and stalls delay them but
  // a later batch never overtakes an earlier one.
  const outbox: { at: number; bytes: Uint8Array<ArrayBuffer> }[] = [];
  let outboxTimer: ReturnType<typeof setTimeout> | undefined;
  const flushOutbox = () => {
    outboxTimer = undefined;
    while (!closed && outbox.length && outbox[0].at <= performance.now()) {
      onBatch((outbox.shift() as { bytes: Uint8Array<ArrayBuffer> }).bytes.buffer);
    }
    if (!closed && outbox.length) outboxTimer = setTimeout(flushOutbox, outbox[0].at - performance.now());
  };
  const deliver = (batch: Packet[]) => {
    const bytes = encodeBatch(batch);
    if (mockNetwork.corruptRate > 0 && Math.random() < mockNetwork.corruptRate) corrupt(bytes);
    const now = performance.now();
    const previous = outbox.length ? outbox[outbox.length - 1].at : 0;
    const at = Math.max(previous, now + Math.random() * mockNetwork.jitterMs, mockNetwork.stalledUntil);
    outbox.push({ at, bytes });
    if (outboxTimer === undefined) flushOutbox();
  };

  const pump = () => {
    timer = undefined;
    if (closed) return;
    for (const batch of pacer.takeDue(performance.now() - openedAt)) deliver(batch);
    const next = pacer.peekDueMs();
    if (next === Infinity) return;
    timer = setTimeout(pump, Math.max(0, next - (performance.now() - openedAt)));
  };
  pump();

  return {
    id: `mock-${mockStreamCounters.opened}`,
    close: async () => {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      clearTimeout(outboxTimer);
      outbox.length = 0;
      mockStreamCounters.open--;
    },
  };
}

/** Overwrites a few bytes after the first packet header, to exercise error handling. */
function corrupt(bytes: Uint8Array): void {
  for (let i = 0; i < 8; i++) {
    const at = 16 + Math.floor(Math.random() * Math.max(1, bytes.length - 16));
    if (at < bytes.length) bytes[at] = Math.floor(Math.random() * 256);
  }
}

// Backsight wire format v1: how media reaches the webview (docs/ARCHITECTURE.md, "Wire format").
//
// A batch is one or more packets back to back. Each packet starts with a 16-byte header, all
// integers little-endian:
//
//   0  u32  total packet length in bytes, including the header
//   4  u8   kind
//   5  u8   flags: bit 0 = keyframe, bit 1 = discontinuity
//   6  u16  reserved, 0
//   8  i64  timestamp, microseconds since the Unix epoch (UTC), 0 when not applicable
//   16 ...  payload
//
// `decodeBatch` never throws: malformed input is reported in `errors`, and decoding continues
// with the next packet whenever the packet boundaries are still known. The encoder is used by the
// mock backend, the fixture tooling and the tests; `app/src-tauri/src/wire.rs` is the real one.
// Both encoders must produce identical bytes (see `__fixtures__/wire/`).
//
// Keep this module free of imports and non-erasable TypeScript syntax: Node scripts import it
// directly.

export const WIRE_VERSION = 1;
export const HEADER_BYTES = 16;

export const PacketKind = {
  VideoConfig: 1,
  VideoFrame: 2,
  AudioConfig: 3,
  AudioPcm: 4,
  Status: 5,
  EndOfStream: 6,
} as const;

export const PacketFlag = {
  Keyframe: 1,
  Discontinuity: 2,
} as const;

/** The JSON part of a `VideoConfig` packet. */
export interface VideoConfigInfo {
  /** RFC 6381 codec string, usable directly as a WebCodecs `codec`, e.g. "avc1.64001F". */
  codec: string;
  codedWidth: number;
  codedHeight: number;
}

/** The payload of an `AudioConfig` packet. */
export interface AudioConfigInfo {
  sampleRate: number;
  channels: number;
  /** Only "s16le" (interleaved signed 16-bit little-endian PCM) exists in v1. */
  format: string;
}

export type StreamStatusState = "buffering" | "playing" | "ended" | "error";

/** The payload of a `Status` packet. */
export interface StreamStatus {
  /** One of `StreamStatusState`; unknown states from newer backends are passed through. */
  state: StreamStatusState | (string & {});
  code?: string;
  message?: string;
}

interface PacketBase {
  /** Microseconds since the Unix epoch (UTC), 0 when not applicable. */
  timestampUs: number;
  /** Reset decoders and the clock before handling this packet. */
  discontinuity: boolean;
}

export interface VideoConfigPacket extends PacketBase {
  kind: "videoConfig";
  config: VideoConfigInfo;
  /** avcC / hvcC decoder configuration record (a copy, safe to keep). */
  description: Uint8Array;
}

export interface VideoFramePacket extends PacketBase {
  kind: "videoFrame";
  keyframe: boolean;
  /** One access unit of length-prefixed NAL units (a view into the batch). */
  data: Uint8Array;
}

export interface AudioConfigPacket extends PacketBase {
  kind: "audioConfig";
  config: AudioConfigInfo;
}

export interface AudioPcmPacket extends PacketBase {
  kind: "audioPcm";
  /** Interleaved samples. */
  samples: Int16Array;
}

export interface StatusPacket extends PacketBase {
  kind: "status";
  status: StreamStatus;
}

export interface EndOfStreamPacket extends PacketBase {
  kind: "endOfStream";
}

export type Packet =
  | VideoConfigPacket
  | VideoFramePacket
  | AudioConfigPacket
  | AudioPcmPacket
  | StatusPacket
  | EndOfStreamPacket;

export type WireErrorCode =
  /** The batch ends in the middle of a packet. Decoding stops. */
  | "truncated"
  /** A packet claims to be shorter than its header. Decoding stops. */
  | "bad_length"
  /** Unknown packet kind (maybe from a newer backend). The packet is skipped. */
  | "unknown_kind"
  /** The timestamp does not fit a JavaScript number. The packet is skipped. */
  | "bad_timestamp"
  /** The payload has the wrong size or shape for its kind. The packet is skipped. */
  | "bad_payload"
  /** The payload's JSON is invalid or misses required fields. The packet is skipped. */
  | "bad_json";

export interface WireError {
  /** Byte offset of the offending packet within the batch. */
  offset: number;
  code: WireErrorCode;
  message: string;
}

export interface DecodedBatch {
  packets: Packet[];
  errors: WireError[];
}

const KIND_NAMES: Record<number, Packet["kind"]> = {
  [PacketKind.VideoConfig]: "videoConfig",
  [PacketKind.VideoFrame]: "videoFrame",
  [PacketKind.AudioConfig]: "audioConfig",
  [PacketKind.AudioPcm]: "audioPcm",
  [PacketKind.Status]: "status",
  [PacketKind.EndOfStream]: "endOfStream",
};

const KIND_CODES: Record<Packet["kind"], number> = {
  videoConfig: PacketKind.VideoConfig,
  videoFrame: PacketKind.VideoFrame,
  audioConfig: PacketKind.AudioConfig,
  audioPcm: PacketKind.AudioPcm,
  status: PacketKind.Status,
  endOfStream: PacketKind.EndOfStream,
};

const TWO_POW_32 = 4294967296;
const utf8Decoder = new TextDecoder("utf-8", { fatal: true });
const utf8Encoder = new TextEncoder();

// ---------------------------------------------------------------------------------------------
// Decoding

type PayloadResult = Packet | { code: WireErrorCode; message: string };

/** Splits a batch into packets. Never throws. */
export function decodeBatch(input: ArrayBuffer | ArrayBufferView): DecodedBatch {
  const bytes = ArrayBuffer.isView(input)
    ? new Uint8Array(input.buffer, input.byteOffset, input.byteLength)
    : new Uint8Array(input);
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const packets: Packet[] = [];
  const errors: WireError[] = [];

  let offset = 0;
  while (offset < bytes.length) {
    const remaining = bytes.length - offset;
    if (remaining < HEADER_BYTES) {
      errors.push({
        offset,
        code: "truncated",
        message: `${remaining} trailing bytes are too short for a packet header`,
      });
      break;
    }
    const total = view.getUint32(offset, true);
    if (total < HEADER_BYTES) {
      errors.push({
        offset,
        code: "bad_length",
        message: `packet length ${total} is shorter than the ${HEADER_BYTES}-byte header`,
      });
      break;
    }
    if (total > remaining) {
      errors.push({
        offset,
        code: "truncated",
        message: `packet length ${total} exceeds the ${remaining} bytes left in the batch`,
      });
      break;
    }

    const kindCode = view.getUint8(offset + 4);
    const flags = view.getUint8(offset + 5);
    const timestampUs = view.getInt32(offset + 12, true) * TWO_POW_32 + view.getUint32(offset + 8, true);
    const kind = KIND_NAMES[kindCode];
    if (kind === undefined) {
      errors.push({ offset, code: "unknown_kind", message: `unknown packet kind ${kindCode}` });
    } else if (!Number.isSafeInteger(timestampUs)) {
      errors.push({ offset, code: "bad_timestamp", message: `${kind} timestamp is out of range` });
    } else {
      const payload = bytes.subarray(offset + HEADER_BYTES, offset + total);
      const result = decodePayload(kind, flags, timestampUs, payload);
      if ("kind" in result) packets.push(result);
      else errors.push({ offset, code: result.code, message: result.message });
    }
    offset += total;
  }
  return { packets, errors };
}

function decodePayload(
  kind: Packet["kind"],
  flags: number,
  timestampUs: number,
  payload: Uint8Array,
): PayloadResult {
  const discontinuity = (flags & PacketFlag.Discontinuity) !== 0;
  switch (kind) {
    case "videoConfig": {
      if (payload.length < 2) {
        return { code: "bad_payload", message: "videoConfig payload has no JSON length" };
      }
      const jsonLength = payload[0] | (payload[1] << 8);
      if (2 + jsonLength > payload.length) {
        return {
          code: "bad_payload",
          message: `videoConfig JSON length ${jsonLength} exceeds its ${payload.length - 2}-byte payload`,
        };
      }
      const json = parseJsonObject(payload.subarray(2, 2 + jsonLength));
      if (typeof json === "string") return { code: "bad_json", message: `videoConfig: ${json}` };
      const { codec, codedWidth, codedHeight } = json;
      if (typeof codec !== "string" || codec.length === 0) {
        return { code: "bad_json", message: "videoConfig: missing codec" };
      }
      if (!isDimension(codedWidth) || !isDimension(codedHeight)) {
        return { code: "bad_json", message: "videoConfig: invalid codedWidth/codedHeight" };
      }
      return {
        kind,
        timestampUs,
        discontinuity,
        config: { codec, codedWidth, codedHeight },
        description: payload.slice(2 + jsonLength),
      };
    }
    case "videoFrame": {
      if (payload.length === 0) return { code: "bad_payload", message: "empty videoFrame" };
      return {
        kind,
        timestampUs,
        discontinuity,
        keyframe: (flags & PacketFlag.Keyframe) !== 0,
        data: payload,
      };
    }
    case "audioConfig": {
      const json = parseJsonObject(payload);
      if (typeof json === "string") return { code: "bad_json", message: `audioConfig: ${json}` };
      const { sampleRate, channels, format } = json;
      if (!isInteger(sampleRate, 1, 768000) || !isInteger(channels, 1, 32) || typeof format !== "string") {
        return { code: "bad_json", message: "audioConfig: invalid sampleRate/channels/format" };
      }
      return { kind, timestampUs, discontinuity, config: { sampleRate, channels, format } };
    }
    case "audioPcm": {
      if (payload.length % 2 !== 0) {
        return { code: "bad_payload", message: `audioPcm has an odd length (${payload.length} bytes)` };
      }
      const samples = new Int16Array(payload.length / 2);
      const view = new DataView(payload.buffer, payload.byteOffset, payload.byteLength);
      for (let i = 0; i < samples.length; i++) samples[i] = view.getInt16(i * 2, true);
      return { kind, timestampUs, discontinuity, samples };
    }
    case "status": {
      const json = parseJsonObject(payload);
      if (typeof json === "string") return { code: "bad_json", message: `status: ${json}` };
      const { state, code, message } = json;
      if (typeof state !== "string") return { code: "bad_json", message: "status: missing state" };
      if ((code !== undefined && typeof code !== "string") || (message !== undefined && typeof message !== "string")) {
        return { code: "bad_json", message: "status: code/message must be strings" };
      }
      const status: StreamStatus = { state };
      if (code !== undefined) status.code = code;
      if (message !== undefined) status.message = message;
      return { kind, timestampUs, discontinuity, status };
    }
    case "endOfStream":
      // A payload is not defined for v1; ignore one if present.
      return { kind, timestampUs, discontinuity };
  }
}

function parseJsonObject(bytes: Uint8Array): Record<string, unknown> | string {
  let text: string;
  try {
    text = utf8Decoder.decode(bytes);
  } catch {
    return "invalid UTF-8";
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return "invalid JSON";
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) return "JSON is not an object";
  return value as Record<string, unknown>;
}

function isInteger(value: unknown, min: number, max: number): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= min && value <= max;
}

function isDimension(value: unknown): value is number {
  return isInteger(value, 1, 65535);
}

// ---------------------------------------------------------------------------------------------
// Encoding

/**
 * Encodes packets into one batch. Throws `RangeError` for values the format can't carry
 * (unsafe timestamps, oversized JSON); callers own their input, unlike the decoder's.
 */
export function encodeBatch(packets: readonly Packet[]): Uint8Array<ArrayBuffer> {
  const encoded = packets.map(encodePayload);
  let total = 0;
  for (const p of encoded) total += HEADER_BYTES + p.payloadLength;
  const out = new Uint8Array(total);
  const view = new DataView(out.buffer);
  let offset = 0;
  for (const p of encoded) {
    const length = HEADER_BYTES + p.payloadLength;
    view.setUint32(offset, length, true);
    view.setUint8(offset + 4, p.kind);
    view.setUint8(offset + 5, p.flags);
    view.setUint16(offset + 6, 0, true);
    view.setBigInt64(offset + 8, BigInt(p.timestampUs), true);
    let at = offset + HEADER_BYTES;
    for (const part of p.parts) {
      out.set(part, at);
      at += part.length;
    }
    offset += length;
  }
  return out;
}

export function encodePacket(packet: Packet): Uint8Array<ArrayBuffer> {
  return encodeBatch([packet]);
}

interface EncodedPayload {
  kind: number;
  flags: number;
  timestampUs: number;
  parts: Uint8Array[];
  payloadLength: number;
}

function encodePayload(packet: Packet): EncodedPayload {
  if (!Number.isSafeInteger(packet.timestampUs)) {
    throw new RangeError(`timestamp ${packet.timestampUs} is not a safe integer`);
  }
  let flags = packet.discontinuity ? PacketFlag.Discontinuity : 0;
  let parts: Uint8Array[];
  switch (packet.kind) {
    case "videoConfig": {
      const { codec, codedWidth, codedHeight } = packet.config;
      const json = utf8Encoder.encode(JSON.stringify({ codec, codedWidth, codedHeight }));
      if (json.length > 0xffff) throw new RangeError("videoConfig JSON is longer than 65535 bytes");
      parts = [new Uint8Array([json.length & 0xff, json.length >> 8]), json, packet.description];
      break;
    }
    case "videoFrame":
      if (packet.keyframe) flags |= PacketFlag.Keyframe;
      parts = [packet.data];
      break;
    case "audioConfig": {
      const { sampleRate, channels, format } = packet.config;
      parts = [utf8Encoder.encode(JSON.stringify({ sampleRate, channels, format }))];
      break;
    }
    case "audioPcm": {
      const pcm = new Uint8Array(packet.samples.length * 2);
      const view = new DataView(pcm.buffer);
      for (let i = 0; i < packet.samples.length; i++) view.setInt16(i * 2, packet.samples[i], true);
      parts = [pcm];
      break;
    }
    case "status": {
      const { state, code, message } = packet.status;
      const json: StreamStatus = { state };
      if (code !== undefined) json.code = code;
      if (message !== undefined) json.message = message;
      parts = [utf8Encoder.encode(JSON.stringify(json))];
      break;
    }
    case "endOfStream":
      parts = [];
      break;
  }
  let payloadLength = 0;
  for (const part of parts) payloadLength += part.length;
  return { kind: KIND_CODES[packet.kind], flags, timestampUs: packet.timestampUs, parts, payloadLength };
}

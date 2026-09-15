// Turns the JSON description of the wire golden vectors (vectors.json) into packets.
// Shared by wire.test.ts and scripts/make-wire-vectors.mjs, so it only has type imports.

import type { Packet } from "../../wire";

export interface VectorSpec {
  name: string;
  packets: PacketSpec[];
}

/** A packet as written in vectors.json: the packet fields, with bytes as lowercase hex. */
export type PacketSpec = {
  kind: Packet["kind"];
  timestampUs: number;
  discontinuity: boolean;
} & Record<string, unknown>;

export function hexToBytes(hex: string): Uint8Array<ArrayBuffer> {
  if (hex.length % 2 !== 0 || /[^0-9a-f]/.test(hex)) throw new Error(`bad hex: ${hex}`);
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function bytesToHex(bytes: Uint8Array): string {
  let hex = "";
  for (const b of bytes) hex += b.toString(16).padStart(2, "0");
  return hex;
}

export function specToPacket(spec: PacketSpec): Packet {
  const { kind, timestampUs, discontinuity } = spec;
  const base = { timestampUs, discontinuity };
  switch (kind) {
    case "videoConfig":
      return {
        ...base,
        kind,
        config: {
          codec: spec.codec as string,
          codedWidth: spec.codedWidth as number,
          codedHeight: spec.codedHeight as number,
        },
        description: hexToBytes(spec.descriptionHex as string),
      };
    case "videoFrame":
      return { ...base, kind, keyframe: spec.keyframe as boolean, data: hexToBytes(spec.dataHex as string) };
    case "audioConfig":
      return {
        ...base,
        kind,
        config: {
          sampleRate: spec.sampleRate as number,
          channels: spec.channels as number,
          format: spec.format as string,
        },
      };
    case "audioPcm":
      return { ...base, kind, samples: Int16Array.from(spec.samples as number[]) };
    case "status": {
      const status: { state: string; code?: string; message?: string } = { state: spec.state as string };
      if (spec.code !== undefined) status.code = spec.code as string;
      if (spec.message !== undefined) status.message = spec.message as string;
      return { ...base, kind, status };
    }
    case "endOfStream":
      return { ...base, kind };
  }
}

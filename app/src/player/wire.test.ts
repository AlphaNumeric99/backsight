import { describe, expect, it } from "vitest";
import spec from "./__fixtures__/wire/vectors.json";
import { hexToBytes, specToPacket, type VectorSpec } from "./__fixtures__/wire/vectors";
import { decodeBatch, encodeBatch, HEADER_BYTES, PacketKind, type Packet } from "./wire";

// Golden files, as data: URLs (Vite inlines any file with `?url&inline`).
const goldenUrls = import.meta.glob<string>("./__fixtures__/wire/*.bspk", {
  query: "?url&inline",
  import: "default",
  eager: true,
});

function golden(name: string): Uint8Array {
  const url = goldenUrls[`./__fixtures__/wire/${name}.bspk`];
  if (!url) throw new Error(`missing golden file ${name}.bspk`);
  const binary = atob(url.slice(url.indexOf(",") + 1));
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

const vectors = spec.vectors as VectorSpec[];

function header(length: number, kind: number, flags = 0, timestampUs = 0n): Uint8Array {
  const out = new Uint8Array(HEADER_BYTES);
  const view = new DataView(out.buffer);
  view.setUint32(0, length, true);
  view.setUint8(4, kind);
  view.setUint8(5, flags);
  view.setBigInt64(8, timestampUs, true);
  return out;
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

const eos = (timestampUs = 0): Packet => ({ kind: "endOfStream", timestampUs, discontinuity: false });

describe("wire golden vectors", () => {
  it("has a .bspk file for every vector and no strays", () => {
    const names = vectors.map((v) => `./__fixtures__/wire/${v.name}.bspk`).sort();
    expect(Object.keys(goldenUrls).sort()).toEqual(names);
  });

  for (const vector of vectors) {
    it(`${vector.name}: decodes to its description`, () => {
      const { packets, errors } = decodeBatch(golden(vector.name));
      expect(errors).toEqual([]);
      expect(packets).toEqual(vector.packets.map(specToPacket));
    });

    it(`${vector.name}: encodes to the golden bytes`, () => {
      expect(encodeBatch(vector.packets.map(specToPacket))).toEqual(golden(vector.name));
    });
  }

  it("decodes packet fields exactly", () => {
    const [config] = decodeBatch(golden("video_config_h264")).packets;
    expect(config).toMatchObject({
      kind: "videoConfig",
      timestampUs: 1790000000000000,
      discontinuity: false,
      config: { codec: "avc1.4D401F", codedWidth: 1280, codedHeight: 720 },
    });
    const [key] = decodeBatch(golden("video_frame_key")).packets;
    expect(key).toMatchObject({ kind: "videoFrame", keyframe: true, discontinuity: true });
    const [status] = decodeBatch(golden("status_error")).packets;
    expect(status).toMatchObject({
      kind: "status",
      status: { state: "error", code: "stream_limit", message: 'Too many viewers on "Porch" — close one' },
    });
  });

  it("round-trips the extreme timestamps", () => {
    const { packets } = decodeBatch(golden("batch_mixed"));
    expect(packets.map((p) => p.timestampUs).slice(-2)).toEqual([Number.MAX_SAFE_INTEGER, -1]);
  });

  it("accepts a batch given as a view with an offset", () => {
    const bytes = golden("batch_mixed");
    const shifted = new Uint8Array(bytes.length + 3);
    shifted.set(bytes, 3);
    const { packets, errors } = decodeBatch(shifted.subarray(3));
    expect(errors).toEqual([]);
    expect(packets.map((p) => p.kind)).toEqual([
      "videoConfig",
      "audioConfig",
      "videoFrame",
      "audioPcm",
      "status",
      "endOfStream",
    ]);
  });

  it("copies the decoder description but views frame data", () => {
    const bytes = golden("batch_mixed");
    const { packets } = decodeBatch(bytes);
    const config = packets[0] as Extract<Packet, { kind: "videoConfig" }>;
    const frame = packets[2] as Extract<Packet, { kind: "videoFrame" }>;
    expect(config.description.buffer).not.toBe(bytes.buffer);
    expect(frame.data.buffer).toBe(bytes.buffer);
  });
});

describe("wire decoder on bad input", () => {
  it("returns nothing for an empty batch", () => {
    expect(decodeBatch(new ArrayBuffer(0))).toEqual({ packets: [], errors: [] });
  });

  it("reports trailing bytes shorter than a header", () => {
    const { packets, errors } = decodeBatch(concat(encodeBatch([eos()]), new Uint8Array(5)));
    expect(packets).toHaveLength(1);
    expect(errors).toMatchObject([{ offset: 16, code: "truncated" }]);
  });

  it("stops at a length shorter than the header", () => {
    const { packets, errors } = decodeBatch(concat(header(8, PacketKind.EndOfStream), encodeBatch([eos()])));
    expect(packets).toEqual([]);
    expect(errors).toMatchObject([{ offset: 0, code: "bad_length" }]);
  });

  it("stops at a length beyond the batch", () => {
    const { packets, errors } = decodeBatch(concat(encodeBatch([eos()]), header(64, PacketKind.EndOfStream)));
    expect(packets).toHaveLength(1);
    expect(errors).toMatchObject([{ offset: 16, code: "truncated" }]);
  });

  it("skips unknown kinds and keeps going", () => {
    const unknown = concat(header(20, 42), new Uint8Array(4));
    const { packets, errors } = decodeBatch(concat(unknown, encodeBatch([eos(7)])));
    expect(packets).toEqual([eos(7)]);
    expect(errors).toMatchObject([{ offset: 0, code: "unknown_kind" }]);
  });

  it("skips timestamps beyond the safe integer range", () => {
    const tooBig = header(16, PacketKind.EndOfStream, 0, 2n ** 53n);
    const tooSmall = header(16, PacketKind.EndOfStream, 0, -(2n ** 53n));
    const { packets, errors } = decodeBatch(concat(tooBig, tooSmall, encodeBatch([eos(1)])));
    expect(packets).toEqual([eos(1)]);
    expect(errors.map((e) => e.code)).toEqual(["bad_timestamp", "bad_timestamp"]);
  });

  it("ignores unknown flag bits and a nonzero reserved field", () => {
    const bytes = encodeBatch([eos(5)]);
    bytes[5] = 0xfc;
    bytes[6] = 0xff;
    expect(decodeBatch(bytes)).toEqual({ packets: [eos(5)], errors: [] });
  });

  it("validates videoConfig payloads", () => {
    const json = (s: string) => new TextEncoder().encode(s);
    const configPacket = (payload: Uint8Array) =>
      concat(header(HEADER_BYTES + payload.length, PacketKind.VideoConfig), payload);
    const withLength = (body: Uint8Array, length = body.length) =>
      concat(new Uint8Array([length & 0xff, length >> 8]), body);

    const cases: [Uint8Array, string][] = [
      [new Uint8Array([1]), "bad_payload"],
      [withLength(json("{}"), 300), "bad_payload"],
      [withLength(json("{not json")), "bad_json"],
      [withLength(json("[1,2]")), "bad_json"],
      [withLength(new Uint8Array([0x7b, 0xff, 0x7d])), "bad_json"],
      [withLength(json('{"codec":"","codedWidth":1,"codedHeight":1}')), "bad_json"],
      [withLength(json('{"codec":"avc1.42E01E","codedWidth":0,"codedHeight":720}')), "bad_json"],
      [withLength(json('{"codec":"avc1.42E01E","codedWidth":1.5,"codedHeight":720}')), "bad_json"],
    ];
    for (const [payload, code] of cases) {
      const { packets, errors } = decodeBatch(concat(configPacket(payload), encodeBatch([eos()])));
      expect(packets, code).toEqual([eos()]);
      expect(errors.map((e) => e.code)).toEqual([code]);
    }
  });

  it("accepts a videoConfig without a description", () => {
    const body = new TextEncoder().encode('{"codec":"vp8","codedWidth":640,"codedHeight":480}');
    const payload = concat(new Uint8Array([body.length, 0]), body);
    const { packets, errors } = decodeBatch(concat(header(HEADER_BYTES + payload.length, 1), payload));
    expect(errors).toEqual([]);
    expect(packets[0]).toMatchObject({ kind: "videoConfig", description: new Uint8Array(0) });
  });

  it("rejects an empty videoFrame and odd PCM", () => {
    const { packets, errors } = decodeBatch(
      concat(header(16, PacketKind.VideoFrame), header(19, PacketKind.AudioPcm), new Uint8Array(3)),
    );
    expect(packets).toEqual([]);
    expect(errors.map((e) => e.code)).toEqual(["bad_payload", "bad_payload"]);
  });

  it("validates audioConfig and status JSON", () => {
    const packet = (kind: number, text: string) => {
      const body = new TextEncoder().encode(text);
      return concat(header(HEADER_BYTES + body.length, kind), body);
    };
    const { packets, errors } = decodeBatch(
      concat(
        packet(PacketKind.AudioConfig, '{"sampleRate":0,"channels":1,"format":"s16le"}'),
        packet(PacketKind.AudioConfig, '{"sampleRate":8000,"channels":1}'),
        packet(PacketKind.Status, '{"code":"x"}'),
        packet(PacketKind.Status, '{"state":"error","message":5}'),
        packet(PacketKind.Status, '{"state":"reconnecting"}'),
      ),
    );
    expect(errors.map((e) => e.code)).toEqual(["bad_json", "bad_json", "bad_json", "bad_json"]);
    // Unknown states from a newer backend pass through.
    expect(packets).toMatchObject([{ kind: "status", status: { state: "reconnecting" } }]);
  });

  it("never throws on random garbage", () => {
    let seed = 12345;
    const random = () => {
      seed = (seed * 1103515245 + 12345) >>> 0;
      return seed / 2 ** 32;
    };
    const valid = golden("batch_mixed");
    for (let round = 0; round < 2000; round++) {
      const bytes =
        round % 2 === 0
          ? Uint8Array.from({ length: Math.floor(random() * 96) }, () => Math.floor(random() * 256))
          : valid.map((b) => (random() < 0.02 ? Math.floor(random() * 256) : b));
      const result = decodeBatch(bytes);
      expect(result.packets.length + result.errors.length).toBeGreaterThanOrEqual(bytes.length ? 1 : 0);
    }
  });
});

describe("wire encoder", () => {
  it("rejects values the format cannot carry", () => {
    expect(() => encodeBatch([eos(2 ** 53)])).toThrow(RangeError);
    expect(() => encodeBatch([eos(0.5)])).toThrow(RangeError);
    const longCodec: Packet = {
      kind: "videoConfig",
      timestampUs: 0,
      discontinuity: false,
      config: { codec: "x".repeat(70000), codedWidth: 1, codedHeight: 1 },
      description: new Uint8Array(0),
    };
    expect(() => encodeBatch([longCodec])).toThrow(RangeError);
  });

  it("writes only the known flag bits", () => {
    const bytes = encodeBatch([
      { kind: "videoFrame", timestampUs: 0, discontinuity: true, keyframe: true, data: hexToBytes("00") },
      { kind: "audioPcm", timestampUs: 0, discontinuity: false, samples: new Int16Array(0) },
    ]);
    expect(bytes[5]).toBe(3);
    expect(bytes[17 + 5]).toBe(0);
  });
});

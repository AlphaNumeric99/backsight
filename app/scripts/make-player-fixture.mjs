// Builds the video fixtures the mock stream plays (app/public/fixtures/*.bspk). Run it once and
// commit the output:
//
//   node scripts/make-player-fixture.mjs
//
// Needs ffmpeg with libx264 and libx265 on PATH. Node 22.18+ runs the .ts import directly.
//
// Outputs:
//   testsrc2-720p15-h264.bspk  10 s of ffmpeg's testsrc2 at 1280x720, 15 fps, H.264 Main with a
//                              2 s GOP and no B-frames, plus a 440 Hz tone as 8 kHz mono s16le PCM.
//                              A white square flashes bottom-left, and the tone gets louder, for
//                              the first 100 ms of every second, to check A/V sync by eye and ear.
//   testsrc2-360p15-h265.bspk  2 s at 640x360, H.265 Main, video only. Most webviews can't decode
//                              it, which exercises the codec_unsupported path.
//
// Each file is a sequence of wire-format packets (docs/ARCHITECTURE.md) in stream order:
// VideoConfig, AudioConfig, then frames and PCM interleaved by timestamp. Timestamps start at 0;
// the mock stream re-stamps them. Frames are in the form the backend sends: length-prefixed NAL
// units with parameter sets and access unit delimiters stripped, described by an avcC / hvcC.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { decodeBatch, encodeBatch } from "../src/player/wire.ts";

const OUT_DIR = fileURLToPath(new URL("../public/fixtures/", import.meta.url));
const FPS = 15;
const AUDIO_RATE = 8000;
/** Samples per AudioPcm packet: 40 ms, about what the backend sends per G.711 payload. */
const AUDIO_CHUNK = 320;

function ffmpeg(args) {
  const result = spawnSync("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...args], { stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`ffmpeg exited with ${result.status}`);
}

function writeFixture(name, video, audio) {
  const packets = [
    { kind: "videoConfig", timestampUs: 0, discontinuity: false, config: video.config, description: video.description },
  ];
  if (audio) packets.push({ kind: "audioConfig", timestampUs: 0, discontinuity: false, config: audio.config });
  const media = [...video.frames, ...(audio ? audio.chunks : [])];
  // Stable sort: at equal timestamps, video first.
  media.sort((a, b) => a.timestampUs - b.timestampUs || (a.kind === "videoFrame" ? -1 : 1));
  packets.push(...media);

  const bytes = encodeBatch(packets);
  const check = decodeBatch(bytes);
  if (check.errors.length || check.packets.length !== packets.length) {
    throw new Error(`${name} does not decode cleanly: ${JSON.stringify(check.errors)}`);
  }
  writeFileSync(join(OUT_DIR, name), bytes);
  const keyframes = video.frames.filter((f) => f.keyframe).length;
  console.log(
    `${name}: ${(bytes.length / 1024).toFixed(0)} KiB, ${video.config.codec} ` +
      `${video.config.codedWidth}x${video.config.codedHeight}, ${video.frames.length} frames ` +
      `(${keyframes} keyframes)${audio ? `, ${audio.chunks.length} PCM packets` : ""}`,
  );
}

function frameTimestampUs(index) {
  return Math.round((index * 1_000_000) / FPS);
}

// --- Audio ------------------------------------------------------------------------------------

function pcmTrack(bytes) {
  // Copy: a Buffer's offset into its ArrayBuffer may be odd. The host is little-endian.
  const samples = new Int16Array(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
  const chunks = [];
  for (let at = 0; at < samples.length; at += AUDIO_CHUNK) {
    chunks.push({
      kind: "audioPcm",
      timestampUs: Math.round((at * 1_000_000) / AUDIO_RATE),
      discontinuity: false,
      samples: samples.slice(at, at + AUDIO_CHUNK),
    });
  }
  return { config: { sampleRate: AUDIO_RATE, channels: 1, format: "s16le" }, chunks };
}

// --- Annex B ----------------------------------------------------------------------------------

/** Splits an Annex B byte stream into NAL units (without start codes). */
function splitAnnexB(bytes) {
  const starts = [];
  for (let i = 0; i + 2 < bytes.length; i++) {
    if (bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 1) {
      starts.push(i + 3);
      i += 2;
    }
  }
  return starts.map((start, n) => {
    let end = n + 1 < starts.length ? starts[n + 1] - 3 : bytes.length;
    while (end > start && bytes[end - 1] === 0) end--; // trailing_zero_8bits / 4-byte start codes
    return bytes.subarray(start, end);
  });
}

/** Removes emulation prevention bytes (00 00 03 → 00 00). */
function unescapeRbsp(nal) {
  const out = new Uint8Array(nal.length);
  let n = 0;
  let zeros = 0;
  for (const byte of nal) {
    if (zeros >= 2 && byte === 3) {
      zeros = 0;
      continue;
    }
    out[n++] = byte;
    zeros = byte === 0 ? zeros + 1 : 0;
  }
  return out.subarray(0, n);
}

class BitReader {
  constructor(bytes) {
    this.bytes = bytes;
    this.pos = 0;
  }
  bit() {
    if (this.pos >= this.bytes.length * 8) throw new Error("read past the end of the RBSP");
    const bit = (this.bytes[this.pos >> 3] >> (7 - (this.pos & 7))) & 1;
    this.pos++;
    return bit;
  }
  bits(n) {
    let v = 0;
    for (let i = 0; i < n; i++) v = v * 2 + this.bit();
    return v;
  }
  skip(n) {
    this.pos += n;
  }
  ue() {
    let zeros = 0;
    while (this.bit() === 0) zeros++;
    return 2 ** zeros - 1 + this.bits(zeros);
  }
  se() {
    const k = this.ue();
    return k % 2 === 1 ? (k + 1) / 2 : -k / 2;
  }
}

/** Joins NAL units into one access unit with 4-byte big-endian length prefixes. */
function toLengthPrefixed(nals) {
  const out = new Uint8Array(nals.reduce((n, nal) => n + 4 + nal.length, 0));
  const view = new DataView(out.buffer);
  let at = 0;
  for (const nal of nals) {
    view.setUint32(at, nal.length);
    out.set(nal, at + 4);
    at += 4 + nal.length;
  }
  return out;
}

function hex2(n) {
  return n.toString(16).toUpperCase().padStart(2, "0");
}

function sameBytes(a, b) {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/**
 * Groups NAL units into access units and turns them into VideoFrame packets.
 * `kinds` classifies NAL units: { type(nal), isVcl(t), isKey(t), isParamSet(t), isAud(t),
 * startsAuBeforeVcl(t), firstSliceInPicture(nal) }.
 */
function accessUnits(nals, kinds) {
  const units = [];
  let current = null;
  for (const nal of nals) {
    const type = kinds.type(nal);
    const vcl = kinds.isVcl(type);
    const newUnit =
      !current ||
      (current.hasVcl && (kinds.startsAuBeforeVcl(type) || (vcl && kinds.firstSliceInPicture(nal))));
    if (newUnit) {
      current = { nals: [], hasVcl: false, keyframe: false };
      units.push(current);
    }
    if (vcl) {
      current.hasVcl = true;
      if (kinds.isKey(type)) current.keyframe = true;
    }
    // Parameter sets and AUDs travel in VideoConfig, not in frames.
    if (!kinds.isParamSet(type) && !kinds.isAud(type)) current.nals.push(nal);
  }
  return units
    .filter((u) => u.hasVcl)
    .map((u, i) => ({
      kind: "videoFrame",
      timestampUs: frameTimestampUs(i),
      discontinuity: false,
      keyframe: u.keyframe,
      data: toLengthPrefixed(u.nals),
    }));
}

/** Returns the single distinct NAL unit of `type`, checking that all repeats are identical. */
function uniqueNal(nals, typeOf, type, name) {
  const found = nals.filter((nal) => typeOf(nal) === type);
  if (found.length === 0) throw new Error(`no ${name} in the stream`);
  if (!found.every((nal) => sameBytes(nal, found[0]))) throw new Error(`${name} changes mid-stream`);
  return found[0];
}

// --- H.264 ------------------------------------------------------------------------------------

function h264Track(bytes) {
  const nals = splitAnnexB(bytes);
  const typeOf = (nal) => nal[0] & 0x1f;
  const sps = uniqueNal(nals, typeOf, 7, "SPS");
  const pps = uniqueNal(nals, typeOf, 8, "PPS");
  const info = parseH264Sps(sps);

  const avcc = [1, sps[1], sps[2], sps[3], 0xff, 0xe1, sps.length >> 8, sps.length & 0xff, ...sps];
  avcc.push(1, pps.length >> 8, pps.length & 0xff, ...pps);
  if ([100, 110, 122, 144].includes(sps[1])) {
    avcc.push(0xfc | info.chromaFormatIdc, 0xf8 | info.bitDepthLumaMinus8, 0xf8 | info.bitDepthChromaMinus8, 0);
  }

  const frames = accessUnits(nals, {
    type: typeOf,
    isVcl: (t) => t >= 1 && t <= 5,
    isKey: (t) => t === 5,
    isParamSet: (t) => t === 7 || t === 8,
    isAud: (t) => t === 9,
    // AUD, SEI, SPS, PPS and types 14..18 start a new access unit after a picture.
    startsAuBeforeVcl: (t) => t === 6 || t === 7 || t === 8 || t === 9 || (t >= 14 && t <= 18),
    // first_mb_in_slice == 0 is ue(v) 0, a single 1 bit.
    firstSliceInPicture: (nal) => (nal[1] & 0x80) !== 0,
  });

  return {
    config: { codec: `avc1.${hex2(sps[1])}${hex2(sps[2])}${hex2(sps[3])}`, codedWidth: info.width, codedHeight: info.height },
    description: Uint8Array.from(avcc),
    frames,
  };
}

function parseH264Sps(nal) {
  const r = new BitReader(unescapeRbsp(nal.subarray(1)));
  const profileIdc = r.bits(8);
  r.skip(8); // constraint flags
  r.skip(8); // level_idc
  r.ue(); // seq_parameter_set_id
  let chromaFormatIdc = 1;
  let bitDepthLumaMinus8 = 0;
  let bitDepthChromaMinus8 = 0;
  if ([100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].includes(profileIdc)) {
    chromaFormatIdc = r.ue();
    if (chromaFormatIdc === 3) r.skip(1); // separate_colour_plane_flag
    bitDepthLumaMinus8 = r.ue();
    bitDepthChromaMinus8 = r.ue();
    r.skip(1); // qpprime_y_zero_transform_bypass_flag
    if (r.bit()) {
      // seq_scaling_matrix_present_flag: skip the scaling lists.
      for (let i = 0; i < (chromaFormatIdc === 3 ? 12 : 8); i++) {
        if (!r.bit()) continue;
        const size = i < 6 ? 16 : 64;
        let last = 8;
        let next = 8;
        for (let j = 0; j < size; j++) {
          if (next !== 0) next = (last + r.se() + 256) % 256;
          last = next === 0 ? last : next;
        }
      }
    }
  }
  r.ue(); // log2_max_frame_num_minus4
  const pocType = r.ue();
  if (pocType === 0) {
    r.ue(); // log2_max_pic_order_cnt_lsb_minus4
  } else if (pocType === 1) {
    r.skip(1); // delta_pic_order_always_zero_flag
    r.se(); // offset_for_non_ref_pic
    r.se(); // offset_for_top_to_bottom_field
    const cycle = r.ue();
    for (let i = 0; i < cycle; i++) r.se();
  }
  r.ue(); // max_num_ref_frames
  r.skip(1); // gaps_in_frame_num_value_allowed_flag
  const widthMbs = r.ue() + 1;
  const heightMapUnits = r.ue() + 1;
  const frameMbsOnly = r.bit();
  if (!frameMbsOnly) r.skip(1); // mb_adaptive_frame_field_flag
  r.skip(1); // direct_8x8_inference_flag
  let crop = [0, 0, 0, 0];
  if (r.bit()) crop = [r.ue(), r.ue(), r.ue(), r.ue()];

  const subWidthC = chromaFormatIdc === 3 ? 1 : 2;
  const subHeightC = chromaFormatIdc === 1 ? 2 : 1;
  const cropUnitX = chromaFormatIdc === 0 ? 1 : subWidthC;
  const cropUnitY = (chromaFormatIdc === 0 ? 1 : subHeightC) * (2 - frameMbsOnly);
  return {
    width: widthMbs * 16 - cropUnitX * (crop[0] + crop[1]),
    height: (2 - frameMbsOnly) * heightMapUnits * 16 - cropUnitY * (crop[2] + crop[3]),
    chromaFormatIdc,
    bitDepthLumaMinus8,
    bitDepthChromaMinus8,
  };
}

// --- H.265 ------------------------------------------------------------------------------------

function h265Track(bytes) {
  const nals = splitAnnexB(bytes);
  const typeOf = (nal) => (nal[0] >> 1) & 0x3f;
  const vps = uniqueNal(nals, typeOf, 32, "VPS");
  const sps = uniqueNal(nals, typeOf, 33, "SPS");
  const pps = uniqueNal(nals, typeOf, 34, "PPS");
  const info = parseH265Sps(sps);
  const ptl = info.ptl;

  const hvcc = [
    1,
    (ptl.profileSpace << 6) | (ptl.tier << 5) | ptl.profileIdc,
    ...ptl.compatibilityBytes,
    ...ptl.constraintBytes,
    ptl.levelIdc,
    0xf0, 0x00, // min_spatial_segmentation_idc = 0
    0xfc, // parallelismType = 0
    0xfc | info.chromaFormatIdc,
    0xf8 | info.bitDepthLumaMinus8,
    0xf8 | info.bitDepthChromaMinus8,
    0x00, 0x00, // avgFrameRate
    // constantFrameRate = 0, numTemporalLayers, temporalIdNested, lengthSizeMinusOne = 3
    ((info.maxSubLayersMinus1 + 1) << 3) | (info.temporalIdNesting << 2) | 3,
    3, // numOfArrays
  ];
  for (const [type, nal] of [[32, vps], [33, sps], [34, pps]]) {
    hvcc.push(0x80 | type, 0, 1, nal.length >> 8, nal.length & 0xff, ...nal);
  }

  const frames = accessUnits(nals, {
    type: typeOf,
    isVcl: (t) => t < 32,
    isKey: (t) => t >= 16 && t <= 23,
    isParamSet: (t) => t === 32 || t === 33 || t === 34,
    isAud: (t) => t === 35,
    // VPS, SPS, PPS, AUD, prefix SEI and types 41..44, 48..55 start a new access unit.
    startsAuBeforeVcl: (t) => (t >= 32 && t <= 35) || t === 39 || (t >= 41 && t <= 44) || (t >= 48 && t <= 55),
    // first_slice_segment_in_pic_flag, the first bit after the 2-byte NAL header.
    firstSliceInPicture: (nal) => (nal[2] & 0x80) !== 0,
  });

  // RFC 6381 / ISO 14496-15 annex E: hvc1.<space><profile>.<compat, bit-reversed>.<tier><level>.<constraints>
  let compat = 0;
  for (let bit = 0; bit < 32; bit++) if ((ptl.compatibilityFlags >>> bit) & 1) compat |= 1 << (31 - bit);
  const constraints = [...ptl.constraintBytes];
  while (constraints.length > 1 && constraints[constraints.length - 1] === 0) constraints.pop();
  const codec = [
    "hvc1",
    `${["", "A", "B", "C"][ptl.profileSpace]}${ptl.profileIdc}`,
    (compat >>> 0).toString(16).toUpperCase(),
    `${ptl.tier ? "H" : "L"}${ptl.levelIdc}`,
    ...constraints.map((b) => b.toString(16).toUpperCase()),
  ].join(".");

  return {
    config: { codec, codedWidth: info.width, codedHeight: info.height },
    description: Uint8Array.from(hvcc),
    frames,
  };
}

function parseH265Sps(nal) {
  const rbsp = unescapeRbsp(nal.subarray(2));
  const r = new BitReader(rbsp);
  r.skip(4); // sps_video_parameter_set_id
  const maxSubLayersMinus1 = r.bits(3);
  const temporalIdNesting = r.bit();

  // profile_tier_level(1, sps_max_sub_layers_minus1)
  const ptlStart = r.pos / 8; // byte aligned here: 4 + 3 + 1 bits
  const profileSpace = r.bits(2);
  const tier = r.bit();
  const profileIdc = r.bits(5);
  const compatibilityFlags = r.bits(32) >>> 0;
  r.skip(48); // progressive/interlaced/non-packed/frame-only + 44 constraint bits
  const levelIdc = r.bits(8);
  const subLayerProfile = [];
  const subLayerLevel = [];
  for (let i = 0; i < maxSubLayersMinus1; i++) {
    subLayerProfile.push(r.bit());
    subLayerLevel.push(r.bit());
  }
  if (maxSubLayersMinus1 > 0) r.skip(2 * (8 - maxSubLayersMinus1));
  for (let i = 0; i < maxSubLayersMinus1; i++) {
    if (subLayerProfile[i]) r.skip(88);
    if (subLayerLevel[i]) r.skip(8);
  }

  r.ue(); // sps_seq_parameter_set_id
  const chromaFormatIdc = r.ue();
  if (chromaFormatIdc === 3) r.skip(1); // separate_colour_plane_flag
  let width = r.ue();
  let height = r.ue();
  if (r.bit()) {
    // conformance window, in chroma units
    const [left, right, top, bottom] = [r.ue(), r.ue(), r.ue(), r.ue()];
    const subWidthC = chromaFormatIdc === 1 || chromaFormatIdc === 2 ? 2 : 1;
    const subHeightC = chromaFormatIdc === 1 ? 2 : 1;
    width -= subWidthC * (left + right);
    height -= subHeightC * (top + bottom);
  }
  const bitDepthLumaMinus8 = r.ue();
  const bitDepthChromaMinus8 = r.ue();

  return {
    width,
    height,
    chromaFormatIdc,
    bitDepthLumaMinus8,
    bitDepthChromaMinus8,
    maxSubLayersMinus1,
    temporalIdNesting,
    ptl: {
      profileSpace,
      tier,
      profileIdc,
      compatibilityFlags,
      compatibilityBytes: [...rbsp.subarray(ptlStart + 1, ptlStart + 5)],
      constraintBytes: [...rbsp.subarray(ptlStart + 5, ptlStart + 11)],
      levelIdc,
    },
  };
}

function main() {
  const work = mkdtempSync(join(tmpdir(), "backsight-fixture-"));
  try {
    mkdirSync(OUT_DIR, { recursive: true });

    const h264 = join(work, "video.h264");
    const pcm = join(work, "audio.pcm");
    ffmpeg([
      "-f", "lavfi",
      "-i", `testsrc2=size=1280x720:rate=${FPS},drawbox=x=40:y=560:w=120:h=120:color=white:t=fill:enable='lt(mod(t,1),0.1)'`,
      "-t", "10",
      "-c:v", "libx264", "-profile:v", "main", "-level", "3.1", "-preset", "medium", "-pix_fmt", "yuv420p",
      "-bf", "0", "-g", "30", "-keyint_min", "30", "-sc_threshold", "0",
      "-b:v", "1000k", "-maxrate", "1300k", "-bufsize", "1300k",
      "-f", "h264", h264,
    ]);
    ffmpeg([
      "-f", "lavfi",
      "-i", `aevalsrc=exprs='0.1*sin(2*PI*440*t)*(1+4*lt(mod(t,1),0.1))':s=${AUDIO_RATE}:d=10`,
      "-ac", "1", "-f", "s16le", pcm,
    ]);
    writeFixture("testsrc2-720p15-h264.bspk", h264Track(readFileSync(h264)), pcmTrack(readFileSync(pcm)));

    const h265 = join(work, "video.h265");
    ffmpeg([
      "-f", "lavfi", "-i", `testsrc2=size=640x360:rate=${FPS}`, "-t", "2",
      "-c:v", "libx265", "-preset", "fast", "-pix_fmt", "yuv420p", "-b:v", "300k",
      "-x265-params", "keyint=30:min-keyint=30:bframes=0:scenecut=0:log-level=error",
      "-f", "hevc", h265,
    ]);
    writeFixture("testsrc2-360p15-h265.bspk", h265Track(readFileSync(h265)), null);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

main();

//! Wire format v1: the media packets the backend sends to the webview player (see
//! `docs/ARCHITECTURE.md`, "Wire format"). The player's decoder is `app/src/player/wire.ts`;
//! both sides are checked against the golden vectors in `app/src/player/__fixtures__/wire/`.
//!
//! Each function appends one packet to a batch. Send a batch (one or a few packets) as one
//! Tauri `Channel` message. Timestamps are microseconds since the Unix epoch (UTC); converting
//! the stream's 90 kHz ticks to wall-clock time is the caller's job.

use serde::Serialize;
use tapo_camera::media::{AudioConfig, VideoConfig, VideoFrame};

/// Size of the packet header in bytes.
#[cfg_attr(not(test), allow(dead_code))]
pub const HEADER_LEN: usize = 16;

/// Packet kinds (header byte 4).
pub mod kind {
    pub const VIDEO_CONFIG: u8 = 1;
    pub const VIDEO_FRAME: u8 = 2;
    pub const AUDIO_CONFIG: u8 = 3;
    pub const AUDIO_PCM: u8 = 4;
    pub const STATUS: u8 = 5;
    pub const END_OF_STREAM: u8 = 6;
}

/// Header flag: the video frame is a keyframe (IDR / IRAP).
pub const FLAG_KEYFRAME: u8 = 1 << 0;
/// Header flag: the player resets its decoders and clock before this packet. Set it on the
/// first packet after every seek.
pub const FLAG_DISCONTINUITY: u8 = 1 << 1;

/// Stream state carried by a [`Status`] packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(serde::Deserialize))]
#[serde(rename_all = "lowercase")]
pub enum StreamState {
    Buffering,
    Playing,
    /// Part of the format; the backend ends streams with an `EndOfStream` packet.
    #[allow(dead_code)]
    Ended,
    Error,
}

/// Payload of a `Status` packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Status<'a> {
    pub state: StreamState,
    /// Machine-readable reason, e.g. an `ApiErrorCode` such as `"stream_limit"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VideoConfigJson<'a> {
    codec: &'a str,
    coded_width: u32,
    coded_height: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AudioConfigJson {
    sample_rate: u32,
    channels: u8,
    format: &'static str,
}

/// Appends a `VideoConfig`: the codec string, the frame size and the avcC / hvcC record.
pub fn push_video_config(
    batch: &mut Vec<u8>,
    timestamp_us: i64,
    discontinuity: bool,
    config: &VideoConfig,
) {
    let start = begin(
        batch,
        kind::VIDEO_CONFIG,
        flags(false, discontinuity),
        timestamp_us,
    );
    let length_at = batch.len();
    batch.extend_from_slice(&[0, 0]);
    write_json(
        batch,
        &VideoConfigJson {
            codec: &config.codec_string,
            coded_width: config.width,
            coded_height: config.height,
        },
    );
    let json_len =
        u16::try_from(batch.len() - length_at - 2).expect("VideoConfig JSON exceeds 64 KiB");
    batch[length_at..length_at + 2].copy_from_slice(&json_len.to_le_bytes());
    batch.extend_from_slice(&config.description);
    finish(batch, start);
}

/// Appends a `VideoFrame`. The keyframe flag comes from the frame.
pub fn push_video_frame(
    batch: &mut Vec<u8>,
    timestamp_us: i64,
    discontinuity: bool,
    frame: &VideoFrame,
) {
    let start = begin(
        batch,
        kind::VIDEO_FRAME,
        flags(frame.keyframe, discontinuity),
        timestamp_us,
    );
    batch.extend_from_slice(&frame.data);
    finish(batch, start);
}

/// Appends an `AudioConfig` describing the PCM that follows. The backend decodes the camera's
/// audio (Tapo G.711 or Qubo AAC) to signed 16-bit PCM before sending it, so the format
/// is always `s16le`.
pub fn push_audio_config(
    batch: &mut Vec<u8>,
    timestamp_us: i64,
    discontinuity: bool,
    config: &AudioConfig,
) {
    let start = begin(
        batch,
        kind::AUDIO_CONFIG,
        flags(false, discontinuity),
        timestamp_us,
    );
    write_json(
        batch,
        &AudioConfigJson {
            sample_rate: config.sample_rate,
            channels: config.channels,
            format: "s16le",
        },
    );
    finish(batch, start);
}

/// Appends an `AudioPcm` packet of interleaved samples. `timestamp_us` is the time of the
/// first sample.
pub fn push_audio_pcm(
    batch: &mut Vec<u8>,
    timestamp_us: i64,
    discontinuity: bool,
    samples: &[i16],
) {
    let start = begin(
        batch,
        kind::AUDIO_PCM,
        flags(false, discontinuity),
        timestamp_us,
    );
    batch.reserve(samples.len() * 2);
    for sample in samples {
        batch.extend_from_slice(&sample.to_le_bytes());
    }
    finish(batch, start);
}

/// Appends a `Status` packet.
pub fn push_status(batch: &mut Vec<u8>, timestamp_us: i64, status: &Status<'_>) {
    let start = begin(batch, kind::STATUS, 0, timestamp_us);
    write_json(batch, status);
    finish(batch, start);
}

/// Appends an `EndOfStream` packet: nothing follows on this stream.
pub fn push_end_of_stream(batch: &mut Vec<u8>, timestamp_us: i64) {
    let start = begin(batch, kind::END_OF_STREAM, 0, timestamp_us);
    finish(batch, start);
}

fn flags(keyframe: bool, discontinuity: bool) -> u8 {
    (if keyframe { FLAG_KEYFRAME } else { 0 })
        | (if discontinuity { FLAG_DISCONTINUITY } else { 0 })
}

/// Writes a header with a placeholder length and returns the packet's start offset.
fn begin(batch: &mut Vec<u8>, kind: u8, flags: u8, timestamp_us: i64) -> usize {
    let start = batch.len();
    batch.extend_from_slice(&[0; 4]);
    batch.extend_from_slice(&[kind, flags, 0, 0]);
    batch.extend_from_slice(&timestamp_us.to_le_bytes());
    start
}

/// Patches the total length of the packet that starts at `start`.
fn finish(batch: &mut [u8], start: usize) {
    let length = u32::try_from(batch.len() - start).expect("packet exceeds 4 GiB");
    batch[start..start + 4].copy_from_slice(&length.to_le_bytes());
}

fn write_json(batch: &mut Vec<u8>, value: &impl Serialize) {
    serde_json::to_writer(batch, value).expect("serializing to a Vec cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use serde::Deserialize;
    use tapo_camera::media::{AudioCodec, VideoCodec};

    macro_rules! golden {
        ($($name:literal),* $(,)?) => {
            &[$(($name, include_bytes!(concat!("../../src/player/__fixtures__/wire/", $name, ".bspk")))),*]
        };
    }

    /// The golden batches, shared with `app/src/player/wire.test.ts`.
    const GOLDEN: &[(&str, &[u8])] = golden![
        "video_config_h264",
        "video_config_h265_discontinuity",
        "video_frame_key",
        "video_frame_delta",
        "audio_config",
        "audio_pcm",
        "status_error",
        "status_buffering",
        "end_of_stream",
        "batch_mixed",
    ];

    const VECTORS: &str = include_str!("../../src/player/__fixtures__/wire/vectors.json");

    #[derive(Deserialize)]
    struct Vectors {
        vectors: Vec<Vector>,
    }

    #[derive(Deserialize)]
    struct Vector {
        name: String,
        packets: Vec<PacketSpec>,
    }

    /// A packet as described in `vectors.json`.
    #[derive(Deserialize)]
    #[serde(
        tag = "kind",
        rename_all = "camelCase",
        rename_all_fields = "camelCase"
    )]
    enum PacketSpec {
        VideoConfig {
            timestamp_us: i64,
            discontinuity: bool,
            codec: String,
            coded_width: u32,
            coded_height: u32,
            description_hex: String,
        },
        VideoFrame {
            timestamp_us: i64,
            discontinuity: bool,
            keyframe: bool,
            data_hex: String,
        },
        AudioConfig {
            timestamp_us: i64,
            discontinuity: bool,
            sample_rate: u32,
            channels: u8,
            format: String,
        },
        AudioPcm {
            timestamp_us: i64,
            discontinuity: bool,
            samples: Vec<i16>,
        },
        Status {
            timestamp_us: i64,
            discontinuity: bool,
            state: StreamState,
            code: Option<String>,
            message: Option<String>,
        },
        EndOfStream {
            timestamp_us: i64,
            discontinuity: bool,
        },
    }

    fn hex(s: &str) -> Bytes {
        assert!(s.len().is_multiple_of(2), "odd hex length");
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("valid hex"))
            .collect::<Vec<u8>>()
            .into()
    }

    fn encode(packets: &[PacketSpec]) -> Vec<u8> {
        let mut batch = Vec::new();
        for packet in packets {
            match packet {
                PacketSpec::VideoConfig {
                    timestamp_us,
                    discontinuity,
                    codec,
                    coded_width,
                    coded_height,
                    description_hex,
                } => {
                    let config = VideoConfig {
                        codec: if codec.starts_with("avc1") {
                            VideoCodec::H264
                        } else {
                            VideoCodec::H265
                        },
                        codec_string: codec.clone(),
                        width: *coded_width,
                        height: *coded_height,
                        description: hex(description_hex),
                    };
                    push_video_config(&mut batch, *timestamp_us, *discontinuity, &config);
                }
                PacketSpec::VideoFrame {
                    timestamp_us,
                    discontinuity,
                    keyframe,
                    data_hex,
                } => {
                    let frame = VideoFrame {
                        pts: 0,
                        dts: 0,
                        keyframe: *keyframe,
                        data: hex(data_hex),
                    };
                    push_video_frame(&mut batch, *timestamp_us, *discontinuity, &frame);
                }
                PacketSpec::AudioConfig {
                    timestamp_us,
                    discontinuity,
                    sample_rate,
                    channels,
                    format,
                } => {
                    assert_eq!(format, "s16le", "the backend only sends s16le PCM");
                    let config = AudioConfig {
                        codec: AudioCodec::PcmAlaw,
                        sample_rate: *sample_rate,
                        channels: *channels,
                    };
                    push_audio_config(&mut batch, *timestamp_us, *discontinuity, &config);
                }
                PacketSpec::AudioPcm {
                    timestamp_us,
                    discontinuity,
                    samples,
                } => push_audio_pcm(&mut batch, *timestamp_us, *discontinuity, samples),
                PacketSpec::Status {
                    timestamp_us,
                    discontinuity,
                    state,
                    code,
                    message,
                } => {
                    assert!(
                        !discontinuity,
                        "status packets never carry the discontinuity flag"
                    );
                    let status = Status {
                        state: *state,
                        code: code.as_deref(),
                        message: message.as_deref(),
                    };
                    push_status(&mut batch, *timestamp_us, &status);
                }
                PacketSpec::EndOfStream {
                    timestamp_us,
                    discontinuity,
                } => {
                    assert!(
                        !discontinuity,
                        "end-of-stream packets never carry the discontinuity flag"
                    );
                    push_end_of_stream(&mut batch, *timestamp_us);
                }
            }
        }
        batch
    }

    fn vectors() -> Vec<Vector> {
        serde_json::from_str::<Vectors>(VECTORS)
            .expect("vectors.json parses")
            .vectors
    }

    #[test]
    fn encodes_the_golden_vectors() {
        for vector in vectors() {
            let (_, golden) = GOLDEN
                .iter()
                .find(|(name, _)| *name == vector.name)
                .unwrap_or_else(|| panic!("{}.bspk is not listed in GOLDEN", vector.name));
            assert_eq!(encode(&vector.packets), *golden, "vector {}", vector.name);
        }
    }

    #[test]
    fn every_golden_file_is_described() {
        let names: Vec<String> = vectors().into_iter().map(|v| v.name).collect();
        for (name, _) in GOLDEN {
            assert!(
                names.iter().any(|n| n == name),
                "{name}.bspk has no entry in vectors.json"
            );
        }
        assert_eq!(names.len(), GOLDEN.len());
    }

    #[test]
    fn writes_the_header_layout() {
        let mut batch = vec![0xAA];
        let frame = VideoFrame {
            pts: 0,
            dts: 0,
            keyframe: true,
            data: Bytes::from_static(&[0, 0, 0, 1, 0x65]),
        };
        push_video_frame(&mut batch, -2, true, &frame);
        assert_eq!(batch[0], 0xAA, "existing batch content is kept");
        let packet = &batch[1..];
        assert_eq!(packet.len(), HEADER_LEN + 5);
        assert_eq!(&packet[0..4], &21u32.to_le_bytes());
        assert_eq!(packet[4], kind::VIDEO_FRAME);
        assert_eq!(packet[5], FLAG_KEYFRAME | FLAG_DISCONTINUITY);
        assert_eq!(&packet[6..8], &[0, 0]);
        assert_eq!(&packet[8..16], &(-2i64).to_le_bytes());
        assert_eq!(&packet[16..], &[0, 0, 0, 1, 0x65]);
    }
}

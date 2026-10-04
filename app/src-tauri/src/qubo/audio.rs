//! Qubo AAC audio: RFC 3640 RTP access units → signed 16-bit PCM.
//!
//! The relay advertises MPEG4-GENERIC / AAC-hbr, with AudioSpecificConfig in SDP.
//! Access-unit headers describe the raw AAC frames (not ADTS). Symphonia decodes
//! those frames in-process; the player still receives its usual PCM wire packets.

use bytes::Bytes;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_AAC, CodecParameters, Decoder, DecoderOptions};
use symphonia::core::formats::Packet;

use super::rtsp::RtpPacket;
use crate::error::{ApiError, ApiResult};

/// The audio track announced by the relay's SDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTrack {
    pub control: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub config: Vec<u8>,
    pub size_length: u8,
    pub index_length: u8,
    pub index_delta_length: u8,
}

/// A complete raw AAC access unit, timestamped in the audio RTP clock.
#[derive(Debug, PartialEq, Eq)]
pub struct AccessUnit {
    pub timestamp: u32,
    pub data: Bytes,
}

struct Fragment {
    timestamp: u32,
    size: usize,
    data: Vec<u8>,
}

/// Depacketizes the AAC-hbr framing used by Wowza (13-bit size, 3-bit index).
pub struct Depacketizer {
    fragment: Option<Fragment>,
    sequence: Option<u16>,
}

impl Depacketizer {
    pub fn new(track: &AudioTrack) -> ApiResult<Self> {
        if (
            track.size_length,
            track.index_length,
            track.index_delta_length,
        ) != (13, 3, 3)
        {
            return Err(ApiError::new(
                "unsupported",
                "The Qubo relay uses unsupported AAC RTP headers.",
            ));
        }
        Ok(Self {
            fragment: None,
            sequence: None,
        })
    }

    pub fn reset(&mut self) {
        self.fragment = None;
        self.sequence = None;
    }

    /// Appends complete access units. A lost fragment is discarded before starting
    /// the next unit; malformed sizes never allocate beyond the 13-bit size limit.
    pub fn push(&mut self, packet: &RtpPacket, out: &mut Vec<AccessUnit>) -> ApiResult<()> {
        if self
            .sequence
            .is_some_and(|seq| packet.sequence != seq.wrapping_add(1))
        {
            self.fragment = None;
        }
        self.sequence = Some(packet.sequence);
        let data = &packet.payload;
        if data.len() < 4 {
            return Err(ApiError::internal("Truncated AAC RTP headers."));
        }
        let header_bits = u16::from_be_bytes([data[0], data[1]]) as usize;
        if header_bits == 0 || !header_bits.is_multiple_of(16) {
            return Err(ApiError::internal("Invalid AAC access-unit header length."));
        }
        let count = header_bits / 16;
        let header_end = 2 + count * 2;
        if header_end > data.len() {
            return Err(ApiError::internal("Truncated AAC access-unit headers."));
        }
        let mut at = header_end;
        for index in 0..count {
            let header_at = 2 + index * 2;
            let header = u16::from_be_bytes([data[header_at], data[header_at + 1]]);
            let size = (header >> 3) as usize;
            let au_index = header & 7;
            if size == 0 || au_index != 0 {
                return Err(ApiError::new(
                    "unsupported",
                    "Interleaved AAC access units are unsupported.",
                ));
            }
            let timestamp = packet.timestamp.wrapping_add(index as u32 * 1024);
            if let Some(fragment) = self.fragment.as_mut() {
                if count != 1 || fragment.timestamp != timestamp || fragment.size != size {
                    self.fragment = None;
                } else {
                    let remaining = size - fragment.data.len();
                    if data.len() - at > remaining {
                        return Err(ApiError::internal(
                            "AAC fragment exceeds its declared size.",
                        ));
                    }
                    fragment.data.extend_from_slice(&data[at..]);
                    if fragment.data.len() == size {
                        let complete = self.fragment.take().unwrap();
                        out.push(AccessUnit {
                            timestamp,
                            data: Bytes::from(complete.data),
                        });
                    } else if packet.marker {
                        self.fragment = None;
                        return Err(ApiError::internal("Incomplete AAC access unit."));
                    }
                    return Ok(());
                }
            }
            if data.len() - at < size {
                if count != 1 || packet.marker {
                    return Err(ApiError::internal("Truncated AAC access unit."));
                }
                self.fragment = Some(Fragment {
                    timestamp,
                    size,
                    data: data[at..].to_vec(),
                });
                return Ok(());
            }
            out.push(AccessUnit {
                timestamp,
                data: data.slice(at..at + size),
            });
            at += size;
        }
        if at != data.len() {
            return Err(ApiError::internal(
                "Unexpected bytes after AAC access units.",
            ));
        }
        Ok(())
    }
}

/// AAC-LC decoder with a reusable interleaved PCM buffer.
pub struct AacDecoder {
    decoder: Box<dyn Decoder>,
    sample_rate: u32,
    channels: u8,
    samples: Option<SampleBuffer<i16>>,
}

impl AacDecoder {
    pub fn new(track: &AudioTrack) -> ApiResult<Self> {
        if track.config.len() < 2 || track.config[0] >> 3 != 2 || track.config[1] & 4 != 0 {
            return Err(ApiError::new(
                "unsupported",
                "The Qubo relay must provide 1024-sample AAC-LC audio.",
            ));
        }
        if !matches!(track.channels, 1 | 2) || !(8_000..=96_000).contains(&track.sample_rate) {
            return Err(ApiError::new(
                "unsupported",
                "Unsupported Qubo audio channel count or sample rate.",
            ));
        }
        let mut parameters = CodecParameters::new();
        parameters
            .for_codec(CODEC_TYPE_AAC)
            .with_sample_rate(track.sample_rate)
            .with_extra_data(track.config.clone().into_boxed_slice());
        let decoder = symphonia::default::get_codecs()
            .make(&parameters, &DecoderOptions::default())
            .map_err(|e| {
                ApiError::new(
                    "unsupported",
                    format!("Could not configure Qubo audio: {e}"),
                )
            })?;
        Ok(Self {
            decoder,
            sample_rate: track.sample_rate,
            channels: track.channels,
            samples: None,
        })
    }

    /// Decodes one raw access unit into the caller's PCM buffer. The decoder's
    /// actual output format must match the SDP rate/channel count we send to the UI.
    pub fn decode(&mut self, unit: &AccessUnit, out: &mut Vec<i16>) -> ApiResult<()> {
        let packet = Packet::new_from_slice(0, u64::from(unit.timestamp), 1024, &unit.data);
        let decoded = self
            .decoder
            .decode(&packet)
            .map_err(|e| ApiError::internal(format!("Could not decode Qubo AAC audio: {e}")))?;
        if decoded.spec().rate != self.sample_rate
            || decoded.spec().channels.count() != usize::from(self.channels)
        {
            return Err(ApiError::new(
                "unsupported",
                "Qubo decoded audio differs from its announced format.",
            ));
        }
        let samples = self
            .samples
            .get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, *decoded.spec()));
        samples.copy_interleaved_ref(decoded);
        out.clear();
        out.extend_from_slice(samples.samples());
        Ok(())
    }

    /// Discards AAC overlap state after a corrupt access unit; the next packet
    /// starts fresh without ending an otherwise healthy video stream.
    pub fn reset(&mut self) {
        self.decoder.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> AudioTrack {
        AudioTrack {
            control: "trackID=2".into(),
            sample_rate: 16_000,
            channels: 1,
            config: vec![0x14, 0x08],
            size_length: 13,
            index_length: 3,
            index_delta_length: 3,
        }
    }

    fn packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> RtpPacket {
        RtpPacket {
            timestamp,
            sequence,
            marker,
            payload: Bytes::copy_from_slice(payload),
        }
    }

    #[test]
    fn multiple_units_get_individual_audio_timestamps() {
        let mut depacketizer = Depacketizer::new(&track()).unwrap();
        let mut units = Vec::new();
        depacketizer
            .push(
                &packet(1, 1000, true, &[0, 32, 0, 24, 0, 16, 1, 2, 3, 4, 5]),
                &mut units,
            )
            .unwrap();
        assert_eq!(
            units,
            vec![
                AccessUnit {
                    timestamp: 1000,
                    data: Bytes::from_static(&[1, 2, 3])
                },
                AccessUnit {
                    timestamp: 2024,
                    data: Bytes::from_static(&[4, 5])
                }
            ]
        );
    }

    #[test]
    fn fragments_reassemble_without_rtp_headers() {
        let mut depacketizer = Depacketizer::new(&track()).unwrap();
        let mut units = Vec::new();
        depacketizer
            .push(&packet(1, 1000, false, &[0, 16, 0, 32, 1, 2]), &mut units)
            .unwrap();
        assert!(units.is_empty());
        depacketizer
            .push(&packet(2, 1000, true, &[0, 16, 0, 32, 3, 4]), &mut units)
            .unwrap();
        assert_eq!(&units[0].data[..], &[1, 2, 3, 4]);
    }

    #[test]
    fn malformed_sizes_are_rejected() {
        let mut depacketizer = Depacketizer::new(&track()).unwrap();
        for payload in [&[0, 16, 0][..], &[0, 15, 0, 8], &[0, 16, 0, 32, 1]] {
            assert!(
                depacketizer
                    .push(&packet(1, 1000, true, payload), &mut Vec::new())
                    .is_err()
            );
        }
    }

    #[test]
    fn decodes_known_aac_tone_to_pcm() {
        // Like the existing media tests, use FFmpeg when installed as an
        // independent encoder; no encoder is bundled with the application.
        let Ok(encoded) = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=1000:sample_rate=16000:duration=0.5",
                "-ac",
                "1",
                "-c:a",
                "aac",
                "-f",
                "adts",
                "pipe:1",
            ])
            .output()
        else {
            return;
        };
        assert!(encoded.status.success());
        let mut decoder = AacDecoder::new(&track()).unwrap();
        let mut pcm = Vec::new();
        let mut total = Vec::new();
        for frame in tapo_camera::media::aac::adts_frames(&encoded.stdout) {
            decoder
                .decode(
                    &AccessUnit {
                        timestamp: 0,
                        data: Bytes::copy_from_slice(frame.payload()),
                    },
                    &mut pcm,
                )
                .unwrap();
            total.extend_from_slice(&pcm);
        }
        assert!(total.len() >= 8000);
        assert!(total.iter().any(|s| s.unsigned_abs() > 1000));
    }
}

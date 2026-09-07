//! Media handling: demuxing the camera's MPEG-TS stream into access units, building
//! codec configuration for decoders (WebCodecs, MP4), G.711 decoding and MP4 export.
//!
//! These types are the contract between the stream layer, the desktop app's player
//! pipeline and the exporter, so keep them small and decoder-friendly.

use std::fmt;

use bytes::Bytes;

mod bits;
pub mod h264;
pub mod h265;

/// Timestamp in 90 kHz MPEG system clock ticks.
pub type Ticks90k = i64;

/// Video codecs Tapo cameras are known to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    H264,
    H265,
}

/// Everything a decoder needs before the first frame. Emitted again whenever the
/// parameter sets change (for example when the camera switches resolution).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoConfig {
    pub codec: VideoCodec,
    /// RFC 6381 codec string such as `avc1.64001F` or `hvc1.1.6.L120.B0`,
    /// usable directly as a WebCodecs `codec`.
    pub codec_string: String,
    pub width: u32,
    pub height: u32,
    /// `AVCDecoderConfigurationRecord` (avcC) for H.264 or
    /// `HEVCDecoderConfigurationRecord` (hvcC) for H.265.
    pub description: Bytes,
}

/// One video access unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    pub pts: Ticks90k,
    pub dts: Ticks90k,
    /// IDR (H.264) or IRAP (H.265) access unit.
    pub keyframe: bool,
    /// NAL units in length-prefixed form (4-byte big-endian sizes), matching the
    /// `description`. Parameter sets and access unit delimiters are stripped; they are
    /// carried by [`VideoConfig`] instead.
    pub data: Bytes,
}

/// Audio codecs Tapo cameras are known to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// G.711 A-law (the common case).
    PcmAlaw,
    /// G.711 µ-law.
    PcmMulaw,
    /// AAC with ADTS framing.
    Aac,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub codec: AudioCodec,
    pub sample_rate: u32,
    pub channels: u8,
}

/// One audio payload as carried in the transport stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFrame {
    pub pts: Ticks90k,
    pub data: Bytes,
}

/// Output of the demuxer, in stream order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaEvent {
    VideoConfig(VideoConfig),
    Video(VideoFrame),
    AudioConfig(AudioConfig),
    Audio(AudioFrame),
}

/// Errors from the media layer: malformed bitstreams, unsupported features, I/O while
/// writing MP4 files.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MediaError {
    /// A bitstream structure (NAL unit, parameter set, ADTS header, …) is truncated or
    /// violates its specification.
    #[error("invalid {what}: {reason}")]
    InvalidData {
        /// The structure being parsed, e.g. `"H.264 SPS"`.
        what: &'static str,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// The data is valid but uses a feature this crate does not handle.
    #[error("unsupported {0}")]
    Unsupported(String),
    /// The demuxer skipped this many bytes without finding MPEG-TS packets, so the input
    /// is probably not MPEG-TS (for example still encrypted). The demuxer stays usable.
    #[error("no MPEG-TS packets found in the last {skipped} bytes")]
    LostSync {
        /// Bytes skipped since sync was last held.
        skipped: u64,
    },
    /// The video configuration changed in the middle of an MP4 file. Finish the file
    /// and start a new one with the new configuration.
    #[error("video configuration changed from {from} to {to}")]
    VideoConfigChanged {
        /// Codec string and size of the configuration the file was started with.
        from: String,
        /// Codec string and size of the new configuration.
        to: String,
    },
    /// An API was used incorrectly, for example audio written without an audio track.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Building MP4 boxes failed.
    #[error("MP4 encoding failed: {0}")]
    Mp4(String),
    /// Reading or writing a file failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl MediaError {
    pub(crate) const fn invalid(what: &'static str, reason: &'static str) -> Self {
        Self::InvalidData { what, reason }
    }
}

impl VideoCodec {
    /// Human-readable codec name, e.g. `"H.264"`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::H265 => "H.265",
        }
    }
}

impl fmt::Display for VideoCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl AudioCodec {
    /// Human-readable codec name, e.g. `"G.711 A-law"`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::PcmAlaw => "G.711 A-law",
            Self::PcmMulaw => "G.711 µ-law",
            Self::Aac => "AAC",
        }
    }
}

impl fmt::Display for AudioCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl VideoFrame {
    /// Iterates over the NAL units in [`data`](Self::data), without their length
    /// prefixes. Stops early if a length prefix is inconsistent with the data.
    pub fn nal_units(&self) -> impl Iterator<Item = &[u8]> + '_ {
        let mut rest: &[u8] = &self.data;
        std::iter::from_fn(move || {
            let (len, tail) = rest.split_first_chunk::<4>()?;
            let len = u32::from_be_bytes(*len) as usize;
            if len > tail.len() {
                rest = &[];
                return None;
            }
            let (nal, next) = tail.split_at(len);
            rest = next;
            Some(nal)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_frame_nal_units_walks_length_prefixes() {
        let frame = VideoFrame {
            pts: 0,
            dts: 0,
            keyframe: true,
            data: Bytes::from_static(&[0, 0, 0, 2, 0x65, 0x88, 0, 0, 0, 1, 0x06]),
        };
        let nals: Vec<&[u8]> = frame.nal_units().collect();
        assert_eq!(nals, vec![&[0x65, 0x88][..], &[0x06][..]]);
    }

    #[test]
    fn video_frame_nal_units_stops_on_bad_length() {
        let frame = VideoFrame {
            pts: 0,
            dts: 0,
            keyframe: false,
            data: Bytes::from_static(&[0, 0, 0, 1, 0x41, 0, 0, 0, 9, 0x01]),
        };
        assert_eq!(frame.nal_units().count(), 1);
    }

    #[test]
    fn codec_names() {
        assert_eq!(VideoCodec::H265.to_string(), "H.265");
        assert_eq!(AudioCodec::PcmAlaw.to_string(), "G.711 A-law");
    }
}

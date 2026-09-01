//! Media handling: demuxing the camera's MPEG-TS stream into access units, building
//! codec configuration for decoders (WebCodecs, MP4), G.711 decoding and MP4 export.
//!
//! These types are the contract between the stream layer, the desktop app's player
//! pipeline and the exporter, so keep them small and decoder-friendly.

use bytes::Bytes;

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

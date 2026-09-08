//! AAC helpers: ADTS frame headers and the `AudioSpecificConfig` that MP4 files and
//! WebCodecs need for raw (header-less) AAC frames.
//!
//! References: ISO/IEC 13818-7 / 14496-3 1.A.2 (ADTS), ISO/IEC 14496-3 1.6.2.1
//! (`AudioSpecificConfig`).

use super::MediaError;
use super::bits::BitReader;

/// Sampling frequencies indexed by `sampling_frequency_index`.
pub const SAMPLE_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// Samples per channel in one AAC-LC frame (raw data block).
pub const SAMPLES_PER_FRAME: u32 = 1024;

/// Returns the `sampling_frequency_index` of a sample rate, if it has one.
pub fn sampling_frequency_index(sample_rate: u32) -> Option<u8> {
    SAMPLE_RATES
        .iter()
        .position(|&rate| rate == sample_rate)
        .map(|i| i as u8)
}

/// Number of output channels for an MPEG-4 `channelConfiguration`; 0 means "defined in a
/// program config element".
pub fn channel_count(channel_configuration: u8) -> u8 {
    match channel_configuration {
        7 => 8,
        n => n,
    }
}

/// A parsed ADTS frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdtsHeader {
    /// MPEG-4 audio object type: 1 = Main, 2 = LC, 3 = SSR, 4 = LTP.
    pub object_type: u8,
    /// Index into [`SAMPLE_RATES`].
    pub sampling_frequency_index: u8,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// MPEG-4 `channelConfiguration`; see [`channel_count`].
    pub channel_configuration: u8,
    /// Whether the header lacks the 16-bit CRC (7-byte header) or has it (9 bytes).
    pub protection_absent: bool,
    /// Length of the whole frame, header included.
    pub frame_length: usize,
    /// `number_of_raw_data_blocks_in_frame + 1`.
    pub raw_data_blocks: u8,
}

impl AdtsHeader {
    /// Length of the fixed plus variable header without CRC.
    pub const MIN_LEN: usize = 7;

    /// Parses the ADTS header at the start of `data` (at least 7 bytes).
    pub fn parse(data: &[u8]) -> Result<Self, MediaError> {
        const WHAT: &str = "ADTS header";
        if data.len() < Self::MIN_LEN {
            return Err(MediaError::invalid(WHAT, "unexpected end of data"));
        }
        if data[0] != 0xFF || data[1] & 0xF6 != 0xF0 {
            return Err(MediaError::invalid(WHAT, "missing sync word"));
        }
        let protection_absent = data[1] & 0x01 != 0;
        let object_type = (data[2] >> 6) + 1;
        let sampling_frequency_index = (data[2] >> 2) & 0x0F;
        let sample_rate = *SAMPLE_RATES
            .get(usize::from(sampling_frequency_index))
            .ok_or(MediaError::invalid(
                WHAT,
                "reserved sampling frequency index",
            ))?;
        let channel_configuration = ((data[2] & 0x01) << 2) | (data[3] >> 6);
        let frame_length = (usize::from(data[3] & 0x03) << 11)
            | (usize::from(data[4]) << 3)
            | usize::from(data[5] >> 5);
        let raw_data_blocks = (data[6] & 0x03) + 1;
        let header = Self {
            object_type,
            sampling_frequency_index,
            sample_rate,
            channel_configuration,
            protection_absent,
            frame_length,
            raw_data_blocks,
        };
        if frame_length < header.header_len() {
            return Err(MediaError::invalid(WHAT, "frame shorter than its header"));
        }
        Ok(header)
    }

    /// Header length: 7 bytes, or 9 with CRC.
    pub fn header_len(&self) -> usize {
        if self.protection_absent { 7 } else { 9 }
    }

    /// Samples per channel carried by the frame.
    pub fn samples(&self) -> u32 {
        SAMPLES_PER_FRAME * u32::from(self.raw_data_blocks)
    }

    /// The `AudioSpecificConfig` describing this stream.
    pub fn audio_specific_config(&self) -> AudioSpecificConfig {
        AudioSpecificConfig {
            object_type: self.object_type,
            sample_rate: self.sample_rate,
            channel_configuration: self.channel_configuration,
        }
    }
}

/// One ADTS frame found by [`adts_frames`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdtsFrame<'a> {
    /// The parsed header.
    pub header: AdtsHeader,
    /// The whole frame, header included.
    pub data: &'a [u8],
}

impl<'a> AdtsFrame<'a> {
    /// The raw AAC payload after the header (and CRC), as stored in MP4 samples.
    pub fn payload(&self) -> &'a [u8] {
        &self.data[self.header.header_len()..]
    }
}

/// Iterates over the complete ADTS frames in `data`, skipping bytes that do not start a
/// valid header. A truncated final frame is not returned; [`AdtsFrames::offset`] tells
/// where the unconsumed tail starts.
pub fn adts_frames(data: &[u8]) -> AdtsFrames<'_> {
    AdtsFrames { data, pos: 0 }
}

/// Iterator returned by [`adts_frames`].
#[derive(Debug, Clone)]
pub struct AdtsFrames<'a> {
    data: &'a [u8],
    pos: usize,
}

impl AdtsFrames<'_> {
    /// Offset of the first byte not consumed yet: after iteration ends, the start of a
    /// truncated trailing frame (or the end of the data).
    pub fn offset(&self) -> usize {
        self.pos
    }
}

impl<'a> Iterator for AdtsFrames<'a> {
    type Item = AdtsFrame<'a>;

    fn next(&mut self) -> Option<AdtsFrame<'a>> {
        while self.data.len() - self.pos >= AdtsHeader::MIN_LEN {
            let rest = &self.data[self.pos..];
            match AdtsHeader::parse(rest) {
                Ok(header) if header.frame_length <= rest.len() => {
                    self.pos += header.frame_length;
                    return Some(AdtsFrame {
                        header,
                        data: &rest[..header.frame_length],
                    });
                }
                // A frame that continues past the end of the data.
                Ok(_) => return None,
                Err(_) => self.pos += 1,
            }
        }
        None
    }
}

/// The MPEG-4 `AudioSpecificConfig` for AAC (the `esds` decoder specific info and the
/// WebCodecs `description` for raw AAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AudioSpecificConfig {
    /// MPEG-4 audio object type, e.g. 2 for AAC-LC.
    pub object_type: u8,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// MPEG-4 `channelConfiguration` (1 = mono, 2 = stereo, …).
    pub channel_configuration: u8,
}

impl AudioSpecificConfig {
    /// Describes an AAC stream. `object_type` 2 is AAC-LC.
    pub fn new(object_type: u8, sample_rate: u32, channel_configuration: u8) -> Self {
        Self {
            object_type,
            sample_rate,
            channel_configuration,
        }
    }

    /// Parses the leading fields of an `AudioSpecificConfig`: object type (with the
    /// escape for types above 30), sampling frequency (index or explicit 24-bit value)
    /// and channel configuration.
    pub fn parse(data: &[u8]) -> Result<Self, MediaError> {
        const WHAT: &str = "AudioSpecificConfig";
        let mut r = BitReader::new(data, WHAT);
        let mut object_type = r.read_u8(5)?;
        if object_type == 31 {
            object_type = 32 + r.read_u8(6)?;
        }
        let index = r.read_u8(4)?;
        let sample_rate = if index == 0x0F {
            r.read_bits(24)?
        } else {
            *SAMPLE_RATES
                .get(usize::from(index))
                .ok_or(MediaError::invalid(
                    WHAT,
                    "reserved sampling frequency index",
                ))?
        };
        if sample_rate == 0 {
            return Err(MediaError::invalid(WHAT, "zero sample rate"));
        }
        let channel_configuration = r.read_u8(4)?;
        Ok(Self {
            object_type,
            sample_rate,
            channel_configuration,
        })
    }

    /// Serializes the config with an empty `GASpecificConfig` (960-sample frames off,
    /// no core coder, no extension), as used for AAC-LC.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bits: u64 = 0;
        let mut len = 0u32;
        let mut put = |value: u64, width: u32| {
            bits = (bits << width) | (value & ((1 << width) - 1));
            len += width;
        };
        if self.object_type >= 31 {
            put(31, 5);
            put(u64::from(self.object_type - 32), 6);
        } else {
            put(u64::from(self.object_type), 5);
        }
        match sampling_frequency_index(self.sample_rate) {
            Some(index) => put(u64::from(index), 4),
            None => {
                put(0x0F, 4);
                put(u64::from(self.sample_rate), 24);
            }
        }
        put(u64::from(self.channel_configuration), 4);
        put(0, 3); // frameLengthFlag, dependsOnCoreCoder, extensionFlag
        let bytes = len.div_ceil(8);
        let bits = bits << (bytes * 8 - len);
        bits.to_be_bytes()[8 - bytes as usize..].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 7-byte ADTS header: AAC-LC, 16 kHz, mono, `frame_length` bytes, one block.
    pub(crate) fn adts_header(frame_length: usize) -> [u8; 7] {
        let profile = 1u8; // LC - 1
        let index = 8u8; // 16 kHz
        let channels = 1u8;
        let fullness = 0x7FFu16;
        [
            0xFF,
            0xF1,
            (profile << 6) | (index << 2) | (channels >> 2),
            ((channels & 3) << 6) | ((frame_length >> 11) as u8 & 0x03),
            (frame_length >> 3) as u8,
            (((frame_length & 7) as u8) << 5) | ((fullness >> 6) as u8 & 0x1F),
            ((fullness & 0x3F) as u8) << 2,
        ]
    }

    #[test]
    fn parses_adts_header() {
        let header = AdtsHeader::parse(&adts_header(300)).unwrap();
        assert_eq!(header.object_type, 2);
        assert_eq!(header.sample_rate, 16_000);
        assert_eq!(header.sampling_frequency_index, 8);
        assert_eq!(header.channel_configuration, 1);
        assert!(header.protection_absent);
        assert_eq!(header.frame_length, 300);
        assert_eq!(header.header_len(), 7);
        assert_eq!(header.samples(), 1024);
        assert_eq!(header.audio_specific_config().to_bytes(), vec![0x14, 0x08]);
    }

    #[test]
    fn rejects_bad_adts_headers() {
        assert!(AdtsHeader::parse(&[0xFF, 0xF1, 0x60]).is_err());
        let mut header = adts_header(300);
        header[1] = 0xE1;
        assert!(AdtsHeader::parse(&header).is_err());
        let mut header = adts_header(300);
        header[2] |= 0x0F << 2; // reserved frequency index 15
        assert!(AdtsHeader::parse(&header).is_err());
        assert!(AdtsHeader::parse(&adts_header(5)).is_err());
    }

    #[test]
    fn iterates_frames_and_skips_garbage() {
        let mut data = vec![0x00, 0x12];
        for len in [20usize, 9, 30] {
            data.extend_from_slice(&adts_header(len));
            data.extend(std::iter::repeat_n(0xAB, len - 7));
        }
        // Truncated tail frame.
        data.extend_from_slice(&adts_header(50));
        data.extend_from_slice(&[0xCD; 3]);

        let mut frames = adts_frames(&data);
        let lengths: Vec<usize> = frames.by_ref().map(|f| f.data.len()).collect();
        assert_eq!(lengths, vec![20, 9, 30]);
        assert_eq!(frames.offset(), 2 + 20 + 9 + 30);

        let first = adts_frames(&data).next().unwrap();
        assert_eq!(first.payload().len(), 13);
        assert!(first.payload().iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn audio_specific_config_round_trip() {
        // AAC-LC 44.1 kHz stereo: the classic 0x12 0x10.
        let asc = AudioSpecificConfig::new(2, 44_100, 2);
        assert_eq!(asc.to_bytes(), vec![0x12, 0x10]);
        assert_eq!(AudioSpecificConfig::parse(&[0x12, 0x10]).unwrap(), asc);

        // AAC-LC 48 kHz mono.
        assert_eq!(
            AudioSpecificConfig::new(2, 48_000, 1).to_bytes(),
            vec![0x11, 0x88]
        );

        // Explicit frequency.
        let odd = AudioSpecificConfig::new(2, 12_345, 1);
        let bytes = odd.to_bytes();
        assert_eq!(bytes.len(), 5);
        assert_eq!(AudioSpecificConfig::parse(&bytes).unwrap(), odd);

        // Escaped object type.
        let escaped = AudioSpecificConfig::new(42, 16_000, 1);
        assert_eq!(
            AudioSpecificConfig::parse(&escaped.to_bytes()).unwrap(),
            escaped
        );

        assert!(AudioSpecificConfig::parse(&[0x12]).is_err());
    }

    #[test]
    fn frequency_tables() {
        assert_eq!(sampling_frequency_index(8_000), Some(11));
        assert_eq!(sampling_frequency_index(16_000), Some(8));
        assert_eq!(sampling_frequency_index(12_345), None);
        assert_eq!(channel_count(7), 8);
        assert_eq!(channel_count(2), 2);
    }
}

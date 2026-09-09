//! H.265 / HEVC: NAL unit types, VPS/SPS/PPS parsing, the `hvcC` decoder configuration
//! record and WebCodecs-compatible codec strings.
//!
//! H.265 uses the same Annex B byte stream format as H.264; see [`super::h264::nal_units`].
//!
//! References: ITU-T H.265 (7.3.1.2 NAL unit header, 7.3.2.2 SPS, 7.3.3
//! `profile_tier_level`), ISO/IEC 14496-15 8.3.3 (`HEVCDecoderConfigurationRecord`) and
//! Annex E (codec parameter strings).

use std::fmt::Write as _;

use bytes::Bytes;

use super::bits::BitReader;
use super::h264::{push_with_u16_length, remove_emulation_prevention};
use super::{MediaError, VideoCodec, VideoConfig};

/// H.265 `nal_unit_type` values (Table 7-1) used by this crate.
pub mod nal_type {
    /// Trailing picture, non-reference.
    pub const TRAIL_N: u8 = 0;
    /// Trailing picture, reference.
    pub const TRAIL_R: u8 = 1;
    /// First IRAP type: broken link access with leading pictures.
    pub const BLA_W_LP: u8 = 16;
    /// IDR that may have RADL leading pictures.
    pub const IDR_W_RADL: u8 = 19;
    /// IDR without leading pictures.
    pub const IDR_N_LP: u8 = 20;
    /// Clean random access.
    pub const CRA_NUT: u8 = 21;
    /// Last type reserved for IRAP pictures.
    pub const RSV_IRAP_23: u8 = 23;
    /// Video parameter set.
    pub const VPS: u8 = 32;
    /// Sequence parameter set.
    pub const SPS: u8 = 33;
    /// Picture parameter set.
    pub const PPS: u8 = 34;
    /// Access unit delimiter.
    pub const AUD: u8 = 35;
    /// End of sequence.
    pub const EOS: u8 = 36;
    /// End of bitstream.
    pub const EOB: u8 = 37;
    /// Filler data.
    pub const FILLER_DATA: u8 = 38;
    /// SEI placed before the VCL NAL units of an access unit.
    pub const PREFIX_SEI: u8 = 39;
    /// SEI placed after the VCL NAL units of an access unit.
    pub const SUFFIX_SEI: u8 = 40;
}

/// Largest accepted picture dimension in luma samples.
const MAX_DIMENSION: u32 = 16_384;

/// Returns the `nal_unit_type` of a NAL unit (bits 1–6 of its two-byte header).
pub fn nal_unit_type(nal: &[u8]) -> Option<u8> {
    (nal.len() >= 2).then(|| (nal[0] >> 1) & 0x3F)
}

/// Returns the `nuh_layer_id` of a NAL unit; 0 for the base layer.
pub fn nuh_layer_id(nal: &[u8]) -> Option<u8> {
    (nal.len() >= 2).then(|| ((nal[0] & 1) << 5) | (nal[1] >> 3))
}

/// Whether a NAL unit type is a VCL (slice segment) type, 0–31.
pub fn is_vcl(nal_type: u8) -> bool {
    nal_type < 32
}

/// Whether a NAL unit type is an intra random access point (IRAP), 16–23: BLA, IDR,
/// CRA and the reserved IRAP types. Access units starting with one are keyframes.
pub fn is_irap(nal_type: u8) -> bool {
    (nal_type::BLA_W_LP..=nal_type::RSV_IRAP_23).contains(&nal_type)
}

/// The general part of `profile_tier_level()` (7.3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileTierLevel {
    /// 0 for all profiles defined so far.
    pub general_profile_space: u8,
    /// `false` = Main tier, `true` = High tier.
    pub general_tier_flag: bool,
    /// 1 = Main, 2 = Main 10, 3 = Main Still Picture, 4 = range extensions, …
    pub general_profile_idc: u8,
    /// `general_profile_compatibility_flag[0..32]`, flag 0 in the most significant bit.
    pub general_profile_compatibility_flags: u32,
    /// The 48 bits from `general_progressive_source_flag` to the end of the general
    /// constraint flags, in the low 48 bits. This is the field of the same name in `hvcC`.
    pub general_constraint_indicator_flags: u64,
    /// Thirty times the level number, e.g. 93 for level 3.1 and 120 for level 4.
    pub general_level_idc: u8,
}

impl ProfileTierLevel {
    fn parse(r: &mut BitReader<'_>, max_sub_layers_minus1: u8) -> Result<Self, MediaError> {
        let general_profile_space = r.read_u8(2)?;
        let general_tier_flag = r.read_flag()?;
        let general_profile_idc = r.read_u8(5)?;
        let general_profile_compatibility_flags = r.read_bits(32)?;
        let general_constraint_indicator_flags = r.read_bits_u64(48)?;
        let general_level_idc = r.read_u8(8)?;

        let sub_layers = usize::from(max_sub_layers_minus1);
        let mut profile_present = [false; 8];
        let mut level_present = [false; 8];
        for i in 0..sub_layers {
            profile_present[i] = r.read_flag()?;
            level_present[i] = r.read_flag()?;
        }
        if sub_layers > 0 {
            // reserved_zero_2bits for the remaining sub-layer slots.
            r.skip_bits(2 * (8 - sub_layers))?;
        }
        for i in 0..sub_layers {
            if profile_present[i] {
                // Space, tier, profile, 32 compatibility flags and 48 constraint bits.
                r.skip_bits(88)?;
            }
            if level_present[i] {
                r.skip_bits(8)?;
            }
        }

        Ok(Self {
            general_profile_space,
            general_tier_flag,
            general_profile_idc,
            general_profile_compatibility_flags,
            general_constraint_indicator_flags,
            general_level_idc,
        })
    }

    /// The codec parameter string of ISO/IEC 14496-15 Annex E for a sample entry type
    /// (`"hvc1"` or `"hev1"`), e.g. `hvc1.1.6.L93.B0`:
    ///
    /// - the profile space as nothing, `A`, `B` or `C`, followed by the profile idc;
    /// - the 32 compatibility flags in reverse bit order, as hex without leading zeros;
    /// - `L` (Main tier) or `H` (High tier) followed by the level idc;
    /// - the six constraint bytes as hex, with trailing zero bytes omitted.
    pub fn codec_string(&self, sample_entry: &str) -> String {
        let space = match self.general_profile_space {
            0 => "",
            1 => "A",
            2 => "B",
            _ => "C",
        };
        let compatibility = self.general_profile_compatibility_flags.reverse_bits();
        let tier = if self.general_tier_flag { 'H' } else { 'L' };
        let mut s = format!(
            "{sample_entry}.{space}{}.{compatibility:X}.{tier}{}",
            self.general_profile_idc, self.general_level_idc
        );
        let flags = self.general_constraint_indicator_flags.to_be_bytes();
        let constraints = &flags[2..];
        let len = constraints
            .iter()
            .rposition(|&b| b != 0)
            .map_or(0, |i| i + 1);
        for byte in &constraints[..len] {
            // Writing to a String cannot fail.
            let _ = write!(s, ".{byte:02X}");
        }
        s
    }
}

/// Conformance cropping window from the SPS (`conf_win_*_offset`), in chroma sample
/// units: two luma samples per unit horizontally for 4:2:0 and 4:2:2, and vertically for
/// 4:2:0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConformanceWindow {
    /// `conf_win_left_offset`.
    pub left: u32,
    /// `conf_win_right_offset`.
    pub right: u32,
    /// `conf_win_top_offset`.
    pub top: u32,
    /// `conf_win_bottom_offset`.
    pub bottom: u32,
}

/// The parts of an H.265 sequence parameter set needed to configure a decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Sps {
    /// `sps_video_parameter_set_id`: the VPS this SPS refers to.
    pub video_parameter_set_id: u8,
    /// `sps_max_sub_layers_minus1 + 1`, the number of temporal layers.
    pub max_sub_layers: u8,
    /// `sps_temporal_id_nesting_flag`.
    pub temporal_id_nesting: bool,
    /// Profile, tier and level of the stream.
    pub profile_tier_level: ProfileTierLevel,
    /// `sps_seq_parameter_set_id` (0–15), which PPSs refer to.
    pub seq_parameter_set_id: u32,
    /// 0 = monochrome, 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4.
    pub chroma_format_idc: u8,
    /// `separate_colour_plane_flag` (4:4:4 only).
    pub separate_colour_plane: bool,
    /// Coded width before cropping.
    pub pic_width_in_luma_samples: u32,
    /// Coded height before cropping.
    pub pic_height_in_luma_samples: u32,
    /// The cropping window, when `conformance_window_flag` is set.
    pub conformance_window: Option<ConformanceWindow>,
    /// Luma bit depth.
    pub bit_depth_luma: u8,
    /// Chroma bit depth.
    pub bit_depth_chroma: u8,
    /// Display width in pixels: coded width minus the conformance window.
    pub width: u32,
    /// Display height in pixels: coded height minus the conformance window.
    pub height: u32,
}

impl Sps {
    /// Parses an SPS NAL unit (two-byte header included, emulation prevention intact).
    pub fn parse(nal: &[u8]) -> Result<Self, MediaError> {
        const WHAT: &str = "H.265 SPS";
        if nal_unit_type(nal) != Some(nal_type::SPS) {
            return Err(MediaError::invalid(WHAT, "not an SPS NAL unit"));
        }
        let rbsp = remove_emulation_prevention(&nal[2..]);
        let mut r = BitReader::new(&rbsp, WHAT);

        let video_parameter_set_id = r.read_u8(4)?;
        let max_sub_layers_minus1 = r.read_u8(3)?;
        if max_sub_layers_minus1 > 6 {
            return Err(MediaError::invalid(
                WHAT,
                "sps_max_sub_layers_minus1 out of range",
            ));
        }
        let temporal_id_nesting = r.read_flag()?;
        let profile_tier_level = ProfileTierLevel::parse(&mut r, max_sub_layers_minus1)?;
        let seq_parameter_set_id = r.read_ue_max(15, "sps_seq_parameter_set_id out of range")?;
        let chroma_format_idc = r.read_ue_max(3, "chroma_format_idc out of range")? as u8;
        let separate_colour_plane = chroma_format_idc == 3 && r.read_flag()?;
        let pic_width_in_luma_samples =
            r.read_ue_max(MAX_DIMENSION, "pic_width_in_luma_samples out of range")?;
        let pic_height_in_luma_samples =
            r.read_ue_max(MAX_DIMENSION, "pic_height_in_luma_samples out of range")?;
        if pic_width_in_luma_samples == 0 || pic_height_in_luma_samples == 0 {
            return Err(MediaError::invalid(WHAT, "empty picture"));
        }
        let conformance_window_flag = r.read_flag()?;
        let conformance_window = if conformance_window_flag {
            Some(ConformanceWindow {
                left: r.read_ue()?,
                right: r.read_ue()?,
                top: r.read_ue()?,
                bottom: r.read_ue()?,
            })
        } else {
            None
        };
        let bit_depth_luma = 8 + r.read_ue_max(8, "bit_depth_luma_minus8 out of range")? as u8;
        let bit_depth_chroma = 8 + r.read_ue_max(8, "bit_depth_chroma_minus8 out of range")? as u8;
        // The rest of the SPS (ordering info, VUI, …) is not needed.

        let mut sps = Self {
            video_parameter_set_id,
            max_sub_layers: max_sub_layers_minus1 + 1,
            temporal_id_nesting,
            profile_tier_level,
            seq_parameter_set_id,
            chroma_format_idc,
            separate_colour_plane,
            pic_width_in_luma_samples,
            pic_height_in_luma_samples,
            conformance_window,
            bit_depth_luma,
            bit_depth_chroma,
            width: 0,
            height: 0,
        };
        let (width, height) = sps.cropped_size()?;
        sps.width = width;
        sps.height = height;
        Ok(sps)
    }

    /// Codec string for an `hvc1` sample entry / WebCodecs, e.g. `hvc1.1.6.L93.B0`.
    pub fn codec_string(&self) -> String {
        self.profile_tier_level.codec_string("hvc1")
    }

    /// Applies the conformance window (H.265 7.4.3.2.1, Table 6-1).
    fn cropped_size(&self) -> Result<(u32, u32), MediaError> {
        let Some(window) = self.conformance_window else {
            return Ok((
                self.pic_width_in_luma_samples,
                self.pic_height_in_luma_samples,
            ));
        };
        let (sub_width, sub_height) = match (self.chroma_format_idc, self.separate_colour_plane) {
            (1, _) => (2u64, 2u64),
            (2, _) => (2, 1),
            _ => (1, 1),
        };
        let crop_x = (u64::from(window.left) + u64::from(window.right)) * sub_width;
        let crop_y = (u64::from(window.top) + u64::from(window.bottom)) * sub_height;
        let width = u64::from(self.pic_width_in_luma_samples);
        let height = u64::from(self.pic_height_in_luma_samples);
        if crop_x >= width || crop_y >= height {
            return Err(MediaError::invalid(
                "H.265 SPS",
                "conformance window exceeds the picture",
            ));
        }
        // Both are smaller than the coded size, which is a u32.
        Ok(((width - crop_x) as u32, (height - crop_y) as u32))
    }
}

/// Returns `vps_video_parameter_set_id` of a VPS NAL unit.
pub fn vps_id(nal: &[u8]) -> Result<u8, MediaError> {
    if nal_unit_type(nal) != Some(nal_type::VPS) || nal.len() < 3 {
        return Err(MediaError::invalid("H.265 VPS", "not a VPS NAL unit"));
    }
    Ok(nal[2] >> 4)
}

/// The identifiers at the start of an H.265 picture parameter set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pps {
    /// `pps_pic_parameter_set_id` (0–63), which slices refer to.
    pub pic_parameter_set_id: u32,
    /// The SPS this PPS refers to.
    pub seq_parameter_set_id: u32,
}

impl Pps {
    /// Parses the ids of a PPS NAL unit (two-byte header included).
    pub fn parse(nal: &[u8]) -> Result<Self, MediaError> {
        const WHAT: &str = "H.265 PPS";
        if nal_unit_type(nal) != Some(nal_type::PPS) {
            return Err(MediaError::invalid(WHAT, "not a PPS NAL unit"));
        }
        let rbsp = remove_emulation_prevention(&nal[2..]);
        let mut r = BitReader::new(&rbsp, WHAT);
        Ok(Self {
            pic_parameter_set_id: r.read_ue_max(63, "pps_pic_parameter_set_id out of range")?,
            seq_parameter_set_id: r.read_ue_max(15, "pps_seq_parameter_set_id out of range")?,
        })
    }
}

/// Builds an `HEVCDecoderConfigurationRecord` (the body of an `hvcC` box and the
/// WebCodecs `description`) with 4-byte NAL length fields.
///
/// Profile, tier, level, chroma format, bit depths and temporal layering come from the
/// first SPS. All parameter sets are in the record and `array_completeness` is set, as
/// an `hvc1` sample entry requires (the frames carry no parameter sets).
/// `min_spatial_segmentation_idc`, `parallelismType` and the frame rate fields are 0
/// ("unknown"), which is always valid.
pub fn hevc_decoder_configuration_record<V, S, P>(
    vps_list: &[V],
    sps_list: &[S],
    pps_list: &[P],
) -> Result<Vec<u8>, MediaError>
where
    V: AsRef<[u8]>,
    S: AsRef<[u8]>,
    P: AsRef<[u8]>,
{
    const WHAT: &str = "hvcC parameter sets";
    if vps_list.is_empty() || sps_list.is_empty() || pps_list.is_empty() {
        return Err(MediaError::invalid(
            WHAT,
            "VPS, SPS and PPS are all required",
        ));
    }
    if !all_of_type(vps_list, nal_type::VPS)
        || !all_of_type(sps_list, nal_type::SPS)
        || !all_of_type(pps_list, nal_type::PPS)
    {
        return Err(MediaError::invalid(WHAT, "wrong NAL unit type"));
    }
    let sps = Sps::parse(sps_list[0].as_ref())?;
    if sps.bit_depth_luma > 15 || sps.bit_depth_chroma > 15 {
        return Err(MediaError::Unsupported(format!(
            "H.265 bit depth {} for hvcC",
            sps.bit_depth_luma.max(sps.bit_depth_chroma)
        )));
    }
    let ptl = sps.profile_tier_level;

    let mut out = Vec::with_capacity(64);
    out.push(1); // configurationVersion
    out.push(
        (ptl.general_profile_space << 6)
            | (u8::from(ptl.general_tier_flag) << 5)
            | ptl.general_profile_idc,
    );
    out.extend_from_slice(&ptl.general_profile_compatibility_flags.to_be_bytes());
    out.extend_from_slice(&ptl.general_constraint_indicator_flags.to_be_bytes()[2..]);
    out.push(ptl.general_level_idc);
    out.extend_from_slice(&[0xF0, 0x00]); // reserved + min_spatial_segmentation_idc = 0
    out.push(0xFC); // reserved + parallelismType = 0
    out.push(0xFC | sps.chroma_format_idc);
    out.push(0xF8 | (sps.bit_depth_luma - 8));
    out.push(0xF8 | (sps.bit_depth_chroma - 8));
    out.extend_from_slice(&[0, 0]); // avgFrameRate
    // constantFrameRate = 0, numTemporalLayers, temporalIdNested, lengthSizeMinusOne = 3
    out.push((sps.max_sub_layers << 3) | (u8::from(sps.temporal_id_nesting) << 2) | 3);
    out.push(3); // numOfArrays
    push_array(&mut out, nal_type::VPS, vps_list, WHAT)?;
    push_array(&mut out, nal_type::SPS, sps_list, WHAT)?;
    push_array(&mut out, nal_type::PPS, pps_list, WHAT)?;
    Ok(out)
}

fn all_of_type<T: AsRef<[u8]>>(nals: &[T], expected: u8) -> bool {
    nals.iter()
        .all(|nal| nal_unit_type(nal.as_ref()) == Some(expected))
}

fn push_array<T: AsRef<[u8]>>(
    out: &mut Vec<u8>,
    nal_type: u8,
    nals: &[T],
    what: &'static str,
) -> Result<(), MediaError> {
    let count = u16::try_from(nals.len())
        .map_err(|_| MediaError::invalid(what, "too many parameter sets"))?;
    out.push(0x80 | nal_type); // array_completeness = 1, reserved = 0
    out.extend_from_slice(&count.to_be_bytes());
    for nal in nals {
        push_with_u16_length(out, nal.as_ref(), what)?;
    }
    Ok(())
}

/// Builds a [`VideoConfig`] from H.265 parameter sets. Size and codec string come from
/// the first SPS.
pub fn video_config<V, S, P>(
    vps_list: &[V],
    sps_list: &[S],
    pps_list: &[P],
) -> Result<VideoConfig, MediaError>
where
    V: AsRef<[u8]>,
    S: AsRef<[u8]>,
    P: AsRef<[u8]>,
{
    let description = hevc_decoder_configuration_record(vps_list, sps_list, pps_list)?;
    // The record builder validated that the first SPS exists and parses.
    let sps = Sps::parse(sps_list[0].as_ref())?;
    Ok(VideoConfig {
        codec: VideoCodec::H265,
        codec_string: sps.codec_string(),
        width: sps.width,
        height: sps.height,
        description: Bytes::from(description),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::media::h264::tests::hex;

    // Parameter sets produced by x265 4.2 (ffmpeg 8.1); expected values checked with
    // ffprobe, records compared with what ffmpeg writes into MP4 files.
    pub(crate) const VPS_640X360: &str = "40010c01ffff01600000030090000003000003003fba0240";
    pub(crate) const SPS_640X360: &str =
        "42010101600000030090000003000003003fa00502016965ba924caf0168080000030008000003007840";
    pub(crate) const PPS_640X360: &str = "4401c172b46240";
    const VPS_1080P: &str = "40010c01ffff016000000300900000030000030078959409";
    const SPS_1080P: &str =
        "420101016000000300900000030000030078a003c0801107cb965654a4c2f0168080000003008000000c84";
    const VPS_1080P_MAIN10: &str = "40010c01ffff022000000300900000030000030078959409";
    const SPS_1080P_MAIN10: &str = "420101022000000300900000030000030078a003c0801107cad965654a4c\
                                    2f016808000003000800000300c840";
    const PPS_1080P: &str = "4401c073c189";

    #[test]
    fn nal_header_fields() {
        assert_eq!(nal_unit_type(&hex(VPS_640X360)), Some(nal_type::VPS));
        assert_eq!(nal_unit_type(&hex(SPS_640X360)), Some(nal_type::SPS));
        assert_eq!(nal_unit_type(&hex(PPS_640X360)), Some(nal_type::PPS));
        assert_eq!(nal_unit_type(&[0x26, 0x01]), Some(nal_type::IDR_W_RADL));
        assert_eq!(nal_unit_type(&[0x26]), None);
        assert_eq!(nuh_layer_id(&[0x26, 0x01]), Some(0));
        assert_eq!(nuh_layer_id(&[0x27, 0x09]), Some(33));
        assert!(is_irap(19) && is_irap(21) && is_irap(16) && is_irap(23));
        assert!(!is_irap(1) && !is_irap(24) && !is_irap(9));
        assert!(is_vcl(0) && is_vcl(31) && !is_vcl(32));
    }

    #[test]
    fn parses_main_sps() {
        let sps = Sps::parse(&hex(SPS_640X360)).unwrap();
        let ptl = sps.profile_tier_level;
        assert_eq!(ptl.general_profile_space, 0);
        assert!(!ptl.general_tier_flag);
        assert_eq!(ptl.general_profile_idc, 1);
        assert_eq!(ptl.general_profile_compatibility_flags, 0x6000_0000);
        assert_eq!(ptl.general_constraint_indicator_flags, 0x9000_0000_0000);
        assert_eq!(ptl.general_level_idc, 63);
        assert_eq!(sps.max_sub_layers, 1);
        assert!(sps.temporal_id_nesting);
        assert_eq!(sps.chroma_format_idc, 1);
        assert_eq!((sps.bit_depth_luma, sps.bit_depth_chroma), (8, 8));
        assert_eq!((sps.width, sps.height), (640, 360));
        assert_eq!(sps.codec_string(), "hvc1.1.6.L63.90");
    }

    #[test]
    fn parses_sps_with_conformance_window() {
        let sps = Sps::parse(&hex(SPS_1080P)).unwrap();
        assert_eq!(
            (
                sps.pic_width_in_luma_samples,
                sps.pic_height_in_luma_samples
            ),
            (1920, 1088)
        );
        assert_eq!(
            sps.conformance_window,
            Some(ConformanceWindow {
                left: 0,
                right: 0,
                top: 0,
                bottom: 4
            })
        );
        assert_eq!((sps.width, sps.height), (1920, 1080));
        assert_eq!(sps.codec_string(), "hvc1.1.6.L120.90");
    }

    #[test]
    fn parses_main10_sps() {
        let sps = Sps::parse(&hex(SPS_1080P_MAIN10)).unwrap();
        assert_eq!(sps.profile_tier_level.general_profile_idc, 2);
        assert_eq!((sps.bit_depth_luma, sps.bit_depth_chroma), (10, 10));
        assert_eq!((sps.width, sps.height), (1920, 1080));
        assert_eq!(sps.codec_string(), "hvc1.2.4.L120.90");
    }

    #[test]
    fn rejects_bad_sps() {
        assert!(Sps::parse(&hex(PPS_1080P)).is_err());
        assert!(Sps::parse(&hex(SPS_640X360)[..10]).is_err());
    }

    #[test]
    fn codec_string_formatting() {
        let mut ptl = ProfileTierLevel {
            general_profile_space: 0,
            general_tier_flag: false,
            general_profile_idc: 1,
            general_profile_compatibility_flags: 0x6000_0000,
            general_constraint_indicator_flags: 0xB000_0000_0000,
            general_level_idc: 93,
        };
        assert_eq!(ptl.codec_string("hvc1"), "hvc1.1.6.L93.B0");
        assert_eq!(ptl.codec_string("hev1"), "hev1.1.6.L93.B0");

        // Trailing zero constraint bytes are dropped, inner ones are kept.
        ptl.general_constraint_indicator_flags = 0x9000_0000_0001;
        assert_eq!(ptl.codec_string("hvc1"), "hvc1.1.6.L93.90.00.00.00.00.01");
        ptl.general_constraint_indicator_flags = 0;
        assert_eq!(ptl.codec_string("hvc1"), "hvc1.1.6.L93");

        // Profile space letter, high tier, reversed compatibility flags.
        ptl.general_profile_space = 1;
        ptl.general_tier_flag = true;
        ptl.general_profile_idc = 4;
        ptl.general_profile_compatibility_flags = 0x0800_0000; // flag 4
        ptl.general_level_idc = 153;
        assert_eq!(ptl.codec_string("hvc1"), "hvc1.A4.10.H153");
        ptl.general_profile_compatibility_flags = 0x0000_0001; // flag 31
        assert_eq!(ptl.codec_string("hvc1"), "hvc1.A4.80000000.H153");
        ptl.general_profile_compatibility_flags = 0;
        assert_eq!(ptl.codec_string("hvc1"), "hvc1.A4.0.H153");
        ptl.general_profile_space = 3;
        assert!(ptl.codec_string("hvc1").starts_with("hvc1.C4."));
    }

    #[test]
    fn parses_ids() {
        assert_eq!(vps_id(&hex(VPS_640X360)).unwrap(), 0);
        let pps = Pps::parse(&hex(PPS_640X360)).unwrap();
        assert_eq!((pps.pic_parameter_set_id, pps.seq_parameter_set_id), (0, 0));
        assert!(vps_id(&hex(PPS_640X360)).is_err());
    }

    /// The records must match what ffmpeg writes (`-tag:v hvc1`) for the same streams.
    #[test]
    fn builds_hvcc_like_ffmpeg() {
        let hvcc = hevc_decoder_configuration_record(
            &[hex(VPS_640X360)],
            &[hex(SPS_640X360)],
            &[hex(PPS_640X360)],
        )
        .unwrap();
        assert_eq!(
            hvcc,
            hex("0101600000009000000000003ff000fcfdf8f800000f03\
                 a00001001840010c01ffff01600000030090000003000003003fba0240\
                 a10001002a42010101600000030090000003000003003fa00502016965ba924caf01680800\
                 00030008000003007840\
                 a2000100074401c172b46240")
        );

        let hvcc = hevc_decoder_configuration_record(
            &[hex(VPS_1080P_MAIN10)],
            &[hex(SPS_1080P_MAIN10)],
            &[hex(PPS_1080P)],
        )
        .unwrap();
        assert_eq!(
            hvcc,
            hex("01022000000090000000000078f000fcfdfafa00000f03\
                 a00001001840010c01ffff022000000300900000030000030078959409\
                 a10001002d420101022000000300900000030000030078a003c0801107cad965654a4c2f01\
                 6808000003000800000300c840\
                 a2000100064401c073c189")
        );
    }

    #[test]
    fn hvcc_requires_all_parameter_sets() {
        let none: [Vec<u8>; 0] = [];
        assert!(
            hevc_decoder_configuration_record(&none, &[hex(SPS_1080P)], &[hex(PPS_1080P)]).is_err()
        );
        assert!(
            hevc_decoder_configuration_record(
                &[hex(SPS_1080P)],
                &[hex(VPS_1080P)],
                &[hex(PPS_1080P)]
            )
            .is_err()
        );
    }

    #[test]
    fn builds_video_config() {
        let config = video_config(&[hex(VPS_1080P)], &[hex(SPS_1080P)], &[hex(PPS_1080P)]).unwrap();
        assert_eq!(config.codec, VideoCodec::H265);
        assert_eq!(config.codec_string, "hvc1.1.6.L120.90");
        assert_eq!((config.width, config.height), (1920, 1080));
    }
}

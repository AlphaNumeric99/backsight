//! H.264 / AVC: Annex B byte streams, NAL units, SPS parsing, the `avcC` decoder
//! configuration record and RFC 6381 codec strings.
//!
//! The Annex B helpers ([`find_start_code`], [`nal_units`], [`remove_emulation_prevention`])
//! also apply to H.265, which uses the same byte stream format.
//!
//! References: ITU-T H.264 (7.3.2.1.1 SPS syntax, Annex B byte stream format),
//! ISO/IEC 14496-15 5.3.3 (`AVCDecoderConfigurationRecord`), RFC 6381 3.3.

use std::borrow::Cow;

use bytes::Bytes;

use super::bits::BitReader;
use super::{MediaError, VideoCodec, VideoConfig};

/// H.264 `nal_unit_type` values (Table 7-1) used by this crate.
pub mod nal_type {
    /// Coded slice of a non-IDR picture.
    pub const NON_IDR_SLICE: u8 = 1;
    /// Coded slice data partition A.
    pub const SLICE_DATA_PARTITION_A: u8 = 2;
    /// Coded slice of an IDR picture.
    pub const IDR_SLICE: u8 = 5;
    /// Supplemental enhancement information.
    pub const SEI: u8 = 6;
    /// Sequence parameter set.
    pub const SPS: u8 = 7;
    /// Picture parameter set.
    pub const PPS: u8 = 8;
    /// Access unit delimiter.
    pub const AUD: u8 = 9;
    /// End of sequence.
    pub const END_OF_SEQUENCE: u8 = 10;
    /// End of stream.
    pub const END_OF_STREAM: u8 = 11;
    /// Filler data.
    pub const FILLER_DATA: u8 = 12;
    /// Sequence parameter set extension.
    pub const SPS_EXTENSION: u8 = 13;
    /// Prefix NAL unit (SVC/MVC).
    pub const PREFIX: u8 = 14;
    /// Subset sequence parameter set (SVC/MVC).
    pub const SUBSET_SPS: u8 = 15;
}

/// Largest accepted `pic_width_in_mbs_minus1` / `pic_height_in_map_units_minus1`
/// (16 368 pixels), far beyond any camera.
const MAX_MBS_MINUS1: u32 = 1023;

/// Returns the index of the next three-byte start code prefix (`00 00 01`) in `data`.
///
/// A four-byte start code (`00 00 00 01`) is found at its second byte; the leading zero
/// is a `zero_byte` that belongs to neither NAL unit.
pub fn find_start_code(data: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + 2 < data.len() {
        match data[i + 2] {
            // No start code can begin at i, i + 1 or i + 2.
            b if b > 1 => i += 3,
            1 if data[i] == 0 && data[i + 1] == 0 => return Some(i),
            1 => i += 3,
            // data[i + 2] == 0: a start code may begin at i + 1 or i + 2.
            _ => i += 1,
        }
    }
    None
}

/// Strips `trailing_zero_8bits` (and zero bytes of a following four-byte start code).
/// A NAL unit never ends in a zero byte.
pub(crate) fn trim_trailing_zeros(nal: &[u8]) -> &[u8] {
    let end = nal.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &nal[..end]
}

/// Iterates over the NAL units of an Annex B byte stream (H.264 or H.265).
///
/// Bytes before the first start code are ignored, as are empty NAL units. Each item is
/// one NAL unit including its header, without start code or trailing zero bytes, and with
/// emulation prevention bytes intact.
pub fn nal_units(data: &[u8]) -> NalUnits<'_> {
    let rest = match find_start_code(data) {
        Some(pos) => &data[pos + 3..],
        None => &[],
    };
    NalUnits { rest }
}

/// Iterator returned by [`nal_units`].
#[derive(Debug, Clone)]
pub struct NalUnits<'a> {
    /// Data right after a start code, or empty when done.
    rest: &'a [u8],
}

impl<'a> Iterator for NalUnits<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        while !self.rest.is_empty() {
            let (nal, next) = match find_start_code(self.rest) {
                Some(pos) => (&self.rest[..pos], &self.rest[pos + 3..]),
                None => (self.rest, &[][..]),
            };
            self.rest = next;
            let nal = trim_trailing_zeros(nal);
            if !nal.is_empty() {
                return Some(nal);
            }
        }
        None
    }
}

/// Removes emulation prevention bytes (`00 00 03` becomes `00 00`), turning NAL unit
/// payload bytes into the RBSP. Borrows the input when there is nothing to remove.
pub fn remove_emulation_prevention(data: &[u8]) -> Cow<'_, [u8]> {
    let mut out: Option<Vec<u8>> = None;
    let mut zeros = 0usize;
    for (i, &byte) in data.iter().enumerate() {
        if zeros >= 2 && byte == 3 {
            out.get_or_insert_with(|| data[..i].to_vec());
            zeros = 0;
            continue;
        }
        if let Some(out) = out.as_mut() {
            out.push(byte);
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    match out {
        Some(rbsp) => Cow::Owned(rbsp),
        None => Cow::Borrowed(data),
    }
}

/// Converts an Annex B byte stream into NAL units with 4-byte big-endian length
/// prefixes (the MP4 / `avcC` / `hvcC` layout with `lengthSizeMinusOne = 3`).
pub fn annex_b_to_length_prefixed(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16);
    for nal in nal_units(data) {
        // NAL units larger than 4 GiB cannot exist in a slice of a real stream.
        out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
        out.extend_from_slice(nal);
    }
    out
}

/// Returns the `nal_unit_type` of a NAL unit (the low five bits of its header byte).
pub fn nal_unit_type(nal: &[u8]) -> Option<u8> {
    nal.first().map(|header| header & 0x1F)
}

/// Whether a NAL unit type carries slice data of the primary coded picture (types 1–5).
pub fn is_vcl(nal_type: u8) -> bool {
    (nal_type::NON_IDR_SLICE..=nal_type::IDR_SLICE).contains(&nal_type)
}

/// Frame cropping offsets from the SPS (`frame_crop_*_offset`), in crop units: two
/// pixels per unit horizontally for 4:2:0 and 4:2:2, and two (four for field coding)
/// pixels per unit vertically for 4:2:0.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameCropping {
    /// `frame_crop_left_offset`.
    pub left: u32,
    /// `frame_crop_right_offset`.
    pub right: u32,
    /// `frame_crop_top_offset`.
    pub top: u32,
    /// `frame_crop_bottom_offset`.
    pub bottom: u32,
}

/// The parts of an H.264 sequence parameter set needed to configure a decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Sps {
    /// `profile_idc`, e.g. 66 (Baseline), 77 (Main), 100 (High).
    pub profile_idc: u8,
    /// The byte after `profile_idc`: `constraint_set0_flag` … `constraint_set5_flag` and
    /// two reserved bits. This is the "profile compatibility" byte of `avcC` and codec strings.
    pub constraint_flags: u8,
    /// `level_idc`, ten times the level number (e.g. 31 for level 3.1).
    pub level_idc: u8,
    /// `seq_parameter_set_id` (0–31), which PPSs refer to.
    pub seq_parameter_set_id: u32,
    /// 0 = monochrome, 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4. Defaults to 1 when absent.
    pub chroma_format_idc: u8,
    /// `separate_colour_plane_flag` (4:4:4 only).
    pub separate_colour_plane: bool,
    /// Luma bit depth (8 when absent).
    pub bit_depth_luma: u8,
    /// Chroma bit depth (8 when absent).
    pub bit_depth_chroma: u8,
    /// `pic_width_in_mbs_minus1 + 1`: coded width in 16-pixel macroblocks.
    pub pic_width_in_mbs: u32,
    /// `pic_height_in_map_units_minus1 + 1`: coded height in macroblocks (in macroblock
    /// pairs for field-coded streams).
    pub pic_height_in_map_units: u32,
    /// `frame_mbs_only_flag`: no field coding.
    pub frame_mbs_only: bool,
    /// The cropping rectangle, when `frame_cropping_flag` is set.
    pub frame_cropping: Option<FrameCropping>,
    /// Display width in pixels: coded width minus cropping.
    pub width: u32,
    /// Display height in pixels: coded height minus cropping.
    pub height: u32,
}

impl Sps {
    /// Parses an SPS NAL unit (header byte included, emulation prevention intact).
    pub fn parse(nal: &[u8]) -> Result<Self, MediaError> {
        const WHAT: &str = "H.264 SPS";
        if nal_unit_type(nal) != Some(nal_type::SPS) {
            return Err(MediaError::invalid(WHAT, "not an SPS NAL unit"));
        }
        let rbsp = remove_emulation_prevention(&nal[1..]);
        let mut r = BitReader::new(&rbsp, WHAT);

        let profile_idc = r.read_u8(8)?;
        let constraint_flags = r.read_u8(8)?;
        let level_idc = r.read_u8(8)?;
        let seq_parameter_set_id = r.read_ue_max(31, "seq_parameter_set_id out of range")?;

        let mut chroma_format_idc = 1;
        let mut separate_colour_plane = false;
        let mut bit_depth_luma = 8;
        let mut bit_depth_chroma = 8;
        if has_chroma_info(profile_idc) {
            chroma_format_idc = r.read_ue_max(3, "chroma_format_idc out of range")? as u8;
            if chroma_format_idc == 3 {
                separate_colour_plane = r.read_flag()?;
            }
            bit_depth_luma = 8 + r.read_ue_max(6, "bit_depth_luma_minus8 out of range")? as u8;
            bit_depth_chroma = 8 + r.read_ue_max(6, "bit_depth_chroma_minus8 out of range")? as u8;
            let _qpprime_y_zero_transform_bypass_flag = r.read_flag()?;
            let seq_scaling_matrix_present_flag = r.read_flag()?;
            if seq_scaling_matrix_present_flag {
                let lists = if chroma_format_idc == 3 { 12 } else { 8 };
                for i in 0..lists {
                    let seq_scaling_list_present_flag = r.read_flag()?;
                    if seq_scaling_list_present_flag {
                        skip_scaling_list(&mut r, if i < 6 { 16 } else { 64 })?;
                    }
                }
            }
        }

        let _log2_max_frame_num_minus4 =
            r.read_ue_max(12, "log2_max_frame_num_minus4 out of range")?;
        let pic_order_cnt_type = r.read_ue_max(2, "pic_order_cnt_type out of range")?;
        if pic_order_cnt_type == 0 {
            let _log2_max_pic_order_cnt_lsb_minus4 =
                r.read_ue_max(12, "log2_max_pic_order_cnt_lsb_minus4 out of range")?;
        } else if pic_order_cnt_type == 1 {
            let _delta_pic_order_always_zero_flag = r.read_flag()?;
            let _offset_for_non_ref_pic = r.read_se()?;
            let _offset_for_top_to_bottom_field = r.read_se()?;
            let cycle = r.read_ue_max(255, "num_ref_frames_in_pic_order_cnt_cycle out of range")?;
            for _ in 0..cycle {
                let _offset_for_ref_frame = r.read_se()?;
            }
        }
        let _max_num_ref_frames = r.read_ue()?;
        let _gaps_in_frame_num_value_allowed_flag = r.read_flag()?;
        let pic_width_in_mbs =
            r.read_ue_max(MAX_MBS_MINUS1, "pic_width_in_mbs_minus1 out of range")? + 1;
        let pic_height_in_map_units = r.read_ue_max(
            MAX_MBS_MINUS1,
            "pic_height_in_map_units_minus1 out of range",
        )? + 1;
        let frame_mbs_only = r.read_flag()?;
        if !frame_mbs_only {
            let _mb_adaptive_frame_field_flag = r.read_flag()?;
        }
        let _direct_8x8_inference_flag = r.read_flag()?;
        let frame_cropping_flag = r.read_flag()?;
        let frame_cropping = if frame_cropping_flag {
            Some(FrameCropping {
                left: r.read_ue()?,
                right: r.read_ue()?,
                top: r.read_ue()?,
                bottom: r.read_ue()?,
            })
        } else {
            None
        };
        // vui_parameters_present_flag and the VUI are not needed.

        let mut sps = Self {
            profile_idc,
            constraint_flags,
            level_idc,
            seq_parameter_set_id,
            chroma_format_idc,
            separate_colour_plane,
            bit_depth_luma,
            bit_depth_chroma,
            pic_width_in_mbs,
            pic_height_in_map_units,
            frame_mbs_only,
            frame_cropping,
            width: 0,
            height: 0,
        };
        let (width, height) = sps.cropped_size()?;
        sps.width = width;
        sps.height = height;
        Ok(sps)
    }

    /// Width of the decoded picture before cropping (a multiple of 16).
    pub fn coded_width(&self) -> u32 {
        self.pic_width_in_mbs * 16
    }

    /// Height of the decoded frame before cropping.
    pub fn coded_height(&self) -> u32 {
        let field_factor = if self.frame_mbs_only { 1 } else { 2 };
        self.pic_height_in_map_units * 16 * field_factor
    }

    /// RFC 6381 codec string, e.g. `avc1.64001F`.
    pub fn codec_string(&self) -> String {
        format!(
            "avc1.{:02X}{:02X}{:02X}",
            self.profile_idc, self.constraint_flags, self.level_idc
        )
    }

    /// Applies the frame cropping rectangle (H.264 equations 7-19 to 7-22).
    fn cropped_size(&self) -> Result<(u32, u32), MediaError> {
        let Some(crop) = self.frame_cropping else {
            return Ok((self.coded_width(), self.coded_height()));
        };
        let field_factor: u64 = if self.frame_mbs_only { 1 } else { 2 };
        let chroma_array_type = if self.separate_colour_plane {
            0
        } else {
            self.chroma_format_idc
        };
        let (crop_unit_x, crop_unit_y): (u64, u64) = match chroma_array_type {
            1 => (2, 2 * field_factor),
            2 => (2, field_factor),
            _ => (1, field_factor),
        };
        let crop_x = (u64::from(crop.left) + u64::from(crop.right)) * crop_unit_x;
        let crop_y = (u64::from(crop.top) + u64::from(crop.bottom)) * crop_unit_y;
        let width = u64::from(self.coded_width());
        let height = u64::from(self.coded_height());
        if crop_x >= width || crop_y >= height {
            return Err(MediaError::invalid(
                "H.264 SPS",
                "frame cropping exceeds the picture",
            ));
        }
        // Both fit: they are smaller than the coded size, which is a u32.
        Ok(((width - crop_x) as u32, (height - crop_y) as u32))
    }
}

/// Profiles whose SPS carries `chroma_format_idc` and bit depths (7.3.2.1.1).
fn has_chroma_info(profile_idc: u8) -> bool {
    matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    )
}

/// Skips a `scaling_list()` (7.3.2.1.1.1).
fn skip_scaling_list(r: &mut BitReader<'_>, size: usize) -> Result<(), MediaError> {
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;
    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = r.read_se()?;
            if !(-128..=127).contains(&delta_scale) {
                return Err(MediaError::invalid("H.264 SPS", "delta_scale out of range"));
            }
            next_scale = (last_scale + delta_scale + 256).rem_euclid(256);
        }
        if next_scale != 0 {
            last_scale = next_scale;
        }
    }
    Ok(())
}

/// The identifiers at the start of an H.264 picture parameter set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Pps {
    /// `pic_parameter_set_id` (0–255), which slices refer to.
    pub pic_parameter_set_id: u32,
    /// The SPS this PPS refers to.
    pub seq_parameter_set_id: u32,
}

impl Pps {
    /// Parses the ids of a PPS NAL unit (header byte included).
    pub fn parse(nal: &[u8]) -> Result<Self, MediaError> {
        const WHAT: &str = "H.264 PPS";
        if nal_unit_type(nal) != Some(nal_type::PPS) {
            return Err(MediaError::invalid(WHAT, "not a PPS NAL unit"));
        }
        let rbsp = remove_emulation_prevention(&nal[1..]);
        let mut r = BitReader::new(&rbsp, WHAT);
        Ok(Self {
            pic_parameter_set_id: r.read_ue_max(255, "pic_parameter_set_id out of range")?,
            seq_parameter_set_id: r.read_ue_max(31, "seq_parameter_set_id out of range")?,
        })
    }
}

/// Builds an `AVCDecoderConfigurationRecord` (the body of an `avcC` box and the
/// WebCodecs `description`) with 4-byte NAL length fields.
///
/// Profile, compatibility and level come from the first SPS. For profiles other than
/// Baseline, Main and Extended the record carries the chroma format and bit depths, as
/// ISO/IEC 14496-15 requires.
pub fn avc_decoder_configuration_record<S, P>(
    sps_list: &[S],
    pps_list: &[P],
) -> Result<Vec<u8>, MediaError>
where
    S: AsRef<[u8]>,
    P: AsRef<[u8]>,
{
    const WHAT: &str = "avcC parameter sets";
    let first = sps_list
        .first()
        .ok_or(MediaError::invalid(WHAT, "at least one SPS is required"))?;
    if pps_list.is_empty() {
        return Err(MediaError::invalid(WHAT, "at least one PPS is required"));
    }
    if sps_list.len() > 31 || pps_list.len() > 255 {
        return Err(MediaError::invalid(WHAT, "too many parameter sets"));
    }
    if sps_list
        .iter()
        .any(|s| nal_unit_type(s.as_ref()) != Some(nal_type::SPS))
        || pps_list
            .iter()
            .any(|p| nal_unit_type(p.as_ref()) != Some(nal_type::PPS))
    {
        return Err(MediaError::invalid(WHAT, "wrong NAL unit type"));
    }
    let sps = Sps::parse(first.as_ref())?;

    let mut out = Vec::with_capacity(
        16 + sps_list.iter().map(|s| s.as_ref().len() + 2).sum::<usize>()
            + pps_list.iter().map(|p| p.as_ref().len() + 2).sum::<usize>(),
    );
    out.push(1); // configurationVersion
    out.push(sps.profile_idc);
    out.push(sps.constraint_flags);
    out.push(sps.level_idc);
    out.push(0xFC | 3); // reserved + lengthSizeMinusOne
    out.push(0xE0 | sps_list.len() as u8); // reserved + numOfSequenceParameterSets
    for s in sps_list {
        push_with_u16_length(&mut out, s.as_ref(), WHAT)?;
    }
    out.push(pps_list.len() as u8);
    for p in pps_list {
        push_with_u16_length(&mut out, p.as_ref(), WHAT)?;
    }
    if !matches!(sps.profile_idc, 66 | 77 | 88) {
        out.push(0xFC | sps.chroma_format_idc);
        out.push(0xF8 | (sps.bit_depth_luma - 8));
        out.push(0xF8 | (sps.bit_depth_chroma - 8));
        out.push(0); // numOfSequenceParameterSetExt
    }
    Ok(out)
}

pub(crate) fn push_with_u16_length(
    out: &mut Vec<u8>,
    nal: &[u8],
    what: &'static str,
) -> Result<(), MediaError> {
    let len = u16::try_from(nal.len())
        .map_err(|_| MediaError::invalid(what, "parameter set longer than 65535 bytes"))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(nal);
    Ok(())
}

/// Builds a [`VideoConfig`] from H.264 parameter sets. Size and codec string come from
/// the first SPS.
pub fn video_config<S, P>(sps_list: &[S], pps_list: &[P]) -> Result<VideoConfig, MediaError>
where
    S: AsRef<[u8]>,
    P: AsRef<[u8]>,
{
    let description = avc_decoder_configuration_record(sps_list, pps_list)?;
    // The record builder validated that the first SPS exists and parses.
    let sps = Sps::parse(sps_list[0].as_ref())?;
    Ok(VideoConfig {
        codec: VideoCodec::H264,
        codec_string: sps.codec_string(),
        width: sps.width,
        height: sps.height,
        description: Bytes::from(description),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn hex(s: &str) -> Vec<u8> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    // Parameter sets produced by x264 (ffmpeg 8.1); expected values checked with ffprobe.
    pub(crate) const SPS_HIGH_640X360: &str = "67640016acb405017fcb8088000003000800000300f078b175";
    pub(crate) const PPS_HIGH_640X360: &str = "68ef3cb0";
    const SPS_BASELINE_1080P: &str = "6742c028da01e0089f97011000000300100000030320f1832a";
    const SPS_HIGH10_1080P: &str = "676e0028a6cb403c0113f2e022000003000200000300641e306540";
    const SPS_HIGH444_720P: &str = "67f4001f9196805005bb011000000300100000030320f1832a";
    const PPS_HIGH444_720P: &str = "68ce0f1920";
    const SPS_BASELINE_CIF: &str = "6742c00dda05825b011000000300100000030320f142aa";
    const PPS_CE0FC8: &str = "68ce0fc8";

    #[test]
    fn finds_start_codes() {
        assert_eq!(find_start_code(&[0, 0, 1]), Some(0));
        assert_eq!(find_start_code(&[0, 0, 0, 1]), Some(1));
        assert_eq!(find_start_code(&[5, 0, 0, 1, 9]), Some(1));
        assert_eq!(find_start_code(&[0, 0, 2, 0, 0, 0, 0, 1]), Some(5));
        assert_eq!(find_start_code(&[1, 0, 1, 0, 0]), None);
        assert_eq!(find_start_code(&[0, 0]), None);
        assert_eq!(find_start_code(&[]), None);
    }

    #[test]
    fn splits_annex_b() {
        let data = [
            0xAA, 0, 0, 0, 1, 0x09, 0x10, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 0, 0, 1, 0x65, 0x88, 0, 0,
            1, 0, 0, 1, 0x41,
        ];
        let nals: Vec<&[u8]> = nal_units(&data).collect();
        assert_eq!(
            nals,
            vec![
                &[0x09, 0x10][..],
                &[0x67, 1, 2][..],
                &[0x65, 0x88][..],
                &[0x41][..]
            ]
        );
        assert_eq!(nal_units(&[1, 2, 3]).count(), 0);
    }

    #[test]
    fn converts_to_length_prefixed() {
        let data = [0, 0, 0, 1, 0x67, 0x42, 0, 0, 1, 0x68, 0xCE];
        assert_eq!(
            annex_b_to_length_prefixed(&data),
            vec![0, 0, 0, 2, 0x67, 0x42, 0, 0, 0, 2, 0x68, 0xCE]
        );
    }

    #[test]
    fn removes_emulation_prevention() {
        assert!(matches!(
            remove_emulation_prevention(&[1, 2, 3]),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            remove_emulation_prevention(&[0, 0, 3, 1, 0, 0, 3, 0, 0, 3]).as_ref(),
            &[0, 0, 1, 0, 0, 0, 0]
        );
        // `00 00 03 03`: only the first 03 is an emulation prevention byte.
        assert_eq!(
            remove_emulation_prevention(&[0, 0, 3, 3, 0]).as_ref(),
            &[0, 0, 3, 0]
        );
        // A 03 not preceded by two zeros is data.
        assert_eq!(
            remove_emulation_prevention(&[0, 3, 0, 0, 2]).as_ref(),
            &[0, 3, 0, 0, 2]
        );
    }

    #[test]
    fn parses_high_profile_sps() {
        let sps = Sps::parse(&hex(SPS_HIGH_640X360)).unwrap();
        assert_eq!(sps.profile_idc, 100);
        assert_eq!(sps.constraint_flags, 0);
        assert_eq!(sps.level_idc, 22);
        assert_eq!(sps.chroma_format_idc, 1);
        assert_eq!((sps.bit_depth_luma, sps.bit_depth_chroma), (8, 8));
        assert_eq!((sps.width, sps.height), (640, 360));
        assert_eq!((sps.coded_width(), sps.coded_height()), (640, 368));
        assert_eq!(sps.codec_string(), "avc1.640016");
    }

    #[test]
    fn parses_baseline_sps_with_cropping() {
        let sps = Sps::parse(&hex(SPS_BASELINE_1080P)).unwrap();
        assert_eq!(sps.profile_idc, 66);
        assert_eq!(sps.constraint_flags, 0xC0);
        assert_eq!(sps.level_idc, 40);
        assert_eq!((sps.coded_width(), sps.coded_height()), (1920, 1088));
        assert_eq!(
            sps.frame_cropping,
            Some(FrameCropping {
                left: 0,
                right: 0,
                top: 0,
                bottom: 4
            })
        );
        assert_eq!((sps.width, sps.height), (1920, 1080));
        assert_eq!(sps.codec_string(), "avc1.42C028");

        let cif = Sps::parse(&hex(SPS_BASELINE_CIF)).unwrap();
        assert_eq!((cif.width, cif.height), (352, 288));
        assert_eq!(cif.codec_string(), "avc1.42C00D");
    }

    #[test]
    fn parses_high10_and_high444_sps() {
        let sps = Sps::parse(&hex(SPS_HIGH10_1080P)).unwrap();
        assert_eq!(sps.profile_idc, 110);
        assert_eq!((sps.bit_depth_luma, sps.bit_depth_chroma), (10, 10));
        assert_eq!((sps.width, sps.height), (1920, 1080));
        assert_eq!(sps.codec_string(), "avc1.6E0028");

        let sps = Sps::parse(&hex(SPS_HIGH444_720P)).unwrap();
        assert_eq!(sps.profile_idc, 244);
        assert_eq!(sps.chroma_format_idc, 3);
        assert!(!sps.separate_colour_plane);
        assert_eq!((sps.width, sps.height), (1280, 720));
        assert_eq!(sps.codec_string(), "avc1.F4001F");
    }

    #[test]
    fn rejects_bad_sps() {
        assert!(Sps::parse(&[]).is_err());
        assert!(Sps::parse(&hex(PPS_CE0FC8)).is_err());
        let truncated = hex(SPS_HIGH_640X360);
        assert!(Sps::parse(&truncated[..8]).is_err());
    }

    #[test]
    fn parses_pps_ids() {
        let pps = Pps::parse(&hex(PPS_HIGH_640X360)).unwrap();
        assert_eq!(pps.pic_parameter_set_id, 0);
        assert_eq!(pps.seq_parameter_set_id, 0);
    }

    /// The records must match what ffmpeg writes for the same streams.
    #[test]
    fn builds_avcc_like_ffmpeg() {
        let avcc =
            avc_decoder_configuration_record(&[hex(SPS_HIGH_640X360)], &[hex(PPS_HIGH_640X360)])
                .unwrap();
        assert_eq!(
            avcc,
            hex(
                "01640016ffe1001967640016acb405017fcb8088000003000800000300f078b175\
                 01000468ef3cb0fdf8f800"
            )
        );

        let avcc = avc_decoder_configuration_record(&[hex(SPS_BASELINE_1080P)], &[hex(PPS_CE0FC8)])
            .unwrap();
        assert_eq!(
            avcc,
            hex(
                "0142c028ffe100196742c028da01e0089f97011000000300100000030320f1832a\
                 01000468ce0fc8"
            )
        );

        let avcc =
            avc_decoder_configuration_record(&[hex(SPS_HIGH10_1080P)], &[hex(PPS_CE0FC8)]).unwrap();
        assert_eq!(
            avcc,
            hex(
                "016e0028ffe1001b676e0028a6cb403c0113f2e022000003000200000300641e306540\
                 01000468ce0fc8fdfafa00"
            )
        );

        let avcc =
            avc_decoder_configuration_record(&[hex(SPS_HIGH444_720P)], &[hex(PPS_HIGH444_720P)])
                .unwrap();
        assert_eq!(
            avcc,
            hex(
                "01f4001fffe1001967f4001f9196805005bb011000000300100000030320f1832a\
                 01000568ce0f1920fff8f800"
            )
        );
    }

    #[test]
    fn avcc_requires_parameter_sets() {
        let none: [Vec<u8>; 0] = [];
        assert!(avc_decoder_configuration_record(&none, &[hex(PPS_CE0FC8)]).is_err());
        assert!(avc_decoder_configuration_record(&[hex(SPS_BASELINE_CIF)], &none).is_err());
        // Swapped lists.
        assert!(
            avc_decoder_configuration_record(&[hex(PPS_CE0FC8)], &[hex(SPS_BASELINE_CIF)]).is_err()
        );
    }

    #[test]
    fn builds_video_config() {
        let config = video_config(&[hex(SPS_HIGH_640X360)], &[hex(PPS_HIGH_640X360)]).unwrap();
        assert_eq!(config.codec, VideoCodec::H264);
        assert_eq!(config.codec_string, "avc1.640016");
        assert_eq!((config.width, config.height), (640, 360));
        assert_eq!(config.description[0], 1);
    }

    #[test]
    fn classifies_nal_types() {
        assert_eq!(nal_unit_type(&[0x65]), Some(nal_type::IDR_SLICE));
        assert_eq!(nal_unit_type(&[0x41]), Some(nal_type::NON_IDR_SLICE));
        assert_eq!(nal_unit_type(&[]), None);
        assert!(is_vcl(1) && is_vcl(5));
        assert!(!is_vcl(6) && !is_vcl(0));
    }
}

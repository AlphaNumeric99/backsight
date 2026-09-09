//! G.711 decoding: A-law and µ-law code words to 16-bit linear PCM.
//!
//! Decoding goes through 256-entry lookup tables computed at compile time from the
//! expansion rules of ITU-T G.711 (Tables 1 and 2). Output uses the full 16-bit range:
//! A-law values are the 13-bit G.711 decoder outputs scaled by 8 (±32 256 at most),
//! µ-law values the 14-bit outputs scaled by 4 (±32 124 at most), the same convention as
//! the ITU-T G.191 reference implementation.

use super::{AudioCodec, MediaError};

/// A-law code word to linear sample, per ITU-T G.711 Table 1.
///
/// The even bits of the transmitted character are inverted (`^ 0x55`); the sign bit is 1
/// for positive values. Segment 0 decodes to `2q + 1`, segment `s > 0` to
/// `(2q + 33) << (s - 1)`, in 13-bit units.
const fn alaw_expand(code: u8) -> i16 {
    let a = code ^ 0x55;
    let segment = (a >> 4) & 0x07;
    let interval = (a & 0x0F) as i16;
    let magnitude = if segment == 0 {
        2 * interval + 1
    } else {
        (2 * interval + 33) << (segment - 1)
    };
    let sample = magnitude << 3;
    if a & 0x80 != 0 { sample } else { -sample }
}

/// µ-law code word to linear sample, per ITU-T G.711 Table 2.
///
/// All bits of the transmitted character are inverted; after inversion the sign bit is 1
/// for negative values. Segment `s` decodes to `((2q + 33) << s) - 33`, in 14-bit units.
const fn mulaw_expand(code: u8) -> i16 {
    let u = !code;
    let segment = (u >> 4) & 0x07;
    let interval = (u & 0x0F) as i16;
    let magnitude = ((2 * interval + 33) << segment) - 33;
    let sample = magnitude << 2;
    if u & 0x80 != 0 { -sample } else { sample }
}

const fn build_alaw_table() -> [i16; 256] {
    let mut table = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = alaw_expand(i as u8);
        i += 1;
    }
    table
}

const fn build_mulaw_table() -> [i16; 256] {
    let mut table = [0i16; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = mulaw_expand(i as u8);
        i += 1;
    }
    table
}

/// Linear value of every A-law code word, indexed by the code word.
pub static ALAW_TO_LINEAR: [i16; 256] = build_alaw_table();

/// Linear value of every µ-law code word, indexed by the code word.
pub static MULAW_TO_LINEAR: [i16; 256] = build_mulaw_table();

/// Decodes one A-law code word.
#[inline]
pub fn alaw_to_linear(code: u8) -> i16 {
    ALAW_TO_LINEAR[usize::from(code)]
}

/// Decodes one µ-law code word.
#[inline]
pub fn mulaw_to_linear(code: u8) -> i16 {
    MULAW_TO_LINEAR[usize::from(code)]
}

/// Decodes A-law bytes, appending one sample per byte to `out`.
pub fn decode_alaw(input: &[u8], out: &mut Vec<i16>) {
    out.extend(input.iter().map(|&code| alaw_to_linear(code)));
}

/// Decodes µ-law bytes, appending one sample per byte to `out`.
pub fn decode_mulaw(input: &[u8], out: &mut Vec<i16>) {
    out.extend(input.iter().map(|&code| mulaw_to_linear(code)));
}

/// Decodes a G.711 payload of the given codec, appending samples to `out`.
///
/// Returns [`MediaError::Unsupported`] for [`AudioCodec::Aac`], which needs a real decoder.
pub fn decode(codec: AudioCodec, input: &[u8], out: &mut Vec<i16>) -> Result<(), MediaError> {
    match codec {
        AudioCodec::PcmAlaw => decode_alaw(input, out),
        AudioCodec::PcmMulaw => decode_mulaw(input, out),
        AudioCodec::Aac => {
            return Err(MediaError::Unsupported(
                "codec for G.711 decoding: AAC".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A-law decoder output values of ITU-T G.711 Table 1 (13-bit units, positive half),
    /// listed per segment: first value and step.
    const ALAW_SEGMENTS: [(i16, i16); 8] = [
        (1, 2),
        (33, 2),
        (66, 4),
        (132, 8),
        (264, 16),
        (528, 32),
        (1056, 64),
        (2112, 128),
    ];

    /// µ-law decoder output values of ITU-T G.711 Table 2 (14-bit units, positive half):
    /// first value and step per segment.
    const MULAW_SEGMENTS: [(i16, i16); 8] = [
        (0, 2),
        (33, 4),
        (99, 8),
        (231, 16),
        (495, 32),
        (1023, 64),
        (2079, 128),
        (4191, 256),
    ];

    #[test]
    fn alaw_matches_itu_table() {
        for (segment, &(first, step)) in ALAW_SEGMENTS.iter().enumerate() {
            for interval in 0..16u8 {
                let expected = (first + step * i16::from(interval)) * 8;
                // Character before even-bit inversion: sign 1, segment, interval.
                let positive = (0x80 | ((segment as u8) << 4) | interval) ^ 0x55;
                let negative = positive ^ 0x80;
                assert_eq!(alaw_to_linear(positive), expected, "code {positive:#04x}");
                assert_eq!(alaw_to_linear(negative), -expected, "code {negative:#04x}");
            }
        }
    }

    #[test]
    fn mulaw_matches_itu_table() {
        for (segment, &(first, step)) in MULAW_SEGMENTS.iter().enumerate() {
            for interval in 0..16u8 {
                let expected = (first + step * i16::from(interval)) * 4;
                // Character before inversion: sign 0 (positive), segment, interval.
                let positive = !(((segment as u8) << 4) | interval);
                let negative = positive & 0x7F;
                assert_eq!(mulaw_to_linear(positive), expected, "code {positive:#04x}");
                assert_eq!(mulaw_to_linear(negative), -expected, "code {negative:#04x}");
            }
        }
    }

    #[test]
    fn well_known_code_words() {
        // A-law: 0xD5/0x55 are the smallest magnitudes, 0xAA/0x2A full scale.
        assert_eq!(alaw_to_linear(0xD5), 8);
        assert_eq!(alaw_to_linear(0x55), -8);
        assert_eq!(alaw_to_linear(0xAA), 32_256);
        assert_eq!(alaw_to_linear(0x2A), -32_256);
        // µ-law: 0xFF/0x7F are zero, 0x80/0x00 full scale.
        assert_eq!(mulaw_to_linear(0xFF), 0);
        assert_eq!(mulaw_to_linear(0x7F), 0);
        assert_eq!(mulaw_to_linear(0x80), 32_124);
        assert_eq!(mulaw_to_linear(0x00), -32_124);
        assert_eq!(mulaw_to_linear(0xFE), 8);
    }

    #[test]
    fn tables_are_odd_symmetric_and_monotonic() {
        let mut alaw: Vec<i16> = (0..=255u8).map(alaw_to_linear).collect();
        let mut mulaw: Vec<i16> = (0..=255u8).map(mulaw_to_linear).collect();
        for code in 0..=255u8 {
            assert_eq!(alaw_to_linear(code), -alaw_to_linear(code ^ 0x80));
            assert_eq!(mulaw_to_linear(code), -mulaw_to_linear(code ^ 0x80));
        }
        alaw.sort_unstable();
        alaw.dedup();
        assert_eq!(
            alaw.len(),
            256,
            "every A-law code word decodes to a distinct value"
        );
        mulaw.sort_unstable();
        mulaw.dedup();
        assert_eq!(mulaw.len(), 255, "only µ-law zero has two code words");
    }

    #[test]
    fn decodes_buffers() {
        let mut out = vec![1];
        decode_alaw(&[0xD5, 0xAA], &mut out);
        assert_eq!(out, vec![1, 8, 32_256]);
        out.clear();
        decode(AudioCodec::PcmMulaw, &[0x80, 0xFF], &mut out).unwrap();
        assert_eq!(out, vec![32_124, 0]);
        assert!(decode(AudioCodec::Aac, &[0], &mut out).is_err());
    }
}

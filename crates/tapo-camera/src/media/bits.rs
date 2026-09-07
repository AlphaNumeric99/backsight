//! MSB-first bit reader with Exp-Golomb support, shared by the H.264 and H.265 parsers.

use super::MediaError;

/// Reads bits most-significant first from an RBSP (emulation prevention already removed).
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Position in bits from the start of `data`.
    pos: usize,
    /// Name of the structure being parsed, used in error messages.
    what: &'static str,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8], what: &'static str) -> Self {
        Self { data, pos: 0, what }
    }

    fn truncated(&self) -> MediaError {
        MediaError::invalid(self.what, "unexpected end of data")
    }

    pub(crate) fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    pub(crate) fn read_bit(&mut self) -> Result<bool, MediaError> {
        let byte = *self
            .data
            .get(self.pos / 8)
            .ok_or_else(|| self.truncated())?;
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(bit == 1)
    }

    pub(crate) fn read_flag(&mut self) -> Result<bool, MediaError> {
        self.read_bit()
    }

    /// Reads up to 64 bits as an unsigned integer.
    pub(crate) fn read_bits_u64(&mut self, count: u32) -> Result<u64, MediaError> {
        debug_assert!(count <= 64);
        if count as usize > self.bits_left() {
            return Err(self.truncated());
        }
        let mut value = 0u64;
        for _ in 0..count {
            let byte = self.data[self.pos / 8];
            let bit = (byte >> (7 - self.pos % 8)) & 1;
            value = (value << 1) | u64::from(bit);
            self.pos += 1;
        }
        Ok(value)
    }

    /// Reads up to 32 bits as an unsigned integer.
    pub(crate) fn read_bits(&mut self, count: u32) -> Result<u32, MediaError> {
        debug_assert!(count <= 32);
        // Lossless: at most 32 bits were read.
        Ok(self.read_bits_u64(count)? as u32)
    }

    /// Reads up to 8 bits.
    pub(crate) fn read_u8(&mut self, count: u32) -> Result<u8, MediaError> {
        debug_assert!(count <= 8);
        Ok(self.read_bits_u64(count)? as u8)
    }

    pub(crate) fn skip_bits(&mut self, count: usize) -> Result<(), MediaError> {
        if count > self.bits_left() {
            return Err(self.truncated());
        }
        self.pos += count;
        Ok(())
    }

    /// Reads an unsigned Exp-Golomb code, `ue(v)`.
    pub(crate) fn read_ue(&mut self) -> Result<u32, MediaError> {
        let mut leading_zeros = 0u32;
        while !self.read_bit()? {
            leading_zeros += 1;
            if leading_zeros > 31 {
                return Err(MediaError::invalid(self.what, "Exp-Golomb code too long"));
            }
        }
        if leading_zeros == 0 {
            return Ok(0);
        }
        let suffix = self.read_bits_u64(leading_zeros)?;
        // At most 2^32 - 2, which fits in a u32.
        Ok(((1u64 << leading_zeros) - 1 + suffix) as u32)
    }

    /// Reads a signed Exp-Golomb code, `se(v)`.
    pub(crate) fn read_se(&mut self) -> Result<i32, MediaError> {
        let code = i64::from(self.read_ue()?);
        let value = if code % 2 == 1 {
            (code + 1) / 2
        } else {
            -(code / 2)
        };
        // |value| <= 2^31 - 1.
        Ok(value as i32)
    }

    /// Reads a `ue(v)` and checks it against an inclusive upper bound.
    pub(crate) fn read_ue_max(
        &mut self,
        max: u32,
        reason: &'static str,
    ) -> Result<u32, MediaError> {
        let value = self.read_ue()?;
        if value > max {
            return Err(MediaError::invalid(self.what, reason));
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bits_msb_first() {
        let mut r = BitReader::new(&[0b1010_0000, 0xFF], "test");
        assert!(r.read_bit().unwrap());
        assert!(!r.read_bit().unwrap());
        assert_eq!(r.read_bits(2).unwrap(), 0b10);
        assert_eq!(r.read_bits(8).unwrap(), 0b0000_1111);
        assert_eq!(r.bits_left(), 4);
        assert_eq!(r.read_bits(4).unwrap(), 0xF);
        assert!(r.read_bit().is_err());
    }

    #[test]
    fn reads_exp_golomb() {
        // ue: 1 -> 0, 010 -> 1, 011 -> 2, 00100 -> 3, 00111 -> 6
        // bits: 1 010 011 00100 00111 -> 1010 0110 0100 0011 1(000)
        let mut r = BitReader::new(&[0b1010_0110, 0b0100_0011, 0b1000_0000], "test");
        assert_eq!(r.read_ue().unwrap(), 0);
        assert_eq!(r.read_ue().unwrap(), 1);
        assert_eq!(r.read_ue().unwrap(), 2);
        assert_eq!(r.read_ue().unwrap(), 3);
        assert_eq!(r.read_ue().unwrap(), 6);
    }

    #[test]
    fn reads_signed_exp_golomb() {
        // se mapping: code 1 -> +1, 2 -> -1, 3 -> +2, 4 -> -2
        // codes 1 (010), 2 (011), 3 (00100), 4 (00101)
        let mut r = BitReader::new(&[0b0100_1100, 0b1000_0101], "test");
        assert_eq!(r.read_se().unwrap(), 1);
        assert_eq!(r.read_se().unwrap(), -1);
        assert_eq!(r.read_se().unwrap(), 2);
        assert_eq!(r.read_se().unwrap(), -2);
    }

    #[test]
    fn reads_max_exp_golomb_value() {
        // 31 leading zeros, a one, then 31 ones: 2^32 - 2.
        let mut data = vec![0u8; 3];
        data.push(0b0000_0001); // 31 zeros then the marker 1
        data.extend([0xFF; 4]);
        let mut r = BitReader::new(&data, "test");
        assert_eq!(r.read_ue().unwrap(), u32::MAX - 1);
    }

    #[test]
    fn rejects_overlong_exp_golomb() {
        let mut r = BitReader::new(&[0, 0, 0, 0, 0xFF], "test");
        assert!(matches!(
            r.read_ue(),
            Err(MediaError::InvalidData {
                reason: "Exp-Golomb code too long",
                ..
            })
        ));
    }

    #[test]
    fn truncated_read_is_an_error() {
        let mut r = BitReader::new(&[0x00], "test");
        assert!(r.read_ue().is_err());
        let mut r = BitReader::new(&[0xAB], "test");
        assert!(r.skip_bits(9).is_err());
        assert!(r.read_bits_u64(9).is_err());
    }
}

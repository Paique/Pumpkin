//! Wire helpers for Axiom plugin-messaging payloads.
//!
//! The protocol uses Minecraft's `FriendlyByteBuf` conventions: `VarInt`s,
//! big-endian fixed-width integers, `writeUtf` strings (`VarInt` byte length +
//! UTF-8) and packed `BlockPos` longs. Identifiers inside payloads are plain
//! `writeUtf` strings on this protocol version (26.x), matching how the
//! fork's `CCustomPayload`/cookie packets encode `namespace:path` keys.

use std::fmt;

/// A malformed payload (never panics; callers turn this into a goodbye/kick).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The payload ended before a field could be read.
    UnexpectedEof,
    /// A `VarInt` used more than 5 bytes.
    VarIntTooLong,
    /// A string length prefix exceeded the sane cap.
    StringTooLong(usize),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "payload ended unexpectedly"),
            Self::VarIntTooLong => write!(f, "varint exceeded 5 bytes"),
            Self::StringTooLong(len) => write!(f, "string length {len} is too long"),
        }
    }
}

impl std::error::Error for WireError {}

/// Maximum `writeUtf` byte length we accept/produce, mirroring the vanilla
/// 32767 limit used by `FriendlyByteBuf.writeUtf`.
const MAX_STRING_BYTES: usize = 32_767;

/// Payload encoder.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// Creates an empty writer.
    #[must_use]
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Consumes the writer, returning the encoded payload.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Appends a single byte.
    pub fn write_byte(&mut self, byte: u8) -> &mut Self {
        self.buf.push(byte);
        self
    }

    /// Appends a boolean as `0x01`/`0x00`.
    pub fn write_bool(&mut self, value: bool) -> &mut Self {
        self.write_byte(u8::from(value));
        self
    }

    /// Appends a signed big-endian 32-bit integer.
    pub fn write_i32_be(&mut self, value: i32) -> &mut Self {
        self.buf.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Appends a signed big-endian 64-bit integer.
    pub fn write_i64_be(&mut self, value: i64) -> &mut Self {
        self.buf.extend_from_slice(&value.to_be_bytes());
        self
    }

    /// Appends a `VarInt`.
    pub fn write_var_int(&mut self, value: i32) -> &mut Self {
        let mut val = value as u32;
        while val > 0x7F {
            self.buf.push((val as u8) | 0x80);
            val >>= 7;
        }
        self.buf.push(val as u8);
        self
    }

    /// Appends a `writeUtf` string (`VarInt` byte length + UTF-8).
    ///
    /// Panics if the string is not valid UTF-8-slicable above the cap; call
    /// sites only pass static/validated strings.
    pub fn write_string(&mut self, value: &str) -> &mut Self {
        self.write_var_int(value.len() as i32);
        self.buf.extend_from_slice(value.as_bytes());
        self
    }
}

/// Payload decoder over a byte slice.
#[derive(Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Creates a reader over the full payload.
    #[must_use]
    pub const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], WireError> {
        if self.pos + len > self.buf.len() {
            return Err(WireError::UnexpectedEof);
        }
        let slice = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(slice)
    }

    /// Reads one byte.
    pub fn read_byte(&mut self) -> Result<u8, WireError> {
        Ok(self.take(1)?[0])
    }

    /// Reads a boolean byte.
    pub fn read_bool(&mut self) -> Result<bool, WireError> {
        Ok(self.read_byte()? != 0)
    }

    /// Reads a signed big-endian 32-bit integer.
    pub fn read_i32_be(&mut self) -> Result<i32, WireError> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| WireError::UnexpectedEof)?;
        Ok(i32::from_be_bytes(bytes))
    }

    /// Reads a signed big-endian 64-bit integer.
    pub fn read_i64_be(&mut self) -> Result<i64, WireError> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| WireError::UnexpectedEof)?;
        Ok(i64::from_be_bytes(bytes))
    }

    /// Reads a `VarInt`.
    pub fn read_var_int(&mut self) -> Result<i32, WireError> {
        let mut val: i32 = 0;
        for i in 0..5 {
            let byte = self.read_byte()?;
            val |= i32::from(byte & 0x7F) << (i * 7);
            if byte & 0x80 == 0 {
                return Ok(val);
            }
        }
        Err(WireError::VarIntTooLong)
    }

    /// Reads a `writeUtf` string.
    pub fn read_string(&mut self) -> Result<String, WireError> {
        let len = self.read_var_int()?;
        if len < 0 || usize::try_from(len).unwrap_or(usize::MAX) > MAX_STRING_BYTES {
            return Err(WireError::StringTooLong(len.unsigned_abs() as usize));
        }
        let bytes = self.take(len as usize)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| WireError::UnexpectedEof)
    }
}

/// Packs block coordinates into a single `BlockPos` long
/// (bits 38..64 = X, 12..38 = Z, 0..12 = Y, two's complement).
#[must_use]
pub fn pack_block_pos(x: i32, y: i32, z: i32) -> i64 {
    ((i64::from(x) & 0x3FF_FFFF) << 38)
        | ((i64::from(z) & 0x3FF_FFFF) << 12)
        | (i64::from(y) & 0xFFF)
}

/// Inclusive block-coordinate bounds announced in `axiom:restrictions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Minimum corner.
    pub min: (i32, i32, i32),
    /// Maximum corner.
    pub max: (i32, i32, i32),
}

impl Bounds {
    /// Clamps coordinates to the protocol's `BlockPos` limits
    /// (X/Z ±33 554 431, Y ±2047) exactly like upstream `Restrictions.java`.
    #[must_use]
    pub fn clamped(min: (i32, i32, i32), max: (i32, i32, i32)) -> Self {
        fn clamp_axis(value: i32, limit: i32) -> i32 {
            value.clamp(-limit, limit)
        }
        Self {
            min: (
                clamp_axis(min.0, 33_554_431),
                clamp_axis(min.1, 2047),
                clamp_axis(min.2, 33_554_431),
            ),
            max: (
                clamp_axis(max.0, 33_554_431),
                clamp_axis(max.1, 2047),
                clamp_axis(max.2, 33_554_431),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn var_int_roundtrip() {
        for value in [0, 1, 127, 128, 255, 25565, 31000, 2_097_152, i32::MAX] {
            let mut writer = Writer::new();
            writer.write_var_int(value);
            let bytes = writer.into_bytes();
            let mut reader = Reader::new(&bytes);
            assert_eq!(reader.read_var_int().unwrap(), value);
        }
    }

    #[test]
    fn string_roundtrip() {
        let mut writer = Writer::new();
        writer.write_string("axiom:hello");
        let bytes = writer.into_bytes();
        assert_eq!(&bytes[..2], &[11, b'a']);
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.read_string().unwrap(), "axiom:hello");
    }

    #[test]
    fn truncated_payload_is_eof() {
        let mut reader = Reader::new(&[0x05, b'a']);
        assert_eq!(reader.read_string(), Err(WireError::UnexpectedEof));
    }

    #[test]
    fn oversized_varint_is_rejected() {
        let mut reader = Reader::new(&[0xFF; 6]);
        assert_eq!(reader.read_var_int(), Err(WireError::VarIntTooLong));
    }

    #[test]
    fn block_pos_packing_matches_vanilla_layout() {
        // Upstream AxiomConstants asserts the packed minimum position long:
        // MIN = (-33554432, -2048, -33554432) packs to a known constant; the
        // clamped minimum (-33554431, -2047, -33554431) must keep sign bits.
        let packed = pack_block_pos(-33_554_431, -2047, -33_554_431);
        let x = (packed >> 38) as i32;
        let z = (packed << 26) >> 38 as i32;
        let y = (packed << 52) >> 52 as i32;
        assert_eq!((x, y, z), (-33_554_431, -2047, -33_554_431));
        assert_eq!(pack_block_pos(0, 0, 0), 0);
    }
}

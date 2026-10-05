//! Little-endian byte cursor with Unreal Engine 2 primitive types.

use std::fmt;

/// A read error, with the byte offset where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadError {
    pub offset: usize,
    pub kind: ReadErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadErrorKind {
    /// Tried to read past the end of the data.
    UnexpectedEof { wanted: usize, available: usize },
    /// A compact index used more than 5 bytes.
    BadCompactIndex,
    /// A string length that cannot be right (negative zero, or longer than the data).
    BadStringLength(i32),
    /// Anything else, described in words.
    Invalid(String),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: ", self.offset)?;
        match &self.kind {
            ReadErrorKind::UnexpectedEof { wanted, available } => {
                write!(f, "unexpected end of data (wanted {wanted} bytes, {available} left)")
            }
            ReadErrorKind::BadCompactIndex => write!(f, "compact index longer than 5 bytes"),
            ReadErrorKind::BadStringLength(n) => write!(f, "bad string length {n}"),
            ReadErrorKind::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ReadError {}

pub type Result<T> = std::result::Result<T, ReadError>;

/// Reads primitives from a byte slice, tracking the position.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Moves to an absolute offset. Fails if it is past the end.
    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(self.error_at(
                pos,
                ReadErrorKind::Invalid(format!("seek past end (file is {} bytes)", self.data.len())),
            ));
        }
        self.pos = pos;
        Ok(())
    }

    pub fn error(&self, kind: ReadErrorKind) -> ReadError {
        self.error_at(self.pos, kind)
    }

    fn error_at(&self, offset: usize, kind: ReadErrorKind) -> ReadError {
        ReadError { offset, kind }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(self.error(ReadErrorKind::UnexpectedEof {
                wanted: n,
                available: self.remaining(),
            }));
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("slice length checked"))
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    /// Unreal's variable-length signed integer ("compact index").
    ///
    /// First byte: bit 7 = sign, bit 6 = more bytes follow, bits 0-5 = value.
    /// Later bytes: bit 7 = more bytes follow, bits 0-6 = value. At most 5 bytes.
    pub fn compact_index(&mut self) -> Result<i32> {
        let start = self.pos;
        let b0 = self.u8()?;
        let negative = b0 & 0x80 != 0;
        let mut value = (b0 & 0x3f) as u32;
        let mut more = b0 & 0x40 != 0;
        let mut shift = 6;
        let mut count = 1;
        while more {
            if count == 5 {
                return Err(self.error_at(start, ReadErrorKind::BadCompactIndex));
            }
            let b = self.u8()?;
            value |= ((b & 0x7f) as u32) << shift;
            more = b & 0x80 != 0;
            shift += 7;
            count += 1;
        }
        let value = value as i32;
        Ok(if negative { value.wrapping_neg() } else { value })
    }

    /// An Unreal string: compact-index length (including the terminating zero),
    /// then the characters. A negative length means UTF-16 characters.
    pub fn fstring(&mut self) -> Result<String> {
        let start = self.pos;
        let len = self.compact_index()?;
        let bad_len = |r: &Self| r.error_at(start, ReadErrorKind::BadStringLength(len));
        if len == 0 {
            return Ok(String::new());
        }
        if len > 0 {
            let len = len as usize;
            if len > self.remaining() {
                return Err(bad_len(self));
            }
            let raw = self.bytes(len)?;
            let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
            // Latin-1: every byte maps directly to the same Unicode code point.
            Ok(raw.iter().map(|&b| b as char).collect())
        } else {
            let chars = len.unsigned_abs() as usize;
            if chars.saturating_mul(2) > self.remaining() {
                return Err(bad_len(self));
            }
            let raw = self.bytes(chars * 2)?;
            let units: Vec<u16> = raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .collect();
            let units = units.strip_suffix(&[0]).unwrap_or(&units);
            Ok(String::from_utf16_lossy(units))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encodes like Unreal does, so tests can round-trip.
    fn encode_compact(v: i32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut a = v.unsigned_abs();
        let mut b0 = (a & 0x3f) as u8;
        if v < 0 {
            b0 |= 0x80;
        }
        a >>= 6;
        if a > 0 {
            b0 |= 0x40;
        }
        out.push(b0);
        while a > 0 {
            let mut b = (a & 0x7f) as u8;
            a >>= 7;
            if a > 0 {
                b |= 0x80;
            }
            out.push(b);
        }
        out
    }

    #[test]
    fn compact_index_known_bytes() {
        assert_eq!(Reader::new(&[0x05]).compact_index().unwrap(), 5);
        assert_eq!(Reader::new(&[0x85]).compact_index().unwrap(), -5);
        // 64 = 0b1_000000: low 6 bits 0 with continue flag, then 1.
        assert_eq!(Reader::new(&[0x40, 0x01]).compact_index().unwrap(), 64);
    }

    #[test]
    fn compact_index_round_trip() {
        for v in [0, 1, 63, 64, 127, 8191, 8192, 1 << 20, i32::MAX, -1, -64, -(1 << 27)] {
            let bytes = encode_compact(v);
            let mut r = Reader::new(&bytes);
            assert_eq!(r.compact_index().unwrap(), v, "value {v}");
            assert_eq!(r.remaining(), 0, "value {v} left bytes unread");
        }
    }

    #[test]
    fn compact_index_too_long_is_error() {
        let err = Reader::new(&[0x40, 0x80, 0x80, 0x80, 0x80, 0x01])
            .compact_index()
            .unwrap_err();
        assert_eq!(err.kind, ReadErrorKind::BadCompactIndex);
    }

    #[test]
    fn fstring_ansi_and_utf16() {
        assert_eq!(Reader::new(b"\x05None\x00").fstring().unwrap(), "None");
        // length -3: two UTF-16 characters plus terminator
        let bytes = [0x83, b'h', 0, b'i', 0, 0, 0];
        assert_eq!(Reader::new(&bytes).fstring().unwrap(), "hi");
    }

    #[test]
    fn eof_is_error_not_panic() {
        let mut r = Reader::new(&[1, 2]);
        assert!(matches!(
            r.u32().unwrap_err().kind,
            ReadErrorKind::UnexpectedEof { wanted: 4, available: 2 }
        ));
    }
}

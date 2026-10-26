//! A minimal cursor over a packet body.
//!
//! Shared by the `u8` sub-command families: they dispatch on a leading byte and
//! then read a per-arm body, which is easier to express as a cursor than as a
//! derive.

use sro_macro::SerializationError;

pub(crate) fn short(what: &'static str) -> SerializationError {
    SerializationError::IoError(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, what))
}

pub(crate) struct Cursor<'a> {
    pub(crate) buf: &'a [u8],
    pub(crate) pos: usize,
    pub(crate) what: &'static str,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(buf: &'a [u8], what: &'static str) -> Self {
        Cursor { buf, pos: 0, what }
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], SerializationError> {
        let end = self.pos.checked_add(n).ok_or_else(|| short(self.what))?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| short(self.what))?;
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, SerializationError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, SerializationError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, SerializationError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// A length-prefixed string: `u16` byte count, then the bytes.
    pub(crate) fn string(&mut self) -> Result<String, SerializationError> {
        let len = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    /// Whether the body was consumed exactly.
    pub(crate) fn at_end(&self) -> bool {
        self.pos == self.buf.len()
    }
}

/// Writes a length-prefixed string.
pub(crate) fn put_string(buf: &mut bytes::BytesMut, s: &str) {
    use bytes::BufMut;
    buf.put_u16_le(s.len() as u16);
    buf.extend_from_slice(s.as_bytes());
}

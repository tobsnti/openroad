//! The one primitive the archive reads through, behind a trait.
//!
//! IDEA. Every read of a PK2 is "`buf.len()` bytes at `offset`": the directory
//! is a chain of fixed-size blocks addressed by absolute offset, and a file
//! body is a length at an offset too. So a source needs no cursor, no `Seek`
//! and no stream — one positioned read is its whole interface. That shape is
//! also exactly an HTTP `Range:` header and a slice of memory, which is what
//! lets the file-backed and the in-memory path share the block walk and the
//! directory expansion instead of each carrying its own copy.

use std::fs::File;
use std::io::{Error, ErrorKind, Result};

use bytes::Bytes;

/// Where an [`crate::pk2::archive::Archive`] gets its bytes from.
pub trait Pk2Source: Send + Sync {
    /// Fills `buf` from `offset`, or fails. A short read is an error and never
    /// a partially-filled buffer: every caller parses fixed-size records out of
    /// it, so leftover bytes would be parsed as data.
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()>;
}

/// Reads without touching a shared cursor, so concurrent readers of the same
/// [`File`] need no synchronization.
#[cfg(unix)]
impl Pk2Source for File {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        use std::os::unix::fs::FileExt;
        FileExt::read_exact_at(self, buf, offset)
    }
}

/// Windows has no true positional read; `seek_read` moves the file cursor,
/// which is fine here because nothing reads through the cursor.
#[cfg(windows)]
impl Pk2Source for File {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        use std::os::windows::fs::FileExt;
        let mut pos = 0;
        while pos < buf.len() {
            let read = self.seek_read(&mut buf[pos..], offset + pos as u64)?;
            if read == 0 {
                return Err(Error::new(
                    ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            pos += read;
        }
        Ok(())
    }
}

/// Neither `FileExt` exists outside unix and windows, so on any other target a
/// `File` cannot serve a positioned read at all — `wasm32-unknown-unknown` is
/// the case in hand. The arm reports it rather than faking a seek, and it is no
/// longer a dead end: such a target supplies a different source.
#[cfg(not(any(unix, windows)))]
impl Pk2Source for File {
    fn read_exact_at(&self, _buf: &mut [u8], _offset: u64) -> Result<()> {
        Err(Error::new(
            ErrorKind::Unsupported,
            "PK2 positional read needs a platform with FileExt; supply an in-memory source instead",
        ))
    }
}

/// An archive that is already in memory, served from the slice itself. This is
/// the half of the abstraction that was in the tree before (as `Cursor<Bytes>`
/// "over an in-memory archive"), and it is what makes the PK2 tests runnable
/// without a file on disk.
impl Pk2Source for Bytes {
    fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> Result<()> {
        let start = usize::try_from(offset).map_err(|_| {
            Error::new(ErrorKind::InvalidInput, "offset outside this address space")
        })?;
        let end = start
            .checked_add(buf.len())
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "read length overflows"))?;
        if end > self.len() {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ));
        }
        buf.copy_from_slice(&self[start..end]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_in_memory_source_reads_at_an_offset() {
        let bytes = Bytes::from_static(b"0123456789");
        let mut buf = [0u8; 3];
        bytes.read_exact_at(&mut buf, 4).expect("in range");
        assert_eq!(&buf, b"456");
    }

    /// A read running past the end must fail, not hand back a half-filled
    /// buffer: the callers parse fixed-size records out of it.
    #[test]
    fn a_read_past_the_end_is_an_error() {
        let bytes = Bytes::from_static(b"0123");
        let mut buf = [0u8; 3];
        let err = bytes.read_exact_at(&mut buf, 2).expect_err("out of range");
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(&buf, &[0u8; 3], "nothing was written");
    }
}

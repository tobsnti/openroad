use std::io;

#[derive(std::fmt::Debug)]
pub enum Error {
    InvalidHeader(&'static str),
    IO(io::Error),
    InvalidBlock(&'static str),
    /// The block chain revisited an offset, or ran past the block cap. A
    /// hostile or corrupt archive can point a block's `next_chain` back at
    /// itself; without this the reader recurses until the stack dies.
    ChainLoop(u64),
    /// The directory tree contains more directories than
    /// [`crate::pk2::constants::MAX_DIRECTORIES`]. A full tree is not a cycle:
    /// this is the backstop against an archive that keeps handing out fresh,
    /// valid directory blocks forever, and it carries the cap that was hit.
    TooManyDirectories(usize),
}

/// One readable line per failure. `Debug` was what callers printed, which put
/// `IO(Os { code: 2, ... })` in front of a user whose only real problem is that
/// a file is not where the client looked.
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::InvalidHeader(what) => write!(f, "not a readable PK2 archive: {what}"),
            Error::IO(err) if err.kind() == io::ErrorKind::NotFound => write!(f, "file not found"),
            Error::IO(err) => write!(f, "cannot be read: {err}"),
            Error::InvalidBlock(what) => write!(f, "malformed archive block: {what}"),
            Error::ChainLoop(offset) => {
                write!(f, "block chain loops or never ends at offset {offset}")
            }
            Error::TooManyDirectories(cap) => {
                write!(f, "archive holds more than {cap} directories")
            }
        }
    }
}

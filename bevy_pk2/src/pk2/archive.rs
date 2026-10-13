use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::io::{AssetReader, AssetReaderError, PathStream, Reader, VecReader};
use bevy::prelude::{debug, error, info};
use bevy::reflect::TypePath;

use crate::pk2::blowfish::Blowfish;
use crate::pk2::constants::HEADER_SIZE;
use crate::pk2::directory::Directory;
use crate::pk2::entry::Entry;
use crate::pk2::errors::Error;
use crate::pk2::header::Header;
use crate::pk2::key::Pk2Key;
use crate::pk2::read_stats;
use crate::pk2::source::Pk2Source;

/// A structure to access an SRO PK2 archive.
#[derive(TypePath, Clone)]
#[allow(dead_code)]
pub struct Archive {
    /// Shared without a lock: every read is positioned
    /// ([`Pk2Source::read_exact_at`]), so concurrent asset loads from one
    /// archive don't serialize on a mutex around a shared cursor. The source is
    /// type-erased so that a file, a buffer in memory or (later) a byte range
    /// over HTTP all reach the same index and the same reader.
    source: Arc<dyn Pk2Source>,
    blowfish: Blowfish,
    strip_prefix: OsString,
    pub root: Directory,
    entries: HashMap<PathBuf, Entry>,
}

/// PK2 archive lookups are case-insensitive — the original Windows client
/// relies on it: textdata routinely references files in a different case than
/// the archive stores them (e.g. `char\china\chinaman_adventurer.bsr` for the
/// stored `Res/Char/China/chinaman_Adventurer.bsr`). Lowercasing both the
/// index keys and every query lets a mixed-case archive (the official-casing
/// PK2s) resolve the client's lowercase asset paths, while an already
/// all-lowercase archive (the normalized vSRO assets) is unaffected.
fn normalize(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_ascii_lowercase())
}

impl Archive {
    /// Open and index an archive, surfacing every failure as an [`Error`]
    /// rather than a panic. The archive is our trust boundary — SRO drops are
    /// treated as potentially hostile — so a malformed header, a short read or
    /// a cyclic block chain must be reportable, not fatal.
    pub fn open<P: AsRef<Path>>(path: P, key: &Pk2Key) -> Result<Archive, Error> {
        let file = File::open(path.as_ref()).map_err(Error::IO)?;
        Archive::from_source(Arc::new(file), key)
    }

    /// Index an archive from any [`Pk2Source`]: the header, the index blocks
    /// and the file bodies are all read by offset, so nothing here needs a
    /// file. That is what makes an archive in memory — a test fixture, or a
    /// byte range fetched over the network — the same thing as one on disk.
    pub fn from_source(source: Arc<dyn Pk2Source>, key: &Pk2Key) -> Result<Archive, Error> {
        let blowfish = Blowfish::from_key(key)
            .map_err(|_| Error::InvalidHeader("PK2 key rejected by the cipher (bad length?)"))?;
        let mut header_buf: [u8; HEADER_SIZE] = [0; HEADER_SIZE];
        source
            .read_exact_at(&mut header_buf, 0)
            .map_err(|_| Error::InvalidHeader("header shorter than 256 bytes"))?;
        let header = Header::from(header_buf);
        header.verify(&blowfish)?;
        let root = Directory::index(&*source, &blowfish)?;
        let entries = root
            .get_all_entries()
            .into_iter()
            .map(|(k, v)| (normalize(&k), v))
            .collect();
        Ok(Archive {
            source,
            blowfish,
            strip_prefix: "".into(),
            root,
            entries,
        })
    }
}

impl Archive {
    /// Convenience wrapper over [`Archive::open`] for the startup paths, where
    /// a missing or corrupt PK2 is genuinely fatal (the client cannot run
    /// without its assets). Anything that wants to handle the failure should
    /// call [`Archive::open`] instead.
    ///
    /// This replaces the old `From<P>` conversion, which could not carry the
    /// key now that one is required.
    pub fn open_or_panic<P: AsRef<Path>>(path: P, key: &Pk2Key) -> Self {
        let path = path.as_ref();
        Archive::open(path, key).unwrap_or_else(|e| {
            error!("failed to open archive {}: {:?}", path.display(), e);
            panic!("failed to open archive {}: {:?}", path.display(), e)
        })
    }

    /// Open one archive with the key resolved from the user's own environment
    /// or `config.yaml` ([`Pk2Key::resolve`]).
    ///
    /// For the one-off callers — tools and tests — that open a single archive
    /// and have no reason to thread a key around. A startup path opening
    /// several archives should resolve the key once and use
    /// [`Archive::open_or_panic`] instead, so a misconfigured key is reported
    /// once rather than five times.
    pub fn configured<P: AsRef<Path>>(path: P) -> Self {
        let key = Pk2Key::resolve().unwrap_or_else(|err| panic!("{err}"));
        Archive::open_or_panic(path, &key)
    }
}

impl AssetReader for Archive {
    // fn read<'a>(&'a self, path: &'a Path) -> BoxedFuture<'a, Result<Box<Reader<'a>>, AssetReaderError>> {
    //     Box::pin(async move {
    //         let stripped_path = path.strip_prefix(&self.strip_prefix).unwrap().to_path_buf();
    //
    //         return match self.entries.get(&stripped_path) {
    //             None => Err(AssetReaderError::NotFound(stripped_path)),
    //             Some(entry) => {
    //                 let file = Arc::clone(&self.file);
    //                 let mut file = file.lock().expect("failed to lock cursor");
    //                 let mut buf = BytesMut::zeroed(entry.size as usize);
    //                 file.seek(SeekFrom::Start(entry.position))?;
    //                 let read_bytes = file.read(&mut buf)?;
    //                 if read_bytes != entry.size as usize {
    //                     warn!("expected {} bytes but read {}", entry.size, read_bytes);
    //                 }
    //
    //                 let buf = buf.freeze();
    //                 Ok(Box::new(buf.reader()))
    //             }
    //         }
    //     })
    // }

    async fn read<'a>(&'a self, path: &'a Path) -> Result<Box<dyn Reader>, AssetReaderError> {
        let stripped_path = normalize(path);
        debug!("reading path: {}", stripped_path.display());
        match self.entries.get(&stripped_path) {
            None => Err(AssetReaderError::NotFound(stripped_path)),
            Some(entry) => {
                // Positioned read straight into the buffer handed to the
                // reader: no lock (a burst of loads from one archive used to
                // serialize on it) and no intermediate copy of the bytes.
                let mut bytes = vec![0u8; entry.size as usize];
                self.source.read_exact_at(&mut bytes, entry.position)?;
                // #434 step 1: count what a cache would have served. Inert
                // unless PK2_READ_STATS is set.
                read_stats::record(&stripped_path, entry.size as u64);
                let reader: Box<dyn Reader> = Box::new(VecReader::new(bytes));
                Ok(reader)
            }
        }
    }

    // fn get_metadata(&self, path: &Path) -> Result<Metadata, AssetReaderError> {
    //     let stripped_path = path.strip_prefix(&self.strip_prefix).unwrap().to_path_buf();
    //
    //     match self.entries.get(&stripped_path) {
    //         None => Err(AssetReaderError::NotFound(stripped_path)),
    //         Some(entry) => Ok(Metadata::from(entry))
    //     }
    // }
    //
    // fn watch_path_for_changes(&self, to_watch: &Path, to_reload: Option<PathBuf>) -> Result<(), AssetReaderError> {
    //     debug!("watch_path_for_changes not implemented");
    //     Ok(())
    // }
    //
    // fn watch_for_changes(&self, configuration: &ChangeWatcher) -> Result<(), AssetReaderError> {
    //     debug!("watch_for_changes not implemented");
    //     Ok(())
    // }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<Box<dyn Reader>, AssetReaderError> {
        debug!("reading meta path: {}", path.display());
        Err(AssetReaderError::NotFound(path.to_path_buf()))
        // let stripped_path = path.strip_prefix(&self.strip_prefix).unwrap().to_path_buf();
        //
        //     match self.entries.get(&stripped_path) {
        //         None => Err(AssetReaderError::NotFound(stripped_path)),
        //         Some(entry) => Ok(Metadata::from(entry))
        //     }
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let stripped_path = normalize(path);
        debug!("reading dir path: {}", stripped_path.display());

        let entry = match self.entries.get(&stripped_path) {
            None => return Err(AssetReaderError::NotFound(path.to_path_buf())),
            Some(entry) => entry,
        };

        if !entry.is_dir() {
            return Err(AssetReaderError::Io(Arc::new(std::io::Error::new(
                ErrorKind::NotADirectory,
                format!("{}", path.display()),
            ))));
        }

        // `root.directories` is keyed by the archive's original casing, so a
        // normalized (lowercase) path can miss it on a mixed-case archive;
        // fail gracefully rather than panic (directory listing isn't on the
        // load-by-path hot path).
        let dir = match self.root.directories.get(&stripped_path) {
            Some(dir) => dir,
            None => return Err(AssetReaderError::NotFound(path.to_path_buf())),
        };

        let entries: Vec<PathBuf> = dir
            .entries
            .keys()
            .map(|k| {
                let mut pb = path.to_path_buf();
                pb.push(k);
                pb.to_owned()
            })
            .collect();
        let read_dir: Box<PathStream> = Box::new(futures::stream::iter(entries));
        Ok(read_dir)
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        let stripped_path = normalize(path);
        debug!("is dir path: {}", stripped_path.display());

        let entry = match self.entries.get(&stripped_path) {
            None => return Err(AssetReaderError::NotFound(path.to_path_buf())),
            Some(entry) => entry,
        };

        Ok(entry.is_dir())
    }
}

impl Archive {
    /// Synchronously reads a contained file's bytes, for callers outside the
    /// asset system (e.g. test-scene tooling reading textdata directly).
    pub fn read_file_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        let entry = self.entries.get(&normalize(path))?;
        if !entry.is_file() {
            return None;
        }
        let mut buf = vec![0u8; entry.size as usize];
        self.source.read_exact_at(&mut buf, entry.position).ok()?;
        read_stats::record(&normalize(path), entry.size as u64);
        Some(buf)
    }

    pub fn print_all_entries(&self) {
        self.entries.iter().for_each(|(k, _v)| {
            info!("Entry: {}", k.display());
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pk2::constants::{BLOCK_SIZE, CHECKSUM, ENTRY_SIZE, SIGNATURE, VERSION};
    use bytes::Bytes;

    /// A fixture key, not the real archive key: the fixture is encrypted and
    /// read back with the same cipher, so any valid key exercises the path.
    fn test_key() -> Pk2Key {
        Pk2Key::from_config("testkey", "00112233445566778899").expect("fixture key is valid")
    }

    /// The smallest thing that is still a real archive: a header that verifies,
    /// one index block carrying the root `.` entry and one file entry, and that
    /// file's bytes behind the block. Offsets are absolute, as in a real PK2.
    fn tiny_archive(blowfish: &Blowfish, name: &str, body: &[u8]) -> Bytes {
        let mut header = [0u8; HEADER_SIZE];
        header[..30].copy_from_slice(SIGNATURE);
        header[30..34].copy_from_slice(&VERSION.to_le_bytes());
        header[34] = 1; // encrypted -> the checksum is checked
        let mut checksum = *CHECKSUM;
        blowfish.encrypt(&mut checksum);
        header[35..51].copy_from_slice(&checksum);

        let mut block = [0u8; BLOCK_SIZE];
        // entry 0: the root directory, which every archive names "."
        block[0] = 1;
        block[1] = b'.';
        block[106..114].copy_from_slice(&(HEADER_SIZE as u64).to_le_bytes());
        // entry 1: the file, its bytes stored plain right after the block
        block[ENTRY_SIZE] = 2;
        block[ENTRY_SIZE + 1..ENTRY_SIZE + 1 + name.len()].copy_from_slice(name.as_bytes());
        let body_at = (HEADER_SIZE + BLOCK_SIZE) as u64;
        block[ENTRY_SIZE + 106..ENTRY_SIZE + 114].copy_from_slice(&body_at.to_le_bytes());
        block[ENTRY_SIZE + 114..ENTRY_SIZE + 118]
            .copy_from_slice(&(body.len() as u32).to_le_bytes());
        blowfish.encrypt(&mut block);

        let mut bytes = Vec::with_capacity(HEADER_SIZE + BLOCK_SIZE + body.len());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&block);
        bytes.extend_from_slice(body);
        Bytes::from(bytes)
    }

    /// The point of the abstraction in one test: an archive that never touches
    /// the filesystem is indexed and served through the same code as a file.
    #[test]
    fn an_archive_is_indexed_and_read_straight_out_of_memory() {
        let key = test_key();
        let blowfish = Blowfish::from_key(&key).expect("fixture key is valid");
        let source = tiny_archive(&blowfish, "a.txt", b"hello pk2");

        let archive = Archive::from_source(Arc::new(source), &key).expect("indexes from memory");

        assert_eq!(
            archive.read_file_bytes(Path::new("a.txt")).as_deref(),
            Some(&b"hello pk2"[..])
        );
        assert!(
            archive.read_file_bytes(Path::new("missing.txt")).is_none(),
            "an absent entry is None, not a panic"
        );
    }

    /// A truncated archive must fail the same way whatever the source is: the
    /// header read is positioned too, so a short buffer is reported, not
    /// zero-filled.
    #[test]
    fn a_source_shorter_than_the_header_is_rejected() {
        let key = test_key();
        let short = Bytes::from_static(b"JoyMax File Manager!");
        assert!(matches!(
            Archive::from_source(Arc::new(short), &key),
            Err(Error::InvalidHeader(_))
        ));
    }
}

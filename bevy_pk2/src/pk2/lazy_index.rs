//! Resolving one path without indexing the whole archive.
//!
//! IDEA. The eager index reads *every* directory block before the first byte of
//! a file can be read. Measured over HTTP against the user's own `Data.pk2`
//! (3 345 625 088 B): **5 024** ranged requests and 12 861 440 bytes of index,
//! then 1 request and 5 526 bytes for the file that was actually wanted — 99,96 %
//! of the traffic is index. But a lookup needs only the directories *on the
//! path*: the root chain to find `res`, `res`'s chain to find `cos`, `cos`'s
//! chain to find the file, and each chain can stop at the first hit. That is
//! **5 blocks instead of 5 022** for the same file: 8 requests, 18 583 bytes.
//!
//! Two properties follow from walking rather than indexing:
//!
//! * The cache is **resumable**, not complete: a directory keeps the entries it
//!   has seen plus the offset where its chain continues, so an early exit is
//!   not paid for twice and the next lookup in the same directory picks up
//!   where the first one stopped. A full listing is the same walk run out.
//! * The archive's root `.` entry is never needed — the chain simply starts at
//!   [`HEADER_SIZE`]. That detour is what makes the eager walk read the root
//!   chain twice (measured: 2 of 5 024 requests).
//!
//! This sits **beside** the eager index, not in its place: `get_all_entries()`
//! has eight callers outside this crate and they all want the whole tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::pk2::blowfish::Blowfish;
use crate::pk2::constants::{
    BLOCK_SIZE, ENTRIES_PER_BLOCK, ENTRY_SIZE, HEADER_SIZE, MAX_CHAIN_BLOCKS,
};
use crate::pk2::entry::{Entry, EntryType};
use crate::pk2::errors::Error;
use crate::pk2::source::Pk2Source;

/// One directory's block chain, as far as it has been walked.
struct DirChain {
    entries: Vec<Entry>,
    /// Where the chain continues, or `None` once it has been walked out.
    resume: Option<u64>,
    /// Offsets already read. The archive is untrusted input, so a `next_chain`
    /// pointing back into the chain has to be an error rather than a loop.
    seen: HashSet<u64>,
}

impl DirChain {
    fn starting_at(offset: u64) -> Self {
        DirChain {
            entries: Vec::new(),
            resume: Some(offset),
            seen: HashSet::new(),
        }
    }

    fn find(&self, name: &str) -> Option<Entry> {
        self.entries
            .iter()
            .find(|e| e.path_buf().to_string_lossy().to_ascii_lowercase() == name)
            .copied()
    }
}

/// The directories that have been walked, keyed by their lowercased path.
///
/// One lock around the whole walk, deliberately: a lookup reads a handful of
/// blocks and is then cached, while the thing that must stay lock-free is the
/// *file body* read — and that happens in [`crate::pk2::archive::Archive`]
/// after this returns, straight through the source.
pub(crate) struct LazyIndex {
    dirs: Mutex<HashMap<PathBuf, DirChain>>,
}

impl LazyIndex {
    pub(crate) fn new() -> Self {
        LazyIndex {
            dirs: Mutex::new(HashMap::new()),
        }
    }

    /// The entry at `path`, reading only the blocks it takes to find it.
    /// `Ok(None)` is "not in this archive"; an `Err` is a malformed archive.
    pub(crate) fn lookup(
        &self,
        source: &dyn Pk2Source,
        blowfish: &Blowfish,
        path: &Path,
    ) -> Result<Option<Entry>, Error> {
        let mut dirs = self.dirs.lock().expect("lazy index lock");
        let mut dir_path = PathBuf::new();
        let mut chain_at = HEADER_SIZE as u64;
        let mut parts = path.iter().peekable();
        while let Some(part) = parts.next() {
            let want = part.to_string_lossy().to_ascii_lowercase();
            let chain = dirs
                .entry(dir_path.clone())
                .or_insert_with(|| DirChain::starting_at(chain_at));
            let Some(entry) = find_or_walk(chain, source, blowfish, &want)? else {
                return Ok(None);
            };
            if parts.peek().is_none() {
                return Ok(Some(entry));
            }
            if !entry.is_dir() {
                return Ok(None); // a file in the middle of a path
            }
            dir_path.push(&want);
            chain_at = entry.position;
        }
        Ok(None) // the empty path is not an entry
    }

    /// Every entry of one directory, which means walking its chain out. The
    /// directory listing is not on the load-by-path hot path, so it pays the
    /// whole chain; the point of this type is that a *lookup* does not.
    pub(crate) fn list(
        &self,
        source: &dyn Pk2Source,
        blowfish: &Blowfish,
        dir: &Path,
    ) -> Result<Option<Vec<Entry>>, Error> {
        let at = if dir.as_os_str().is_empty() {
            HEADER_SIZE as u64
        } else {
            match self.lookup(source, blowfish, dir)? {
                Some(entry) if entry.is_dir() => entry.position,
                _ => return Ok(None),
            }
        };
        let key = lowercased(dir);
        let mut dirs = self.dirs.lock().expect("lazy index lock");
        let chain = dirs.entry(key).or_insert_with(|| DirChain::starting_at(at));
        while chain.resume.is_some() {
            walk_one_block(chain, source, blowfish)?;
        }
        Ok(Some(chain.entries.clone()))
    }
}

/// A path with every component lowercased, which is how the chains are keyed —
/// the same case-insensitivity the eager index gets from `normalize()`.
fn lowercased(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_ascii_lowercase())
}

/// Look in what has been read; extend the chain one block at a time while the
/// name is still missing. This is where the saving is: the loop ends at the
/// block that holds the entry, not at the end of the chain.
fn find_or_walk(
    chain: &mut DirChain,
    source: &dyn Pk2Source,
    blowfish: &Blowfish,
    want: &str,
) -> Result<Option<Entry>, Error> {
    loop {
        if let Some(entry) = chain.find(want) {
            return Ok(Some(entry));
        }
        if chain.resume.is_none() {
            return Ok(None);
        }
        walk_one_block(chain, source, blowfish)?;
    }
}

/// Read the block the chain is resumed at, decrypt it, keep its entries and
/// remember where the chain goes next.
fn walk_one_block(
    chain: &mut DirChain,
    source: &dyn Pk2Source,
    blowfish: &Blowfish,
) -> Result<(), Error> {
    let Some(offset) = chain.resume else {
        return Ok(());
    };
    if chain.seen.len() >= MAX_CHAIN_BLOCKS || !chain.seen.insert(offset) {
        return Err(Error::ChainLoop(offset));
    }
    let mut buf = [0u8; BLOCK_SIZE];
    source
        .read_exact_at(&mut buf, offset)
        .map_err(|_| Error::InvalidBlock("short block"))?;
    blowfish.decrypt(&mut buf);
    let block: Vec<Entry> = buf.chunks_exact(ENTRY_SIZE).map(Entry::from).collect();
    // The chain pointer lives on the 20th entry, whatever that entry's type is.
    chain.resume = block
        .get(ENTRIES_PER_BLOCK - 1)
        .map(|e| e.next_chain)
        .filter(|next| *next > 0);
    chain.entries.extend(
        block
            .into_iter()
            // 0x2E -> "."; the self-reference is the detour this walk avoids
            .filter(|e| !e.is_empty() && e.name[0] != 0x2E),
    );
    Ok(())
}

/// The archive root as an entry, so a lazily-indexed [`crate::pk2::archive::Archive`]
/// can carry a `root` of the same type as an eagerly-indexed one. It is
/// synthetic on purpose: the chain starts at [`HEADER_SIZE`], and reading the
/// archive's own `.` entry to learn that is exactly the duplicated read.
pub(crate) fn root_entry() -> Entry {
    let mut raw = [0u8; ENTRY_SIZE];
    raw[0] = EntryType::Dir as u8;
    raw[106..114].copy_from_slice(&(HEADER_SIZE as u64).to_le_bytes());
    Entry::from(&raw[..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pk2::archive::tests::test_key;
    use crate::pk2::constants::{CHECKSUM, SIGNATURE, VERSION};
    use crate::pk2::key::Pk2Key;
    use crate::prelude::Archive;
    use bevy::asset::io::AssetReader;
    use bytes::Bytes;
    use futures::StreamExt;
    use std::io::Result as IoResult;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const BEAR: &[u8] = b"bear bytes";
    const SIBLING: &[u8] = b"sibling bytes";
    const X: &[u8] = b"x bytes";

    /// Counts the block-sized reads, because over HTTP each one is exactly one
    /// ranged request. The header read (256 B) and a file body are not blocks
    /// and are not counted.
    struct Counting {
        inner: Bytes,
        blocks: AtomicUsize,
    }

    impl Counting {
        fn over(bytes: Bytes) -> Arc<Self> {
            Arc::new(Counting {
                inner: bytes,
                blocks: AtomicUsize::new(0),
            })
        }

        fn blocks(&self) -> usize {
            self.blocks.load(Ordering::SeqCst)
        }
    }

    impl Pk2Source for Counting {
        fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> IoResult<()> {
            if buf.len() == BLOCK_SIZE {
                self.blocks.fetch_add(1, Ordering::SeqCst);
            }
            self.inner.read_exact_at(buf, offset)
        }
    }

    fn at(block: usize) -> u64 {
        (HEADER_SIZE + block * BLOCK_SIZE) as u64
    }

    fn entry_bytes(typ: u8, name: &str, position: u64, size: u32) -> [u8; ENTRY_SIZE] {
        let mut e = [0u8; ENTRY_SIZE];
        e[0] = typ;
        e[1..1 + name.len()].copy_from_slice(name.as_bytes());
        e[106..114].copy_from_slice(&position.to_le_bytes());
        e[114..118].copy_from_slice(&size.to_le_bytes());
        e
    }

    /// One encrypted block: the entries in the first slots, the chain pointer
    /// on the 20th, as a real archive has it.
    fn block(entries: &[[u8; ENTRY_SIZE]], next: u64, bf: &Blowfish) -> [u8; BLOCK_SIZE] {
        let mut buf = [0u8; BLOCK_SIZE];
        for (i, e) in entries.iter().enumerate() {
            buf[i * ENTRY_SIZE..(i + 1) * ENTRY_SIZE].copy_from_slice(e);
        }
        let last = (ENTRIES_PER_BLOCK - 1) * ENTRY_SIZE;
        buf[last + 118..last + 126].copy_from_slice(&next.to_le_bytes());
        bf.encrypt(&mut buf);
        buf
    }

    fn header(bf: &Blowfish) -> [u8; HEADER_SIZE] {
        let mut header = [0u8; HEADER_SIZE];
        header[..30].copy_from_slice(SIGNATURE);
        header[30..34].copy_from_slice(&VERSION.to_le_bytes());
        header[34] = 1; // encrypted -> the checksum is checked
        let mut checksum = *CHECKSUM;
        bf.encrypt(&mut checksum);
        header[35..51].copy_from_slice(&checksum);
        header
    }

    /// A three-level archive with chains and a sibling subtree, so "only the
    /// path" is measurably different from "everything":
    ///
    /// ```text
    /// root     R0 -> R1    R0: ".", "other"    R1: "Res"
    /// Res      D0 -> D1    D0: "Cos"           D1: "unused.txt"
    /// Res/Cos  C0 -> C1    C0: "Bear.bsr"      C1: "sibling.txt"
    /// other    X0 -> X1    X0: "x.txt"         X1: (empty)
    /// ```
    ///
    /// The mixed case is deliberate: the original client asks in lowercase for
    /// files an official archive stores capitalized.
    fn nested_archive(bf: &Blowfish) -> Bytes {
        let bear_at = at(8);
        let sibling_at = bear_at + BEAR.len() as u64;
        let x_at = sibling_at + SIBLING.len() as u64;
        let blocks = [
            block(
                &[
                    entry_bytes(1, ".", HEADER_SIZE as u64, 0),
                    entry_bytes(1, "other", at(6), 0),
                ],
                at(1),
                bf,
            ),
            block(&[entry_bytes(1, "Res", at(2), 0)], 0, bf),
            block(&[entry_bytes(1, "Cos", at(4), 0)], at(3), bf),
            block(&[entry_bytes(2, "unused.txt", x_at, 0)], 0, bf),
            block(
                &[entry_bytes(2, "Bear.bsr", bear_at, BEAR.len() as u32)],
                at(5),
                bf,
            ),
            block(
                &[entry_bytes(
                    2,
                    "sibling.txt",
                    sibling_at,
                    SIBLING.len() as u32,
                )],
                0,
                bf,
            ),
            block(&[entry_bytes(2, "x.txt", x_at, X.len() as u32)], at(7), bf),
            block(&[], 0, bf),
        ];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header(bf));
        for b in blocks {
            bytes.extend_from_slice(&b);
        }
        bytes.extend_from_slice(BEAR);
        bytes.extend_from_slice(SIBLING);
        bytes.extend_from_slice(X);
        Bytes::from(bytes)
    }

    fn fixture() -> (Pk2Key, Blowfish, Bytes) {
        let key = test_key();
        let bf = Blowfish::from_key(&key).expect("fixture key is valid");
        let bytes = nested_archive(&bf);
        (key, bf, bytes)
    }

    /// The whole point, as a number: the same file out of the same archive,
    /// read both ways, with the block reads counted. On the user's `Data.pk2`
    /// this ratio is 5 against 5 022; on this fixture it is 4 against 10.
    ///
    /// The eager 10 includes the duplicated root chain (R0 and R1 are read
    /// once by `Directory::index` and again when the `.` entry is expanded) —
    /// the same 2 duplicate reads measured over HTTP against the real archive.
    #[test]
    fn a_lookup_reads_only_the_blocks_on_the_path() {
        let (key, _bf, bytes) = fixture();

        let lazy_source = Counting::over(bytes.clone());
        let lazy = Archive::from_source_lazy(lazy_source.clone(), &key).expect("opens");
        assert_eq!(
            lazy.read_file_bytes(Path::new("res/cos/bear.bsr"))
                .as_deref(),
            Some(BEAR)
        );
        assert_eq!(
            lazy_source.blocks(),
            4,
            "root chain R0+R1, then Res's D0, then Cos's C0 — and nothing else"
        );

        let eager_source = Counting::over(bytes);
        let eager = Archive::from_source(eager_source.clone(), &key).expect("opens");
        assert_eq!(
            eager
                .read_file_bytes(Path::new("res/cos/bear.bsr"))
                .as_deref(),
            Some(BEAR),
            "positive control: the eager path reads the same bytes"
        );
        assert_eq!(
            eager_source.blocks(),
            10,
            "every block of the archive, the root chain twice"
        );
    }

    /// The additive guarantee, as a test: an eagerly indexed archive still
    /// knows the whole tree. Eight callers outside this crate depend on it.
    #[test]
    fn the_eager_tree_is_still_complete() {
        let (key, _bf, bytes) = fixture();
        let eager = Archive::from_source(Arc::new(bytes), &key).expect("opens");
        let all = eager.root.get_all_entries();
        let mut paths: Vec<String> = all
            .keys()
            .map(|p| p.to_string_lossy().to_ascii_lowercase())
            .collect();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "other",
                "other/x.txt",
                "res",
                "res/cos",
                "res/cos/bear.bsr",
                "res/cos/sibling.txt",
                "res/unused.txt",
            ]
        );
    }

    /// A lazily indexed archive deliberately has no tree to list, and says so
    /// by being empty rather than by half-filling it.
    #[test]
    fn a_lazy_archive_has_no_eager_tree() {
        let (key, _bf, bytes) = fixture();
        let lazy = Archive::from_source_lazy(Arc::new(bytes), &key).expect("opens");
        assert!(lazy.root.get_all_entries().is_empty());
    }

    /// Both modes answer with the same bytes for every file in the fixture —
    /// the positive control for the walk, not just for one lucky path.
    #[test]
    fn both_index_modes_return_the_same_bytes() {
        let (key, _bf, bytes) = fixture();
        let lazy = Archive::from_source_lazy(Arc::new(bytes.clone()), &key).expect("opens");
        let eager = Archive::from_source(Arc::new(bytes), &key).expect("opens");
        for (path, want) in [
            ("res/cos/bear.bsr", BEAR),
            ("res/cos/sibling.txt", SIBLING),
            ("other/x.txt", X),
        ] {
            let from_lazy = lazy.read_file_bytes(Path::new(path));
            let from_eager = eager.read_file_bytes(Path::new(path));
            assert_eq!(from_lazy.as_deref(), Some(want), "lazy {path}");
            assert_eq!(from_lazy, from_eager, "both modes agree on {path}");
        }
    }

    /// The cache is resumable: a sibling in the same directory costs the one
    /// block its chain had not reached yet, and a repeat costs nothing.
    #[test]
    fn a_walked_chain_is_not_walked_again() {
        let (key, _bf, bytes) = fixture();
        let source = Counting::over(bytes);
        let lazy = Archive::from_source_lazy(source.clone(), &key).expect("opens");

        assert!(lazy
            .read_file_bytes(Path::new("res/cos/bear.bsr"))
            .is_some());
        assert_eq!(source.blocks(), 4);

        assert!(lazy
            .read_file_bytes(Path::new("res/cos/sibling.txt"))
            .is_some());
        assert_eq!(source.blocks(), 5, "only C1, where the chain had stopped");

        assert!(lazy
            .read_file_bytes(Path::new("res/cos/bear.bsr"))
            .is_some());
        assert_eq!(source.blocks(), 5, "the second lookup reads no block");

        assert!(lazy
            .read_file_bytes(Path::new("res/cos/nope.bsr"))
            .is_none());
        assert_eq!(
            source.blocks(),
            5,
            "a miss in a walked-out chain reads no block either"
        );
    }

    /// Case-insensitive like the eager map, which is not cosmetic: textdata
    /// references `char/china/...` for files stored as `Char/China/...`.
    #[test]
    fn a_lookup_is_case_insensitive() {
        let (key, _bf, bytes) = fixture();
        let lazy = Archive::from_source_lazy(Arc::new(bytes), &key).expect("opens");
        assert_eq!(
            lazy.read_file_bytes(Path::new("RES/COS/BEAR.BSR"))
                .as_deref(),
            Some(BEAR)
        );
    }

    /// An absent name is `None`, not an error and not a panic — at the last
    /// component and in the middle of the path.
    #[test]
    fn a_missing_entry_is_none() {
        let (key, bf, bytes) = fixture();
        let index = LazyIndex::new();
        for missing in ["nope", "res/nope/bear.bsr", "res/cos/nope.bsr"] {
            assert!(
                index
                    .lookup(&bytes, &bf, Path::new(missing))
                    .expect("a miss is not an error")
                    .is_none(),
                "{missing}"
            );
        }
        let lazy = Archive::from_source_lazy(Arc::new(bytes), &key).expect("opens");
        assert!(lazy
            .read_file_bytes(Path::new("res/cos/bear.bsr/deeper"))
            .is_none());
    }

    /// Listing walks the chain out, which a lookup does not: both blocks of
    /// `Res/Cos` and both entries, in the archive's own casing.
    #[test]
    fn a_listing_walks_the_whole_chain() {
        let (_key, bf, bytes) = fixture();
        let index = LazyIndex::new();
        let listed = index
            .list(&bytes, &bf, Path::new("res/cos"))
            .expect("walks")
            .expect("the directory is there");
        let mut names: Vec<String> = listed
            .iter()
            .map(|e| e.path_buf().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["Bear.bsr", "sibling.txt"]);

        let root = index
            .list(&bytes, &bf, Path::new(""))
            .expect("walks")
            .expect("the root is there");
        let mut root_names: Vec<String> = root
            .iter()
            .map(|e| e.path_buf().to_string_lossy().to_string())
            .collect();
        root_names.sort();
        assert_eq!(root_names, vec!["Res", "other"], "the `.` entry is not one");
    }

    /// The asset system's directory listing goes through the same walk.
    #[test]
    fn the_asset_reader_lists_a_lazy_directory() {
        let (key, _bf, bytes) = fixture();
        let lazy = Archive::from_source_lazy(Arc::new(bytes), &key).expect("opens");
        let stream =
            futures::executor::block_on(lazy.read_directory(Path::new("res/cos"))).expect("lists");
        let mut paths: Vec<String> = futures::executor::block_on(stream.collect::<Vec<PathBuf>>())
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        paths.sort();
        assert_eq!(paths, vec!["res/cos/Bear.bsr", "res/cos/sibling.txt"]);
    }

    /// Red control for the trust boundary: a chain pointing back at itself
    /// must be an error, not a loop. The archive is untrusted input.
    #[test]
    fn a_cyclic_chain_is_rejected() {
        let key = test_key();
        let bf = Blowfish::from_key(&key).expect("fixture key is valid");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header(&bf));
        // the root block chains to itself
        bytes.extend_from_slice(&block(&[entry_bytes(1, "d", at(0), 0)], at(0), &bf));
        let source = Bytes::from(bytes);
        assert!(matches!(
            LazyIndex::new().lookup(&source, &bf, Path::new("nope")),
            Err(Error::ChainLoop(_))
        ));
    }

    /// A truncated chain block is an error rather than 20 entries parsed out
    /// of a half-filled buffer.
    #[test]
    fn a_short_block_is_rejected() {
        let key = test_key();
        let bf = Blowfish::from_key(&key).expect("fixture key is valid");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header(&bf));
        bytes.extend_from_slice(&block(&[entry_bytes(1, "d", at(1), 0)], 0, &bf)[..64]);
        let source = Bytes::from(bytes);
        assert!(matches!(
            LazyIndex::new().lookup(&source, &bf, Path::new("d")),
            Err(Error::InvalidBlock(_))
        ));
    }
}

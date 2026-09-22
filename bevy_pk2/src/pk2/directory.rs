use bevy::asset::io::AssetReaderError;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Cursor;
use std::path::PathBuf;

use bevy::prelude::info;
use bytes::Bytes;

use crate::pk2::blowfish::Blowfish;
use crate::pk2::constants::{HEADER_SIZE, MAX_DIRECTORIES};
use crate::pk2::entry::Entry;
use crate::pk2::errors::Error;
use crate::pk2::util::{read_block, CursorExt};

/// Represents a directory entry in the PK2 archive.
#[derive(Clone)]
pub struct Directory {
    entry: Entry,
    pub entries: HashMap<PathBuf, Entry>,
    pub directories: HashMap<PathBuf, Directory>,
}

impl From<Entry> for Directory {
    /// Creates a [Directory] from a given [Entry].
    fn from(entry: Entry) -> Self {
        if !entry.is_dir() {
            panic!("{:?} is not a directory", entry.path_buf().as_path());
        }
        Directory {
            entry,
            entries: HashMap::new(),
            directories: HashMap::new(),
        }
    }
}

impl Directory {
    /// Expands a directory recursively. Used for indexing.
    ///
    /// The directory tree is walked with a visited-set on block positions: a
    /// hostile or corrupt archive can point a subdirectory back at an ancestor,
    /// which used to recurse until the stack died. An already-expanded position
    /// is skipped, and errors propagate instead of `unwrap`-ing.
    ///
    /// The set holds one entry per *directory*, so its size is bounded by
    /// [`MAX_DIRECTORIES`], not by the per-chain [`crate::pk2::constants::MAX_CHAIN_BLOCKS`]
    /// (which `util::read_block` enforces on the block chain of a single
    /// directory). Conflating the two capped how many directories an archive
    /// was allowed to have and reported the overflow as a chain loop.
    pub fn expand(&mut self, cursor: &mut Cursor<Bytes>, blowfish: &Blowfish) -> Result<(), Error> {
        self.expand_bounded(cursor, blowfish, MAX_DIRECTORIES)
    }

    /// [`Self::expand`] with the directory cap given explicitly, so the cap's
    /// behaviour can be exercised without synthesizing a 65536-directory
    /// archive (that would be ~170 MB of blocks per test run).
    pub(crate) fn expand_bounded(
        &mut self,
        cursor: &mut Cursor<Bytes>,
        blowfish: &Blowfish,
        max_dirs: usize,
    ) -> Result<(), Error> {
        let mut visited = HashSet::new();
        self.expand_guarded(cursor, blowfish, &mut visited, max_dirs)
    }

    fn expand_guarded(
        &mut self,
        cursor: &mut Cursor<Bytes>,
        blowfish: &Blowfish,
        visited: &mut HashSet<u64>,
        max_dirs: usize,
    ) -> Result<(), Error> {
        if visited.len() >= max_dirs {
            return Err(Error::TooManyDirectories(max_dirs));
        }
        if !visited.insert(self.entry.position) {
            // already expanded from this block - a cycle in the tree
            return Ok(());
        }
        let entries = cursor.read_block(self.entry.position, blowfish)?;
        let mapped_entries: HashMap<PathBuf, Entry> = entries
            .iter()
            .filter(|e| !e.is_empty())
            .filter(|e| e.name[0] != 0x2E)
            .map(|entry| (entry.path_buf(), *entry))
            .collect();
        self.entries.extend(mapped_entries);

        let mut dirs: HashMap<PathBuf, Directory> = entries
            .iter()
            .filter(|e| e.is_dir())
            .filter(|e| e.name[0] != 0x2E) // 0x2E -> "."
            .map(|e| Directory::from(*e))
            .map(|d| (d.entry.path_buf(), d))
            .collect();

        for d in dirs.values_mut() {
            d.expand_guarded(cursor, blowfish, visited, max_dirs)?;
        }

        self.directories.extend(dirs);

        Ok(())
    }

    /// Indexes all archive entries recursively
    pub fn index(file: &mut File, blowfish: &Blowfish) -> Result<Directory, Error> {
        let entries = read_block(file, HEADER_SIZE as u64, blowfish)?;
        // 0x2E -> "."
        let mut root_dir_entry = *entries
            .iter()
            .find(|e| e.is_dir() && e.name[0] == 0x2e)
            .ok_or(Error::InvalidBlock("archive has no root directory entry"))?;
        root_dir_entry.name[0] = 0;
        let mut root_dir = Directory::from(root_dir_entry);
        root_dir.expand_from_file(file, blowfish)?;
        Ok(root_dir)
    }

    /// File-backed counterpart of [`Self::expand`], with the same cycle guard.
    pub fn expand_from_file(&mut self, file: &mut File, blowfish: &Blowfish) -> Result<(), Error> {
        let mut visited = HashSet::new();
        self.expand_from_file_guarded(file, blowfish, &mut visited)
    }

    fn expand_from_file_guarded(
        &mut self,
        file: &mut File,
        blowfish: &Blowfish,
        visited: &mut HashSet<u64>,
    ) -> Result<(), Error> {
        if visited.len() >= MAX_DIRECTORIES {
            return Err(Error::TooManyDirectories(MAX_DIRECTORIES));
        }
        if !visited.insert(self.entry.position) {
            return Ok(());
        }
        let entries = read_block(file, self.entry.position, blowfish)?;
        let path = self.entry.path_buf().clone();
        let mapped_entries: HashMap<PathBuf, Entry> = entries
            .iter()
            .filter(|e| !e.is_empty())
            .filter(|e| e.name[0] != 0x2E)
            .map(|entry| {
                let mut cloned_path = path.clone();
                cloned_path.push(entry.path_buf());
                (entry.path_buf(), *entry)
            })
            .collect();
        self.entries.extend(mapped_entries);

        let mut dirs: HashMap<PathBuf, Directory> = entries
            .iter()
            .filter(|e| e.is_dir())
            .filter(|e| e.name[0] != 0x2E) // 0x2E -> "."
            .map(|e| Directory::from(*e))
            .map(|d| (d.entry.path_buf(), d))
            .collect();

        for d in dirs.values_mut() {
            d.expand_from_file_guarded(file, blowfish, visited)?;
        }

        self.directories.extend(dirs);

        Ok(())
    }

    /// Prints out all entries of a directory recursively.
    pub fn print_entries(&self) {
        self.directories.iter().for_each(|(p, d)| {
            info!("Dir: {:?}", p.as_path());
            d.print_entries();
        });
        self.entries
            .iter()
            .filter(|(_, e)| e.is_file())
            .for_each(|(p, _)| {
                info!("File: {:?}", p.as_path());
            });
    }

    pub fn load_file(&self, path: PathBuf) -> Result<(u64, usize), AssetReaderError> {
        let p = path.clone();
        let mut components = p.iter();
        let entry_path = match components.next() {
            None => return Err(AssetReaderError::NotFound(path)),
            Some(path) => PathBuf::from(path),
        };

        let entry = self
            .entries
            .get(&entry_path)
            .ok_or(AssetReaderError::NotFound(entry_path.clone()))?;

        if entry.is_dir() {
            let dir = self
                .directories
                .get(&entry_path)
                .ok_or(AssetReaderError::NotFound(entry_path.clone()))?;
            let sub_path = PathBuf::from_iter(components);
            return dir.load_file(sub_path);
        } else if entry.is_file() {
            return Ok((entry.position, entry.size as usize));
        }

        Err(AssetReaderError::NotFound(entry_path))
    }

    pub fn get_all_entries(&self) -> HashMap<PathBuf, Entry> {
        let base_path = self.entry.path_buf();
        let mut entries = HashMap::new();

        for (entry_path, entry) in &self.entries {
            if entry.is_dir() {
                // a dir entry whose block was skipped as a cycle has no
                // expanded Directory - list the entry itself and move on
                let Some(dir) = self.directories.get(entry_path) else {
                    let mut pb = base_path.clone();
                    pb.push(entry.path_buf());
                    entries.insert(pb, *entry);
                    continue;
                };
                let sub_entries = dir.get_all_entries();
                sub_entries.iter().for_each(|(k, v)| {
                    let mut pb = base_path.clone();
                    pb.push(k);
                    entries.insert(pb, *v);
                });
                let mut pb = base_path.clone();
                pb.push(entry.path_buf());
                entries.insert(pb, *entry);
            } else if entry.is_file() {
                let mut bp = base_path.clone();
                bp.push(entry.path_buf());
                entries.insert(bp, *entry);
            }
        }

        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pk2::constants::{BLOCK_SIZE, ENTRIES_PER_BLOCK, ENTRY_SIZE, MAX_DIRECTORIES};
    use crate::pk2::key::Pk2Key;

    /// A fixture cipher: these tests encrypt the blocks they then read back, so
    /// any valid key exercises the tree walk they are about. The real archive
    /// key is the player's own and deliberately absent from this crate.
    fn test_cipher() -> Blowfish {
        let key = Pk2Key::from_config("testkey", "00112233445566778899").unwrap();
        Blowfish::from_key(&key).expect("fixture key is valid")
    }

    /// One plaintext block: `dirs` gives (name, block offset) for each child
    /// directory entry, `next` is the chain pointer on the 20th entry.
    fn block(dirs: &[(String, u64)], next: u64, blowfish: &Blowfish) -> Vec<u8> {
        let mut buf = [0u8; BLOCK_SIZE];
        for (i, (name, position)) in dirs.iter().enumerate() {
            let at = i * ENTRY_SIZE;
            buf[at] = 1; // EntryType::Dir
            buf[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());
            buf[at + 106..at + 114].copy_from_slice(&position.to_le_bytes());
        }
        let last = (ENTRIES_PER_BLOCK - 1) * ENTRY_SIZE;
        buf[last + 118..last + 126].copy_from_slice(&next.to_le_bytes());
        blowfish.encrypt(&mut buf);
        buf.to_vec()
    }

    /// A root chain listing `count` child directories (19 per block, chained
    /// through the 20th entry), each child an empty, properly terminated block
    /// of its own. Returns the archive bytes and a root [`Directory`] at offset 0.
    fn tree(count: usize, blowfish: &Blowfish) -> (Bytes, Directory) {
        let per_block = ENTRIES_PER_BLOCK - 1;
        let root_blocks = count.div_ceil(per_block).max(1);
        let children: Vec<(String, u64)> = (0..count)
            .map(|i| (format!("d{i}"), ((root_blocks + i) * BLOCK_SIZE) as u64))
            .collect();
        let mut bytes = Vec::with_capacity((root_blocks + count) * BLOCK_SIZE);
        for j in 0..root_blocks {
            let slice = &children[(j * per_block).min(count)..((j + 1) * per_block).min(count)];
            let next = if j + 1 < root_blocks {
                ((j + 1) * BLOCK_SIZE) as u64
            } else {
                0
            };
            bytes.extend_from_slice(&block(slice, next, blowfish));
        }
        for _ in 0..count {
            bytes.extend_from_slice(&block(&[], 0, blowfish));
        }
        let mut root_entry = Entry::from(&[0u8; ENTRY_SIZE][..]);
        root_entry.typ = 1;
        root_entry.name[0] = b'r';
        root_entry.position = 0;
        (Bytes::from(bytes), Directory::from(root_entry))
    }

    /// (a) A wide tree below the cap indexes completely. The archive that
    /// exposed this defect has 5361 directories against a 4096 cap that was
    /// never meant to count directories at all.
    #[test]
    fn a_tree_below_the_directory_cap_expands() {
        let blowfish = test_cipher();
        let (bytes, mut root) = tree(12, &blowfish);
        let mut cursor = Cursor::new(bytes);
        root.expand_bounded(&mut cursor, &blowfish, 16)
            .expect("a tree below the cap indexes");
        assert_eq!(root.directories.len(), 12, "every child directory expanded");
    }

    /// (b) Above the cap the archive is refused as too large — and explicitly
    /// NOT as a chain loop: a full tree is not a cycle, and calling it one sent
    /// a previous reader hunting for corruption in a perfectly intact block.
    #[test]
    fn a_tree_above_the_directory_cap_is_not_a_chain_loop() {
        let blowfish = test_cipher();
        let (bytes, mut root) = tree(12, &blowfish);
        let mut cursor = Cursor::new(bytes);
        let err = root
            .expand_bounded(&mut cursor, &blowfish, 4)
            .expect_err("a tree above the cap is refused");
        assert!(
            matches!(err, Error::TooManyDirectories(4)),
            "want TooManyDirectories, got {err:?}"
        );
    }

    /// (c) A directory whose block chain points back at itself is still a
    /// chain loop — the protection the cap was mistakenly doing double duty for.
    #[test]
    fn a_self_chaining_directory_block_is_still_a_chain_loop() {
        let blowfish = test_cipher();
        let one = BLOCK_SIZE as u64;
        let mut bytes = vec![0u8; BLOCK_SIZE]; // filler so offset `one` is real
        bytes.extend_from_slice(&block(&[], one, &blowfish)); // at `one` -> `one`
        let mut root_entry = Entry::from(&[0u8; ENTRY_SIZE][..]);
        root_entry.typ = 1;
        root_entry.name[0] = b'r';
        root_entry.position = one;
        let mut root = Directory::from(root_entry);
        let mut cursor = Cursor::new(Bytes::from(bytes));
        let err = root
            .expand_bounded(&mut cursor, &blowfish, MAX_DIRECTORIES)
            .expect_err("a self-chaining block is refused");
        assert!(
            matches!(err, Error::ChainLoop(o) if o == one),
            "want ChainLoop, got {err:?}"
        );
    }

    /// (d) The foreign archive that exposed the defect, at its real size: 5361
    /// directories through the public [`Directory::expand`] with the shipped cap.
    /// Under the old shared 4096 cap this failed as a bogus `ChainLoop`.
    #[test]
    fn a_5361_directory_archive_indexes_with_the_shipped_cap() {
        let blowfish = test_cipher();
        let (bytes, mut root) = tree(5361, &blowfish);
        let mut cursor = Cursor::new(bytes);
        root.expand(&mut cursor, &blowfish)
            .expect("5361 directories are a valid archive, not a chain loop");
        assert_eq!(root.directories.len(), 5361);
    }

    /// The two caps must stay apart: the directory cap has to clear the
    /// directory counts real archives have (2469 and 5361), while the per-chain
    /// cap stays at the scale of one chain (322 blocks in the longest).
    const _: () = {
        assert!(
            MAX_DIRECTORIES >= 4 * 5361,
            "clear real archives with headroom"
        );
        assert!(MAX_DIRECTORIES <= 1 << 22, "still bound the walk");
    };
}

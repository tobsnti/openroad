pub const ENTRY_SIZE: usize = 128;
pub const BLOCK_SIZE: usize = 20 * ENTRY_SIZE;
pub const HEADER_SIZE: usize = 256;
// The archive key and its salt are deliberately NOT here. They are supplied by
// the user at runtime through local configuration and carried in
// [`crate::Pk2Key`] — see `key.rs` for why. The constants below are container
// format identifiers, not key material: they are what a PK2 *is*, and are
// needed to recognize and parse one at all.
pub const CHECKSUM: &[u8; 16] = b"Joymax Pak File\0";
pub const SIGNATURE: &[u8; 30] = b"JoyMax File Manager!\x0a\x00\x00\x00\x00\x00\x00\x00\x00\x00";
pub const VERSION: u32 = 0x0100_0002;
/// Entries per block — the 20th carries the chain pointer.
pub const ENTRIES_PER_BLOCK: usize = BLOCK_SIZE / ENTRY_SIZE;
/// Hard cap on how many blocks ONE directory chain may span (see
/// `util::read_block`). The longest chain in a real archive is 322 blocks, so
/// 4096 is roughly 12x headroom; it exists so a hostile or
/// corrupt `next_chain` cannot spin forever even when every offset is unique.
pub const MAX_CHAIN_BLOCKS: usize = 4096;
/// Hard cap on how many DIRECTORIES one archive's tree may contain (see
/// `directory.rs`). This used to be [`MAX_CHAIN_BLOCKS`] as well, which made a
/// per-chain limit silently act as a per-archive directory count and rejected
/// perfectly valid archives: one archive carries 2469 directories (below
/// 4096, so it loaded) versus 5361 in another (above 4096, so it failed with a
/// bogus chain-loop error). 65536 is over 12x the largest count observed while
/// still bounding the walk — one directory costs at least one block read, so
/// the work stays linear and finite even for a hostile archive.
pub const MAX_DIRECTORIES: usize = 65_536;

// Parser-only library surface of the client crate.
//
// The client binary keeps its full module tree in main.rs; this lib target
// exposes only the pure SRO format parsers (bsr/bms/bmt/bsk/ban/ddj and
// their helpers) so external tools (tools/src/bin/bsr2glb) can reuse them
// without duplicating byte-level format knowledge. The subset is closed:
// these modules reference only each other, never plugins/scenes/commands.
//
// Trade-off: the listed files compile twice (once per target) and their
// #[cfg(test)] suites run for both, which is benign. Extend the list if a
// new transitive module reference appears.
pub mod util {
    pub mod binread;
    pub mod buf_ext;
    pub mod mesh;
    pub mod mips;
    /// Tween helpers. Only here because `assets::intro_scene` builds its
    /// camera `Sequence` from them; like the rest of this surface the module
    /// references nothing outside `util`.
    pub mod tweening_ext;
}

// The server half of the SRO security handshake, the framing and Blowfish.
// Exposed for `tools/src/bin/sro_peer`, our own local test peer: a peer has to
// *drive* the same handshake the client answers, and re-implementing it in the
// tool would put a second copy of the byte layout next to this one — the very
// duplication this lib target exists to avoid.
//
// Deliberately NOT `net/mod.rs`: that module also declares `connection`,
// `reader` and `entity_spawn`, and `entity_spawn` reaches into
// `crate::plugins::textdata`, which would break the "closed subset" property
// above. The seven modules listed here reference only each other.
// `dead_code` is allowed for the subset, and only here: the lib target
// compiles these seven modules *without* the client that drives them, so
// `handshake.rs`'s `pub(crate)` helpers (`setup_handshake`, `finalize`, the
// four setup flags, …) read as unused although the bin target and
// `sro_peer` both use them. The alternative — an `#[allow]` per item — would
// put the noise in the net files, where it would outlive the reason.
#[allow(dead_code)]
pub mod net {
    pub mod blowfish;
    pub mod codec;
    pub mod crc;
    pub mod frame;
    pub mod handshake;
    pub mod security;
    pub mod sequence;
}

pub mod assets {
    pub mod ainav;
    pub mod ban;
    pub mod bms;
    pub mod bmt;
    pub mod bsk;
    pub mod bsr;
    pub mod ddj;
    pub mod dof;
    /// The `.intro` cutscene camera path. Needed by
    /// `tools/src/bin/intro_convert`, which converts the original's
    /// `script/intro/<name>.txt` camera block into one (#569) — the parser and
    /// the converter must not hold two copies of the field layout.
    pub mod intro_scene;
    pub mod nvm;
    /// Textdata tables. Verified closed like the rest of this surface: every
    /// `use crate::` in `assets/textdata/**` points at `assets` only, never at
    /// plugins/scenes/commands. Needed by `tools/src/bin/pk2_probe` to
    /// histogram itemdata/skilldata columns through the real parsers (and by
    /// `dungeon_scan` for the dungeoninfo table + string decoder).
    pub mod textdata;
    pub mod tile_tint;
}

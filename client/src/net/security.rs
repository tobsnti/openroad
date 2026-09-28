// SRO's session security layer: the handshake state machine, key agreement and
// the per-frame blowfish/CRC/sequence wrapping.
//
// PROVENANCE. The algorithm is described by several fan reimplementations
// (xBot's `SecurityAPI/Security.cs`, `SilkroadSecurityJS`), which are all ports
// of one upstream project — pushedx/jMerlin's `SilkroadSecurityApi` — and so
// count as a single source rather than independent confirmations
// (`docs/re/round2/evidence-lineage.md`). That upstream carries no licence, so
// this file is written from the *described behaviour* and from our own capture
// corpus; no code is copied from any of them. What forces the behaviour is wire
// compatibility with a real vSRO server, which is not negotiable.
use crate::net::blowfish::Blowfish;
use crate::net::crc::CRC;
use crate::net::sequence::Sequence;

#[derive(Clone)]
pub enum SilkroadSecurity {
    None,
    Initialized,
    Established,
}

#[derive(Clone)]
pub struct SilkroadSecurityState {
    pub(crate) state: SilkroadSecurity,
    pub(crate) context: SilkroadSecurityData,
}

impl SilkroadSecurityState {
    pub fn new() -> Self {
        Self {
            state: SilkroadSecurity::None,
            context: SilkroadSecurityData::new(),
        }
    }

    /// A session that is **already past the handshake**, built from the three
    /// values the handshake agreed on: the final blowfish key and the two
    /// security-byte seeds.
    ///
    /// Exists for the side of the wire this crate does not otherwise play: a
    /// server half (`tools/src/bin/sro_peer`) derives those three values itself
    /// instead of reading them out of a `0x5000`, and every field of
    /// [`SilkroadSecurityData`] is `pub(crate)` — deliberately, so callers
    /// cannot half-initialise a crypto context. One constructor that takes
    /// exactly the agreed parameters keeps that property while letting the peer
    /// reuse `frame.rs` verbatim.
    pub fn established(
        blowfish_key: &[u8],
        sequence_seed: u32,
        crc_seed: u32,
    ) -> Result<Self, crate::net::blowfish::InvalidKey> {
        let mut context = SilkroadSecurityData::new();
        context.blowfish = Some(Blowfish::new(blowfish_key)?);
        context.sequence = Sequence::from(sequence_seed);
        context.crc = Box::new(CRC::from(crc_seed));
        context.sequence_seed = sequence_seed;
        // Same bookkeeping as the client half's `setup_handshake`, which keeps
        // the shifted copy (`handshake.rs`, `sec_data.crc_seed = crc_seed << 8`).
        context.crc_seed = crc_seed << 8;
        Ok(Self {
            state: SilkroadSecurity::Established,
            context,
        })
    }

    /// Switch this session to the **server** role: stop stamping the security
    /// bytes on everything it sends. See
    /// [`SilkroadSecurityData::stamps_security_bytes`] — the original client
    /// resets the connection over a stamped inbound frame.
    ///
    /// Takes `self` although it is named `as_*`, which is why the convention
    /// lint is silenced here: it is the tail of the builder that
    /// [`Self::established`] starts, and a borrowing variant would hand out a
    /// session that is only *half* switched until the caller stores it.
    #[allow(clippy::wrong_self_convention)]
    pub fn as_server_role(mut self) -> Self {
        self.context.stamps_security_bytes = false;
        self
    }
}

#[derive(Clone)]
pub struct SilkroadSecurityData {
    /// The flag byte of the phase-1 `0x5000`. The original keeps the same
    /// accumulator (`004b1da0:120,162`) and gates the final handshake message
    /// on it; see `handshake::check_body`.
    pub(crate) setup_flags: u8,
    pub(crate) sequence_seed: u32,
    pub(crate) crc_seed: u32,
    pub(crate) local_public: u32,
    pub(crate) local_challenge: u64,
    pub(crate) remote_public: u32,
    pub(crate) common_secret: u32,
    pub(crate) handshake_key: u64,
    pub(crate) generator: u32,
    pub(crate) prime: u32,
    pub(crate) blowfish: Option<Blowfish>,
    /// Whether *outgoing* frames get their count/CRC bytes stamped.
    ///
    /// True for the client role (us, normally). False for the server role: the
    /// original leaves offsets 4/5 at zero on everything it sends (xBot
    /// `Security.cs:700-712` guards the stamping on the client role, and a live
    /// gateway capture confirms `0000` in every S→C frame), and the original
    /// *client* resets the connection if we stamp them.
    pub(crate) stamps_security_bytes: bool,
    pub(crate) sequence: Sequence,
    // must be boxed because of stack-size issues on Windows
    pub(crate) crc: Box<CRC>,
}

impl SilkroadSecurityData {
    pub fn new() -> Self {
        Self {
            stamps_security_bytes: true,
            setup_flags: 0,
            sequence_seed: 0,
            crc_seed: 0,
            local_public: 0,
            local_challenge: 0,
            remote_public: 0,
            common_secret: 0,
            handshake_key: 0,
            generator: 0,
            prime: 0,
            blowfish: None,
            crc: Box::new(CRC::from(0)),
            sequence: Sequence::from(0),
        }
    }
}

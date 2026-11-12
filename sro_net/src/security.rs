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
use crate::blowfish::Blowfish;
use crate::crc::CRC;
use crate::sequence::Sequence;

#[derive(Clone)]
pub enum SilkroadSecurity {
    None,
    Initialized,
    Established,
}

#[derive(Clone)]
pub struct SilkroadSecurityState {
    pub state: SilkroadSecurity,
    pub context: SilkroadSecurityData,
}

impl SilkroadSecurityState {
    pub fn new() -> Self {
        Self {
            state: SilkroadSecurity::None,
            context: SilkroadSecurityData::new(),
        }
    }
}

#[derive(Clone)]
pub struct SilkroadSecurityData {
    /// The flag byte of the phase-1 `0x5000`. The original keeps the same
    /// accumulator (`004b1da0:120,162`) and gates the final handshake message
    /// on it; see `handshake::check_body`.
    pub setup_flags: u8,
    pub sequence_seed: u32,
    pub crc_seed: u32,
    pub local_public: u32,
    pub local_challenge: u64,
    pub remote_public: u32,
    pub common_secret: u32,
    pub handshake_key: u64,
    pub generator: u32,
    pub prime: u32,
    pub blowfish: Option<Blowfish>,
    pub sequence: Sequence,
    // must be boxed because of stack-size issues on Windows
    pub crc: Box<CRC>,
}

impl SilkroadSecurityData {
    pub fn new() -> Self {
        Self {
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
            crc: Box::new(CRC::unseeded()),
            sequence: Sequence::unseeded(),
        }
    }
}

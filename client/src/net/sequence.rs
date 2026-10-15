// The per-frame sequence byte.
//
// A small bit-mangling generator seeded from the handshake; it carries no key
// material of its own. The construction is a fact about the wire format, taken
// from our own capture corpus and cross-checked against the fan descriptions
// whose shared provenance is documented in `security.rs`.
/// What the original puts in place of a zero sequence seed (`004b0cf7`). It is
/// also entry 129 of the CRC base table -- the same number, reused.
const ZERO_SEED_SUBSTITUTE: u32 = 0x9ABF_B3B6;

#[derive(Copy, Clone)]
pub(crate) struct Sequence {
    b0: u8,
    b1: u8,
    b2: u8,
}

impl From<u32> for Sequence {
    /// The wire setter. A seed of 0 is one in 256 sessions and the original does
    /// not run the mixer on it: it substitutes the 32-bit constant below
    /// (`sro_client.exe` 004b0cf7, and again in the standalone setter at
    /// 004afb54). Unlike the CRC seed this one is used whole -- no masking --
    /// which is why the substitute is 32 bits wide and not 1.
    ///
    /// Without it our two post-mix guards turn a zero seed into `b1 = b2 = 1`,
    /// `b0 = 0`, and `((!0) + 1) * 1` truncates to 0 for every frame: the guards
    /// that exist to avoid a trivial zero byte produce exactly that. See
    /// `unseeded` for the state that is *meant* to emit zeros.
    fn from(value: u32) -> Self {
        let value = if value == 0 {
            ZERO_SEED_SUBSTITUTE
        } else {
            value
        };
        let round1 = generate_value(value);
        let round2 = generate_value(round1);
        let round3 = generate_value(round2);
        let round4 = generate_value(round3);
        let mut b1 = ((round1 & 0xFF) ^ (round2 & 0xFF)) as u8;
        let mut b2 = ((round4 & 0xFF) ^ (round3 & 0xFF)) as u8;
        if b1 == 0 {
            b1 = 1;
        }

        if b2 == 0 {
            b2 = 1;
        }
        let b0 = (b2 ^ b1) as u8;

        Self { b0, b1, b2 }
    }
}

impl Sequence {
    /// The state before a handshake: all three bytes zero, so `next` returns 0
    /// for every frame. The original's constructor zeroes the same two bytes
    /// (`004affed`, `004afff3`) instead of deriving them, and only stamps the
    /// count byte at all when the setup negotiated EDC (`004b0da4`); our
    /// `frame.rs` stamps unconditionally, so this state is what keeps those
    /// frames at 0.
    pub(crate) fn unseeded() -> Self {
        Self {
            b0: 0,
            b1: 0,
            b2: 0,
        }
    }

    pub(crate) fn next(&mut self) -> u8 {
        let result = (self.b2 as u32 * ((!self.b0) as u32 + self.b1 as u32) as u32) as u8;
        let result = (result ^ (result >> 4)) as u8;
        self.b0 = result;
        result
    }
}

fn generate_value(value: u32) -> u32 {
    let mut new_val = value;
    let complement = !1_u32;

    for _ in 0..32 {
        let mut v = new_val;
        v = (v >> 2) ^ new_val;
        v = (v >> 2) ^ new_val;
        v = (v >> 1) ^ new_val;
        v = (v >> 1) ^ new_val;
        v = (v >> 1) ^ new_val;
        new_val = ((new_val >> 1) | (new_val << 31)) & complement | (v & 1);
    }

    return new_val;
}

#[cfg(test)]
mod tests {
    use super::{Sequence, ZERO_SEED_SUBSTITUTE};

    fn stream(seed: u32, n: usize) -> Vec<u8> {
        let mut seq = Sequence::from(seed);
        (0..n).map(|_| seq.next()).collect()
    }

    /// Anchor first, and on somebody else's numbers: a recorded session in
    /// `fixes/BROWSERNETZ-rec` carries sequence seed 0xD73C2803 in its `0x5000`
    /// setup, and the paired client stream stamps 98, 4, (encrypted), 35, 105 on
    /// its plaintext frames. The generator has to reproduce that before any new
    /// expectation here means anything.
    #[test]
    fn a_recorded_session_reproduces_its_count_bytes() {
        assert_eq!(stream(0xD73C_2803, 5), vec![98, 4, 57, 35, 105]);
    }

    /// The substitution itself: a zero seed must run on 0x9ABFB3B6.
    #[test]
    fn a_zero_seed_runs_on_the_substitute() {
        assert_eq!(stream(0, 16), stream(ZERO_SEED_SUBSTITUTE, 16));
        assert_eq!(
            stream(0, 8),
            vec![0xA6, 0x62, 0x73, 0xFF, 0xCC, 0x48, 0x59, 0xD5]
        );
    }

    /// What the defect looked like on the wire: the same byte, forever, which is
    /// what a client without EDC sends.
    #[test]
    fn a_zero_seed_no_longer_emits_a_constant_zero() {
        assert_eq!(Sequence::unseeded().b1, 0);
        assert!(stream(0, 16).iter().any(|byte| *byte != 0));
    }

    /// The counter-check: no seed the protocol can otherwise carry is folded onto
    /// the substitute, and the live gateway's own seed (0x54) keeps its stream.
    #[test]
    fn only_a_zero_seed_is_substituted() {
        for seed in 1u32..=0xFF {
            assert_ne!(
                stream(seed, 4),
                stream(ZERO_SEED_SUBSTITUTE, 4),
                "seed {seed} was substituted"
            );
        }
        assert_eq!(stream(84, 8), vec![191, 12, 170, 93, 200, 17, 21, 174]);
    }

    /// The handshake-less state stays the one that emits zeros, and the call site
    /// in `security.rs` has to use it -- with `Sequence::from(0)` back there this
    /// reads 0xA6.
    #[test]
    fn the_pre_handshake_security_state_emits_zero() {
        let mut state = crate::net::security::SilkroadSecurityData::new().sequence;
        assert_eq!(
            (0..16).map(|_| state.next()).collect::<Vec<_>>(),
            vec![0; 16]
        );
    }
}

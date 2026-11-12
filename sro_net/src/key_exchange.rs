//! The arithmetic of the security handshake, the same for both roles: the
//! Diffie-Hellman modpow, the handshake key, the challenge each side signs,
//! and the transform that turns them into Blowfish keys. The exchange itself
//! (who sends what, and when) lives with each role.

use bytes::{Bytes, BytesMut};

pub fn calc_key(common_secret: u32, secret1: u32, secret2: u32) -> Bytes {
    let s1s2 = concat_u32_as_bytes(secret1, secret2);
    key_transform_value(s1s2, common_secret, (common_secret & 3) as u8)
}

pub fn calc_challenge(common_secret: u32, secret1: u32, secret2: u32) -> Bytes {
    let s1s2 = concat_u32_as_bytes(secret1, secret2);
    key_transform_value(s1s2, common_secret, (secret1 & 7) as u8)
}

pub fn concat_u32_as_bytes(a: u32, b: u32) -> Bytes {
    let a_bytes: [u8; 4] = a.to_le_bytes();
    let b_bytes: [u8; 4] = b.to_le_bytes();

    let bytes = &[
        a_bytes[0], a_bytes[1], a_bytes[2], a_bytes[3], b_bytes[0], b_bytes[1], b_bytes[2],
        b_bytes[3],
    ];

    return Bytes::copy_from_slice(bytes);
}

pub fn key_transform_value(v: Bytes, key: u32, key_byte: u8) -> Bytes {
    let mut value = BytesMut::new();
    value.extend_from_slice(&v);
    let key_byte = key_byte as u32;
    value[0] ^= (value[0] as u32 + ((key >> 0) & 0xFF_u32) + key_byte) as u8;
    value[1] ^= (value[1] as u32 + ((key >> 8) & 0xFF_u32) + key_byte) as u8;
    value[2] ^= (value[2] as u32 + ((key >> 16) & 0xFF_u32) + key_byte) as u8;
    value[3] ^= (value[3] as u32 + ((key >> 24) & 0xFF_u32) + key_byte) as u8;

    value[4] ^= (value[4] as u32 + ((key >> 0) & 0xFF_u32) + key_byte) as u8;
    value[5] ^= (value[5] as u32 + ((key >> 8) & 0xFF_u32) + key_byte) as u8;
    value[6] ^= (value[6] as u32 + ((key >> 16) & 0xFF_u32) + key_byte) as u8;
    value[7] ^= (value[7] as u32 + ((key >> 24) & 0xFF_u32) + key_byte) as u8;

    return value.freeze();
}

pub fn g_pow_x_mod_p(generator: u32, private: u32, prime: u32) -> u32 {
    let prime = prime as i64;
    if prime <= 1 {
        return 0;
    }
    let mut result: i64 = 1;
    let mut p = private;
    // Reduce before the first squaring. `generator` comes straight off the
    // wire, and a peer sending g > 0xB504F333 would otherwise overflow the
    // i64 in `mult * mult` — a panic in debug builds, silent wraparound in
    // release. Every later value is already < prime, and for a well-formed
    // g < p this reduction is a no-op.
    let mut mult = (generator as i64) % prime;

    while p != 0 {
        if (p & 1) > 0 {
            result = (mult * result) % prime;
        }
        p >>= 1;
        mult = (mult * mult) % prime;
    }
    result as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `generator` is read straight off the wire. A peer sending a value
    /// above 0xB504F333 used to overflow the i64 in `mult * mult` — panic in
    /// debug, silent wraparound in release — before any of the handshake's
    /// own validation could run.
    #[test]
    fn modpow_survives_a_generator_larger_than_the_prime() {
        let prime = 0x7FFF_FFC3u32;
        let huge = 0xF2E1_5D3Au32;
        // no panic, and the result is the properly reduced one
        assert_eq!(
            g_pow_x_mod_p(huge, 0x0BAD_C0DE, prime),
            g_pow_x_mod_p(huge % prime, 0x0BAD_C0DE, prime)
        );
        // a degenerate prime must not divide by zero either
        assert_eq!(g_pow_x_mod_p(huge, 5, 0), 0);
        assert_eq!(g_pow_x_mod_p(huge, 5, 1), 0);
    }

    /// Sanity: the modpow still agrees with a plain reference implementation.
    #[test]
    fn modpow_matches_a_reference_implementation() {
        let reference = |g: u32, x: u32, p: u32| -> u32 {
            let mut acc: u64 = 1;
            for _ in 0..x {
                acc = (acc * g as u64) % p as u64;
            }
            acc as u32
        };
        for (g, x, p) in [(5u32, 17u32, 23u32), (7, 11, 1009), (2, 20, 65_537)] {
            assert_eq!(
                g_pow_x_mod_p(g, x, p),
                reference(g, x, p),
                "{g}^{x} mod {p}"
            );
        }
    }
}

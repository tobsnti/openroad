//! The server's side of the security handshake, on logical packets.
//!
//! The server opens with 0x5000: an initial Blowfish key, the seeds of the
//! sequence and CRC bytes the client puts on every frame, and its
//! Diffie-Hellman values. The client answers 0x5000 with its public value and
//! a signature only a peer with the shared secret can make. The server checks
//! it, answers with the mirrored signature, and both switch to the session
//! key; the client's 0x9000 completes it. The arithmetic is the client's, in
//! `sro_net::key_exchange`. Framing (the sequence and CRC checks, decryption)
//! is the transport's business: this module only decides what the bodies say.

use bytes::{BufMut, Bytes, BytesMut};
use rand_core::{OsRng, RngCore};
use sro_net::blowfish::Blowfish;
use sro_net::key_exchange::{calc_challenge, calc_key, g_pow_x_mod_p, key_transform_value};

/// The flags of the opening 0x5000: initial Blowfish key, the two seeds, key
/// exchange — the three blocks the client reads [V original handler].
const SETUP_FLAGS: u8 = 0x02 | 0x04 | 0x08;
/// The flag of the server's challenge.
const CHALLENGE_FLAG: u8 = 0x10;

/// What a session needs once the handshake is done.
#[derive(Clone)]
pub struct SessionKeys {
    pub blowfish: Blowfish,
    pub sequence_seed: u32,
    pub crc_seed: u32,
}

/// Why the handshake ended without a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Not the packet this step waits for.
    Opcode(u16),
    /// The client's answer is not 4 + 8 bytes.
    Length(usize),
    /// The client did not derive the shared secret.
    Signature,
}

/// Where a handshake stands.
pub enum Handshake {
    /// The opening 0x5000 is out; the client's answer is next.
    Opened(Opening),
    /// The challenge is out; the client's 0x9000 is next.
    Challenged(Box<SessionKeys>),
}

/// The server's secrets of one opening.
pub struct Opening {
    private: u32,
    public: u32,
    prime: u32,
    handshake_key: u64,
    sequence_seed: u32,
    crc_seed: u32,
}

/// What the server does after a client packet.
pub enum Step {
    /// Send this 0x5000 body and wait for the next packet.
    Answer(Handshake, Bytes),
    /// The handshake is done.
    Done(Box<SessionKeys>),
}

impl Handshake {
    /// A new handshake and the opening 0x5000 body. Every value is drawn per
    /// session; generator, prime and private value are 31-bit, as the
    /// client's private value [S original client].
    pub fn open() -> (Handshake, Bytes) {
        let draw31 = || OsRng.next_u32() & 0x7FFF_FFFF;
        let (generator, prime, private) = (draw31(), draw31(), draw31());
        let opening = Opening {
            private,
            public: g_pow_x_mod_p(generator, private, prime),
            prime,
            handshake_key: OsRng.next_u64(),
            sequence_seed: OsRng.next_u32(),
            crc_seed: OsRng.next_u32(),
        };
        let mut body = BytesMut::with_capacity(37);
        body.put_u8(SETUP_FLAGS);
        body.put_u64_le(OsRng.next_u64());
        body.put_u32_le(opening.sequence_seed);
        body.put_u32_le(opening.crc_seed);
        body.put_u64_le(opening.handshake_key);
        body.put_u32_le(generator);
        body.put_u32_le(prime);
        body.put_u32_le(opening.public);
        (Handshake::Opened(opening), body.freeze())
    }

    /// The seeds of the client's sequence and CRC bytes, which it uses from
    /// its first answer on.
    pub fn seeds(&self) -> (u32, u32) {
        match self {
            Handshake::Opened(o) => (o.sequence_seed, o.crc_seed),
            Handshake::Challenged(keys) => (keys.sequence_seed, keys.crc_seed),
        }
    }

    /// One client packet.
    pub fn receive(self, opcode: u16, body: &[u8]) -> Result<Step, Refused> {
        match (self, opcode) {
            (Handshake::Opened(opening), 0x5000) => opening.answer(body),
            (Handshake::Challenged(keys), 0x9000) if body.is_empty() => Ok(Step::Done(keys)),
            (_, opcode) => Err(Refused::Opcode(opcode)),
        }
    }
}

impl Opening {
    /// The client's public value and signature; the mirrored signature back.
    fn answer(self, body: &[u8]) -> Result<Step, Refused> {
        let (Ok(client), Ok(signature)) = (
            body.get(..4)
                .unwrap_or_default()
                .try_into()
                .map(u32::from_le_bytes),
            body.get(4..)
                .unwrap_or_default()
                .try_into()
                .map(u64::from_le_bytes),
        ) else {
            return Err(Refused::Length(body.len()));
        };
        let secret = g_pow_x_mod_p(client, self.private, self.prime);
        let handshake = Blowfish::new(&calc_key(secret, self.public, client))
            .expect("the handshake key has eight bytes");
        let sign = |first: u32, second: u32| {
            let mut value = BytesMut::from(&calc_challenge(secret, first, second)[..]);
            handshake.encrypt(&mut value);
            u64::from_le_bytes(value[..8].try_into().expect("a challenge has eight bytes"))
        };
        if sign(client, self.public) != signature {
            return Err(Refused::Signature);
        }
        let session = key_transform_value(
            Bytes::copy_from_slice(&self.handshake_key.to_le_bytes()),
            secret,
            3,
        );
        let keys = SessionKeys {
            blowfish: Blowfish::new(&session).expect("the session key has eight bytes"),
            sequence_seed: self.sequence_seed,
            crc_seed: self.crc_seed,
        };
        let mut challenge = BytesMut::with_capacity(9);
        challenge.put_u8(CHALLENGE_FLAG);
        challenge.put_u64_le(sign(self.public, client));
        Ok(Step::Answer(
            Handshake::Challenged(Box::new(keys)),
            challenge.freeze(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The client's half, computed as `client/src/net/handshake.rs` does: its
    /// public value, the shared secret, and its signing function.
    struct Client {
        public: u32,
        server_public: u32,
        secret: u32,
        handshake_key: u64,
        blowfish: Blowfish,
    }

    impl Client {
        fn read(opening: &[u8]) -> Client {
            let u32_at = |at: usize| u32::from_le_bytes(opening[at..at + 4].try_into().unwrap());
            let handshake_key = u64::from_le_bytes(opening[17..25].try_into().unwrap());
            let (generator, prime, server_public) = (u32_at(25), u32_at(29), u32_at(33));
            let private = 0x0BAD_C0DE;
            let public = g_pow_x_mod_p(generator, private, prime);
            let secret = g_pow_x_mod_p(server_public, private, prime);
            let blowfish = Blowfish::new(&calc_key(secret, server_public, public)).unwrap();
            Client {
                public,
                server_public,
                secret,
                handshake_key,
                blowfish,
            }
        }

        fn sign(&self, first: u32, second: u32) -> u64 {
            let mut value = BytesMut::from(&calc_challenge(self.secret, first, second)[..]);
            self.blowfish.encrypt(&mut value);
            u64::from_le_bytes(value[..8].try_into().unwrap())
        }

        /// The 0x5000 answer: public value and the client's own signature.
        fn answer(&self) -> Vec<u8> {
            let signature = self.sign(self.public, self.server_public);
            [&self.public.to_le_bytes()[..], &signature.to_le_bytes()].concat()
        }
    }

    #[test]
    fn a_client_with_the_shared_secret_ends_with_the_same_session_key() {
        let (handshake, opening) = Handshake::open();
        // 0x0E and 1 + 8 + 8 + 20 bytes: what the client requires of it.
        assert_eq!((opening[0], opening.len()), (0x0E, 37));
        let client = Client::read(&opening);

        let Ok(Step::Answer(handshake, challenge)) = handshake.receive(0x5000, &client.answer())
        else {
            panic!("the answer is accepted");
        };
        let expected = client.sign(client.server_public, client.public);
        assert_eq!(
            challenge[..],
            [&[0x10][..], &expected.to_le_bytes()].concat()
        );

        let Ok(Step::Done(keys)) = handshake.receive(0x9000, &[]) else {
            panic!("0x9000 completes it");
        };
        let handshake_key = Bytes::copy_from_slice(&client.handshake_key.to_le_bytes());
        let session = key_transform_value(handshake_key, client.secret, 3);
        let (mut theirs, mut ours) = ([7u8; 8], [7u8; 8]);
        Blowfish::new(&session).unwrap().encrypt(&mut theirs);
        keys.blowfish.encrypt(&mut ours);
        assert_eq!(theirs, ours);
    }

    #[test]
    fn a_signature_without_the_shared_secret_is_refused() {
        let (handshake, opening) = Handshake::open();
        let mut answer = Client::read(&opening).answer();
        answer[11] ^= 1;
        assert!(matches!(
            handshake.receive(0x5000, &answer),
            Err(Refused::Signature)
        ));
    }

    #[test]
    fn an_answer_of_another_length_is_refused() {
        for len in [11, 13] {
            let (handshake, _) = Handshake::open();
            let refused = handshake.receive(0x5000, &vec![0; len]);
            assert!(matches!(refused, Err(Refused::Length(l)) if l == len));
        }
    }

    #[test]
    fn the_steps_come_in_their_order() {
        let (handshake, _) = Handshake::open();
        assert!(matches!(
            handshake.receive(0x9000, &[]),
            Err(Refused::Opcode(0x9000))
        ));
    }
}

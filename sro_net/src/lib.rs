//! The Silkroad wire codec, shared by the client and the server (ADR 0013 §4):
//! framing, Blowfish, the CRC and sequence bytes, and the security state they
//! share. Nothing here does I/O; the handshake that fills the security state
//! lives with each role.

pub mod blowfish;
pub mod codec;
pub mod crc;
pub mod frame;
pub mod security;
pub mod sequence;

pub mod academy;
pub mod alchemy;
pub mod barena;
pub mod character_data;
pub mod chat;
pub(crate) mod cursor;
pub mod exchange;
pub mod guild;
pub mod guild_leadership;
pub mod guild_storage;
pub mod guild_union;
pub mod guild_war;
pub mod ingame;
pub mod inventory;
pub mod job;
pub mod lobby;
pub mod mail;
pub mod party;
pub mod pet;
pub mod quest;
pub mod siege;
pub mod stall;
pub mod storage;

use bevy::prelude::Message;
use bytes::{Buf, BufMut, Bytes, BytesMut};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

pub mod prelude {
    pub use crate::agent::academy::*;
    pub use crate::agent::alchemy::*;
    pub use crate::agent::barena::*;
    pub use crate::agent::character_data::ClientSettingsSave;
    pub use crate::agent::chat::*;
    pub use crate::agent::exchange::*;
    pub use crate::agent::guild::*;
    pub use crate::agent::guild_leadership::*;
    pub use crate::agent::guild_storage::*;
    pub use crate::agent::guild_union::*;
    pub use crate::agent::guild_war::*;
    pub use crate::agent::ingame::*;
    pub use crate::agent::inventory::*;
    pub use crate::agent::job::*;
    pub use crate::agent::lobby::*;
    pub use crate::agent::mail::*;
    pub use crate::agent::party::*;
    pub use crate::agent::pet::*;
    pub use crate::agent::quest::*;
    pub use crate::agent::siege::*;
    pub use crate::agent::stall::*;
    pub use crate::agent::storage::*;
    pub use crate::agent::*;
}

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct AgentLoginRequest {
    pub token: u32,
    pub username: String,
    pub password: String,
    pub content_id: u8,
    pub mac_address: [u8; 6],
}

/// 0xA103 — the agent server's answer to [`AgentLoginRequest`].
///
/// Three forms are captured, measured with `dump_census` over 99 dump roots
/// (945 bodies, 69 of them spoken to the **original** client):
///
/// | frames | body | meaning |
/// |---|---|---|
/// | 731 (37 original) | `01` | success, nothing else |
/// | 196 (**32 original**) | `01` + `03 00` | success **plus a `u16`** |
/// | 13 | `02` + code | failure, `error_code` |
///
/// The `u16` is read because the wire carries it, not because we know what it
/// is: it is `3` in every one of the 196 frames, so `[U]` — a constant of this
/// corpus is not an invariant. Without it those frames decoded to "success" and
/// left two bytes over, which is how the census found them.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct AgentLoginResponse {
    pub result: u8,
    // Decoded as a raw u8, not a typed enum: an unknown/new error code must
    // never crash the client. A live vSRO server emitted code 3 (already
    // connected, on an admin re-login while a prior session lingered), which the
    // old typed enum had no variant for and panicked the deserializer on.
    pub error_code: Option<u8>,
    /// The trailing `u16` of the long success form. `[U]`.
    pub extra: Option<u16>,
}

impl TryFrom<Bytes> for AgentLoginResponse {
    type Error = SerializationError;
    fn try_from(mut value: Bytes) -> Result<Self, SerializationError> {
        if value.is_empty() {
            return Err(SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "0xA103 body is empty",
            )));
        }
        let result = value.get_u8();
        let mut out = AgentLoginResponse {
            result,
            error_code: None,
            extra: None,
        };
        if result == 2 {
            if value.is_empty() {
                return Err(SerializationError::IoError(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "0xA103 failure without its error code",
                )));
            }
            out.error_code = Some(value.get_u8());
        } else if value.len() >= 2 {
            out.extra = Some(value.get_u16_le());
        }
        Ok(out)
    }
}

impl From<AgentLoginResponse> for Bytes {
    fn from(p: AgentLoginResponse) -> Self {
        let mut buf = BytesMut::with_capacity(3);
        buf.put_u8(p.result);
        if let Some(code) = p.error_code {
            buf.put_u8(code);
        }
        if let Some(extra) = p.extra {
            buf.put_u16_le(extra);
        }
        buf.freeze()
    }
}

impl ByteSize for AgentLoginResponse {
    fn byte_size(&self) -> usize {
        1 + self.error_code.map_or(0, |_| 1) + self.extra.map_or(0, |_| 2)
    }
}

/// Human-readable name for an agent-auth error code (`AgentLoginResponse` with
/// `result == 2`). Kept as a lookup rather than a wire enum so unknown codes
/// degrade to "unknown" instead of failing deserialization.
pub fn describe_agent_auth_error(code: u8) -> &'static str {
    match code {
        3 => "already connected",
        4 => "server full",
        5 => "IP limit reached",
        _ => "unknown error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three captured `0xA103` forms. The middle one is why `extra` exists:
    /// 196 frames, 32 of them from the original client, all carrying `u16 3`
    /// after a success byte.
    #[test]
    fn the_captured_forms_round_trip() {
        for (wire, result, code, extra) in [
            (&b"\x01"[..], 1u8, None, None),
            (&b"\x01\x03\x00"[..], 1, None, Some(3u16)),
            (&b"\x02\x03"[..], 2, Some(3u8), None),
        ] {
            let bytes = Bytes::from_static(wire);
            let decoded = AgentLoginResponse::try_from(bytes.clone()).expect("decodes");
            assert_eq!(decoded.result, result, "{wire:?}");
            assert_eq!(
                (decoded.error_code, decoded.extra),
                (code, extra),
                "{wire:?}"
            );
            assert_eq!(decoded.byte_size(), bytes.len(), "{wire:?}");
            assert_eq!(Bytes::from(decoded), bytes, "{wire:?}");
        }
        // a failure without its code is an error, not a silent success
        assert!(AgentLoginResponse::try_from(Bytes::from_static(b"\x02")).is_err());
    }
}

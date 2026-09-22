use std::net::Ipv4Addr;

use bevy::prelude::Message;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// `0x6100 CLIENT_GATEWAY_PATCH_REQUEST` — the version check the **original**
/// launcher sends first on the gateway connection, before the shard list.
///
/// The body of the original v1.208 client is
/// `16 | 0900 "SR_Client" | d0 00 00 00` = locale `0x16` (22, vSRO), a
/// u16-length-prefixed module name, then the build version as a `u32`
/// (0xD0 = 208). It is encrypted on the wire — `0x6100` is in the
/// outbound-encryption allowlist (`client/src/net/frame.rs`).
///
/// OpenRoad's own client does not send it (its launcher does an `SV.T`
/// preflight instead), so this exists to *read* what a real client asks and to
/// let a test peer answer it. No in-tree sender yet.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PatchRequest {
    /// Region/locale byte the gateway gates content on (`0x16` = 22 for vSRO).
    pub locale: u8,
    /// Module name the client compiles in — `"SR_Client"` on the v1.208
    /// client.
    pub module_name: String,
    /// Client build number; `208` on the v1.208 client.
    pub version: u32,
}

/// `0xA100 SERVER_GATEWAY_PATCH_RESPONSE` — the launcher's go/no-go verdict.
///
/// `result == 1` means "up to date" and carries **no further body**: one byte
/// is the whole packet. `result == 2` carries a `PatchErrorCode`, and only code
/// `2` ("update available") is followed by the download-server triple and the
/// file list.
///
/// The launcher branches on exactly `1` (proceed) and `2` (version incorrect)
/// and reads nothing more in the success case.
///
/// **Minimal on purpose**: the `error_code == 2` file list (a `has-more`
/// sequence of id/name/path/size/packed entries) is *not* modelled. We never
/// send it, and modelling it would be guesswork. A real `result == 2` with
/// `error_code == 2` therefore deserializes only as far as the triple.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PatchResponse {
    /// `1` = up to date, `2` = rejected (see [`PatchError`]).
    pub result: u8,
    #[sro_packet(when = "result == 0x02")]
    pub error: Option<PatchError>,
}

/// The `result == 2` body. `error_code` is a `PatchErrorCode`: `1` invalid
/// version, `2` update available (the only code with a payload), `3` gateway
/// not in service, `4` abnormal module, `5` patch disabled. Kept a raw `u8` so
/// a code we have no name for cannot fail deserialization — the same reasoning
/// as [`crate::login::LoginError`].
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PatchError {
    pub error_code: u8,
    #[sro_packet(when = "error_code == 0x02")]
    pub download_server_ip: Option<String>,
    #[sro_packet(when = "error_code == 0x02")]
    pub download_server_port: Option<u16>,
    #[sro_packet(when = "error_code == 0x02")]
    pub latest_version: Option<u32>,
}

/// `0x6104 CLIENT_GATEWAY_NOTICE_REQUEST` — the launcher's news request, the
/// packet it sends a few milliseconds after the patch verdict. Body: one
/// content-id byte, `0x16` on the v1.208 client — the same locale byte the
/// patch request carries.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct NoticeRequest {
    pub content_id: u8,
}

/// `0xA104 SERVER_GATEWAY_NOTICE_RESPONSE` — the launcher's news list, and the
/// packet it blocks on: with the notice service dead the launcher keeps
/// keepaliving forever and never offers its Start button.
///
/// The layout is `u8 noticeCount`, then per notice a u16-length-prefixed
/// subject and article followed by six `u16` date fields (year, month, day,
/// hour, minute, second) and a `u32` nanosecond.
///
/// **Only the count is modelled**, on purpose and twice over: our derive cannot
/// express a counted list of *structs* (`sro_macro_derive` rejects nested
/// collection-like types), and the only answer we ever *send* is the empty one
/// — `noticeCount = 0`, a single byte, which is the honest answer for a server
/// that has no notices. A real non-empty `0xA104` therefore decodes to its
/// count with the entries left unread; OpenRoad's own client never sends
/// `0x6104`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct NoticeResponse {
    pub notice_count: u8,
}

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct ShardListRequest;

/// 0xA106 — the gateway's answer to the farm ping: a `u8`-counted list of
/// farms, nothing else. The original parses it inline in the login-UI pump as
/// a plain loop: it reads the count, then each entry's id and address, so a
/// count of `0` is an empty list and the packet ends there — there is no
/// success flag and no error code.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ShardListPingResponse {
    pub farms: Vec<Farm>,
}

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct Farm {
    pub id: u8,
    /// Four octets in wire order: the original's dotted-quad formatting
    /// prints the first wire byte as the first octet.
    pub ip: Ipv4Addr,
}

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct ShardListPingRequest;

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct ShardListResponse {
    #[sro_packet(list_type = "has-more")]
    pub farms: Vec<FarmDetails>,
    #[sro_packet(list_type = "has-more")]
    pub shards: Vec<Shard>,
}

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct FarmDetails {
    pub id: u8,
    pub name: String,
}

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct Shard {
    pub id: u16,
    pub name: String,
    pub online_count: u16,
    pub capacity: u16,
    pub is_operating: bool,
    pub farm_id: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// A real answer: `01` count, `14` farm id, `7f 00 00 01` = 127.0.0.1 —
    /// the first wire byte is the first octet.
    const CAPTURED: &[u8] = &[0x01, 0x14, 0x7f, 0x00, 0x00, 0x01];

    #[test]
    fn captured_single_farm_roundtrips() {
        let decoded =
            ShardListPingResponse::try_from(Bytes::from_static(CAPTURED)).expect("decodes");
        assert_eq!(
            decoded,
            ShardListPingResponse {
                farms: vec![Farm {
                    id: 0x14,
                    ip: Ipv4Addr::new(127, 0, 0, 1),
                }],
            }
        );
        let encoded: Bytes = decoded.into();
        assert_eq!(encoded.as_ref(), CAPTURED);
    }

    #[test]
    fn a_two_farm_body_decodes_to_two_entries() {
        // Synthetic: a server with a single farm is exactly why the old
        // bool-plus-optional model round-tripped.
        let body = Bytes::from_static(&[
            0x02, 0x14, 0x7f, 0x00, 0x00, 0x01, 0x15, 0xc0, 0xa8, 0x01, 0x0a,
        ]);
        let decoded = ShardListPingResponse::try_from(body).expect("decodes");
        assert_eq!(decoded.farms.len(), 2);
        assert_eq!(decoded.farms[1].id, 0x15);
        assert_eq!(decoded.farms[1].ip, Ipv4Addr::new(192, 168, 1, 10));
    }

    #[test]
    fn a_zero_count_is_an_empty_list_not_an_error_code() {
        let decoded =
            ShardListPingResponse::try_from(Bytes::from_static(&[0x00])).expect("decodes");
        assert!(decoded.farms.is_empty());
    }
}

use bevy::prelude::Message;
use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CharacterSelectionActionRequest {
    pub action: CharacterSelectionAction,
    /// Actions that target an existing slot by name (Delete, CheckName,
    /// Restore) carry just the name — identical wire shape, no presence flag.
    /// Create carries its name inside [`CharacterCreate`] instead.
    ///
    /// The wire form of every name-carrying action is `action` + u16 LE
    /// length + ASCII name, e.g. `03 0700 506c6179657231` and
    /// `05 0700 506c6179657231` (both `Player1`), answered by `0xB007 03 01` /
    /// `05 01`. The shape is identical across actions 3, 4 and 5. A `Restore`
    /// left out of this gate would send a bare `[05]` with the name dropped.
    #[sro_packet(
        when = "action == CharacterSelectionAction::Delete || action == CharacterSelectionAction::CheckName || action == CharacterSelectionAction::Restore"
    )]
    pub name: Option<String>,
    /// New-character payload, present only for `Create`.
    #[sro_packet(when = "action == CharacterSelectionAction::Create")]
    pub create: Option<CharacterCreate>,
}

/// New-character creation body (0x7007 `action == Create`).
///
/// The original v1.188 client creates the character `Player001` as:
///
/// ```text
/// 01 0900 506c61796572303031 73070000 22 350e0000 360e0000 370e0000 300e0000
/// ^1 "Player001" (u16 LE len)  1907   0x22  3637     3638     3639     3632
/// ```
///
/// i.e. the name, the starter body model, the body scale, then a fixed set of
/// four starter-item ref-obj-ids — **not** a length-prefixed list, exactly the
/// shape below. The weapon is the **last** u32: moving only the weapon
/// selector leaves the first three ids at 3637/3638/3639 while the fourth
/// walks 3632 (step 1, info panel "Sword ... can wear a shield") -> 3634
/// (step 3) -> 3636 (step 5, "Bow ... cannot wear a shield"). Five selector
/// steps map to five consecutive ref-obj-ids 3632..=3636, so the layout is
/// three armour pieces and then the weapon.
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CharacterCreate {
    /// Candidate character name.
    pub name: String,
    /// Starter body `CharacterData.txt` ref-obj-id (selects race/gender/model).
    pub ref_obj_id: u32,
    /// Body scale — **two nibbles, not one number**: low nibble = Height,
    /// high nibble = Volume, each 0..4 with 2 as the factory middle, so the
    /// default frame carries `0x22`. Moving only Height one step up gives
    /// `0x23`, moving only Volume one step up gives `0x32`. Height's lower
    /// stop (slider fully left) is `0x20` and its upper stop (slider fully
    /// right) is `0x24`, so the 0..4 range is pinned at both ends:
    /// `0x20`/`0x22`/`0x23`/`0x24` for Height plus `0x32` for Volume. Kept as
    /// a plain `u8` because that is the wire type; the
    /// split belongs to the UI layer.
    pub scale: u8,
    /// Starter chest-armor ref-obj-id.
    pub chest: u32,
    /// Starter leg-armor ref-obj-id.
    pub pants: u32,
    /// Starter boots ref-obj-id.
    pub boots: u32,
    /// Starter weapon ref-obj-id.
    pub weapon: u32,
}

#[derive(Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
#[repr(u8)]
pub enum CharacterSelectionAction {
    #[sro_packet(value = 1)]
    Create,
    #[sro_packet(value = 2)]
    List,
    #[sro_packet(value = 3)]
    Delete,
    #[sro_packet(value = 4)]
    CheckName,
    #[sro_packet(value = 5)]
    Restore,
    #[sro_packet(value = 9)]
    ShowJobSpread,
    #[sro_packet(value = 0x10)]
    AssignJob,
}

/// 0xB007 — answer to [`CharacterSelectionActionRequest`].
///
/// Idea of the two gates: the frame is `action | result` and *both* tails are
/// conditional, never both present. A success answer for Create/Delete/Restore
/// is the bare two bytes: the original client receives `01 01`, `03 01` and
/// `05 01` and nothing else, which is why only `List` reads a body on
/// `result == 1`. The error code hangs on `result == 2` and is a u16 LE, e.g.
/// `04 02 1004` = `0x0410` for action `04`, and `01 02 1004` for a rejected
/// Create.
///
/// The failure arm of actions `03` and `05` cannot be provoked from the GUI
/// (the Restore button only exists for a delete-scheduled character), but it
/// has the same shape: `05 02 1904` (Restore of a character that is not
/// delete-scheduled, `0x0419`) and `03 02 0b04` (Delete of a name that does
/// not exist, `0x040B`). So the u16 tail is unconditional for `result == 2`
/// across every action the client sends — no optional read needed.
///
/// **Consumers must expect duplicates.** A failed Create can arrive **twice,
/// byte-identical**, 20 ms apart. That is the server's behaviour, not a
/// framing artefact, so
/// decoding stays one-frame-one-message here; anything that reacts *once* per
/// user action (opening a dialog, playing an error sound) has to debounce on
/// the consuming side. That debounce is a UI concern and deliberately not
/// modelled in `packets`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CharacterSelectionActionResponse {
    pub action: CharacterSelectionAction,
    pub result: u8,
    #[sro_packet(when = "result == 1 && action == CharacterSelectionAction::List")]
    pub characters: Option<CharacterData>,
    #[sro_packet(when = "result == 2")]
    pub error_code: Option<u16>,
}

#[derive(Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CharacterData {
    pub count: u8,
    #[sro_packet(list_type = "by-size-field", size_field = "count")]
    pub characters: Vec<LobbyCharacter>,
}
#[derive(Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct LobbyCharacter {
    pub ref_obj_id: u32,
    pub name: String,
    pub scale: u8,
    pub level: u8,
    pub exp_offset: u64,
    pub str: u16,
    pub int: u16,
    pub stat_points: u16,
    pub hp: u32,
    pub mp: u32,
    pub is_deleting: bool,
    /// Remaining time of the soft-deletion window, **in MINUTES** (u32 LE),
    /// present only while `is_deleting`. A list response taken right after a
    /// delete is exactly 4 bytes longer (253 -> 257 B), the flag byte flips
    /// `00` -> `01`, and the four new bytes are `60 27 00 00` = 10080 =
    /// 7 * 24 * 60 — the 7-day window in minutes, not seconds.
    #[sro_packet(when = "is_deleting")]
    pub deletion_time: Option<u32>,
    pub guild_member_class: u8,
    pub is_guild_rename_required: bool,
    #[sro_packet(when = "is_guild_rename_required")]
    pub current_guild_name: Option<String>,
    pub academy_member_class: u8,
    pub char_items: Vec<LobbyItem>,
    pub avatar_items: Vec<LobbyItem>,
}

#[derive(Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct LobbyItem {
    pub id: u32,
    pub plus: u8,
}

/// 0x7001 — asks the agent server to enter the world with the given
/// character. The server answers with [`CharacterJoinResponse`] and then
/// starts the ingame spawn sequence (character data, position, ... — not
/// implemented yet).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CharacterJoinRequest {
    pub character_name: String,
}

/// 0xB001 — result of [`CharacterJoinRequest`] (1 = ok, 2 = error).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug)]
pub struct CharacterJoinResponse {
    pub result: u8,
    #[sro_packet(when = "result == 2")]
    pub error: Option<u16>,
}

/// The lobby error catalogue the original resolves a `0xB007`/`0xB001` error
/// code through.
///
/// Idea: this lives in `packets` rather than in one screen because **one**
/// dispatcher serves the whole lobby in the original — login, select and the
/// create screen all hand their error code to the same jump table. A
/// per-screen copy would drift. The mapping returns only `(textuisystem key,
/// shipped English fallback)`; resolving the key against the loaded
/// `textuisystem.txt` stays in the client, so `packets` keeps no dependency on
/// client string tables.
///
/// Shape of the table, read from the original's dispatcher: `0x0401` returns
/// before selecting any string ([`LobbyErrorText::Silent`]); otherwise
/// `idx = code - 0x402` indexes a 0x17-entry table. The arms reach the message
/// box through **two** helpers: one shows the row as-is, the other appends the
/// raw code as `(S<code>)` ([`LobbyErrorText::code_suffix`]). Codes past the
/// table (`> 0x0418`, which includes `0x0419`) take the
/// default arm, which passes an empty key into the appending helper: the user
/// sees the suffix and nothing else ([`LobbyErrorText::CodeOnly`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LobbyErrorText {
    /// `0x0401` — the dispatcher returns before selecting any string.
    Silent,
    /// A `textuisystem` row.
    Text {
        key: &'static str,
        fallback: &'static str,
        /// The arm goes through the appending helper.
        code_suffix: bool,
    },
    /// `code > 0x0418`: no jump-table arm, so the key the original looks up is
    /// the empty string and only the appended code remains.
    CodeOnly,
}

pub fn lobby_error_text(code: u16) -> LobbyErrorText {
    // The in-range default; it is *not* what out-of-range codes get.
    let generic = LobbyErrorText::Text {
        key: "UIO_MSG_ERROR_SEVER_CONNECT",
        fallback: "Failed to connect to server.",
        code_suffix: true,
    };
    // `keyed!(key, fallback, suffix)` keeps the 13 arms one line each, so the
    // table stays readable next to the jump table it transcribes.
    macro_rules! keyed {
        ($key:expr, $fallback:expr, $suffix:expr) => {
            LobbyErrorText::Text {
                key: $key,
                fallback: $fallback,
                code_suffix: $suffix,
            }
        };
    }
    match code {
        0x0401 => LobbyErrorText::Silent,
        // jt[1], appending helper
        0x0403 => keyed!(
            "UIO_SMERR_INVALID_CHARGEN_INFO",
            "Failed to create a character. Please try to connect again.",
            true
        ),
        // jt[2], plain helper
        0x0404 => keyed!(
            "UIO_MSG_ERROR_CHARACTER_SELECTWEAPON",
            "Select a Weapon.",
            false
        ),
        // jt[3], plain helper
        0x0405 => keyed!(
            "UIO_MSG_ERROR_CHARACTER_OVER_3",
            "A maximum of %d characters can be created.",
            false
        ),
        // jt[4], appending helper
        0x0406 => keyed!(
            "UIO_SMERR_FAILED_TO_CREATE_CHARACTER",
            "Failed to create a character. Please try to connect again.",
            true
        ),
        // jt[5], appending helper
        0x0409 => keyed!(
            "UIO_SMERR_CANT_FIND_GAMESERVER",
            "The server is not running.. Please try to connect again later.",
            true
        ),
        // jt[6], plain helper
        0x040c => keyed!(
            "UIO_MSG_ERROR_CHARACTER_NAME_STRING",
            "Exceeded the letter limit. \nOnly 12 English letters are available.[Min., Max.]",
            false
        ),
        // jt[7], plain helper
        0x040d => keyed!(
            "UIO_SMERR_NOT_ALLOWED_CHARNAME",
            "Invalid character name.",
            false
        ),
        // jt[8], appending helper
        0x040f => keyed!(
            "UIO_SMERR_CANT_ACCESS_PARENT_SERVER",
            "The server is not running.. Please try to connect again later.",
            true
        ),
        // jt[9], plain helper.
        // Wire form of this one, byte for byte [V]: `packet_dump/proxy/0xb007.log`
        // holds `04021004` at 2026-08-21T11:50:05.944Z — subaction `04`,
        // result `02` (error), then the code as **u16 little endian**
        // `10 04` = `0x0410` = 1040. The neighbouring `0401` lines at 11:49:45
        // and 11:50:12 are the "name is free" answers and carry no code at all.
        // This does not contradict the 1027 of commit 4baa6924: that is
        // `0x0403` = `UIO_SMERR_INVALID_CHARGEN_INFO`, a *content* rejection of
        // the Create body, and it sits in this same table two arms above.
        0x0410 => keyed!("UIO_MSG_ERROR_ID", "This ID already exists.", false),
        // jt[10], plain helper.
        // Full shipped text [V] (`Media.pk2/textuisystem.txt`, column 9); the
        // short form that stood here was a truncation, not the original string.
        0x0411 => keyed!(
            "UIO_MSG_ERROR_OVERLAP",
            "This user is already connected. The user may still be connected \
             because of an error that forced the game to close. Please try \
             again in 5 minutes.",
            false
        ),
        // jt[11], appending helper
        0x0412 => keyed!(
            "UIO_SMERR_FAILED_TO_CREATE_NEW_USER",
            "Failed to create a character. Please try to connect again.",
            true
        ),
        // jt[12], plain helper
        0x0414 => keyed!(
            "UIO_SMERR_MAX_USER_EXCEEDED",
            "Cannot connect to the server because the server reached its capacity.",
            false
        ),
        // jt[13], appending helper
        0x0415 => keyed!("UIO_SMERR_FAILED_TO_ENTERLOBBY", "Login failed", true),
        // Everything the index array maps to `jt[0]`.
        0x0402 | 0x0407 | 0x0408 | 0x040a | 0x040b | 0x040e | 0x0413 | 0x0416..=0x0418 => generic,
        _ => LobbyErrorText::CodeOnly,
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    // `when`-conditional fields must serialize without the presence flag a
    // plain Option carries — these tests pin the exact wire bytes.
    #[test]
    fn delete_request_carries_name_without_flag_byte() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Delete,
            name: Some("Foo".to_string()),
            create: None,
        };
        assert_eq!(request.byte_size(), 6);
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x03, 0x03, 0x00, b'F', b'o', b'o']);
    }

    /// #202: Restore (action 5) carries the name in exactly the same shape as
    /// Delete — one string for Delete/CheckName/Restore alike.
    #[test]
    fn restore_request_carries_name_without_flag_byte() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Restore,
            name: Some("Foo".to_string()),
            create: None,
        };
        assert_eq!(request.byte_size(), 6);
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x05, 0x03, 0x00, b'F', b'o', b'o']);
    }

    #[test]
    fn list_request_is_a_single_action_byte() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::List,
            name: None,
            create: None,
        };
        assert_eq!(request.byte_size(), 1);
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x02]);
    }

    #[test]
    fn check_name_request_carries_name_without_flag_byte() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::CheckName,
            name: Some("Foo".to_string()),
            create: None,
        };
        assert_eq!(request.byte_size(), 6);
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x04, 0x03, 0x00, b'F', b'o', b'o']);
    }

    #[test]
    fn create_request_serializes_name_model_scale_and_starter_items() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Create,
            name: None,
            create: Some(CharacterCreate {
                name: "Foo".to_string(),
                ref_obj_id: 0x04030201,
                scale: 0x2A,
                chest: 0x11111111,
                pants: 0x22222222,
                boots: 0x33333333,
                weapon: 0x44444444,
            }),
        };
        assert_eq!(request.byte_size(), 27);
        let bytes: Bytes = request.into();
        assert_eq!(
            bytes.as_ref(),
            &[
                0x01, // action = Create
                0x03, 0x00, b'F', b'o', b'o', // name = "Foo"
                0x01, 0x02, 0x03, 0x04, // ref_obj_id (LE)
                0x2A, // scale
                0x11, 0x11, 0x11, 0x11, // chest
                0x22, 0x22, 0x22, 0x22, // pants
                0x33, 0x33, 0x33, 0x33, // boots
                0x44, 0x44, 0x44, 0x44, // weapon
            ]
        );
    }

    /// A real Create frame of the original v1.188 client, pinned byte for
    /// byte: `Player001` (body 1907, scale 0x22, HEAVY chest/pants/boots
    /// 3637/3638/3639 + Sword 3632). If the serializer ever drifts, this test
    /// fails against real bytes rather than against an assumption.
    #[test]
    fn create_request_matches_the_original_client_frame() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Create,
            name: None,
            create: Some(CharacterCreate {
                name: "Player001".to_string(),
                ref_obj_id: 1907,
                scale: 0x22,
                chest: 3637,
                pants: 3638,
                boots: 3639,
                weapon: 3632,
            }),
        };
        // The log line, written out in two-hex-digit pairs.
        let wire: &[u8] = &[
            0x01, // action = Create
            0x09, 0x00, // name length, u16 LE
            b'P', b'l', b'a', b'y', b'e', b'r', b'0', b'0', b'1', //
            0x73, 0x07, 0x00, 0x00, // ref_obj_id 1907
            0x22, // scale
            0x35, 0x0e, 0x00, 0x00, // chest 3637
            0x36, 0x0e, 0x00, 0x00, // pants 3638
            0x37, 0x0e, 0x00, 0x00, // boots 3639
            0x30, 0x0e, 0x00, 0x00, // weapon 3632
        ];
        assert_eq!(request.byte_size(), wire.len());
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), wire);
    }

    /// The real Delete frame of the original v1.188 client, byte for byte:
    /// `03 0700 506c6179657231` (action 3, u16 LE len 7, `Player1`).
    #[test]
    fn delete_request_matches_the_original_client_frame() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Delete,
            name: Some("Player1".to_string()),
            create: None,
        };
        let wire: &[u8] = &[
            0x03, // action = Delete
            0x07, 0x00, // name length, u16 LE
            b'P', b'l', b'a', b'y', b'e', b'r', b'1',
        ];
        assert_eq!(request.byte_size(), wire.len());
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), wire);
    }

    /// The Restore of the very character deleted above:
    /// `05 0700 506c6179657231` — identical shape, only the action byte
    /// differs.
    #[test]
    fn restore_request_matches_the_original_client_frame() {
        let request = CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Restore,
            name: Some("Player1".to_string()),
            create: None,
        };
        let wire: &[u8] = &[
            0x05, // action = Restore
            0x07, 0x00, // name length, u16 LE
            b'P', b'l', b'a', b'y', b'e', b'r', b'1',
        ];
        assert_eq!(request.byte_size(), wire.len());
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), wire);
    }

    /// The scale nibbles, pinned against two real frames: one with only the
    /// Height slider moved one step (`0x23`), one with only the Volume slider
    /// moved (`0x32`). Everything else in both frames is the factory default,
    /// so the two differ *only* in name and scale byte — which is what
    /// "low nibble = Height, high nibble = Volume" rests on.
    #[test]
    fn create_request_matches_the_two_live_scale_control_runs() {
        let frame = |name: &str, scale: u8| -> Bytes {
            CharacterSelectionActionRequest {
                action: CharacterSelectionAction::Create,
                name: None,
                create: Some(CharacterCreate {
                    name: name.to_string(),
                    ref_obj_id: 1907,
                    scale,
                    chest: 3637,
                    pants: 3638,
                    boots: 3639,
                    weapon: 3632,
                }),
            }
            .into()
        };
        let expected = |name: &[u8; 7], scale: u8| -> Vec<u8> {
            let mut out = vec![0x01, 0x07, 0x00];
            out.extend_from_slice(name);
            out.extend_from_slice(&[0x73, 0x07, 0x00, 0x00]);
            out.push(scale);
            out.extend_from_slice(&[0x35, 0x0e, 0x00, 0x00]);
            out.extend_from_slice(&[0x36, 0x0e, 0x00, 0x00]);
            out.extend_from_slice(&[0x37, 0x0e, 0x00, 0x00]);
            out.extend_from_slice(&[0x30, 0x0e, 0x00, 0x00]);
            out
        };
        // Run A: Height +1 -> low nibble 3.
        assert_eq!(
            frame("Player2", 0x23).as_ref(),
            expected(b"Player2", 0x23).as_slice()
        );
        // Run B: Volume +1 -> high nibble 3.
        assert_eq!(
            frame("Player1", 0x32).as_ref(),
            expected(b"Player1", 0x32).as_slice()
        );
    }

    /// The success answers carry **no tail at all** for all three actions
    /// (`01 01`, `03 01`, `05 01`). This pins the deserializer gates:
    /// `characters` only for `List`, `error_code` only for
    /// `result == 2`, so a two-byte frame must decode with both `None` and
    /// leave nothing unread.
    #[test]
    fn action_response_success_has_no_tail_for_create_delete_and_restore() {
        for (raw, action) in [
            ([0x01u8, 0x01], CharacterSelectionAction::Create),
            ([0x03, 0x01], CharacterSelectionAction::Delete),
            ([0x05, 0x01], CharacterSelectionAction::Restore),
        ] {
            let response =
                CharacterSelectionActionResponse::try_from(Bytes::copy_from_slice(&raw)).unwrap();
            assert_eq!(response.action, action);
            assert_eq!(response.result, 1);
            assert!(response.characters.is_none());
            assert!(response.error_code.is_none());
        }
    }

    /// The error arm against a real failure frame: `04 02 1004` — CheckName,
    /// result 2, code `0x0410` as u16 LE. A duplicate-name Create is answered
    /// `01 02 1004`, the same shape, so the `result == 2` gate holds for
    /// action 1 as well. The server can send that frame twice, which is why
    /// decoding is idempotent and debouncing is left to the consumer.
    #[test]
    fn create_failure_response_reads_the_live_error_code() {
        let raw = Bytes::from_static(&[0x01, 0x02, 0x10, 0x04]);
        let first = CharacterSelectionActionResponse::try_from(raw.clone()).unwrap();
        let second = CharacterSelectionActionResponse::try_from(raw).unwrap();
        for response in [first, second] {
            assert_eq!(response.action, CharacterSelectionAction::Create);
            assert_eq!(response.result, 2);
            assert_eq!(response.error_code, Some(0x0410));
            assert!(response.characters.is_none());
        }
    }

    /// The last two actions whose *failure* arm was an assumption are now
    /// measured too — raw frames from our own clientless bot on account
    /// `<account 1>`, in the lobby, against the operator's own server (2026-08-22,
    /// a local packet dump; the requests are in
    /// a local packet dump):
    ///
    /// | c2s | s2c | meaning |
    /// |---|---|---|
    /// | `05 0400 "Devi"` (06:11:33.239Z) | `05 02 1904` (06:11:33.287Z) | Restore of a character that is **not** delete-scheduled -> code `0x0419` |
    /// | `03 0800 "Nichtda1"` (06:11:45.253Z) | `03 02 0b04` (06:11:45.286Z) | Delete of a name that does not exist -> code `0x040B` |
    ///
    /// Why this matters: the `result == 2` gate reads a u16 unconditionally, so
    /// a server that answered a bare `05 02` would make the deserializer
    /// overrun. It does not — the tail is there for action 5 and 3 as well, and
    /// both frames are exactly 4 bytes. Neither code appears anywhere else in
    /// `docs/`; both fall into the generic arm of the original's dispatcher
    /// (`idx = code - 0x402` is only valid up to `0x16`, i.e. `0x418`), so the
    /// original shows `UIO_MSG_ERROR_SEVER_CONNECT` for them.
    #[test]
    fn restore_and_delete_failures_carry_the_error_code_too() {
        for (raw, action, code) in [
            (
                [0x05u8, 0x02, 0x19, 0x04],
                CharacterSelectionAction::Restore,
                0x0419u16,
            ),
            (
                [0x03, 0x02, 0x0b, 0x04],
                CharacterSelectionAction::Delete,
                0x040B,
            ),
        ] {
            let response =
                CharacterSelectionActionResponse::try_from(Bytes::copy_from_slice(&raw)).unwrap();
            assert_eq!(response.action, action);
            assert_eq!(response.result, 2);
            assert_eq!(response.error_code, Some(code));
            assert!(response.characters.is_none());
        }
    }

    #[test]
    fn action_response_error_reads_u16_code() {
        let response = CharacterSelectionActionResponse::try_from(Bytes::from_static(&[
            0x04, 0x02, 0x10, 0x04,
        ]))
        .unwrap();
        assert_eq!(response.action, CharacterSelectionAction::CheckName);
        assert_eq!(response.result, 2);
        assert_eq!(response.error_code, Some(0x0410));
        assert!(response.characters.is_none());
    }

    /// The weapon sits in the **last** u32, and the scale range is pinned at
    /// both ends. Both facts come from control runs at the original client: only
    /// the weapon selector moved and only the fourth id changed (3632 -> 3634
    /// -> 3636), and Height fully left/right produced `0x20`/`0x24`.
    #[test]
    fn create_request_puts_the_weapon_last_and_covers_both_scale_stops() {
        for (weapon, scale) in [(3632u32, 0x20u8), (3634, 0x22), (3636, 0x24)] {
            let request = CharacterSelectionActionRequest {
                action: CharacterSelectionAction::Create,
                name: None,
                create: Some(CharacterCreate {
                    name: "Player".to_string(),
                    ref_obj_id: 1907,
                    scale,
                    chest: 3637,
                    pants: 3638,
                    boots: 3639,
                    weapon,
                }),
            };
            let bytes: Bytes = request.into();
            let body = bytes.as_ref();
            // action + u16 len + 6 name bytes + u32 ref = 13, then the scale.
            assert_eq!(&body[..3], &[0x01, 0x06, 0x00]);
            assert_eq!(&body[9..13], &1907u32.to_le_bytes());
            assert_eq!(body[13], scale);
            assert_eq!(&body[14..18], &3637u32.to_le_bytes());
            assert_eq!(&body[18..22], &3638u32.to_le_bytes());
            assert_eq!(&body[22..26], &3639u32.to_le_bytes());
            // The weapon is the trailing u32 — nothing follows it.
            assert_eq!(&body[26..], &weapon.to_le_bytes());
        }
    }

    #[test]
    fn join_request_serializes_name() {
        let request = CharacterJoinRequest {
            character_name: "Foo".to_string(),
        };
        assert_eq!(request.byte_size(), 5);
        let bytes: Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x03, 0x00, b'F', b'o', b'o']);
    }

    #[test]
    fn join_response_success_has_no_error() {
        let response = CharacterJoinResponse::try_from(Bytes::from_static(&[0x01])).unwrap();
        assert_eq!(response.result, 1);
        assert!(response.error.is_none());
    }

    #[test]
    fn join_response_error_reads_code() {
        let response =
            CharacterJoinResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x04])).unwrap();
        assert_eq!(response.result, 2);
        assert_eq!(response.error, Some(0x0403));
    }

    /// One case per *class* of the dispatcher, not per code: the silent arm,
    /// an appending arm, a plain arm, the in-range default and the
    /// out-of-range default. The last one is the one that matters — it pins
    /// that a code the jump table has no arm for stays keyless instead of being
    /// dressed in a nearby string (`0x0419` is the real
    /// restore-not-scheduled code).
    #[test]
    fn the_lobby_error_table_keeps_its_five_classes_apart() {
        assert_eq!(lobby_error_text(0x0401), LobbyErrorText::Silent);
        assert_eq!(
            lobby_error_text(0x0415),
            LobbyErrorText::Text {
                key: "UIO_SMERR_FAILED_TO_ENTERLOBBY",
                fallback: "Login failed",
                code_suffix: true,
            }
        );
        assert_eq!(
            lobby_error_text(0x0410),
            LobbyErrorText::Text {
                key: "UIO_MSG_ERROR_ID",
                fallback: "This ID already exists.",
                code_suffix: false,
            }
        );
        // index entry 0x09 is 0 -> `jt[0]`
        for code in [0x0402u16, 0x0407, 0x040b, 0x0413, 0x0418] {
            assert_eq!(
                lobby_error_text(code),
                LobbyErrorText::Text {
                    key: "UIO_MSG_ERROR_SEVER_CONNECT",
                    fallback: "Failed to connect to server.",
                    code_suffix: true,
                },
                "{code:#06x} is an in-range hole and must keep the generic row"
            );
        }
        // 0x0419 is the first code past the 0x17-entry array
        assert_eq!(lobby_error_text(0x0419), LobbyErrorText::CodeOnly);
        assert_eq!(lobby_error_text(0xffff), LobbyErrorText::CodeOnly);
    }
}

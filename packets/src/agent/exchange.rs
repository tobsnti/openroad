//! Player-to-player exchange (trade) wire opcodes: the window's lifecycle
//! (0x3085–0x3088), the peer's staged gold/items (0x3089 / 0x308C) and the
//! confirm/approve/exit request-response pairs (0x708x / 0xB08x).
//!
//! Codes whose meaning is not established are passed through as plain numbers
//! rather than named.
//!
//! Two neighbours deliberately live elsewhere:
//! - the **invitation** (0x7081 / 0xB081) belongs to `docs/net-invite-0x3080.md`;
//! - **your own** staging is not an exchange opcode at all — it rides
//!   `0x7034`/`0xB034` sub-ops [`InventoryToExchange`], [`ExchangeToInventory`]
//!   and [`InventoryGoldToExchange`]
//!   ([`InventoryOperationRequest`](crate::agent::inventory::InventoryOperationRequest)).
//!
//! [`InventoryToExchange`]: crate::agent::inventory::InventoryOperationRequest::InventoryToExchange
//! [`ExchangeToInventory`]: crate::agent::inventory::InventoryOperationRequest::ExchangeToInventory
//! [`InventoryGoldToExchange`]: crate::agent::inventory::InventoryOperationRequest::InventoryGoldToExchange

use bevy::prelude::Message;
use bytes::Bytes;

use crate::agent::character_data::{InventoryItem, ItemClassResolver};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// 0x3085 — server → client: the trade window opened against `partner_unique_id`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeStarted {
    pub partner_unique_id: u32,
}

/// 0x3086 — server → client: the *partner* pressed confirm. Empty body.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangePlayerConfirmed;

/// 0x3087 — server → client: the trade went through. Empty body.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeCompleted;

/// 0x3088 — server → client: the trade (or a still-pending petition) was
/// called off, with the reason.
///
/// The original's parser reads nothing here, but the server always sends a
/// `u16`: every observed body is two bytes and never zero. The value is
/// little-endian like every other integer on this wire, so the byte pair
/// `2c18` is `0x182C`, the code the server uses when it terminates the trade
/// session; a big-endian reading (`0x2C18`) matches no known code.
///
/// A body shorter than two bytes never occurs and is rejected rather than
/// defaulted: a fabricated `reason = 0` would be indistinguishable from a real
/// code on screen.
///
/// See [`CANCEL_REASON_DECLINED`] and friends for the four known values.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeCanceled {
    pub reason: u16,
}

/// `0x1828` — the invitee pressed "Refuse" on the 0x3080 petition. Both sides
/// get the 0x3088 within a few milliseconds of each other; the inviter gets
/// **no** 0xB081 at all.
pub const CANCEL_REASON_DECLINED: u16 = 0x1828;

/// `0x182B` — the other participant left the game while the petition was still
/// pending. The cancel trails that client's 0x7005 logout **request** by a few
/// tens of milliseconds and precedes its 0xB005 ack, so it belongs to the
/// character teardown and is not a petition timeout.
pub const CANCEL_REASON_PARTNER_LEFT: u16 = 0x182B;

/// `0x182C` — somebody pressed exit (0x7084). The exiter also gets
/// `0xB084 01`; the peer gets no 0xB084 at all and learns about the end of the
/// trade from this 0x3088 alone.
pub const CANCEL_REASON_EXIT: u16 = 0x182C;

/// `0x181F` — protocol violation: a **second** 0x7082 from the same side. The
/// offender additionally gets `0xB082 02 1f18` and the server terminates the
/// trade for both sides.
pub const CANCEL_REASON_PROTOCOL_VIOLATION: u16 = 0x181F;

/// 0x3089 — server → client: the gold the *partner* has staged.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeGoldUpdate {
    /// Semantics unknown; always seen as `01`.
    pub unk_byte01: u8,
    pub gold: u64,
}

/// 0x308C — server → client: a staged item list. **Two shapes exist on the
/// wire**, and which one arrives is decided by whose list it is.
///
/// | shape | goes to | per entry |
/// |---|---|---|
/// | peer copy | the *other* side, at confirm time | `u8 slot_exchange, item` |
/// | self echo | the player who just staged | `u8 slot_inventory, u8 slot_exchange, item` |
///
/// For the same item the two bodies are byte-identical apart from that extra
/// leading `slot_inventory`, so the peer copy is one byte shorter than the
/// self echo (32 against 33 bytes for one equipment entry). Feeding a
/// self echo to the peer decoder therefore shifts every field by one byte,
/// which is why the shape must be chosen by the caller (it knows its own
/// unique id) rather than guessed from the length — the item block is
/// class-dependent and has no fixed width.
///
/// The list is kept as a raw tail for that same reason: an item record's width
/// is only known once its `ref_id` has been resolved through an itemdata
/// lookup, which the wire types cannot do. This mirrors how the 0xB034 pickup
/// payload is handled
/// ([`InventoryOperationResult::pickup_item`](crate::agent::inventory::InventoryOperationResult::pickup_item))
/// — decode on demand via [`Self::items`] (peer) or [`Self::own_items`] (self).
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ExchangeItemsUpdate {
    pub player_unique_id: u32,
    pub item_count: u8,
    /// `item_count` records; the entry layout depends on the shape above. See
    /// [`Self::items`] and [`Self::own_items`].
    pub tail: Bytes,
}

/// One entry of a **self** 0x308C echo: our own bag slot, the exchange-pane
/// slot it was staged into, and the item itself.
#[derive(Clone, Debug, PartialEq)]
pub struct StagedItem {
    /// Where the item still physically is — the trade only moves it on 0x3087.
    /// `item.slot` carries the same number, so a completed trade can clear the
    /// bag slot without a second lookup.
    pub slot_inventory: u8,
    /// The pane slot, 0-based in staging order: `00` for the first staged item,
    /// `01` for the second.
    pub slot_exchange: u8,
    pub item: InventoryItem,
}

impl ExchangeItemsUpdate {
    /// Decode the **peer copy** (`u8 slot_exchange, item` per entry), given an
    /// itemdata class resolver.
    ///
    /// The single slot byte is the *exchange* slot, so `item.slot` is a pane
    /// index (0-based, `00` then `01`), **not** the partner's bag slot, which
    /// this side never learns and does not need. Returns
    /// `None` if any record fails, because a partial list would silently
    /// misrepresent what the peer is offering.
    pub fn items(&self, resolver: &impl ItemClassResolver) -> Option<Vec<InventoryItem>> {
        let mut cursor = std::io::Cursor::new(self.tail.as_ref());
        let mut items = Vec::with_capacity(self.item_count as usize);
        for _ in 0..self.item_count {
            items.push(InventoryItem::read_with(&mut cursor, resolver).ok()?);
        }
        Some(items)
    }

    /// Decode the **self echo** (`u8 slot_inventory, u8 slot_exchange, item`).
    ///
    /// [`InventoryItem::read_with`] reads the slot byte as part of the record,
    /// so the inventory slot is re-prepended to the remaining bytes and the
    /// cursor is advanced by whatever the item parser consumed minus that one
    /// byte. That keeps a single item-record parser for both shapes instead of
    /// a second, drift-prone copy.
    pub fn own_items(&self, resolver: &impl ItemClassResolver) -> Option<Vec<StagedItem>> {
        let tail = self.tail.as_ref();
        let mut at = 0usize;
        let mut items = Vec::with_capacity(self.item_count as usize);
        for _ in 0..self.item_count {
            let slot_inventory = *tail.get(at)?;
            let slot_exchange = *tail.get(at + 1)?;
            let body = tail.get(at + 2..)?;

            let mut record = Vec::with_capacity(body.len() + 1);
            record.push(slot_inventory);
            record.extend_from_slice(body);
            let mut cursor = std::io::Cursor::new(record.as_slice());
            let item = InventoryItem::read_with(&mut cursor, resolver).ok()?;

            // `- 1` undoes the prepended slot byte; the rest is what the item
            // block really occupied on the wire.
            at += 2 + (cursor.position() as usize - 1);
            items.push(StagedItem {
                slot_inventory,
                slot_exchange,
                item,
            });
        }
        Some(items)
    }
}

impl TryFrom<Bytes> for ExchangeItemsUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let short = || {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "0x308C body too short",
            ))
        };
        let player_unique_id =
            u32::from_le_bytes(value.get(0..4).ok_or_else(short)?.try_into().unwrap());
        Ok(ExchangeItemsUpdate {
            player_unique_id,
            item_count: *value.get(4).ok_or_else(short)?,
            tail: value.slice(5..),
        })
    }
}

impl From<ExchangeItemsUpdate> for Bytes {
    fn from(p: ExchangeItemsUpdate) -> Self {
        let mut buf = bytes::BytesMut::new();
        bytes::BufMut::put_u32_le(&mut buf, p.player_unique_id);
        bytes::BufMut::put_u8(&mut buf, p.item_count);
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

/// 0x7082 — client → server: confirm (lock in) my side. Empty body.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeConfirmRequest;

/// 0x7083 — client → server: approve the trade. Empty body.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeApproveRequest;

/// 0x7084 — client → server: back out of the window.
///
/// The empty body is inferred rather than confirmed: the original has no
/// builder for this opcode. It matches its siblings and its ack carries only a
/// success flag.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct ExchangeExitRequest;

/// `result == 1` — the family-wide success byte, same as 0xB034's.
pub const EXCHANGE_RESULT_SUCCESS: u8 = 1;

/// `0x181B` — "you are not in a trade". Answers a 0x7082/0x7083/0x7084 sent
/// outside a session, as `021b18`. The same code answers the inventory staging
/// path on 0xB034.
pub const EXCHANGE_ERROR_NO_SESSION: u16 = 0x181B;

/// `0x181F` — protocol violation: a second 0x7082 from the same side. Unlike
/// [`EXCHANGE_ERROR_NO_SESSION`] this one is fatal — the server also sends
/// `0x3088 1f18` to both participants and the trade is gone. On the wire the
/// ack body is `021f18`.
pub const EXCHANGE_ERROR_PROTOCOL_VIOLATION: u16 = 0x181F;

/// The shape all three 0xB08x acks share: `u8 result` plus, on failure, a
/// `u16` error code.
///
/// A success is the single byte `01`; a failure is `02` followed by two bytes,
/// such as `021b18` or `021f18`. Distinguishing them matters: `0x181B` leaves
/// the trade untouched, `0x181F` has already destroyed it. `error` stays an
/// `Option<u16>` and is *not* interpreted here; codes other than the two named
/// ones are passed through as numbers.
macro_rules! exchange_ack {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Message, Clone, Debug, PartialEq)]
        pub struct $name {
            pub result: u8,
            pub error: Option<u16>,
        }

        impl $name {
            pub fn is_success(&self) -> bool {
                self.result == EXCHANGE_RESULT_SUCCESS
            }
        }

        impl TryFrom<Bytes> for $name {
            type Error = SerializationError;
            fn try_from(value: Bytes) -> Result<Self, SerializationError> {
                let result = *value.first().ok_or_else(|| {
                    SerializationError::IoError(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        concat!("empty ", stringify!($name), " body"),
                    ))
                })?;
                // A success carries no tail, so the code is read only on the
                // failure branch — and tolerantly, because an ack whose code we
                // cannot read is still an ack the UI must react to.
                let error = if result == EXCHANGE_RESULT_SUCCESS {
                    None
                } else {
                    value
                        .get(1..3)
                        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
                };
                Ok($name { result, error })
            }
        }

        impl From<$name> for Bytes {
            fn from(p: $name) -> Self {
                let mut buf = bytes::BytesMut::new();
                bytes::BufMut::put_u8(&mut buf, p.result);
                if p.result != EXCHANGE_RESULT_SUCCESS {
                    if let Some(error) = p.error {
                        bytes::BufMut::put_u16_le(&mut buf, error);
                    }
                }
                buf.freeze()
            }
        }
    };
}

exchange_ack!(
    /// 0xB082 — server → client: ack for [`ExchangeConfirmRequest`].
    ExchangeConfirmResponse
);

exchange_ack!(
    /// 0xB083 — server → client: ack for [`ExchangeApproveRequest`].
    ExchangeApproveResponse
);

exchange_ack!(
    /// 0xB084 — server → client: ack for [`ExchangeExitRequest`]. Only the
    /// *exiting* side gets one, carrying `01`; the peer learns about it from
    /// `0x3088 2c18` alone.
    ExchangeExitResponse
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::character_data::ItemClass;

    struct MockResolver(ItemClass);
    impl ItemClassResolver for MockResolver {
        fn item_class(&self, _ref_id: u32) -> ItemClass {
            self.0
        }
    }

    /// The window's lifecycle opcodes are empty bodies; they must round-trip to
    /// nothing rather than consuming a byte that is not there.
    #[test]
    fn the_empty_lifecycle_bodies_round_trip() {
        let empty = Bytes::new();
        assert_eq!(
            Bytes::from(ExchangePlayerConfirmed::try_from(empty.clone()).unwrap()),
            empty
        );
        assert_eq!(
            Bytes::from(ExchangeCompleted::try_from(empty.clone()).unwrap()),
            empty
        );
        assert_eq!(
            Bytes::from(ExchangeConfirmRequest::try_from(empty.clone()).unwrap()),
            empty
        );
    }

    /// One 0x3088 body per cancel reason, byte for byte. Round-tripping them
    /// shows the field is a plain little-endian `u16` and that nothing else
    /// rides behind it.
    #[test]
    fn every_cancel_reason_round_trips() {
        for (wire, reason, origin) in [
            (
                [0x28u8, 0x18],
                CANCEL_REASON_DECLINED,
                "the peer refused the petition",
            ),
            (
                [0x2b, 0x18],
                CANCEL_REASON_PARTNER_LEFT,
                "the inviter logged out",
            ),
            ([0x2c, 0x18], CANCEL_REASON_EXIT, "somebody pressed exit"),
            (
                [0x1f, 0x18],
                CANCEL_REASON_PROTOCOL_VIOLATION,
                "a second 0x7082",
            ),
        ] {
            let wire = Bytes::copy_from_slice(&wire);
            let decoded = ExchangeCanceled::try_from(wire.clone())
                .unwrap_or_else(|e| panic!("{origin}: {e:?}"));
            assert_eq!(decoded.reason, reason, "{origin}");
            assert_eq!(Bytes::from(decoded), wire, "{origin}");
        }
    }

    /// A 0x3088 without the reason does not occur; every body is two bytes.
    /// Defaulting to 0 would put a code on screen that no server sent, so a
    /// short body is an error. Positive control on the same read path: the
    /// two-byte form above decodes.
    #[test]
    fn a_cancel_without_a_reason_is_rejected_rather_than_defaulted() {
        assert!(ExchangeCanceled::try_from(Bytes::new()).is_err());
        assert!(ExchangeCanceled::try_from(Bytes::from_static(&[0x2c])).is_err());
    }

    #[test]
    fn the_started_and_gold_bodies_round_trip() {
        let wire = Bytes::from_static(&[0x80, 0xAB, 0x01, 0x00]);
        let decoded = ExchangeStarted::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.partner_unique_id, 0x1AB80);
        assert_eq!(Bytes::from(decoded), wire);

        // unk byte then u64 gold
        let wire = Bytes::from_static(&[0x01, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]);
        let decoded = ExchangeGoldUpdate::try_from(wire.clone()).unwrap();
        assert_eq!((decoded.unk_byte01, decoded.gold), (1, 1000));
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// The 0xB08x acks in both outcomes. The failure form is the one that
    /// matters: `0x181B` is harmless, `0x181F` means the trade is already dead,
    /// and a `bool` could not tell them apart.
    #[test]
    fn the_acks_round_trip_with_their_error_codes() {
        // The success form of all three acks: a single `01` byte.
        let ok = Bytes::from_static(&[0x01]);
        let decoded = ExchangeConfirmResponse::try_from(ok.clone()).unwrap();
        assert!(decoded.is_success());
        assert_eq!(decoded.error, None);
        assert_eq!(Bytes::from(decoded), ok);
        assert!(ExchangeApproveResponse::try_from(ok.clone())
            .unwrap()
            .is_success());
        assert!(ExchangeExitResponse::try_from(ok).unwrap().is_success());

        // All three sent outside a trade.
        let no_session = Bytes::from_static(&[0x02, 0x1b, 0x18]);
        let decoded = ExchangeConfirmResponse::try_from(no_session.clone()).unwrap();
        assert!(!decoded.is_success());
        assert_eq!(decoded.result, 2);
        assert_eq!(decoded.error, Some(EXCHANGE_ERROR_NO_SESSION));
        assert_eq!(Bytes::from(decoded), no_session);
        assert_eq!(
            ExchangeApproveResponse::try_from(no_session.clone())
                .unwrap()
                .error,
            Some(EXCHANGE_ERROR_NO_SESSION)
        );
        assert_eq!(
            ExchangeExitResponse::try_from(no_session).unwrap().error,
            Some(EXCHANGE_ERROR_NO_SESSION)
        );

        // The second confirm, which also kills the trade: 0x3088 1f18 goes to
        // both sides in the same millisecond.
        let violation = Bytes::from_static(&[0x02, 0x1f, 0x18]);
        let decoded = ExchangeConfirmResponse::try_from(violation.clone()).unwrap();
        assert_eq!(decoded.error, Some(EXCHANGE_ERROR_PROTOCOL_VIOLATION));
        assert_eq!(Bytes::from(decoded), violation);
    }

    /// An unnamed failure code must survive as a number instead of being
    /// swallowed or renamed — and a failure whose code got truncated is still a
    /// failure the UI has to unlock the window for.
    #[test]
    fn an_unknown_ack_code_is_passed_through_and_a_truncated_one_still_fails() {
        let decoded = ExchangeConfirmResponse::try_from(Bytes::from_static(&[0x02, 0x99, 0x18]))
            .expect("an unknown code is not a decode failure");
        assert_eq!(decoded.error, Some(0x1899));

        let decoded = ExchangeConfirmResponse::try_from(Bytes::from_static(&[0x02])).unwrap();
        assert!(!decoded.is_success());
        assert_eq!(decoded.error, None);

        assert!(ExchangeConfirmResponse::try_from(Bytes::new()).is_err());
    }

    /// 0x308C keeps its list as a raw tail because an item record's width is
    /// only knowable after resolving its `ref_id`. This drives the accessor
    /// with two expendable records to prove the tail really is
    /// slot-then-item-block repeated, with no exchange-slot byte between them.
    #[test]
    fn the_staged_item_list_decodes_through_the_resolver() {
        let mut body: Vec<u8> = 0x1AB80u32.to_le_bytes().to_vec();
        body.push(2); // item_count
        for (slot, stack) in [(13u8, 5u16), (14u8, 50u16)] {
            body.push(slot);
            body.extend(0u32.to_le_bytes()); // rent_type 0 -> no tail
            body.extend(11623u32.to_le_bytes()); // ref_id
            body.extend(stack.to_le_bytes()); // expendable stack_count
        }
        let wire = Bytes::from(body);

        let decoded = ExchangeItemsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.player_unique_id, 0x1AB80);
        assert_eq!(decoded.item_count, 2);

        let items = decoded
            .items(&MockResolver(ItemClass::Expendable { tid3: 0, tid4: 0 }))
            .expect("both records resolve");
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].slot, items[1].slot), (13, 14));
        assert_eq!(items[1].ref_id, 11623);
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// A truncated tail must yield `None` rather than a partial list — half a
    /// roster would misrepresent what the peer is actually offering.
    #[test]
    fn a_truncated_staged_list_is_none_not_partial() {
        let mut body: Vec<u8> = 1u32.to_le_bytes().to_vec();
        body.push(2); // claims two records...
        body.push(13); // ...but only one, and truncated
        body.extend(0u32.to_le_bytes());
        let decoded = ExchangeItemsUpdate::try_from(Bytes::from(body)).unwrap();

        assert!(decoded
            .items(&MockResolver(ItemClass::Expendable { tid3: 0, tid4: 0 }))
            .is_none());
    }

    /// Parse a wire body written as a plain hex string.
    fn hex(s: &str) -> Bytes {
        Bytes::from(
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        )
    }

    /// The ref ids used in the bodies below, classed the way itemdata classes
    /// them: `ITEM_CH_BLADE_01_A` 107 and `ITEM_CH_BLADE_01_C_RARE` 4052 are
    /// equipment, ref 4 is expendable.
    struct StubResolver;
    impl ItemClassResolver for StubResolver {
        fn item_class(&self, ref_id: u32) -> ItemClass {
            match ref_id {
                107 | 4052 => ItemClass::Equipment,
                _ => ItemClass::Expendable { tid3: 0, tid4: 0 },
            }
        }
    }

    /// The **peer copy**, byte for byte: the partner (uid 170207 = `df980200`)
    /// confirmed, so this client is shown one staged item. One slot byte per
    /// entry, and it is the *exchange* slot.
    #[test]
    fn the_peer_copy_decodes_with_one_slot_byte() {
        let wire = hex("df9802000100000000006b000000000000000000000000410000000001000200");
        let decoded = ExchangeItemsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.player_unique_id, 170207);
        assert_eq!(decoded.item_count, 1);

        let items = decoded.items(&StubResolver).expect("the record resolves");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].slot, 0, "exchange pane slot 0");
        assert_eq!(items[0].ref_id, 107);
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// The **self echo** of the very same item: one byte longer, and the extra
    /// byte `29` is the owner's bag slot 41, which the peer never learns.
    #[test]
    fn the_self_echo_decodes_with_two_slot_bytes() {
        let wire = hex("df980200012900000000006b000000000000000000000000410000000001000200");
        let decoded = ExchangeItemsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.player_unique_id, 170207);

        let staged = decoded
            .own_items(&StubResolver)
            .expect("the record resolves");
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].slot_inventory, 0x29);
        assert_eq!(staged[0].slot_exchange, 0);
        assert_eq!(staged[0].item.ref_id, 107);
        assert_eq!(
            staged[0].item.slot, 0x29,
            "the item keeps its bag slot; the trade only moves it on 0x3087"
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// The pair above differs by exactly one byte: deleting the `29` from the
    /// self echo produces the peer copy verbatim. That is why the shape is
    /// chosen by the caller and never guessed.
    #[test]
    fn the_self_echo_is_the_peer_copy_plus_the_inventory_slot() {
        let peer = hex("df9802000100000000006b000000000000000000000000410000000001000200");
        let own = hex("df980200012900000000006b000000000000000000000000410000000001000200");
        assert_eq!(own.len(), peer.len() + 1);

        let mut stripped = own.to_vec();
        stripped.remove(5); // the slot_inventory byte
        assert_eq!(Bytes::from(stripped), peer);

        // And the wrong decoder on the wrong shape is not a silent misread we
        // could tolerate: the peer decoder reads `29` as the slot and `00` as
        // the first rent byte, shifting everything after it.
        let misread = ExchangeItemsUpdate::try_from(own)
            .unwrap()
            .items(&StubResolver)
            .expect("it still parses — that is the danger");
        assert_ne!(misread[0].ref_id, 107);
    }

    /// Two entries in one self echo: bag 0x18 → pane 0 (ref 4, expendable) and
    /// bag 0x19 → pane 1 (ref 0x0fd4 = 4052, equipment). Two records of *different*
    /// widths in one body is what makes the class-resolving parser necessary.
    #[test]
    fn a_two_entry_self_echo_walks_records_of_different_widths() {
        let wire = hex(concat!(
            "0d990200021800000000000400000001001901",
            "00000000d40f00000800000000000000004f0000000001000200"
        ));
        let decoded = ExchangeItemsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.player_unique_id, 170253);
        assert_eq!(decoded.item_count, 2);

        let staged = decoded
            .own_items(&StubResolver)
            .expect("both records resolve");
        assert_eq!(
            staged
                .iter()
                .map(|s| (s.slot_inventory, s.slot_exchange, s.item.ref_id))
                .collect::<Vec<_>>(),
            vec![(0x18, 0, 4), (0x19, 1, 4052)]
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// Withdrawing the only staged item leaves `item_count = 0` and no tail at
    /// all; this echoes the 0x7034 sub-op 5 that removed it. Both decoders must
    /// return an empty list, not `None`: an empty offer is a valid offer.
    #[test]
    fn the_empty_echo_is_an_empty_list_not_a_failure() {
        let wire = hex("df98020000");
        let decoded = ExchangeItemsUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.item_count, 0);
        assert!(decoded.items(&StubResolver).unwrap().is_empty());
        assert!(decoded.own_items(&StubResolver).unwrap().is_empty());
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// Same rule as [`a_truncated_staged_list_is_none_not_partial`] for the
    /// self shape: a half-read own pane would show an offer we are not making.
    #[test]
    fn a_truncated_self_echo_is_none_not_partial() {
        let mut body: Vec<u8> = 170207u32.to_le_bytes().to_vec();
        body.push(2); // claims two records...
        body.extend([0x29, 0x00]); // ...and stops after the first pair of slots
        let decoded = ExchangeItemsUpdate::try_from(Bytes::from(body)).unwrap();
        assert!(decoded.own_items(&StubResolver).is_none());
    }
}

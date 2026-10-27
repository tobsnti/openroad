//! Inventory operation packets (0x7034 request / 0xB034 response), the
//! server-pushed equip/unequip notifications (0x3038 / 0x3039), and the
//! pickup-animation broadcast (0x3036).
//!
//! The *move* operation (op byte 0) matches go-sro (also covers equipping:
//! moving between a bag slot 13+ and an equipment slot 0-12). The *pickup*
//! operation (op byte 6, skrillax `PickupItem`) is what the server answers a
//! 0x7074 Pickup action with; its item payload is type-dependent (rent info +
//! ref id + class-dependent content), so it stays a raw tail with best-effort
//! accessors until live captures pin it. Unknown op bytes are preserved
//! raw — never a decode failure.

use bevy::prelude::Message;
use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::agent::character_data::{InventoryItem, ItemClassResolver};

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// The pseudo inventory slot gold pickups report (skrillax `GOLD_SLOT`).
pub const INVENTORY_SLOT_GOLD: u8 = 0xFE;

/// A sell ack's `slot_buyback` when the sold item cannot be bought back
/// (`docs/re/systems/npc-shop.md` §3).
pub const BUYBACK_SLOT_NONE: u8 = 0xFF;

/// Op 28's `slot` sentinel: the pick pet grabbed the item, but it did **not**
/// land in the character's inventory — and the body **ends there**, with no
/// item record behind it (`docs/re/systems/pet-pick-cos.md` §3, xBot
/// `PacketParser.cs:2122-2127`). It pairs with the client's own
/// `UIIT_MSG_COSPETERR_CANT_PICKITEM2` ("grab setting turned OFF because the
/// character's inventory is full"). A decoder that reads an item body
/// unconditionally after op 28 desyncs the stream, which is why this is a
/// named constant and not an inline `254`.
pub const PET_PICKUP_SLOT_NONE: u8 = 0xFE;

/// 0x7034 — client → server inventory operation, discriminated on the leading
/// op byte. `amount` is the moved stack size (full stack when not splitting);
/// the server clamps it to what is actually present.
///
/// Buy/sell are EXPERIMENTAL community-documented vSRO ops (8/9), not yet
/// verified against our reference server (`docs/net-inventory-0x7034.md`).
///
/// The storage ops (1/2/3 items, 11/12 gold) come from xBot-WinForms'
/// `SRTypes.InventoryItemMovement` enum + its `MoveItem` builders
/// (2026-08-08, `docs/net-storage-0x3047-0x3049.md`), not yet verified live.
/// The item ops use the builder's *uniqueID* overload
/// (`type, slotInitial, slotFinal, npc u32`) — the only storage call site in
/// that project does; the alternate overload writes a `quantity u16` instead,
/// which is the probe variant if these are rejected. The gold ops mirror
/// `EditGoldExchange` (`type, gold u64`) with the NPC id appended, which is
/// the unverified part there.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub enum InventoryOperationRequest {
    #[sro_packet(value = 0)]
    Move { source: u8, target: u8, amount: u16 },
    /// Move an item between two storage slots (storage session open).
    #[sro_packet(value = 1)]
    StorageToStorage {
        source: u8,
        target: u8,
        npc_unique_id: u32,
    },
    /// Deposit: inventory `source` → storage `target`. The fee is applied
    /// server-side with no confirmation; gold arrives via 0x304E.
    #[sro_packet(value = 2)]
    InventoryToStorage {
        source: u8,
        target: u8,
        npc_unique_id: u32,
    },
    /// Withdraw: storage `source` → inventory `target`.
    #[sro_packet(value = 3)]
    StorageToInventory {
        source: u8,
        target: u8,
        npc_unique_id: u32,
    },
    /// Buy `quantity` of the store good at `tab`/`slot` from the NPC store.
    #[sro_packet(value = 8)]
    Buy {
        tab: u8,
        slot: u8,
        quantity: u16,
        npc_unique_id: u32,
    },
    /// Sell `quantity` from the player's inventory `slot` to the NPC store.
    #[sro_packet(value = 9)]
    Sell {
        slot: u8,
        quantity: u16,
        npc_unique_id: u32,
    },
    /// Drop the whole of inventory `slot` on the ground at the player's feet.
    ///
    /// **[S], and the weakest-sourced op in this enum.** Only skrillax's
    /// `InventoryOperation` table names op **7** as the item drop
    /// (`docs/net-inventory-0x7034.md:24-26`); go-sro implements neither it nor
    /// any of the 26 further ops it merely names. Our own corpus is silent:
    /// **191 captured C→S 0x7034 bodies carry ops 00/08/09 and nothing else**,
    /// so this arm has never been observed on the wire.
    ///
    /// The body is the slot alone. A drop has no destination to name, and no
    /// quantity — stack splitting is a separate mechanism the client does not
    /// implement (`Inventory::apply_move` is whole-slot throughout, mirroring
    /// go-sro's own bookkeeping).
    ///
    /// ⚠️ **A wrong op byte here is not a no-op.** A rejected 0x7034 op draws
    /// no response at all (`docs/net-inventory-0x7034.md:113`), and repeating a
    /// mis-shaped opcode is what reset the connection for 0x704C (#215). The
    /// first live drop is therefore the capture that settles this: read the
    /// leading byte back out of `packet_dump/c2s/0x7034.log` and the answer out
    /// of `packet_dump/0xb034.log`, and record the result in
    /// `docs/re/ui/hud-inventory.md` §U1 either way.
    #[sro_packet(value = 7)]
    Drop { slot: u8 },
    /// Move an item between two of a pick pet's own bag slots.
    ///
    /// **[S] — the request bodies for ops 25/26/27 are in no recovered source.**
    /// The 0xB034 *responses* are [V] (`docs/re/systems/pet-pick-cos.md` §3),
    /// and go-sro names these ops but ships no handler arm, so the shape here
    /// is inferred the way every other op pair in this packet is built: the
    /// request carries the same subject the response echoes — the COS uid plus
    /// the slots — in response order. `[U]` until CAPTURE_LIST F5 records one.
    ///
    /// Ops 26/27 carry **no quantity**: the response layout has none, so pet
    /// moves are whole-slot and there is no split to request.
    #[sro_packet(value = 25)]
    PetToPet {
        cos_unique_id: u32,
        source: u8,
        target: u8,
        amount: u16,
    },
    /// Take out of the pet bag into the character's inventory. [S], see
    /// [`Self::PetToPet`].
    #[sro_packet(value = 26)]
    PetToInventory {
        cos_unique_id: u32,
        pet_slot: u8,
        inventory_slot: u8,
    },
    /// Put an inventory item into the pet bag. [S], see [`Self::PetToPet`].
    #[sro_packet(value = 27)]
    InventoryToPet {
        cos_unique_id: u32,
        inventory_slot: u8,
        pet_slot: u8,
    },
    /// Withdraw gold: storage → inventory (op 10 is drop-gold-to-ground).
    ///
    /// **No NPC id.** The gold arm writes the `u64` and returns; the session
    /// the server books it against is the one `0x703C` opened. Op 11 itself
    /// has no capture, but its two siblings on the same arm do — see op 12.
    #[sro_packet(value = 11)]
    StorageGoldToInventory { amount: u64 },
    /// Deposit gold: inventory → storage. No NPC id, same arm as 11/13.
    ///
    /// `[V]`: two captured frames, both nine bytes —
    /// `0c 0b00000000000000` (op 12, amount 11). A body with the NPC id this
    /// used to declare would be thirteen.
    #[sro_packet(value = 12)]
    InventoryGoldToStorage { amount: u64 },
    /// Stage gold on our side of a trade — an absolute amount, not a delta.
    ///
    /// `[V]`: eight captured frames, all nine bytes, e.g.
    /// `0d 6400000000000000` (100) and `0d 0100000000000000` (1).
    /// `agent/exchange.rs` already names this op as missing; it is the same
    /// gold arm as 11/12, which is why it lands with them.
    #[sro_packet(value = 13)]
    InventoryGoldToExchange { amount: u64 },
    /// Op 29 — move an item between two **guild** warehouse slots.
    ///
    /// `[S]`, no capture: no guild-storage op appears in any of the 99
    /// captured `0x7034` frames. The body is the old line's reading, which
    /// takes it as the same arm as op 1 plus a quantity. **Note the
    /// discrepancy:** today's op 1 carries no `amount`, the line this is
    /// ported from gave op 1 one. Neither has a frame, so op 1 is left
    /// untouched and this is `[U]` until a capture settles the pair.
    #[sro_packet(value = 29)]
    GuildStorageToGuildStorage {
        source: u8,
        target: u8,
        amount: u16,
        npc_unique_id: u32,
    },
    /// Op 30 — deposit: inventory `source` → guild `target`. Whole slot, no
    /// quantity (same arm as op 2). `[S]`, see op 29.
    #[sro_packet(value = 30)]
    InventoryToGuildStorage {
        source: u8,
        target: u8,
        npc_unique_id: u32,
    },
    /// Op 31 — withdraw: guild `source` → inventory `target`. Whole slot
    /// (same arm as op 3). `[S]`, see op 29.
    #[sro_packet(value = 31)]
    GuildStorageToInventory {
        source: u8,
        target: u8,
        npc_unique_id: u32,
    },
    /// Op 32 — deposit gold into the guild account. No NPC id, same arm as
    /// ops 11/12/13 — and that arm is measured (op 12/13), which is the one
    /// piece of this family that does not rest on a reading alone.
    #[sro_packet(value = 32)]
    InventoryGoldToGuildStorage { amount: u64 },
    /// Op 33 — withdraw gold from the guild account. `[S]`, see op 32.
    #[sro_packet(value = 33)]
    GuildStorageGoldToInventory { amount: u64 },
    /// Op 35 (`0x23`) — take a costume **off**: avatar `source` → inventory
    /// `target`.
    ///
    /// `[S]`, and the mark is measured rather than assumed: across all 13
    /// `packet_dump` roots on this machine there is exactly **one** `0x7034`
    /// capture (7 frames, `or-gate/packet_dump/c2s/0x7034.log`) and **every
    /// one of them is op 8** (buy). So neither avatar op has ever been on the
    /// wire here, and the body below rests on the original's own item-move
    /// builder, not on a frame.
    ///
    /// What that builder does, for the avatar container `0x4E`: the same arm
    /// that writes ops 2/3/26/27 writes **two bare slot bytes** and returns —
    /// `+3 = source`, `+4 = target` — so there is no quantity here. An avatar
    /// move is whole-slot, exactly as the pet ops above are, and for the same
    /// reason: the response layout carries no count either.
    #[sro_packet(value = 35)]
    AvatarToInventory { source: u8, target: u8 },
    /// Op 36 (`0x24`) — put a costume **on**: inventory `source` → avatar
    /// `target`. Same arm and therefore the same two bytes as op 35 with the
    /// slots swapped. `[S]`, see op 35.
    #[sro_packet(value = 36)]
    InventoryToAvatar { source: u8, target: u8 },
}

/// 0x703E — client → server item repair at an NPC (AGENT_INVENTORY_ITEM_REPAIR
/// per go-sro/SilkroadDoc/SilkroadBot opcode tables). EXPERIMENTAL: the
/// opcode pair is triple-sourced but NO source documents the body.
///
/// Layout v2 — **npc unique id FIRST**, then a type byte (1 = single +
/// slot, 2 = all), the community-folklore shape. The v1 guess (0x7034 house
/// style, discriminator first / npc id last) got clean `02 <code u16>`
/// rejections (`02 0000` for One, `02 0003` for All, 2026-08-07) — no
/// connection reset, so the server tolerated the LENGTHS; whether it
/// misparsed the fields or refused semantically is still open. Hand-written
/// wire impls: the derive writes the enum tag first, which can't express
/// npc-first.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemRepairRequest {
    /// Repair the single item in inventory `slot`.
    One { slot: u8, npc_unique_id: u32 },
    /// Repair every damaged equipment item.
    All { npc_unique_id: u32 },
}

impl TryFrom<Bytes> for ItemRepairRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let short = || {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "0x703E body too short",
            ))
        };
        let npc_unique_id =
            u32::from_le_bytes(value.get(0..4).ok_or_else(short)?.try_into().unwrap());
        match value.get(4).ok_or_else(short)? {
            1 => Ok(ItemRepairRequest::One {
                slot: *value.get(5).ok_or_else(short)?,
                npc_unique_id,
            }),
            2 => Ok(ItemRepairRequest::All { npc_unique_id }),
            other => Err(SerializationError::UnknownVariation(
                *other as usize,
                "ItemRepairRequest",
            )),
        }
    }
}

impl From<ItemRepairRequest> for Bytes {
    fn from(p: ItemRepairRequest) -> Self {
        let mut buf = BytesMut::new();
        match p {
            ItemRepairRequest::One {
                slot,
                npc_unique_id,
            } => {
                buf.put_u32_le(npc_unique_id);
                buf.put_u8(1);
                buf.put_u8(slot);
            }
            ItemRepairRequest::All { npc_unique_id } => {
                buf.put_u32_le(npc_unique_id);
                buf.put_u8(2);
            }
        }
        buf.freeze()
    }
}

/// 0xB03E — server → client repair ack. Never captured (no repair was ever
/// performed against the reference server) — decoded tolerantly as
/// `result u8` + raw tail, log-only; the first live repair pins it via
/// `packet_dump/0xb03e.log`.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct ItemRepairResponse {
    pub result: u8,
    pub tail: Bytes,
}

impl ItemRepairResponse {
    pub fn is_success(&self) -> bool {
        self.result == 1
    }
}

impl TryFrom<Bytes> for ItemRepairResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let result = *value.first().ok_or_else(|| {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "empty 0xB03E body",
            ))
        })?;
        Ok(ItemRepairResponse {
            result,
            tail: value.slice(1..),
        })
    }
}

impl From<ItemRepairResponse> for Bytes {
    fn from(p: ItemRepairResponse) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.result);
        buf.extend_from_slice(&p.tail);
        buf.freeze()
    }
}

/// 0x704C — client → server item use (CLIENT_ITEM_USE per SilkroadDoc).
///
/// **The body is switched on the item's class, not fixed** (#454). The original
/// ships two builders and never sends a bare 3-byte body for a class that wants
/// a tail; a server that reads a fixed-size body past the end of a short payload
/// simply drops the connection, which is the better explanation for #215's reset
/// than "this server does not implement 0x704C".
///
/// Every variant starts with the same head — `slot u8`, `type_id u16 LE` (the
/// packed TypeInfo word, `docs/net-item-use-0x704c.md`) — and differs in the
/// tail:
///
/// | variant | tail | builder |
/// |---|---|---|
/// | [`Self::Simple`] | none | `FUN_00796660:550-570` with no target/extra set |
/// | [`Self::WithTarget`] | `target u32 LE`, `kind u8` | `FUN_00796660:286-300` |
/// | [`Self::WithName`] | `len u16 LE` + `len` ASCII bytes | `FUN_007901c0` (TypeID mask `0xf800 == 0x2800`) |
///
/// `FUN_007901c0` writes the string through `FUN_0053d8c0`, which emits the
/// `std::string` length as a `u16` and then that many bytes; `FUN_0053d460`
/// (the `type_id` writer) is `FUN_00508fe0(x, 2)`, i.e. exactly two bytes.
///
/// ## The `+0x0D` slot bias is already in `slot`
///
/// The original's builders write `(char)param_1 + '\r'` — their index is
/// **bag-relative**, so the bias skips the 13 equipment slots
/// (`FUN_007901c0`, `FUN_00796660:286`), and the `0xB04C` handler subtracts it
/// again (`FUN_0087a7b0`: `local_23c = (byte)((char)local_22c - 0xd)`). Our
/// slots are the **server's own numbering**, where 0..12 are the equipment
/// slots and the bag starts at 13 (`client/src/plugins/net/inventory.rs`), i.e.
/// the biased number already. So `slot` here is the wire slot and **nothing may
/// add `0x0D` to it** — doing so would land 13 slots past the item.
///
/// ## Parsing (we only ever send this)
///
/// The wire carries no discriminator: the class lives in the sender's item data.
/// [`TryFrom`] therefore decides by length — 3 bytes `Simple`, 8 bytes
/// `WithTarget`, otherwise a `WithName` whose `u16` length must account for
/// exactly the rest of the body. An 8-byte body is genuinely ambiguous (it could
/// be a 3-character name); `WithTarget` wins because that is the common class.
/// UNVERIFIED by capture — group C still has no `0x704C`/`0xB04C` line.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemUseRequest {
    /// Potions, pills, plain return scrolls — no tail.
    Simple { slot: u8, type_id: u16 },
    /// COS potions, bombs and the like: the target's entity id plus a one-byte
    /// kind selector (`param_3` of `FUN_00796660`).
    WithTarget {
        slot: u8,
        type_id: u16,
        target: u32,
        kind: u8,
    },
    /// The classes that act on **another inventory item** rather than on the
    /// user or a world entity: COS revive ("Grass of life", TID `3,3,1,6`),
    /// rent extension, reinforce, pet helper, nasrun extension
    /// (`3,3,13,{8,11,12,15,16}`). See
    /// [`ItemDataRow::needs_target_slot`](../../../client/src/assets/textdata/itemdata.rs).
    ///
    /// ⚠️ **`[U]` — the tail is spec-derived, not recovered.** `u8 targetSlot`
    /// comes from Hyperbot's TID table (`docs/re/notes/item-use.md:78-92`),
    /// which is explicitly uncaptured. It has *not* been read off the
    /// original's own builder, and the precedent for trusting a table like this
    /// is bad: `0x70CB` was wired the same way and the recovered builder later
    /// showed it wrong in both width and field order.
    ///
    /// A wrong body here does not fail quietly — the server reads past the end
    /// and drops the connection, which is the behaviour that motivated this
    /// variant in the first place (we were sending a 3-byte [`Self::Simple`]
    /// for a class that wants four). Recovering `FUN_00796660`'s tail switch is
    /// what turns this `[U]` into `[V]`.
    ///
    /// `target_slot` is in the **server's own slot numbering**, exactly like
    /// `slot` — the `+0x0D` equipment bias is already baked into our slot
    /// numbers, so nothing may re-add it. Whether the *original* biases this
    /// second byte too is one of the things the RE has to confirm.
    WithSlot {
        slot: u8,
        type_id: u16,
        target_slot: u8,
    },
    /// The `0xf800 == 0x2800` class (return scroll with a destination name,
    /// megaphone, …): a length-prefixed ASCII string.
    WithName {
        slot: u8,
        type_id: u16,
        name: String,
    },
}

impl ItemUseRequest {
    /// The wire slot (already `+0x0D`-biased — see the type docs).
    pub fn slot(&self) -> u8 {
        match self {
            ItemUseRequest::Simple { slot, .. }
            | ItemUseRequest::WithTarget { slot, .. }
            | ItemUseRequest::WithSlot { slot, .. }
            | ItemUseRequest::WithName { slot, .. } => *slot,
        }
    }

    /// The packed TypeInfo word.
    pub fn type_id(&self) -> u16 {
        match self {
            ItemUseRequest::Simple { type_id, .. }
            | ItemUseRequest::WithTarget { type_id, .. }
            | ItemUseRequest::WithSlot { type_id, .. }
            | ItemUseRequest::WithName { type_id, .. } => *type_id,
        }
    }
}

impl TryFrom<Bytes> for ItemUseRequest {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let bad = |what: &'static str| {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                what,
            ))
        };
        let slot = *value.first().ok_or_else(|| bad("0x704C body too short"))?;
        let type_id = u16::from_le_bytes(
            value
                .get(1..3)
                .ok_or_else(|| bad("0x704C body too short"))?
                .try_into()
                .unwrap(),
        );
        match value.len() {
            3 => Ok(ItemUseRequest::Simple { slot, type_id }),
            // One trailing byte is the `targetSlot` class. Without this arm a
            // 4-byte body falls through to `WithName` and dies reading a `u16`
            // length out of a 1-byte remainder.
            4 => Ok(ItemUseRequest::WithSlot {
                slot,
                type_id,
                target_slot: value[3],
            }),
            8 => Ok(ItemUseRequest::WithTarget {
                slot,
                type_id,
                target: u32::from_le_bytes(value[3..7].try_into().unwrap()),
                kind: value[7],
            }),
            _ => {
                let len = u16::from_le_bytes(
                    value
                        .get(3..5)
                        .ok_or_else(|| bad("0x704C body too short"))?
                        .try_into()
                        .unwrap(),
                ) as usize;
                let name = value
                    .get(5..5 + len)
                    .ok_or_else(|| bad("0x704C name runs past the body"))?;
                if 5 + len != value.len() {
                    return Err(bad("0x704C trailing bytes after the name"));
                }
                Ok(ItemUseRequest::WithName {
                    slot,
                    type_id,
                    name: String::from_utf8_lossy(name).into_owned(),
                })
            }
        }
    }
}

impl From<ItemUseRequest> for Bytes {
    fn from(p: ItemUseRequest) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.slot());
        buf.put_u16_le(p.type_id());
        match p {
            ItemUseRequest::Simple { .. } => {}
            ItemUseRequest::WithTarget { target, kind, .. } => {
                buf.put_u32_le(target);
                buf.put_u8(kind);
            }
            ItemUseRequest::WithSlot { target_slot, .. } => buf.put_u8(target_slot),
            ItemUseRequest::WithName { name, .. } => {
                buf.put_u16_le(name.len() as u16);
                buf.extend_from_slice(name.as_bytes());
            }
        }
        buf.freeze()
    }
}

/// 0xB04C error codes (non-exhaustive — always keep the raw `code`).
pub const ITEM_USE_ERROR_REUSE_DELAY: u16 = 0x185B;
/// ⚠️ **Label refuted.** A live server returned this for a COS scroll used by a
/// living character (`docs/net-item-use-0x704c.md`), so the SPEC name is wrong
/// and the real meaning is `[U]`. Kept under its historical name only so the
/// number is not silently reused.
pub const ITEM_USE_ERROR_CHARACTER_DEAD: u16 = 0x1889;
pub const ITEM_USE_ERROR_ITEM_GONE: u16 = 0x1809;
/// The COS behind this scroll cannot be summoned — it is dead, or the scroll's
/// rental period has expired. **[V]**, observed live 2026-08-18: clicking a
/// dead attack pet's scroll answered `02 a4 18`.
pub const ITEM_USE_ERROR_COS_NOT_SUMMONABLE: u16 = 0x18A4;

// --- Observed live, meaning UNKNOWN -----------------------------------------
//
// These are named only so the numbers are not silently reused and so a reader
// can see what the capture actually contains. **None of them has a message**
// (`item_use_error_message` returns `None`), and that is deliberate: the
// code→string table is server-side and is *not* derivable from the client's
// archive. `textuisystem.txt` does carry the neighbouring COS strings, but the
// two codes we can anchor sit 73 apart while their keys are 109 entries apart,
// so the ordering hypothesis is refuted. Guessing here would put words in the
// server's mouth.

/// **`[U]`** — by far the most common refusal in `packet_dump/0xb04c.log`
/// (7 of 11 errors). Every occurrence is a COS summon on a scroll whose pet was
/// unsummoned moments earlier, and it persists for 13+ minutes, which rules out
/// a cooldown. See the timeline in `docs/net-item-use-0x704c.md`.
pub const ITEM_USE_ERROR_COS_UNKNOWN_18A5: u16 = 0x18A5;
/// **`[U]`** — observed 3× in the capture, always on a *rentable* COS scroll
/// (`(3,2,1,2)`). Note the value is not in the `0x18xx` family the other
/// item-use errors share.
pub const ITEM_USE_ERROR_UNKNOWN_0003: u16 = 0x0003;
/// **`[U]`** — observed once, 2026-08-18.
pub const ITEM_USE_ERROR_UNKNOWN_1849: u16 = 0x1849;

/// The textdata key describing an item-use rejection, or `None` when we have no
/// evidence for that code.
///
/// Returns a **key**, not English: the wording then comes from the user's own
/// `textuisystem.txt` in their own language, the same way every other
/// server-driven message is rendered.
///
/// Deliberately sparse. Of the seven codes named above only two have a message
/// we can defend — one from a live capture, one from the client's own reuse
/// gate — and `0x1889`'s documented label is already **refuted** by a capture.
///
/// An unmapped code is **not** silent to the player: the caller
/// (`hud::underbar::cast::log_item_use_response`) falls back to a neutral line
/// quoting the raw hex. Showing the number is honest; inventing a sentence for
/// it is not.
pub fn item_use_error_message(code: u16) -> Option<&'static str> {
    match code {
        // "Cannot summon because the period of summon item has expired, or pet
        // of the summon item is dead." — textuisystem.txt. Matches the observed
        // cause exactly (a dead pet's scroll). [V]
        ITEM_USE_ERROR_COS_NOT_SUMMONABLE => Some("UIIT_MSG_COS_STATE_IS_NOT_SUMMONABLE"),
        // "Still have time to reuse the item." The client gates reuse locally
        // too, so the meaning is not in doubt even though the code itself is
        // spec-derived. [S]
        ITEM_USE_ERROR_REUSE_DELAY => Some("UIIT_MSG_STRGERR_WAIT_FOR_REUSE_DELAY"),
        _ => None,
    }
}

/// 0xB04C — server → client item-use result. `result == 1` is success (the used
/// `slot`, the NEW stack count, and the echoed `type_id`); anything else is a
/// u16 error code (§1.5, e.g. [`ITEM_USE_ERROR_REUSE_DELAY`]). The "else = error"
/// discrimination is not a clean value tag, so the wire impls are hand-written
/// (`docs/re/DRAFT_LAYOUTS.md` §1.6).
///
/// EXPERIMENTAL / SPEC-derived (`docs/re/DRAFT_LAYOUTS.md` §1.2) — capture group
/// C verifies the wire order.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum ItemUseResponse {
    Success {
        slot: u8,
        remaining: u16,
        type_id: u16,
    },
    Error {
        code: u16,
    },
}

impl TryFrom<Bytes> for ItemUseResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let short = || {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "0xB04C body too short",
            ))
        };
        let result = *value.first().ok_or_else(short)?;
        if result == 1 {
            Ok(ItemUseResponse::Success {
                slot: *value.get(1).ok_or_else(short)?,
                remaining: u16::from_le_bytes(
                    value.get(2..4).ok_or_else(short)?.try_into().unwrap(),
                ),
                type_id: u16::from_le_bytes(value.get(4..6).ok_or_else(short)?.try_into().unwrap()),
            })
        } else {
            Ok(ItemUseResponse::Error {
                code: u16::from_le_bytes(value.get(1..3).ok_or_else(short)?.try_into().unwrap()),
            })
        }
    }
}

impl From<ItemUseResponse> for Bytes {
    fn from(p: ItemUseResponse) -> Self {
        let mut buf = BytesMut::new();
        match p {
            ItemUseResponse::Success {
                slot,
                remaining,
                type_id,
            } => {
                buf.put_u8(1);
                buf.put_u8(slot);
                buf.put_u16_le(remaining);
                buf.put_u16_le(type_id);
            }
            ItemUseResponse::Error { code } => {
                buf.put_u8(2);
                buf.put_u16_le(code);
            }
        }
        buf.freeze()
    }
}

/// A move the server made on its own, appended to the ack of the move that
/// caused it. See [`InventoryOperationResult::Move::chained`].
///
/// Five bytes: `op u8, source u8, target u8, amount u16`. The leading `op` is
/// `0` (a move) in every observed sample; it is kept rather than assumed so an
/// unexpected value is visible instead of being silently read as a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainedMove {
    pub op: u8,
    pub source: u8,
    pub target: u8,
    pub amount: u16,
}

/// Width of one [`ChainedMove`] on the wire.
const CHAINED_MOVE_LEN: usize = 5;

/// The per-operation payload of a successful [`InventoryOperationResponse`].
#[derive(Clone, Debug, PartialEq)]
pub enum InventoryOperationResult {
    /// Op 0 — echoes a move request.
    ///
    /// `chained` carries the moves the **server** made on its own as a
    /// consequence of this one, appended to the same body. In practice that is
    /// the shield: equipping a two-handed weapon moves it out of the shield
    /// slot, and going back to a one-handed weapon moves it back in.
    ///
    /// The trailing byte after `amount` used to be called `unknown`; it is the
    /// **count** of those extra records. Across the whole capture it is `0` on
    /// every plain move and `1` on exactly five acks — all of them moves into
    /// weapon slot 6, all naming shield slot 7 in the tail:
    ///
    /// ```text
    /// 01 00 20 06 0100 01 | 00 1b 07 0000    32->6, then bag 27 -> shield 7
    /// 01 00 19 06 0100 01 | 00 07 1b 0000    25->6, then shield 7 -> bag 27
    /// 01 00 3d 06 0100 01 | 00 07 2d 0000    61->6, then shield 7 -> bag 45
    /// ```
    ///
    /// The direction is independently corroborated for the third line: a
    /// `0x3039 EntityUnequip` for slot 7 arrives 100 ms earlier, so the tail's
    /// `(source, target)` order is `7 -> bag`, not the reverse.
    ///
    /// Dropping these was why an auto-unequipped shield stayed stuck in its
    /// slot, and why every later move of it drew `0x1809` "item gone".
    Move {
        source: u8,
        target: u8,
        amount: u16,
        /// Server-initiated moves caused by this one, in wire order.
        chained: Vec<ChainedMove>,
    },
    /// Op 7 — the item in `slot` was dropped on the ground.
    ///
    /// **`[V]` capture-confirmed** (2026-08-26): `01 07 48` and `01 07 20`,
    /// each answering a `07 <slot>` request, and in the second case followed by
    /// a `0x3015` ground-item spawn and a successful op-6 pickup of the same
    /// item two seconds later. The ack echoes the slot and nothing else.
    Drop { slot: u8 },
    /// Op 6 — an item/gold pickup landed in `slot`. The payload is
    /// type-dependent (skrillax `ItemPickupData`), kept raw with the
    /// best-effort accessors below until live captures pin it.
    Pickup { slot: u8, tail: Bytes },
    /// Op 8 — a store buy succeeded. EXPERIMENTAL: kept raw (assumed shape
    /// `count u8, slot u8 × count, quantity u16`, see [`Self::bought`])
    /// until live captures pin it.
    Buy { tail: Bytes },
    /// Op 9 — a store sell succeeded. EXPERIMENTAL: kept raw (shape
    /// `slot u8, quantity u16, npc_model u32, slot_buyback u8`, see
    /// [`Self::sold`] / [`Self::sold_buyback`]) until captures pin the rest.
    Sell { tail: Bytes },
    /// Op 34 — a store buyback succeeded: the tray slot `slot_buyback` came
    /// back into inventory `slot` as `quantity` pieces (xBot
    /// `PacketParser.cs:2233-2245`). The C→S request body is UNKNOWN
    /// (`docs/re/systems/npc-shop.md` §9.1), so nothing sends op 34 yet —
    /// this decodes the ack should the server ever push one.
    BuyBack {
        slot: u8,
        slot_buyback: u8,
        quantity: u16,
    },
    /// Op 1 — a within-storage move: `source, target, amount` (xBot
    /// `InventoryItemMovement_StorageToStorage`, same shape as op 0 minus the
    /// trailing byte).
    StorageToStorage { source: u8, target: u8, amount: u16 },
    /// Op 2 — a deposit: `inventory slot, storage slot` (xBot
    /// `InventoryItemMovement_InventoryToStorage`; whole-slot, no quantity).
    InventoryToStorage { source: u8, target: u8 },
    /// Op 3 — a withdraw: `storage slot, inventory slot` (xBot
    /// `InventoryItemMovement_StorageToInventory`).
    StorageToInventory { source: u8, target: u8 },
    /// Op 11 — gold moved storage → inventory; `amount` is subtracted from
    /// the storage total.
    StorageGoldToInventory { amount: u64 },
    /// Op 29 — a move inside the **guild** warehouse: `source, target,
    /// amount u16` — the same shape as op 1, only the container differs
    /// (xBot `PacketParser.cs:2142-2194`, GPL-3.0, cited).
    GuildStorageToGuildStorage { source: u8, target: u8, amount: u16 },
    /// Op 30 — a deposit into the guild warehouse: inventory slot, guild
    /// slot; whole record, the inventory slot is cleared (`:2195-2207`).
    InventoryToGuildStorage { source: u8, target: u8 },
    /// Op 31 — a withdraw: guild slot, inventory slot (`:2208-2220`).
    GuildStorageToInventory { source: u8, target: u8 },
    /// Op 32 — gold moved inventory → guild account; `amount` is a **delta**
    /// added to the guild total (`:2221-2226`), exactly like op 12.
    InventoryGoldToGuildStorage { amount: u64 },
    /// Op 33 — gold moved guild account → inventory; `amount` is subtracted
    /// from the guild total (`:2227-2232`).
    GuildStorageGoldToInventory { amount: u64 },
    /// Op 12 — gold moved inventory → storage; `amount` is added to the
    /// storage total.
    InventoryGoldToStorage { amount: u64 },
    /// Op 17 — the pick pet grabbed an item off the ground straight into the
    /// **pet's** bag: `cosUid:u32, slot:u8, <item body>`
    /// (`docs/re/systems/pet-pick-cos.md` §3, xBot
    /// `PacketParser.cs:1983-1986`). The item body is the same
    /// type-dependent record op 6 carries, so it is kept raw and decoded
    /// through [`Self::pet_pickup_item`] rather than as a fixed stack read.
    ///
    /// The doc also records a trailing `ownerName` that xBot has commented
    /// out — `[U]`, so nothing here consumes it; whatever follows the item
    /// record stays inside `tail`.
    GroundToPet {
        cos_unique_id: u32,
        slot: u8,
        tail: Bytes,
    },
    /// Op 25 — a move inside the pet's own bag:
    /// `cosUid:u32, slotFrom:u8, slotTo:u8, quantity:u16` (`:2039-2042`).
    /// Intra-container, so this one *can* split a stack.
    PetToPet {
        cos_unique_id: u32,
        source: u8,
        target: u8,
        amount: u16,
    },
    /// Op 26 — pet bag → character inventory: `cosUid:u32, petSlot:u8,
    /// invSlot:u8` (`:2094-2096`). **No quantity**: pet↔inventory moves are
    /// whole-slot only.
    PetToInventory {
        cos_unique_id: u32,
        pet_slot: u8,
        inventory_slot: u8,
    },
    /// Op 27 — character inventory → pet bag: `cosUid:u32, invSlot:u8,
    /// petSlot:u8` (`:2108-2110`). **No quantity**, same rule as op 26.
    InventoryToPet {
        cos_unique_id: u32,
        inventory_slot: u8,
        pet_slot: u8,
    },
    /// Op 35 — the costume moved out of the avatar container into the bag.
    ///
    /// `[S]`, same reading as the request (`InventoryOperationRequest::
    /// AvatarToInventory`): the client's own serializer answers this op on the
    /// arm that writes `+2 u8, +3 u8` and then the repeated-slot list op 0's
    /// ack also carries. Only the two slots are decoded, and the list is
    /// ignored exactly as op 0 ignores it.
    AvatarToInventory { source: u8, target: u8 },
    /// Op 36 — the bag item moved into the avatar container. Same arm and
    /// layout as op 35. `[S]`.
    InventoryToAvatar { source: u8, target: u8 },
    /// Op 28 — the pick pet grabbed an item and passed it through to the
    /// character's inventory: `cosUid:u32, slot:u8, [<item body> only if
    /// slot != 254]` (`:2122-2127`). See [`PET_PICKUP_SLOT_NONE`] — on the
    /// sentinel the body ends at the slot byte and `tail` is empty.
    GroundToPetToInventory {
        cos_unique_id: u32,
        slot: u8,
        tail: Bytes,
    },
    /// Any other op byte — kept raw, log-only.
    Unknown { op: u8, tail: Bytes },
}

impl InventoryOperationResult {
    /// Pickup of a gold pile: the amount (slot is the gold pseudo-slot,
    /// payload leads with the u32 amount).
    pub fn pickup_gold(&self) -> Option<u32> {
        match self {
            InventoryOperationResult::Pickup { slot, tail }
                if *slot == INVENTORY_SLOT_GOLD && tail.len() >= 4 =>
            {
                Some(u32::from_le_bytes(tail[0..4].try_into().unwrap()))
            }
            _ => None,
        }
    }

    /// A store buy's destination slots + stack size. Tail shape
    /// capture-VERIFIED on vSRO 1.188 (2026-08-06, two live buys):
    /// `tab u8, slot u8, dest_count u8, dest_slot u8 × count,
    /// quantity u16, unk u32 (0)` — e.g. `02 00 01 33 fa00 00000000` =
    /// store tab 2 slot 0 landed as 250 pieces in bag slot 0x33.
    pub fn bought(&self) -> Option<(Vec<u8>, u16)> {
        let InventoryOperationResult::Buy { tail } = self else {
            return None;
        };
        let count = *tail.get(2)? as usize;
        let slots = tail.get(3..3 + count)?.to_vec();
        let quantity_at = 3 + count;
        let quantity =
            u16::from_le_bytes(tail.get(quantity_at..quantity_at + 2)?.try_into().unwrap());
        Some((slots, quantity))
    }

    /// A store sell's source slot + stack size (`slot u8, quantity u16`);
    /// the identified tail behind it is read by [`Self::sold_buyback`].
    pub fn sold(&self) -> Option<(u8, u16)> {
        let InventoryOperationResult::Sell { tail } = self else {
            return None;
        };
        if tail.len() < 3 {
            return None;
        }
        Some((tail[0], u16::from_le_bytes(tail[1..3].try_into().unwrap())))
    }

    /// The sell ack's trailing `npc_model u32, slot_buyback u8` — the shop
    /// the item went to and the buyback-tray index it landed in
    /// (`docs/re/systems/npc-shop.md` §3, xBot `PacketParser.cs:1857-1888`;
    /// confirmed by `packet_dump/0xb034.log` line `01091b0100d700000001`).
    /// [`BUYBACK_SLOT_NONE`] means the item cannot be bought back. `None`
    /// when the tail is absent (older/shorter acks stay tolerated).
    pub fn sold_buyback(&self) -> Option<(u32, u8)> {
        let InventoryOperationResult::Sell { tail } = self else {
            return None;
        };
        if tail.len() < 8 {
            return None;
        }
        Some((u32::from_le_bytes(tail[3..7].try_into().unwrap()), tail[7]))
    }

    /// The item a pick-pet grab landed (op 17 into the pet's bag, op 28 passed
    /// through to the character's inventory), parsed exactly like
    /// [`Self::pickup_item`] — slot byte + the type-dependent record.
    ///
    /// `None` on op 28's [`PET_PICKUP_SLOT_NONE`] sentinel, because there is
    /// no item body to parse there; that is the whole point of the sentinel.
    pub fn pet_pickup_item(&self, resolver: &impl ItemClassResolver) -> Option<InventoryItem> {
        let (slot, tail) = match self {
            InventoryOperationResult::GroundToPet { slot, tail, .. } => (*slot, tail),
            InventoryOperationResult::GroundToPetToInventory { slot, tail, .. } => {
                if *slot == PET_PICKUP_SLOT_NONE {
                    return None;
                }
                (*slot, tail)
            }
            _ => return None,
        };
        if tail.is_empty() {
            return None;
        }
        let mut buf = Vec::with_capacity(tail.len() + 1);
        buf.push(slot);
        buf.extend_from_slice(tail);
        InventoryItem::read_with(&mut std::io::Cursor::new(buf.as_slice()), resolver).ok()
    }

    /// Pickup of a (non-gold) item, parsed into a full [`InventoryItem`].
    ///
    /// The payload after the slot is `rent, ref_id, content` — the exact tail
    /// of an item record (skrillax `ItemPickupData::Item`), where `content` is
    /// class-dependent (a bare stack for expendables, but opt-level / variance
    /// / durability / sockets for equipment). Parsing it as a fixed stack read
    /// the wrong bytes for equipment (huge bogus counts), so it is decoded via
    /// the same resolver-driven [`InventoryItem`] parser as CHARACTER_DATA by
    /// prepending the slot byte.
    pub fn pickup_item(&self, resolver: &impl ItemClassResolver) -> Option<InventoryItem> {
        let InventoryOperationResult::Pickup { slot, tail } = self else {
            return None;
        };
        if *slot == INVENTORY_SLOT_GOLD {
            return None;
        }
        let mut buf = Vec::with_capacity(tail.len() + 1);
        buf.push(*slot);
        buf.extend_from_slice(tail);
        InventoryItem::read_with(&mut std::io::Cursor::new(buf.as_slice()), resolver).ok()
    }
}

/// 0xB034 — server → client result of an inventory operation (a 0x7034 move
/// or a 0x7074 pickup action). `result == 1` is success; anything else
/// carries an error code.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct InventoryOperationResponse {
    pub result: u8,
    pub operation: Option<InventoryOperationResult>,
    pub error: Option<u16>,
}

impl TryFrom<Bytes> for InventoryOperationResponse {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let short = || {
            SerializationError::IoError(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "packet too short",
            ))
        };
        let result = *value.first().ok_or_else(short)?;
        if result != 1 {
            let error = value
                .get(1..3)
                .map(|b| u16::from_le_bytes(b.try_into().unwrap()));
            return Ok(InventoryOperationResponse {
                result,
                operation: None,
                error,
            });
        }
        let op = *value.get(1).ok_or_else(short)?;
        let operation = match op {
            0 if value.len() >= 7 => {
                // value[6] is the count of server-initiated moves appended
                // after it, not an unknown byte — see `Move::chained`. Read as
                // many as the body actually holds so a truncated or
                // longer-than-expected tail degrades instead of panicking.
                let count = value[6] as usize;
                let mut chained = Vec::with_capacity(count.min(4));
                for index in 0..count {
                    let at = 7 + index * CHAINED_MOVE_LEN;
                    let Some(record) = value.get(at..at + CHAINED_MOVE_LEN) else {
                        break;
                    };
                    chained.push(ChainedMove {
                        op: record[0],
                        source: record[1],
                        target: record[2],
                        amount: u16::from_le_bytes(record[3..5].try_into().unwrap()),
                    });
                }
                InventoryOperationResult::Move {
                    source: value[2],
                    target: value[3],
                    amount: u16::from_le_bytes(value[4..6].try_into().unwrap()),
                    chained,
                }
            }
            7 if value.len() >= 3 => InventoryOperationResult::Drop { slot: value[2] },
            6 if value.len() >= 3 => InventoryOperationResult::Pickup {
                slot: value[2],
                tail: value.slice(3..),
            },
            8 => InventoryOperationResult::Buy {
                tail: value.slice(2..),
            },
            9 => InventoryOperationResult::Sell {
                tail: value.slice(2..),
            },
            34 if value.len() >= 6 => InventoryOperationResult::BuyBack {
                slot: value[2],
                slot_buyback: value[3],
                quantity: u16::from_le_bytes(value[4..6].try_into().unwrap()),
            },
            1 if value.len() >= 6 => InventoryOperationResult::StorageToStorage {
                source: value[2],
                target: value[3],
                amount: u16::from_le_bytes(value[4..6].try_into().unwrap()),
            },
            2 if value.len() >= 4 => InventoryOperationResult::InventoryToStorage {
                source: value[2],
                target: value[3],
            },
            3 if value.len() >= 4 => InventoryOperationResult::StorageToInventory {
                source: value[2],
                target: value[3],
            },
            11 if value.len() >= 10 => InventoryOperationResult::StorageGoldToInventory {
                amount: u64::from_le_bytes(value[2..10].try_into().unwrap()),
            },
            12 if value.len() >= 10 => InventoryOperationResult::InventoryGoldToStorage {
                amount: u64::from_le_bytes(value[2..10].try_into().unwrap()),
            },
            // --- guild warehouse (ops 29..=33). Same four shapes as the NPC
            // storage ops above, only the container differs; the ack bodies
            // are xBot's reading (`PacketParser.cs:2142-2232`, GPL-3.0, cited)
            // and carry no capture — see the request variants.
            29 if value.len() >= 6 => InventoryOperationResult::GuildStorageToGuildStorage {
                source: value[2],
                target: value[3],
                amount: u16::from_le_bytes(value[4..6].try_into().unwrap()),
            },
            30 if value.len() >= 4 => InventoryOperationResult::InventoryToGuildStorage {
                source: value[2],
                target: value[3],
            },
            31 if value.len() >= 4 => InventoryOperationResult::GuildStorageToInventory {
                source: value[2],
                target: value[3],
            },
            32 if value.len() >= 10 => InventoryOperationResult::InventoryGoldToGuildStorage {
                amount: u64::from_le_bytes(value[2..10].try_into().unwrap()),
            },
            33 if value.len() >= 10 => InventoryOperationResult::GuildStorageGoldToInventory {
                amount: u64::from_le_bytes(value[2..10].try_into().unwrap()),
            },
            // --- pick-pet / COS bag ops (`docs/re/systems/pet-pick-cos.md` §3)
            17 if value.len() >= 7 => InventoryOperationResult::GroundToPet {
                cos_unique_id: u32::from_le_bytes(value[2..6].try_into().unwrap()),
                slot: value[6],
                tail: value.slice(7..),
            },
            25 if value.len() >= 10 => InventoryOperationResult::PetToPet {
                cos_unique_id: u32::from_le_bytes(value[2..6].try_into().unwrap()),
                source: value[6],
                target: value[7],
                amount: u16::from_le_bytes(value[8..10].try_into().unwrap()),
            },
            26 if value.len() >= 8 => InventoryOperationResult::PetToInventory {
                cos_unique_id: u32::from_le_bytes(value[2..6].try_into().unwrap()),
                pet_slot: value[6],
                inventory_slot: value[7],
            },
            27 if value.len() >= 8 => InventoryOperationResult::InventoryToPet {
                cos_unique_id: u32::from_le_bytes(value[2..6].try_into().unwrap()),
                inventory_slot: value[6],
                pet_slot: value[7],
            },
            // The two avatar ops: two bare slots, and whatever list follows is
            // ignored the way op 0's is.
            35 if value.len() >= 4 => InventoryOperationResult::AvatarToInventory {
                source: value[2],
                target: value[3],
            },
            36 if value.len() >= 4 => InventoryOperationResult::InventoryToAvatar {
                source: value[2],
                target: value[3],
            },
            // The sentinel is not a special case *here*: `slice(7..)` is empty
            // exactly when the body ended at the slot byte, which is what the
            // sentinel produces. Reading an item body unconditionally is what
            // desyncs (`pet-pick-cos.md` §3), and this never does.
            28 if value.len() >= 7 => InventoryOperationResult::GroundToPetToInventory {
                cos_unique_id: u32::from_le_bytes(value[2..6].try_into().unwrap()),
                slot: value[6],
                tail: value.slice(7..),
            },
            _ => InventoryOperationResult::Unknown {
                op,
                tail: value.slice(2..),
            },
        };
        Ok(InventoryOperationResponse {
            result,
            operation: Some(operation),
            error: None,
        })
    }
}

impl From<InventoryOperationResponse> for Bytes {
    fn from(p: InventoryOperationResponse) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.result);
        match p.operation {
            Some(InventoryOperationResult::Move {
                source,
                target,
                amount,
                chained,
            }) => {
                buf.put_u8(0);
                buf.put_u8(source);
                buf.put_u8(target);
                buf.put_u16_le(amount);
                buf.put_u8(chained.len() as u8);
                for record in chained {
                    buf.put_u8(record.op);
                    buf.put_u8(record.source);
                    buf.put_u8(record.target);
                    buf.put_u16_le(record.amount);
                }
            }
            Some(InventoryOperationResult::Drop { slot }) => {
                buf.put_u8(7);
                buf.put_u8(slot);
            }
            Some(InventoryOperationResult::Pickup { slot, tail }) => {
                buf.put_u8(6);
                buf.put_u8(slot);
                buf.extend_from_slice(&tail);
            }
            Some(InventoryOperationResult::Buy { tail }) => {
                buf.put_u8(8);
                buf.extend_from_slice(&tail);
            }
            Some(InventoryOperationResult::Sell { tail }) => {
                buf.put_u8(9);
                buf.extend_from_slice(&tail);
            }
            Some(InventoryOperationResult::BuyBack {
                slot,
                slot_buyback,
                quantity,
            }) => {
                buf.put_u8(34);
                buf.put_u8(slot);
                buf.put_u8(slot_buyback);
                buf.put_u16_le(quantity);
            }
            Some(InventoryOperationResult::StorageToStorage {
                source,
                target,
                amount,
            }) => {
                buf.put_u8(1);
                buf.put_u8(source);
                buf.put_u8(target);
                buf.put_u16_le(amount);
            }
            Some(InventoryOperationResult::InventoryToStorage { source, target }) => {
                buf.put_u8(2);
                buf.put_u8(source);
                buf.put_u8(target);
            }
            Some(InventoryOperationResult::StorageToInventory { source, target }) => {
                buf.put_u8(3);
                buf.put_u8(source);
                buf.put_u8(target);
            }
            Some(InventoryOperationResult::StorageGoldToInventory { amount }) => {
                buf.put_u8(11);
                buf.put_u64_le(amount);
            }
            Some(InventoryOperationResult::InventoryGoldToStorage { amount }) => {
                buf.put_u8(12);
                buf.put_u64_le(amount);
            }
            // The guild warehouse mirrors the four NPC-storage shapes above;
            // only the op byte and the container differ.
            Some(InventoryOperationResult::GuildStorageToGuildStorage {
                source,
                target,
                amount,
            }) => {
                buf.put_u8(29);
                buf.put_u8(source);
                buf.put_u8(target);
                buf.put_u16_le(amount);
            }
            Some(InventoryOperationResult::InventoryToGuildStorage { source, target }) => {
                buf.put_u8(30);
                buf.put_u8(source);
                buf.put_u8(target);
            }
            Some(InventoryOperationResult::GuildStorageToInventory { source, target }) => {
                buf.put_u8(31);
                buf.put_u8(source);
                buf.put_u8(target);
            }
            Some(InventoryOperationResult::InventoryGoldToGuildStorage { amount }) => {
                buf.put_u8(32);
                buf.put_u64_le(amount);
            }
            Some(InventoryOperationResult::GuildStorageGoldToInventory { amount }) => {
                buf.put_u8(33);
                buf.put_u64_le(amount);
            }
            Some(InventoryOperationResult::GroundToPet {
                cos_unique_id,
                slot,
                tail,
            }) => {
                buf.put_u8(17);
                buf.put_u32_le(cos_unique_id);
                buf.put_u8(slot);
                buf.extend_from_slice(&tail);
            }
            Some(InventoryOperationResult::PetToPet {
                cos_unique_id,
                source,
                target,
                amount,
            }) => {
                buf.put_u8(25);
                buf.put_u32_le(cos_unique_id);
                buf.put_u8(source);
                buf.put_u8(target);
                buf.put_u16_le(amount);
            }
            Some(InventoryOperationResult::PetToInventory {
                cos_unique_id,
                pet_slot,
                inventory_slot,
            }) => {
                buf.put_u8(26);
                buf.put_u32_le(cos_unique_id);
                buf.put_u8(pet_slot);
                buf.put_u8(inventory_slot);
            }
            Some(InventoryOperationResult::InventoryToPet {
                cos_unique_id,
                inventory_slot,
                pet_slot,
            }) => {
                buf.put_u8(27);
                buf.put_u32_le(cos_unique_id);
                buf.put_u8(inventory_slot);
                buf.put_u8(pet_slot);
            }
            Some(InventoryOperationResult::AvatarToInventory { source, target }) => {
                buf.put_u8(35);
                buf.put_u8(source);
                buf.put_u8(target);
            }
            Some(InventoryOperationResult::InventoryToAvatar { source, target }) => {
                buf.put_u8(36);
                buf.put_u8(source);
                buf.put_u8(target);
            }
            Some(InventoryOperationResult::GroundToPetToInventory {
                cos_unique_id,
                slot,
                tail,
            }) => {
                buf.put_u8(28);
                buf.put_u32_le(cos_unique_id);
                buf.put_u8(slot);
                buf.extend_from_slice(&tail);
            }
            Some(InventoryOperationResult::Unknown { op, tail }) => {
                buf.put_u8(op);
                buf.extend_from_slice(&tail);
            }
            None => {
                if let Some(error) = p.error {
                    buf.put_u16_le(error);
                }
            }
        }
        buf.freeze()
    }
}

/// 0x3036 — server → client: the entity bends down to pick something up
/// (broadcast to everyone in range; the animation is implied by the opcode).
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct PlayerPickupAnimation {
    pub unique_id: u32,
    pub rotation: u8,
}

/// 0x3038 — server → client: an entity equipped an item into `slot`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityEquip {
    pub unique_id: u32,
    pub slot: u8,
    pub ref_id: u32,
    /// go-sro sends whether the item is a one-handed weapon (animation hint).
    pub one_handed: u8,
}

/// 0x3039 — server → client: an entity removed the item from `slot`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct EntityUnequip {
    pub unique_id: u32,
    pub slot: u8,
    pub ref_id: u32,
}

/// 0x3052 — server → client: the authoritative durability of one slot's
/// equipment. Dump-verified against `packet_dump/0x3052.log` (five samples,
/// all exactly 5 bytes; slots 1 and 6, values falling 49→47→46 as the gear
/// wears). See `docs/net-inventory-events-0x3040.md`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct InventoryItemDurabilityUpdate {
    pub slot: u8,
    pub durability: u32,
}

/// Largest magic-param list the original accepts, from its own assert:
/// `m_btMagicParamNum <= MAX_MAGPARAM_PER_ITEM` (`SOItem.cpp:378`, reached
/// through the `0x20` sub-parser `FUN_008bcdf0`).
pub const MAX_MAGPARAM_PER_ITEM: usize = 12;

/// 0x3040 — server → client: a delta on one inventory slot. It does **not**
/// re-embed the item block; only the named fields change.
///
/// **The second byte is a bitmask, not a discriminator.** The original client
/// tests it with eight independent `if ((mask & bit) != 0)` blocks and reads
/// one block per set bit in a fixed order (handler `FUN_0088bf30`, module
/// `SOItem.cpp`; see `docs/net-inventory-events-0x3040.md` in the local RE
/// store). xBot reads it as a `switch` and gets away with it only because the
/// two cases it handles happen to be single-bit — and this type followed xBot:
/// a combined push such as `0x18` (durability + quantity) decoded to "unknown
/// type, no fields" and dropped both. Six of the eight bits were unmodelled,
/// **durability among them**, so `0x3040` could carry exactly what `0x3052`
/// carries and we ignored it.
///
/// Block order and widths, with the tag each one carries:
///
/// | bit | field | width | tag |
/// |---|---|---|---|
/// | `0x01` | `unknown01` (`item+0x34`) | u32 | semantics [U] |
/// | `0x02` | `plus` | u8 | [S] |
/// | `0x04` | `variance` | u64 | [S] |
/// | `0x08` | `quantity` | u16 | [V] |
/// | `0x10` | `durability` | u32 | [V] |
/// | `0x20` | `magic_options` | u8 count + count × u64 | [V] |
/// | `0x40` | `cos_state` | u8 | [V] |
/// | `0x80` | `duration_seconds` | u32 | [V] |
///
/// The magic-param records stay `u64`: the original splits each into a lookup
/// key in the low `u16` and a value, but what the halves *mean* is [U], and a
/// named struct would be an invention.
///
/// A body that stops early is a decode error rather than a silent zero — the
/// mask says what must follow. Trailing bytes beyond the masked blocks are
/// ignored, as the original's handler also stops after its eight tests.
#[derive(Message, Clone, Debug, PartialEq, Default)]
pub struct InventoryItemUpdate {
    pub slot: u8,
    /// The raw mask, kept so a consumer can log a bit we do not read.
    pub mask: u8,
    pub unknown01: Option<u32>,
    pub plus: Option<u8>,
    pub variance: Option<u64>,
    pub quantity: Option<u16>,
    pub durability: Option<u32>,
    pub magic_options: Option<Vec<u64>>,
    pub cos_state: Option<u8>,
    pub duration_seconds: Option<u32>,
}

impl InventoryItemUpdate {
    pub const UNKNOWN01: u8 = 0x01;
    pub const PLUS: u8 = 0x02;
    pub const VARIANCE: u8 = 0x04;
    pub const QUANTITY: u8 = 0x08;
    pub const DURABILITY: u8 = 0x10;
    pub const MAGIC_OPTIONS: u8 = 0x20;
    pub const COS_STATE: u8 = 0x40;
    pub const DURATION: u8 = 0x80;
}

impl TryFrom<Bytes> for InventoryItemUpdate {
    type Error = SerializationError;

    fn try_from(value: Bytes) -> Result<Self, Self::Error> {
        let mut buf = value;
        let need = |buf: &Bytes, bytes: usize| -> Result<(), SerializationError> {
            if buf.len() < bytes {
                return Err(SerializationError::from(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "0x3040: body ends inside a block its mask announced",
                )));
            }
            Ok(())
        };
        need(&buf, 2)?;
        let slot = buf.get_u8();
        let mask = buf.get_u8();
        let mut out = InventoryItemUpdate {
            slot,
            mask,
            ..Default::default()
        };
        if mask & Self::UNKNOWN01 != 0 {
            need(&buf, 4)?;
            out.unknown01 = Some(buf.get_u32_le());
        }
        if mask & Self::PLUS != 0 {
            need(&buf, 1)?;
            out.plus = Some(buf.get_u8());
        }
        if mask & Self::VARIANCE != 0 {
            need(&buf, 8)?;
            out.variance = Some(buf.get_u64_le());
        }
        if mask & Self::QUANTITY != 0 {
            need(&buf, 2)?;
            out.quantity = Some(buf.get_u16_le());
        }
        if mask & Self::DURABILITY != 0 {
            need(&buf, 4)?;
            out.durability = Some(buf.get_u32_le());
        }
        if mask & Self::MAGIC_OPTIONS != 0 {
            need(&buf, 1)?;
            let count = buf.get_u8() as usize;
            if count > MAX_MAGPARAM_PER_ITEM {
                return Err(SerializationError::from(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "0x3040: magic-param count above MAX_MAGPARAM_PER_ITEM",
                )));
            }
            need(&buf, count * 8)?;
            out.magic_options = Some((0..count).map(|_| buf.get_u64_le()).collect());
        }
        if mask & Self::COS_STATE != 0 {
            need(&buf, 1)?;
            out.cos_state = Some(buf.get_u8());
        }
        if mask & Self::DURATION != 0 {
            need(&buf, 4)?;
            out.duration_seconds = Some(buf.get_u32_le());
        }
        Ok(out)
    }
}

impl From<InventoryItemUpdate> for Bytes {
    fn from(p: InventoryItemUpdate) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.slot);
        buf.put_u8(p.mask);
        if let Some(value) = p.unknown01 {
            buf.put_u32_le(value);
        }
        if let Some(value) = p.plus {
            buf.put_u8(value);
        }
        if let Some(value) = p.variance {
            buf.put_u64_le(value);
        }
        if let Some(value) = p.quantity {
            buf.put_u16_le(value);
        }
        if let Some(value) = p.durability {
            buf.put_u32_le(value);
        }
        if let Some(options) = &p.magic_options {
            buf.put_u8(options.len() as u8);
            for option in options {
                buf.put_u64_le(*option);
            }
        }
        if let Some(value) = p.cos_state {
            buf.put_u8(value);
        }
        if let Some(value) = p.duration_seconds {
            buf.put_u32_le(value);
        }
        buf.freeze()
    }
}

impl ByteSize for InventoryItemUpdate {
    fn byte_size(&self) -> usize {
        2 + self.unknown01.map_or(0, |_| 4)
            + self.plus.map_or(0, |_| 1)
            + self.variance.map_or(0, |_| 8)
            + self.quantity.map_or(0, |_| 2)
            + self.durability.map_or(0, |_| 4)
            + self
                .magic_options
                .as_ref()
                .map_or(0, |options| 1 + options.len() * 8)
            + self.cos_state.map_or(0, |_| 1)
            + self.duration_seconds.map_or(0, |_| 4)
    }
}

/// 0x3092 — server → client: the result of a bag-expansion purchase. The new
/// slot count follows only on success.
///
/// Layout is xBot-sourced only — there is no `packet_dump/0x3092.log`, so a
/// failure tail (if any) is unknown and stays unparsed.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct InventoryCapacityUpdate {
    pub success: bool,
    #[sro_packet(when = "success")]
    pub new_capacity: Option<u8>,
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::agent::character_data::{ItemClass, ItemTypeData};

    /// The revive class's body is **four** bytes. Without a 4-byte arm in the
    /// length switch it falls through to `WithName` and dies reading a `u16`
    /// length out of one remaining byte — and the whole reason this variant
    /// exists is that a body of the wrong length makes the server drop the
    /// connection.
    #[test]
    fn with_slot_is_four_bytes_and_round_trips() {
        let req = ItemUseRequest::WithSlot {
            slot: 13,
            type_id: 0x30CC,
            target_slot: 14,
        };
        let wire: Bytes = req.clone().into();

        assert_eq!(&wire[..], &[13, 0xCC, 0x30, 14]);
        assert_eq!(wire.len(), 4);
        assert_eq!(ItemUseRequest::try_from(wire).unwrap(), req);
        // ...and the neighbouring lengths still resolve to their own classes.
        assert!(matches!(
            ItemUseRequest::try_from(Bytes::from_static(&[13, 0, 0])).unwrap(),
            ItemUseRequest::Simple { .. }
        ));
        assert!(matches!(
            ItemUseRequest::try_from(Bytes::from_static(&[13, 0, 0, 1, 0, 0, 0, 2])).unwrap(),
            ItemUseRequest::WithTarget { .. }
        ));
    }

    /// The table maps only what we can defend. A plausible-looking sentence
    /// attached to the wrong code is worse than a raw hex value — `0x1889`'s
    /// documented label was already refuted by a capture, which is exactly why
    /// it stays unmapped.
    #[test]
    fn item_use_errors_map_only_what_is_evidenced() {
        assert_eq!(
            item_use_error_message(ITEM_USE_ERROR_COS_NOT_SUMMONABLE),
            Some("UIIT_MSG_COS_STATE_IS_NOT_SUMMONABLE"),
            "observed live for a dead pet's scroll",
        );
        assert_eq!(
            item_use_error_message(ITEM_USE_ERROR_REUSE_DELAY),
            Some("UIIT_MSG_STRGERR_WAIT_FOR_REUSE_DELAY"),
        );
        // Refuted label, and a code we have no reading for: both stay silent
        // so the caller falls back to logging the raw code.
        assert_eq!(item_use_error_message(ITEM_USE_ERROR_CHARACTER_DEAD), None);
        assert_eq!(item_use_error_message(ITEM_USE_ERROR_ITEM_GONE), None);
        assert_eq!(item_use_error_message(0xFFFF), None);
        // The observed code, spelled out: `02 a4 18` on the wire.
        assert_eq!(ITEM_USE_ERROR_COS_NOT_SUMMONABLE, 0x18A4);
    }

    /// The three codes seen live with no known meaning are named but **not**
    /// given a message. Naming them stops the numbers being reused; mapping
    /// them would be inventing the server's words.
    ///
    /// `0x18A5` is the one to watch: it is 7 of the 11 refusals in the capture
    /// and it sits one past `COS_NOT_SUMMONABLE`, which makes a COS meaning
    /// tempting — and unevidenced. The code→string table is server-side.
    #[test]
    fn codes_observed_without_a_reading_stay_unmapped() {
        for code in [
            ITEM_USE_ERROR_COS_UNKNOWN_18A5,
            ITEM_USE_ERROR_UNKNOWN_0003,
            ITEM_USE_ERROR_UNKNOWN_1849,
        ] {
            assert_eq!(
                item_use_error_message(code),
                None,
                "{code:#06x} has no evidenced meaning and must not be given one",
            );
        }
        // pinned as the capture spells them
        assert_eq!(ITEM_USE_ERROR_COS_UNKNOWN_18A5, 0x18A5);
        assert_eq!(ITEM_USE_ERROR_UNKNOWN_0003, 0x0003);
        assert_eq!(ITEM_USE_ERROR_UNKNOWN_1849, 0x1849);
        // and they are distinct from the two we *can* name
        assert_ne!(
            ITEM_USE_ERROR_COS_UNKNOWN_18A5,
            ITEM_USE_ERROR_COS_NOT_SUMMONABLE
        );
    }

    /// `cos_is_dead` gates both the inventory tint and the tooltip line, so it
    /// must be true for exactly one of the four `SRCoS.State` values.
    #[test]
    fn only_state_four_reads_as_a_dead_pet() {
        use crate::agent::character_data::{
            COS_STATE_DEAD, COS_STATE_NEVER_SUMMONED, COS_STATE_SUMMONED, COS_STATE_UNSUMMONED,
        };
        let pet = |state| ItemTypeData::CosPet {
            state,
            cos_ref_id: None,
            name: None,
            rent_seconds: None,
            unk: None,
        };

        assert!(pet(COS_STATE_DEAD).cos_is_dead());
        for alive in [
            COS_STATE_NEVER_SUMMONED,
            COS_STATE_SUMMONED,
            COS_STATE_UNSUMMONED,
        ] {
            assert!(!pet(alive).cos_is_dead(), "state {alive}");
            assert_eq!(pet(alive).cos_state(), Some(alive));
        }
        // A non-COS item has no state at all, and is never "dead".
        let potion = ItemTypeData::Expendable {
            stack_count: 1,
            assimilation_prob: None,
            mag_params: Vec::new(),
        };
        assert_eq!(potion.cos_state(), None);
        assert!(!potion.cos_is_dead());
    }

    #[test]
    fn move_request_roundtrips() {
        let request = InventoryOperationRequest::Move {
            source: 13,
            target: 6,
            amount: 1,
        };
        assert_eq!(request.byte_size(), 5);
        let bytes: Bytes = request.clone().into();
        assert_eq!(bytes.as_ref(), &[0x00, 13, 6, 0x01, 0x00]);
        assert_eq!(InventoryOperationRequest::try_from(bytes).unwrap(), request);
    }

    #[test]
    fn buy_and_sell_requests_roundtrip() {
        let buy = InventoryOperationRequest::Buy {
            tab: 0,
            slot: 3,
            quantity: 5,
            npc_unique_id: 0x539,
        };
        let bytes: Bytes = buy.clone().into();
        assert_eq!(bytes.as_ref(), &[0x08, 0, 3, 5, 0, 0x39, 0x05, 0, 0]);
        assert_eq!(InventoryOperationRequest::try_from(bytes).unwrap(), buy);

        let sell = InventoryOperationRequest::Sell {
            slot: 14,
            quantity: 2,
            npc_unique_id: 0x539,
        };
        let bytes: Bytes = sell.clone().into();
        assert_eq!(bytes.as_ref(), &[0x09, 14, 2, 0, 0x39, 0x05, 0, 0]);
        assert_eq!(InventoryOperationRequest::try_from(bytes).unwrap(), sell);
    }

    #[test]
    fn buy_and_sell_responses_parse_best_effort() {
        // live capture (vSRO 1.188): 01 08 <tab 2> <slot 0> <count 1>
        // <dest 0x33> <quantity 250 LE> <unk u32 0>
        let bytes = Bytes::from_static(&[
            0x01, 0x08, 0x02, 0x00, 0x01, 0x33, 0xFA, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        let op = response.operation.clone().unwrap();
        assert_eq!(op.bought(), Some((vec![0x33], 250)));
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // a truncated tail stays raw and returns None
        let bytes = Bytes::from_static(&[0x01, 0x08, 9, 9]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(response.operation.unwrap().bought(), None);

        // assumed sell ack: 01 09 <slot 14> <quantity u16 2> (+ tolerated tail)
        let bytes = Bytes::from_static(&[0x01, 0x09, 14, 2, 0, 7]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(response.operation.unwrap().sold(), Some((14, 2)));
    }

    #[test]
    fn sell_ack_tail_is_npc_model_and_buyback_slot() {
        // real capture, packet_dump/0xb034.log: 01 09 1b 0100 d7000000 01
        let bytes =
            Bytes::from_static(&[0x01, 0x09, 0x1b, 0x01, 0x00, 0xd7, 0x00, 0x00, 0x00, 0x01]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        let op = response.operation.clone().unwrap();
        assert_eq!(op.sold(), Some((0x1b, 1)));
        assert_eq!(op.sold_buyback(), Some((0xd7, 1)));
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // short (pre-tail) acks keep working and report no buyback slot
        let short = Bytes::from_static(&[0x01, 0x09, 14, 2, 0]);
        let op = InventoryOperationResponse::try_from(short)
            .unwrap()
            .operation
            .unwrap();
        assert_eq!(op.sold(), Some((14, 2)));
        assert_eq!(op.sold_buyback(), None);
    }

    #[test]
    fn buyback_ack_roundtrips() {
        // xBot PacketParser.cs:2233-2245 — 01 22 <slot> <slot_buyback> <qty u16>
        let bytes = Bytes::from_static(&[0x01, 34, 0x0d, 0x02, 0x03, 0x00]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::BuyBack {
                slot: 0x0d,
                slot_buyback: 2,
                quantity: 3,
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
    }

    #[test]
    fn move_response_success_roundtrips() {
        // bool(1), op(0), source, target, amount u16, chained-record count
        let bytes = Bytes::from_static(&[0x01, 0x00, 13, 6, 0x01, 0x00, 0x00]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(
            response,
            InventoryOperationResponse {
                result: 1,
                operation: Some(InventoryOperationResult::Move {
                    source: 13,
                    target: 6,
                    amount: 1,
                    chained: Vec::new(),
                }),
                error: None,
            }
        );
    }

    /// A two-handed equip: the server moves the weapon in and, in the same
    /// body, moves the shield out of slot 7 on its own.
    ///
    /// **These are the real captured bytes** (2026-08-26T20:50:32.313Z), so
    /// this pins the parser against the wire rather than against a reading of
    /// it. Dropping the tail is what left the shield stuck in its slot and made
    /// every later move of it fail with `0x1809` "item gone".
    #[test]
    fn a_move_ack_carries_the_servers_own_follow_up_moves() {
        let bytes = Bytes::from_static(&[
            0x01, 0x00, 0x3d, 0x06, 0x01, 0x00, 0x01, 0x00, 0x07, 0x2d, 0x00, 0x00,
        ]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        let Some(InventoryOperationResult::Move {
            source,
            target,
            ref chained,
            ..
        }) = response.operation
        else {
            panic!("not parsed as a move: {response:?}");
        };
        let chained = chained.clone();
        assert_eq!((source, target), (0x3d, 6), "the glaive, bag 61 -> weapon");
        assert_eq!(
            chained,
            vec![ChainedMove {
                op: 0,
                source: 7,
                target: 0x2d,
                amount: 0,
            }],
            "the shield's own move, 7 -> bag 45, must survive the parse"
        );
        // and it re-encodes to exactly what came in
        assert_eq!(Bytes::from(response), bytes);
    }

    /// A truncated tail must degrade, not panic: the count byte is the
    /// server's, and a body that does not actually carry that many records
    /// would otherwise index past the end.
    #[test]
    fn a_lying_chain_count_does_not_panic() {
        // claims 4 chained records, carries one and a half
        let bytes = Bytes::from_static(&[
            0x01, 0x00, 0x3d, 0x06, 0x01, 0x00, 0x04, 0x00, 0x07, 0x2d, 0x00, 0x00, 0x00, 0x07,
        ]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        let Some(InventoryOperationResult::Move { chained, .. }) = response.operation else {
            panic!("not a move");
        };
        assert_eq!(chained.len(), 1, "only the complete record is read");
    }

    /// Op 7 — `[V]`, the real bytes of a drop ack (2026-08-26T20:52:07.778Z).
    /// It echoes the slot and nothing else.
    #[test]
    fn a_drop_ack_names_the_emptied_slot() {
        let bytes = Bytes::from_static(&[0x01, 0x07, 0x20]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::Drop { slot: 0x20 })
        );
        assert_eq!(Bytes::from(response), bytes);
    }

    #[test]
    fn move_response_failure_carries_error_code() {
        let bytes = Bytes::from_static(&[0x02, 0x07, 0x00]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(response.result, 2);
        assert_eq!(response.operation, None);
        assert_eq!(response.error, Some(7));
    }

    struct MockResolver(ItemClass);
    impl ItemClassResolver for MockResolver {
        fn item_class(&self, _ref_id: u32) -> ItemClass {
            self.0
        }
    }

    /// The five COS-bag ops of `docs/re/systems/pet-pick-cos.md` §3, each with
    /// the exact body the doc records — and each round-tripping, which is what
    /// proves no field was invented or dropped.
    #[test]
    fn pick_pet_bag_ops_decode_the_documented_bodies() {
        // op 25 PetToPet — the only pet op that carries a quantity (:2039-2042)
        let bytes = Bytes::from_static(&[0x01, 25, 0x39, 0x05, 0, 0, 3, 7, 0x05, 0x00]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::PetToPet {
                cos_unique_id: 0x539,
                source: 3,
                target: 7,
                amount: 5,
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // op 26 PetToInventory — pet slot then inventory slot (:2094-2096)
        let bytes = Bytes::from_static(&[0x01, 26, 0x39, 0x05, 0, 0, 4, 13]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::PetToInventory {
                cos_unique_id: 0x539,
                pet_slot: 4,
                inventory_slot: 13,
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // op 27 InventoryToPet — the two slots the other way round (:2108-2110)
        let bytes = Bytes::from_static(&[0x01, 27, 0x39, 0x05, 0, 0, 13, 4]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::InventoryToPet {
                cos_unique_id: 0x539,
                inventory_slot: 13,
                pet_slot: 4,
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
    }

    /// The two avatar ops (35/36): two bare slots, nothing else, and the
    /// round-trip holds. `[S]` — no capture of either exists on this machine
    /// (one `0x7034` dump, 7 frames, all op 8), so this pins the reading, not
    /// a frame.
    #[test]
    fn the_avatar_ops_carry_two_bare_slots() {
        // op 35 — take off: avatar slot 2 -> bag slot 14
        let bytes = Bytes::from_static(&[0x01, 35, 2, 14]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::AvatarToInventory {
                source: 2,
                target: 14,
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // op 36 — put on: the same two bytes with the slots swapped
        let bytes = Bytes::from_static(&[0x01, 36, 14, 2]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::InventoryToAvatar {
                source: 14,
                target: 2,
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
    }

    /// Ops 26/27 carry **no** quantity, so a pet<->inventory move is
    /// whole-slot only; only the intra-container op 25 can split a stack
    /// (`pet-pick-cos.md` §3, second trap). Pinned on the byte lengths so a
    /// later "helpful" quantity field cannot be added silently.
    #[test]
    fn pet_inventory_moves_carry_no_quantity() {
        let whole_slot = Bytes::from_static(&[0x01, 26, 0x39, 0x05, 0, 0, 4, 13]);
        let split = Bytes::from_static(&[0x01, 25, 0x39, 0x05, 0, 0, 3, 7, 0x05, 0x00]);
        assert_eq!(whole_slot.len(), 8);
        assert_eq!(split.len(), 10);
        for bytes in [whole_slot, split] {
            let back: Bytes = InventoryOperationResponse::try_from(bytes.clone())
                .unwrap()
                .into();
            assert_eq!(back, bytes, "op body length changed");
        }
    }

    /// Op 17 grabs straight into the pet's bag and carries a full item record
    /// behind the slot (:1983-1986), decoded through the same resolver-driven
    /// parser as op 6.
    #[test]
    fn ground_to_pet_carries_an_item_record() {
        // 01 11 <cosUid u32> <slot> | rent 0, ref 4, stack 2
        let bytes = Bytes::from_static(&[
            0x01, 17, 0x39, 0x05, 0, 0, 5, 0, 0, 0, 0, 0x04, 0, 0, 0, 0x02, 0,
        ]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        let op = response.operation.clone().unwrap();
        assert!(matches!(
            op,
            InventoryOperationResult::GroundToPet {
                cos_unique_id: 0x539,
                slot: 5,
                ..
            }
        ));
        let item = op
            .pet_pickup_item(&MockResolver(ItemClass::Expendable { tid3: 0, tid4: 0 }))
            .unwrap();
        assert_eq!(item.slot, 5);
        assert_eq!(item.ref_id, 4);
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
    }

    /// The op-28 trap: `slot == 254` means the grab did **not** land in the
    /// character's inventory and the body **ends at the slot byte**. Reading
    /// an item record there is what desyncs the stream
    /// (`pet-pick-cos.md` §3, first trap), so the sentinel path must consume
    /// nothing and yield no item.
    #[test]
    fn op28_slot_254_sentinel_reads_no_item_body() {
        let bytes = Bytes::from_static(&[0x01, 28, 0x39, 0x05, 0, 0, PET_PICKUP_SLOT_NONE]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        let op = response.operation.clone().unwrap();
        assert_eq!(
            op,
            InventoryOperationResult::GroundToPetToInventory {
                cos_unique_id: 0x539,
                slot: PET_PICKUP_SLOT_NONE,
                tail: Bytes::new(),
            }
        );
        assert!(op
            .pet_pickup_item(&MockResolver(ItemClass::Equipment))
            .is_none());
        // the whole body is exactly 7 bytes — nothing was consumed past it
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
        assert_eq!(back.len(), 7);

        // ...and the non-sentinel form of the same op does carry the record
        let bytes = Bytes::from_static(&[
            0x01, 28, 0x39, 0x05, 0, 0, 13, 0, 0, 0, 0, 0x04, 0, 0, 0, 0x02, 0,
        ]);
        let op = InventoryOperationResponse::try_from(bytes.clone())
            .unwrap()
            .operation
            .unwrap();
        let item = op
            .pet_pickup_item(&MockResolver(ItemClass::Expendable { tid3: 0, tid4: 0 }))
            .unwrap();
        assert_eq!(item.slot, 13);
        assert_eq!(item.ref_id, 4);
    }

    #[test]
    fn pickup_response_accessors() {
        // gold pile: op 6, gold pseudo-slot, u32 amount
        let bytes = Bytes::from_static(&[0x01, 0x06, 0xFE, 0x2C, 0, 0, 0]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        let op = response.operation.clone().unwrap();
        assert_eq!(op.pickup_gold(), Some(44));
        assert!(op
            .pickup_item(&MockResolver(ItemClass::Equipment))
            .is_none());
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // expendable item: op 6, slot, rent 0, ref id, stack u16
        let bytes = Bytes::from_static(&[0x01, 0x06, 13, 0, 0, 0, 0, 0x04, 0, 0, 0, 0x02, 0]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        let op = response.operation.unwrap();
        let item = op
            .pickup_item(&MockResolver(ItemClass::Expendable { tid3: 0, tid4: 0 }))
            .unwrap();
        assert_eq!(item.slot, 13);
        assert_eq!(item.ref_id, 4);
        assert_eq!(
            item.data,
            ItemTypeData::Expendable {
                stack_count: 2,
                assimilation_prob: None,
                mag_params: Vec::new()
            }
        );
        assert_eq!(op.pickup_gold(), None);

        // equipment item (live capture): opt_level, variance, durability,
        // sockets — must NOT be misread as a giant stack
        let bytes = Bytes::from_static(&[
            0x01, 0x06, 24, 0, 0, 0, 0, 0xC3, 0x05, 0, 0, // rent 0, ref 1475
            0x00, 0xC3, 0x84, 0x47, 0x12, 0, 0, 0, 0, // opt 0, variance
            0x2C, 0, 0, 0, // durability 44
            0x00, 0x01, 0x00, 0x02, 0x00, // magparams 0, socket tag/count, elixir tag/count
        ]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        let op = response.operation.unwrap();
        let item = op.pickup_item(&MockResolver(ItemClass::Equipment)).unwrap();
        assert_eq!(item.ref_id, 1475);
        assert!(
            matches!(item.data, ItemTypeData::Equipment(ref eq) if eq.opt_level == 0 && eq.durability == 44)
        );

        // unknown op stays raw and round-trips
        let bytes = Bytes::from_static(&[0x01, 0x0A, 1, 2, 3]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert!(matches!(
            response.operation,
            Some(InventoryOperationResult::Unknown { op: 0x0A, .. })
        ));
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
    }

    #[test]
    fn equip_and_unequip_parse() {
        let equip = EntityEquip::try_from(Bytes::from_static(&[
            0x39, 0x05, 0, 0, 6, 0x81, 0x0E, 0, 0, 1,
        ]))
        .unwrap();
        assert_eq!(equip.unique_id, 0x539);
        assert_eq!(equip.slot, 6);
        assert_eq!(equip.ref_id, 3713);
        assert_eq!(equip.one_handed, 1);

        let unequip =
            EntityUnequip::try_from(Bytes::from_static(&[0x39, 0x05, 0, 0, 6, 0x81, 0x0E, 0, 0]))
                .unwrap();
        assert_eq!(unequip.unique_id, 0x539);
        assert_eq!(unequip.slot, 6);
        assert_eq!(unequip.ref_id, 3713);
    }

    #[test]
    fn storage_requests_roundtrip() {
        // layouts from xBot's MoveItem(..., uint uniqueID) overload:
        // type, slotInitial, slotFinal, npc u32
        let mv = InventoryOperationRequest::StorageToStorage {
            source: 2,
            target: 7,
            npc_unique_id: 0x539,
        };
        let bytes: Bytes = mv.clone().into();
        assert_eq!(bytes.as_ref(), &[0x01, 2, 7, 0x39, 0x05, 0, 0]);
        assert_eq!(InventoryOperationRequest::try_from(bytes).unwrap(), mv);

        let deposit = InventoryOperationRequest::InventoryToStorage {
            source: 14,
            target: 3,
            npc_unique_id: 0x539,
        };
        let bytes: Bytes = deposit.clone().into();
        assert_eq!(bytes.as_ref(), &[0x02, 14, 3, 0x39, 0x05, 0, 0]);
        assert_eq!(InventoryOperationRequest::try_from(bytes).unwrap(), deposit);

        let withdraw = InventoryOperationRequest::StorageToInventory {
            source: 3,
            target: 14,
            npc_unique_id: 0x539,
        };
        let bytes: Bytes = withdraw.clone().into();
        assert_eq!(bytes.as_ref(), &[0x03, 3, 14, 0x39, 0x05, 0, 0]);
        assert_eq!(
            InventoryOperationRequest::try_from(bytes).unwrap(),
            withdraw
        );

        // gold DEPOSIT is op 12 (inventory -> storage); op 11 is the withdraw.
        // Nine bytes, not thirteen: the gold arm carries no NPC id — two
        // captured frames say so (`0c 0b00000000000000`), and the session the
        // server books against is the one 0x703C opened.
        let gold = InventoryOperationRequest::InventoryGoldToStorage { amount: 1000 };
        let bytes: Bytes = gold.clone().into();
        assert_eq!(bytes.as_ref(), &[0x0C, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]);
        assert_eq!(InventoryOperationRequest::try_from(bytes).unwrap(), gold);
    }

    #[test]
    fn storage_responses_roundtrip() {
        // deposit ack: 01 02 <inventory slot 14> <storage slot 3>
        let bytes = Bytes::from_static(&[0x01, 0x02, 14, 3]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::InventoryToStorage {
                source: 14,
                target: 3
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // withdraw ack: 01 03 <storage slot 3> <inventory slot 14>
        let bytes = Bytes::from_static(&[0x01, 0x03, 3, 14]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::StorageToInventory {
                source: 3,
                target: 14
            })
        );

        // within-storage move ack: 01 01 <source> <target> <amount u16>
        let bytes = Bytes::from_static(&[0x01, 0x01, 2, 7, 5, 0]);
        let response = InventoryOperationResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::StorageToStorage {
                source: 2,
                target: 7,
                amount: 5
            })
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // gold acks carry only the amount; 11 = out of storage, 12 = into it
        let bytes = Bytes::from_static(&[0x01, 0x0B, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::StorageGoldToInventory { amount: 1000 })
        );
        let bytes = Bytes::from_static(&[0x01, 0x0C, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert_eq!(
            response.operation,
            Some(InventoryOperationResult::InventoryGoldToStorage { amount: 1000 })
        );

        // a truncated storage ack degrades to Unknown instead of erroring
        let bytes = Bytes::from_static(&[0x01, 0x02, 14]);
        let response = InventoryOperationResponse::try_from(bytes).unwrap();
        assert!(matches!(
            response.operation,
            Some(InventoryOperationResult::Unknown { op: 2, .. })
        ));
    }

    #[test]
    fn repair_request_roundtrips_and_response_keeps_tail() {
        // EXPERIMENTAL body (undocumented) — the test pins OUR v2 wire
        // shape: npc id first, then type (1 single + slot / 2 all)
        let one = ItemRepairRequest::One {
            slot: 3,
            npc_unique_id: 343,
        };
        let bytes: Bytes = one.clone().into();
        assert_eq!(&bytes[..], &[0x57, 0x01, 0, 0, 1, 3]);
        let decoded: ItemRepairRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, one);

        let all = ItemRepairRequest::All { npc_unique_id: 343 };
        let bytes: Bytes = all.clone().into();
        assert_eq!(&bytes[..], &[0x57, 0x01, 0, 0, 2]);
        let decoded: ItemRepairRequest = bytes.try_into().unwrap();
        assert_eq!(decoded, all);

        let wire = Bytes::from_static(&[0x01, 0xAA, 0xBB]);
        let response = ItemRepairResponse::try_from(wire.clone()).unwrap();
        assert!(response.is_success());
        assert_eq!(&response.tail[..], &[0xAA, 0xBB]);
        let back: Bytes = response.into();
        assert_eq!(back, wire);
    }

    /// #454: the body is switched on item class. One wire vector per builder,
    /// slot 13 = the first bag slot (already the `+0x0D`-biased wire number).
    #[test]
    fn item_use_request_roundtrips_per_variant() {
        // FUN_00796660:550 with no target set — HP potion (type_id 0x08EC).
        let simple = ItemUseRequest::Simple {
            slot: 13,
            type_id: 0x08EC,
        };
        let bytes: Bytes = simple.clone().into();
        assert_eq!(bytes.as_ref(), &[13, 0xEC, 0x08]);
        assert_eq!(ItemUseRequest::try_from(bytes).unwrap(), simple);

        // FUN_00796660:286-300 — slot, type_id, target u32 LE, kind u8.
        let targeted = ItemUseRequest::WithTarget {
            slot: 14,
            type_id: 0x08EC,
            target: 0x0000_2A1F,
            kind: 0x46,
        };
        let bytes: Bytes = targeted.clone().into();
        assert_eq!(
            bytes.as_ref(),
            &[14, 0xEC, 0x08, 0x1F, 0x2A, 0x00, 0x00, 0x46]
        );
        assert_eq!(ItemUseRequest::try_from(bytes).unwrap(), targeted);

        // FUN_007901c0 — slot, type_id, then FUN_0053d8c0's u16 length + ASCII.
        let named = ItemUseRequest::WithName {
            slot: 15,
            type_id: 0x09EC,
            name: "Jangan".to_string(),
        };
        let bytes: Bytes = named.clone().into();
        assert_eq!(
            bytes.as_ref(),
            &[15, 0xEC, 0x09, 0x06, 0x00, b'J', b'a', b'n', b'g', b'a', b'n']
        );
        assert_eq!(ItemUseRequest::try_from(bytes).unwrap(), named);
    }

    /// A truncated or over-long name must not silently become a short packet —
    /// sending a body the server reads past is exactly the #215 failure mode.
    #[test]
    fn item_use_request_rejects_a_malformed_name_body() {
        // len says 6, only 3 bytes follow.
        let wire = Bytes::from_static(&[15, 0xEC, 0x09, 0x06, 0x00, b'J', b'a', b'n', b'g']);
        assert!(ItemUseRequest::try_from(wire).is_err());
        // len says 2, but 4 bytes follow. (Deliberately 9 bytes long: at
        // exactly 8 the length rule reads the body as `WithTarget`, which is
        // the documented ambiguity, not a malformed body.)
        let wire = Bytes::from_static(&[15, 0xEC, 0x09, 0x02, 0x00, b'J', b'a', b'n', b'g']);
        assert!(ItemUseRequest::try_from(wire).is_err());
    }

    #[test]
    fn item_use_response_success_and_error_roundtrip() {
        // success: 01 <slot> <remaining u16> <type_id u16>
        let bytes = Bytes::from_static(&[0x01, 13, 0x04, 0x00, 0xEC, 0x08]);
        let response = ItemUseResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response,
            ItemUseResponse::Success {
                slot: 13,
                remaining: 4,
                type_id: 0x08EC,
            }
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);

        // failure: 02 <code u16> — reuse-delay (on cooldown)
        let bytes = Bytes::from_static(&[0x02, 0x5B, 0x18]);
        let response = ItemUseResponse::try_from(bytes.clone()).unwrap();
        assert_eq!(
            response,
            ItemUseResponse::Error {
                code: ITEM_USE_ERROR_REUSE_DELAY
            }
        );
        let back: Bytes = response.into();
        assert_eq!(back, bytes);
    }

    /// All five `packet_dump/0x3052.log` bodies: 5 bytes each, slot 6 is the
    /// weapon and slot 1 the chest, both wearing down over the session.
    #[test]
    fn inventory_durability_update_decodes_live_captures() {
        for (wire, slot, durability) in [
            ([0x06u8, 0x31, 0, 0, 0], 6u8, 49u32),
            ([0x01, 0x2F, 0, 0, 0], 1, 47),
            ([0x01, 0x2E, 0, 0, 0], 1, 46),
            ([0x06, 0x44, 0, 0, 0], 6, 68),
            ([0x06, 0x43, 0, 0, 0], 6, 67),
        ] {
            let bytes = Bytes::copy_from_slice(&wire);
            let decoded = InventoryItemDurabilityUpdate::try_from(bytes.clone()).unwrap();
            assert_eq!((decoded.slot, decoded.durability), (slot, durability));
            let back: Bytes = decoded.into();
            assert_eq!(back, bytes);
        }
    }

    /// The second byte is a mask, so the single-bit forms must still decode and
    /// re-encode byte-exactly — including the one real capture of this opcode
    /// in the workspace, a 3-byte COS push.
    #[test]
    fn single_bit_masks_round_trip() {
        // quantity u16 (0x08)
        let bytes = Bytes::from_static(&[0x0D, 0x08, 0x09, 0x00]);
        let decoded = InventoryItemUpdate::try_from(bytes.clone()).unwrap();
        assert_eq!(decoded.slot, 13);
        assert_eq!((decoded.quantity, decoded.cos_state), (Some(9), None));
        assert_eq!(Bytes::from(decoded), bytes);

        // COS state u8 (0x40), 4 = dead — the body of `packet_dump/0x3040.log`
        let bytes = Bytes::from_static(&[0x17, 0x40, 0x04]);
        let decoded = InventoryItemUpdate::try_from(bytes.clone()).unwrap();
        assert_eq!((decoded.slot, decoded.cos_state), (0x17, Some(4)));
        assert_eq!(Bytes::from(decoded), bytes);
    }

    /// The reason this type was rewritten: a combined mask used to decode to
    /// "unknown type, no fields" and drop everything it carried. `0x18` is
    /// durability **and** quantity, in the original's block order.
    #[test]
    fn a_combined_mask_yields_every_block_it_announces() {
        let bytes = Bytes::from_static(&[
            0x0D, 0x18, // slot 13, mask = QUANTITY | DURABILITY
            0x09, 0x00, // quantity 9   (0x08 comes first)
            0x2E, 0x00, 0x00, 0x00, // durability 46
        ]);
        let decoded = InventoryItemUpdate::try_from(bytes.clone()).unwrap();
        assert_eq!(decoded.quantity, Some(9));
        assert_eq!(decoded.durability, Some(46));
        assert_eq!(Bytes::from(decoded), bytes);
    }

    /// All eight blocks at once, in the order the original reads them. This is
    /// the test that pins the order: swap any two and the round trip breaks.
    #[test]
    fn every_block_in_the_original_s_order() {
        let bytes = Bytes::from_static(&[
            0x0D, 0xFF, // every bit
            0x01, 0x02, 0x03, 0x04, // 0x01 u32
            0x07, // 0x02 plus
            0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, // 0x04 variance
            0x09, 0x00, // 0x08 quantity
            0x2E, 0x00, 0x00, 0x00, // 0x10 durability
            0x02, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, // 0x20 count + 2 records
            0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, //
            0x02, // 0x40 cos state
            0x3C, 0x00, 0x00, 0x00, // 0x80 duration seconds
        ]);
        let decoded = InventoryItemUpdate::try_from(bytes.clone()).unwrap();
        assert_eq!(decoded.unknown01, Some(0x04030201));
        assert_eq!(decoded.plus, Some(7));
        assert_eq!(decoded.variance, Some(0x1817161514131211));
        assert_eq!(decoded.quantity, Some(9));
        assert_eq!(decoded.durability, Some(46));
        assert_eq!(decoded.magic_options.as_deref().map(<[u64]>::len), Some(2));
        assert_eq!(decoded.cos_state, Some(2));
        assert_eq!(decoded.duration_seconds, Some(60));
        assert_eq!(decoded.byte_size(), bytes.len());
        assert_eq!(Bytes::from(decoded), bytes);
    }

    /// A body that stops inside a block the mask announced is an error, not a
    /// silent zero — and a magic-param count above the original's own assert
    /// (`m_btMagicParamNum <= MAX_MAGPARAM_PER_ITEM`) is refused.
    #[test]
    fn a_truncated_or_overlong_body_is_refused() {
        // mask says durability, body has two of the four bytes
        assert!(
            InventoryItemUpdate::try_from(Bytes::from_static(&[0x0D, 0x10, 0x2E, 0x00])).is_err()
        );
        // count 13 > MAX_MAGPARAM_PER_ITEM
        assert!(InventoryItemUpdate::try_from(Bytes::from_static(&[0x0D, 0x20, 0x0D])).is_err());
        // mask 0 is a legal "nothing changed" push and keeps the mask readable
        let decoded = InventoryItemUpdate::try_from(Bytes::from_static(&[0x0F, 0x00])).unwrap();
        assert_eq!((decoded.slot, decoded.mask), (0x0F, 0));
    }

    /// 0x3092 carries the new slot count only on success.
    #[test]
    fn inventory_capacity_update_tail_follows_success() {
        let bytes = Bytes::from_static(&[0x01, 0x6D]);
        let decoded = InventoryCapacityUpdate::try_from(bytes.clone()).unwrap();
        assert!(decoded.success);
        assert_eq!(decoded.new_capacity, Some(109));
        let back: Bytes = decoded.into();
        assert_eq!(back, bytes);

        let decoded = InventoryCapacityUpdate::try_from(Bytes::from_static(&[0x00])).unwrap();
        assert!(!decoded.success);
        assert_eq!(decoded.new_capacity, None);
    }

    /// The gold arm carries no NPC id. Two captured op-12 frames and eight
    /// op-13 frames are all nine bytes — `op + u64` — and a body with the NPC
    /// id this type used to declare would be thirteen.
    #[test]
    fn the_gold_ops_are_nine_bytes_like_the_capture() {
        for (req, want) in [
            (
                InventoryOperationRequest::InventoryGoldToStorage { amount: 11 },
                &[0x0c, 0x0b, 0, 0, 0, 0, 0, 0, 0][..],
            ),
            (
                InventoryOperationRequest::InventoryGoldToExchange { amount: 100 },
                &[0x0d, 0x64, 0, 0, 0, 0, 0, 0, 0][..],
            ),
        ] {
            let wire: Bytes = req.clone().into();
            assert_eq!(&wire[..], want);
            assert_eq!(wire.len(), 9);
            assert_eq!(InventoryOperationRequest::try_from(wire).unwrap(), req);
        }
    }

    /// The five guild-warehouse ops. No capture holds one — none of the 99
    /// captured 0x7034 frames carries op 29..=33 — so what is pinned here is
    /// the shape the port claims, byte for byte, and that each op byte is the
    /// one the enum promises.
    #[test]
    fn the_guild_warehouse_ops_roundtrip_their_claimed_shapes() {
        let cases: Vec<(InventoryOperationRequest, Vec<u8>)> = vec![
            (
                InventoryOperationRequest::GuildStorageToGuildStorage {
                    source: 0x0d,
                    target: 0x0e,
                    amount: 1,
                    npc_unique_id: 0x0001_60AA,
                },
                vec![29, 0x0d, 0x0e, 0x01, 0x00, 0xAA, 0x60, 0x01, 0x00],
            ),
            (
                InventoryOperationRequest::InventoryToGuildStorage {
                    source: 0x0d,
                    target: 0x00,
                    npc_unique_id: 0x0001_60AA,
                },
                vec![30, 0x0d, 0x00, 0xAA, 0x60, 0x01, 0x00],
            ),
            (
                InventoryOperationRequest::GuildStorageToInventory {
                    source: 0x00,
                    target: 0x0d,
                    npc_unique_id: 0x0001_60AA,
                },
                vec![31, 0x00, 0x0d, 0xAA, 0x60, 0x01, 0x00],
            ),
            (
                InventoryOperationRequest::InventoryGoldToGuildStorage { amount: 500 },
                vec![32, 0xf4, 0x01, 0, 0, 0, 0, 0, 0],
            ),
            (
                InventoryOperationRequest::GuildStorageGoldToInventory { amount: 500 },
                vec![33, 0xf4, 0x01, 0, 0, 0, 0, 0, 0],
            ),
        ];
        for (req, want) in cases {
            let wire: Bytes = req.clone().into();
            assert_eq!(&wire[..], &want[..], "{req:?}");
            assert_eq!(InventoryOperationRequest::try_from(wire).unwrap(), req);
        }
    }

    /// The acks for the same five ops, read the way xBot reads them.
    #[test]
    fn the_guild_warehouse_acks_decode_their_claimed_shapes() {
        let ok = |body: &[u8]| {
            let mut v = vec![0x01u8];
            v.extend_from_slice(body);
            InventoryOperationResponse::try_from(Bytes::from(v)).unwrap()
        };
        let success = |r: InventoryOperationResponse| {
            assert_eq!(r.result, 1, "the fixture builds a success ack");
            r.operation.expect("a success ack carries its operation")
        };
        assert_eq!(
            success(ok(&[29, 0x0d, 0x0e, 0x01, 0x00])),
            InventoryOperationResult::GuildStorageToGuildStorage {
                source: 0x0d,
                target: 0x0e,
                amount: 1,
            }
        );
        assert_eq!(
            success(ok(&[30, 0x0d, 0x00])),
            InventoryOperationResult::InventoryToGuildStorage {
                source: 0x0d,
                target: 0x00,
            }
        );
        assert_eq!(
            success(ok(&[31, 0x00, 0x0d])),
            InventoryOperationResult::GuildStorageToInventory {
                source: 0x00,
                target: 0x0d,
            }
        );
        assert_eq!(
            success(ok(&[32, 0xf4, 0x01, 0, 0, 0, 0, 0, 0])),
            InventoryOperationResult::InventoryGoldToGuildStorage { amount: 500 }
        );
        assert_eq!(
            success(ok(&[33, 0xf4, 0x01, 0, 0, 0, 0, 0, 0])),
            InventoryOperationResult::GuildStorageGoldToInventory { amount: 500 }
        );
    }
}

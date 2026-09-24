//! Guild storage (guild warehouse) session state: the client half of the
//! 0x7250 / 0x7251 / 0x7252 / 0xB250 / 0x3253 / 0x3255 / 0x3254 family.
//!
//! Idea: this is the guild analogue of `hud::storage`, with one structural
//! difference that decides the shape of the code — **the item stream is a
//! reply, not a push the server volunteers**. The original's 0xB250 success
//! arm sends 0x7252 itself (the original's builder,
//! `docs/net-guild-storage-0x7250.md`), so a consumer that opens and then
//! waits for 0x3253 waits forever. Hence [`on_guild_storage_response`] is the
//! system that asks for the contents.
//!
//! The rest mirrors the personal warehouse deliberately: the 0x3255 chunks are
//! meaningless alone and accumulate in [`GuildStorageDataBuffer`] until the
//! 0x3254 marker parses them, and the three handlers are `.chain()`ed —
//! unordered, `end` parses before `chunk` has filled the buffer, which is the
//! exact defect `docs/net-storage-0x3047-0x3049.md:162-165` records for the
//! personal family.
//!
//! The window is [`super::ui`] — `GDR_GUILDSTORAGEROOM`
//! (`resinfo/ginterface.txt:1353-1371`), the same `CIFStorageRoom` class and
//! the same 254x317 frame as the personal warehouse, which is why it borrows
//! `ifstorageroom.txt`'s interior rather than a layout file of its own (there
//! is none). The guild `0x7034` item ops **29/30/31** are wired in `ui.rs` and
//! applied here by [`on_guild_storage_operation`]; the gold ops 32/33 have
//! encoders but no button yet (see `ui.rs`).
//!
//! Nothing here is confirmed on the wire: none of the seven opcodes has ever
//! been seen on a live line and go-sro implements none of them.

use bevy::prelude::*;

use packets::agent::guild::GuildPermissions;
use packets::agent::guild_storage::{
    parse_guild_storage_items, GuildStorageCloseRequest, GuildStorageDataBegin,
    GuildStorageDataChunk, GuildStorageDataEnd, GuildStorageListRequest, GuildStorageOpenRequest,
    GuildStorageResponse,
};
use packets::agent::prelude::{InventoryOperationResponse, InventoryOperationResult};
use packets::{hexdump, Packet};

use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::npc_dialog::model::{NpcDialogState, OpenGuildStorage};
use crate::plugins::hud::system_message::model::format_template;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::net::entities::NetworkId;
use crate::plugins::net::guild::GuildRoster;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

/// "At the moment guild member %s is using guild storage, so it cannot be
/// opened." — the original formats exactly this key with the name the 0xB250
/// in-use arm carries (`textuisystem.txt:1423`).
pub const IN_USE_KEY: &str = "UIIT_MSG_GUILD_WAREHOUSE_USE";
pub const IN_USE_FALLBACK: &str =
    "At the moment guild member %s is using guild storage, so it cannot be opened.";

/// "Guild level must be 2 or higher to use guild storage."
/// (`textuisystem.txt:1424`) — the gate the dialog label itself advertises
/// ("Use guild storage. (level 2 or above)").
pub const LEVEL_KEY: &str = "UIIT_MSG_GUILD_WAREHOUSE_LIMIT";
pub const LEVEL_FALLBACK: &str = "Guild level must be 2 or higher to use guild storage.";

/// "You are not authorized." (`textuisystem.txt:1406`) — the guild family's own
/// permission refusal, reused for the `Storage = 8` bit rather than inventing a
/// storage-specific string.
pub const DENIED_KEY: &str = "UIIT_MSG_GUILDERR_PERMISSION_DENIED";
pub const DENIED_FALLBACK: &str = "You are not authorized.";

/// The guild level the client's own string names as the minimum
/// (`UIIT_MSG_GUILD_WAREHOUSE_LIMIT`, and the dialog label "(level 2 or
/// above)"). Sourced, not chosen.
pub const MIN_GUILD_LEVEL: u8 = 2;

/// The 0x3255 chunks between a 0x3253 begin and its 0x3254 end.
#[derive(Resource, Default)]
pub struct GuildStorageDataBuffer(pub Vec<u8>);

#[derive(Clone, Debug, PartialEq)]
pub struct GuildStorageSession {
    /// The warehouse NPC the session belongs to — the 0x7251 close needs the
    /// same id the 0x7250 open carried.
    pub npc: Entity,
    pub npc_id: u32,
}

/// The guild warehouse as the server last described it. Model only: the
/// window in `guild_storage::ui` renders *from* this resource and never
/// writes wire state into it.
#[derive(Resource, Default)]
pub struct GuildStorageState {
    pub session: Option<GuildStorageSession>,
    /// Slots + the guild account's gold, mirrored ONLY from server packets,
    /// reusing [`Inventory`] exactly like the personal warehouse does.
    pub items: Inventory,
    /// The item stream arrived for this session.
    pub synced: bool,
    /// Visible page of the 6x5 window grid (`ui::sync_guild_storage_window`).
    /// Lives here, not in the UI, for the personal warehouse's reason: the
    /// window is rebuilt from this resource, so a page flip *is* a state change.
    pub active_page: u8,
}

impl GuildStorageState {
    /// Pages the window can flip through. Ceil-division of the server-declared
    /// capacity (the `0x3254` `capacity:u8`) by the wire page size; at least 1,
    /// so an unsynced warehouse still renders one empty page.
    pub fn page_count(&self) -> u8 {
        self.items
            .size()
            .div_ceil(crate::plugins::hud::storage::model::STORAGE_SLOTS_PER_PAGE)
            .max(1)
    }
}

/// Why an open attempt did not reach the wire. Split out so the gate is
/// testable without a Bevy `App` and without a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuildStorageGate {
    Allowed,
    /// The player is in no guild, or is not in the pushed roster.
    NotAMember,
    /// In the guild, without the `Storage = 8` bit.
    NotAuthorized,
    /// Guild level below [`MIN_GUILD_LEVEL`].
    GuildLevelTooLow,
}

/// The client-side gate the original advertises in its own strings: guild
/// membership, then the level, then the `Storage` permission bit
/// (`GUILD_PERMISSION_STORAGE = 8`, `xBot/…/SRGuildMember.cs:35`).
///
/// Deviation, stated (ADR-0009): the *server* is authoritative here and would
/// refuse anyway. The client pre-check exists because the refusal it would send
/// back is an unknown error code that cannot be mapped to a message, whereas
/// these three strings are in the player's own data.
pub fn gate(level: Option<u8>, permissions: Option<GuildPermissions>) -> GuildStorageGate {
    let (Some(level), Some(permissions)) = (level, permissions) else {
        return GuildStorageGate::NotAMember;
    };
    if level < MIN_GUILD_LEVEL {
        return GuildStorageGate::GuildLevelTooLow;
    }
    if !permissions.can_use_storage() {
        return GuildStorageGate::NotAuthorized;
    }
    GuildStorageGate::Allowed
}

/// The dialog's "Use guild storage." option: check the gate, then ask the
/// server to open (0x7250).
#[allow(clippy::too_many_arguments)]
pub fn open_guild_storage(
    mut requests: MessageReader<OpenGuildStorage>,
    npcs: Query<&NetworkId>,
    players: Query<&CharacterInfo, With<Player>>,
    roster: Res<GuildRoster>,
    ui_strings: Res<ClientUiStrings>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut state: ResMut<GuildStorageState>,
    mut buffer: ResMut<GuildStorageDataBuffer>,
    mut history: ResMut<ChatHistory>,
) {
    for OpenGuildStorage { npc } in requests.read() {
        let Ok(network_id) = npcs.get(*npc) else {
            warn!("guild storage: OpenGuildStorage for an entity without a network id");
            continue;
        };
        let name = players.single().ok().and_then(|info| info.name.clone());
        let permissions = name.as_deref().and_then(|n| roster.permissions_for(n));
        match gate(roster.level(), permissions) {
            GuildStorageGate::Allowed => {}
            GuildStorageGate::GuildLevelTooLow => {
                history.push(ChatLine::system(
                    ui_strings.get_or(LEVEL_KEY, LEVEL_FALLBACK),
                ));
                continue;
            }
            GuildStorageGate::NotAuthorized | GuildStorageGate::NotAMember => {
                history.push(ChatLine::system(
                    ui_strings.get_or(DENIED_KEY, DENIED_FALLBACK),
                ));
                continue;
            }
        }
        state.session = Some(GuildStorageSession {
            npc: *npc,
            npc_id: network_id.0,
        });
        state.items = Inventory::default();
        state.synced = false;
        state.active_page = 0;
        buffer.0.clear();
        let Ok(conn) = conn.single() else {
            continue;
        };
        info!("guild storage: opening (0x7250) at npc {}", network_id.0);
        let request = GuildStorageOpenRequest {
            npc_unique_id: network_id.0,
        };
        if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
            error!("network: failed to send GuildStorageOpenRequest: {}", e.0);
        }
    }
}

/// 0xB250 — the ack, and the packet that makes this family different: on
/// success the original **immediately sends 0x7252**, and the item stream is
/// the answer to that. On the in-use arm it names the member holding the
/// guild-wide lock.
pub fn on_guild_storage_response(
    mut reader: MessageReader<GuildStorageResponse>,
    ui_strings: Res<ClientUiStrings>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut state: ResMut<GuildStorageState>,
    mut history: ResMut<ChatHistory>,
) {
    for msg in reader.read() {
        if !msg.is_success() {
            match msg.lock_holder() {
                Some(holder) => {
                    let text =
                        format_template(ui_strings.get_or(IN_USE_KEY, IN_USE_FALLBACK), &[holder]);
                    warn!("guild storage: in use by '{holder}' (0xB250 0x4C48)");
                    history.push(ChatLine::system(text));
                }
                None => warn!(
                    "guild storage: open refused (0xB250 result {}, error {:#06X}) — unknown code, see docs/net-guild-storage-0x7250.md",
                    msg.result,
                    msg.error_code.unwrap_or_default()
                ),
            }
            // The open failed, so there is no server-side session: drop the
            // one we optimistically set at 0x7250 (:186). Leaving it standing
            // opened a dead, empty window, kept the NPC dialog hidden
            // (npc_dialog/ui.rs `hide_dialog_while_store_open`) and would send
            // a 0x7251 close for a session the server never granted — which,
            // on the in-use arm, is a close for *another member's* lock.
            state.session = None;
            state.synced = false;
            state.items = Inventory::default();
            continue;
        }
        let Some(session) = state.session.as_ref() else {
            warn!("guild storage: unsolicited 0xB250 success — no open session, not listing");
            continue;
        };
        let Ok(conn) = conn.single() else {
            continue;
        };
        // Which value the original echoes here is unknown: its 0xB250 arm
        // stores a `u32` from a client-side getter (0xB250's success arm
        // carries no body) and 0x7252 sends that. The NPC id the window opened
        // with is the only u32 this side of the exchange has.
        info!(
            "guild storage: requesting contents (0x7252, storage_id = npc {})",
            session.npc_id
        );
        let request = GuildStorageListRequest {
            storage_id: session.npc_id,
        };
        if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
            error!("network: failed to send GuildStorageListRequest: {}", e.0);
        }
    }
}

/// 0x3253 — the stream begins and carries the guild account's gold.
///
/// **The gold trap, checked against the personal warehouse** (task of this
/// change): `storage/model.rs:329-347` applies gold-op acks to the STORAGE side
/// only, because the server sends the player's authoritative new total in a
/// `0x304E` right before the ack, and subtracting it a second time double-counted
/// (a 1234 deposit read as 2468 gone). The guild family has the same trap
/// **and it is unreachable today**: ops 32/33
/// (guild gold deposit/withdraw) carry a *delta* — the client adds the amount
/// to (or subtracts it from) its guild-storage gold (xBot `SRTypes.cs:157-161`,
/// `PacketParser.cs:2221-2232`, no licence — facts only), byte-identical in
/// shape to personal 11/12 —
/// and the server moves the character's gold in its own transaction
/// (`_UPDATE_CHAR_GOLD_FOR_GUILDCHEST`), i.e. the
/// 0x304E path again. Since `packets` has no encoder for ops 29-33, the only
/// writer of guild gold is this handler, so nothing can double-count yet.
/// Whoever adds those ops applies **only** `state.items.gold` and leaves the
/// player's `Inventory::gold` to `inventory::model::on_gold_update`. This rests
/// on xBot plus the server strings and stays unconfirmed on the wire.
pub fn on_guild_storage_begin(
    mut reader: MessageReader<GuildStorageDataBegin>,
    mut state: ResMut<GuildStorageState>,
    mut buffer: ResMut<GuildStorageDataBuffer>,
) {
    for msg in reader.read() {
        info!("guild storage: data begin (0x3253), gold {}", msg.gold);
        if !msg.tail.is_empty() {
            debug!(
                "guild storage: 0x3253 has an unexpected tail {} — not decoded",
                hexdump(&msg.tail, 24)
            );
        }
        state.items.gold = msg.gold;
        buffer.0.clear();
    }
}

/// 0x3255 — accumulate; the item section can span several packets.
pub fn on_guild_storage_chunk(
    mut reader: MessageReader<GuildStorageDataChunk>,
    mut buffer: ResMut<GuildStorageDataBuffer>,
) {
    for msg in reader.read() {
        buffer.0.extend_from_slice(&msg.raw);
    }
}

/// 0x3254 — the accumulated chunks are complete: parse the item section.
pub fn on_guild_storage_end(
    mut reader: MessageReader<GuildStorageDataEnd>,
    item_data: Res<ClientItemData>,
    mut state: ResMut<GuildStorageState>,
    mut buffer: ResMut<GuildStorageDataBuffer>,
) {
    for _ in reader.read() {
        let raw = std::mem::take(&mut buffer.0);
        if raw.is_empty() {
            warn!("guild storage: 0x3254 end with no 0x3255 chunks — storage stays unsynced");
            continue;
        }
        match parse_guild_storage_items(&raw, &*item_data) {
            Ok((size, items)) => {
                info!(
                    "guild storage: data end (0x3254) — size {size}, {} items",
                    items.len()
                );
                let gold = state.items.gold;
                let mut slots = vec![None; size as usize];
                for item in items {
                    let index = item.slot as usize;
                    if index >= slots.len() {
                        slots.resize(index + 1, None);
                    }
                    slots[index] = Some(item);
                }
                state.items = Inventory {
                    slots,
                    avatar_slots: Vec::new(),
                    gold,
                };
                state.synced = true;
                // a smaller warehouse than the last stream must not leave the
                // window paged past its end (the page grid reads
                // `active_page * 30 + cell`)
                let pages = state.page_count();
                state.active_page = state.active_page.min(pages - 1);
            }
            Err(e) => warn!(
                "guild storage: item section parse failed ({e:?}) — {} bytes: {}",
                raw.len(),
                hexdump(&raw, 64)
            ),
        }
    }
}

/// Apply the 0xB034 guild-warehouse acks (ops 29/30/31/32/33) — the exact
/// mirror of `storage::model::on_storage_response`, and for the same reason:
/// the ack carries the authoritative slots, so item records move between the
/// player [`Inventory`] and [`GuildStorageState::items`] only on
/// confirmation, never optimistically.
///
/// Gold (ops 32/33) applies to the **guild side only**. The player's own total
/// is corrected by the 0x304E the server sends alongside (handled in
/// `inventory::model::on_gold_update`); subtracting the amount here as well
/// double-counted it in the personal warehouse — a 1234 deposit read as 2468
/// gone until relog (`storage/model.rs`, the same trap as pickup gold). The
/// rule is carried over rather than rediscovered.
pub fn on_guild_storage_operation(
    mut reader: MessageReader<InventoryOperationResponse>,
    mut state: ResMut<GuildStorageState>,
    mut inventories: Query<&mut Inventory, With<Player>>,
    // For `apply_move`'s stack merging (#862): same-ref stackables combine
    // instead of trading places, and that needs the item's `max_stack`.
    item_data: Res<crate::plugins::textdata::ClientItemData>,
) {
    for msg in reader.read() {
        let Some(operation) = &msg.operation else {
            continue;
        };
        match *operation {
            InventoryOperationResult::GuildStorageToGuildStorage {
                source,
                target,
                amount,
            } => {
                debug!("guild storage: moved {source} -> {target} (x{amount})");
                state.items.apply_move(source, target, amount, &item_data);
            }
            InventoryOperationResult::InventoryToGuildStorage { source, target } => {
                let Ok(mut inventory) = inventories.single_mut() else {
                    continue;
                };
                let Some(mut item) = inventory
                    .slots
                    .get_mut(source as usize)
                    .and_then(|slot| slot.take())
                else {
                    warn!("guild storage: deposit ack for an empty inventory slot {source}");
                    continue;
                };
                debug!("guild storage: deposited slot {source} -> guild {target}");
                item.slot = target;
                state.items.gain_item(item);
            }
            InventoryOperationResult::GuildStorageToInventory { source, target } => {
                let Some(mut item) = state
                    .items
                    .slots
                    .get_mut(source as usize)
                    .and_then(|slot| slot.take())
                else {
                    warn!("guild storage: withdraw ack for an empty guild slot {source}");
                    continue;
                };
                debug!("guild storage: withdrew guild {source} -> slot {target}");
                item.slot = target;
                for mut inventory in inventories.iter_mut() {
                    inventory.gain_item(item.clone());
                }
            }
            // guild side only — see the note above
            InventoryOperationResult::InventoryGoldToGuildStorage { amount } => {
                debug!("guild storage: deposited {amount} gold (player total via 0x304E)");
                state.items.gold = state.items.gold.saturating_add(amount);
            }
            InventoryOperationResult::GuildStorageGoldToInventory { amount } => {
                debug!("guild storage: withdrew {amount} gold (player total via 0x304E)");
                state.items.gold = state.items.gold.saturating_sub(amount);
            }
            _ => {}
        }
    }
}

/// Guild storage is a **guild-wide exclusive lock** — that is why the family
/// has a close opcode at all (`0xB250` error `0x4C48` names whoever holds it).
/// So leaving the NPC's dialog must send 0x7251; a session we forget to close
/// locks the warehouse for the whole guild.
pub fn close_guild_storage_with_dialog(
    dialog: Res<NpcDialogState>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut state: ResMut<GuildStorageState>,
) {
    let Some(session) = state.session.as_ref() else {
        return;
    };
    if dialog.npc() == Some(session.npc) {
        return;
    }
    let npc_id = session.npc_id;
    state.session = None;
    state.synced = false;
    let Ok(conn) = conn.single() else {
        return;
    };
    info!("guild storage: closing (0x7251) at npc {npc_id}");
    let request = GuildStorageCloseRequest {
        npc_unique_id: npc_id,
    };
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send GuildStorageCloseRequest: {}", e.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three refusals the client's own strings distinguish. A missing
    /// roster is NOT "no permissions": the player may simply not be in a
    /// guild, and the original has a different string for each.
    #[test]
    fn the_gate_separates_membership_level_and_permission() {
        assert_eq!(gate(None, None), GuildStorageGate::NotAMember);
        assert_eq!(
            gate(Some(5), None),
            GuildStorageGate::NotAMember,
            "in a guild whose roster does not list us is still not a member"
        );
        assert_eq!(
            gate(Some(1), Some(GuildPermissions(GuildPermissions::ALL))),
            GuildStorageGate::GuildLevelTooLow,
            "UIIT_MSG_GUILD_WAREHOUSE_LIMIT: level 2 or higher"
        );
        assert_eq!(
            gate(Some(2), Some(GuildPermissions(GuildPermissions::JOIN))),
            GuildStorageGate::NotAuthorized
        );
        assert_eq!(
            gate(Some(2), Some(GuildPermissions(GuildPermissions::STORAGE))),
            GuildStorageGate::Allowed
        );
        // the master sentinel is every bit set, so it passes without the
        // named bit being special-cased
        assert_eq!(
            gate(Some(2), Some(GuildPermissions(GuildPermissions::MASTER))),
            GuildStorageGate::Allowed
        );
    }

    /// The 0x3255 chunks are meaningless alone. Drive the real systems in
    /// their registered order with a section split across two packets: an
    /// unordered (or unbuffered) consumer decodes "0 bytes" here — the exact
    /// defect `docs/net-storage-0x3047-0x3049.md:162-165` records for the
    /// personal family.
    #[test]
    fn the_guild_item_stream_is_buffered_across_chunks_before_it_parses() {
        use bytes::Bytes;

        let mut app = App::new();
        app.add_message::<GuildStorageDataBegin>()
            .add_message::<GuildStorageDataChunk>()
            .add_message::<GuildStorageDataEnd>()
            .init_resource::<GuildStorageState>()
            .init_resource::<GuildStorageDataBuffer>()
            .init_resource::<ClientItemData>()
            .add_systems(
                Update,
                (
                    on_guild_storage_begin,
                    on_guild_storage_chunk,
                    on_guild_storage_end,
                )
                    .chain(),
            );

        app.world_mut().write_message(GuildStorageDataBegin {
            gold: 1000,
            tail: Bytes::new(),
        });
        // capacity 60, count 0 — split so the end marker cannot parse a
        // complete section from either half on its own
        app.world_mut().write_message(GuildStorageDataChunk {
            raw: Bytes::from_static(&[60]),
        });
        app.world_mut().write_message(GuildStorageDataChunk {
            raw: Bytes::from_static(&[0]),
        });
        app.world_mut().write_message(GuildStorageDataEnd);
        app.update();

        let state = app.world().resource::<GuildStorageState>();
        assert!(state.synced, "the section parsed from the joined chunks");
        assert_eq!(state.items.size(), 60);
        assert_eq!(state.items.gold, 1000, "0x3253 carries the guild gold");
        assert!(
            app.world()
                .resource::<GuildStorageDataBuffer>()
                .0
                .is_empty(),
            "the buffer is consumed, so a second stream cannot inherit it"
        );
    }

    /// A refused open must not leave the optimistic session standing: the
    /// window (`ui::sync_guild_storage_window`) renders on `session.is_some()`
    /// and `npc_dialog::hide_dialog_while_store_open` hides the conversation
    /// on it, so a stale session shows a dead, empty warehouse over a hidden
    /// dialog — and later sends a 0x7251 close for a lock we never held.
    #[test]
    fn a_refused_open_drops_the_optimistic_session() {
        let mut app = App::new();
        app.add_message::<GuildStorageResponse>()
            .init_resource::<ClientUiStrings>()
            .init_resource::<ChatHistory>()
            .insert_resource(GuildStorageState {
                session: Some(GuildStorageSession {
                    npc: Entity::from_raw_u32(7).unwrap(),
                    npc_id: 42,
                }),
                synced: true,
                ..default()
            })
            .add_systems(Update, on_guild_storage_response);

        // the in-use refusal (0xB250 result 2, error 0x4C48, holder name)
        app.world_mut().write_message(GuildStorageResponse {
            result: 2,
            error_code: Some(packets::agent::guild_storage::GUILD_STORAGE_IN_USE),
            holder_name: Some("Kong".into()),
        });
        app.update();

        let state = app.world().resource::<GuildStorageState>();
        assert!(
            state.session.is_none(),
            "the refusal ends the session we opened optimistically"
        );
        assert!(!state.synced);
    }

    /// The in-use refusal is the one 0xB250 arm the original special-cases,
    /// and it is only useful *with* the name filled in — an unformatted
    /// template would show the player a literal `%s`.
    #[test]
    fn the_in_use_refusal_names_the_lock_holder() {
        let text = format_template(IN_USE_FALLBACK, &["Grunt"]);
        assert!(text.contains("Grunt"), "{text}");
        assert!(!text.contains("%s"), "{text}");
    }
}

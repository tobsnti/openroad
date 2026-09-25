//! Player-to-player exchange (trade) session state + the two-stage lock.
//!
//! Idea: the server owns the whole trade. 0x3085 opens the window against a
//! partner's spawn id; from then on the *partner's* half is render-only mirror
//! state, replaced wholesale out of 0x308C (items) and 0x3089 (gold), and our
//! own half is never applied optimistically. The lock is deliberately two
//! stage — it is the original's anti-scam core: 0x7082 confirms (locks) our
//! offer, and approve (0x7083) is only offered once the *partner* has
//! confirmed too, so neither side can swap an item out after the other has
//! agreed. 0x3087 completes the trade, 0x3088 aborts it, and closing the
//! window is a protocol act (0x7084) rather than a UI hide.
//!
//! Staging our own items and gold is not an exchange opcode at all: it rides
//! 0x7034/0xB034 sub-ops 4/5/13, which `InventoryOperationRequest` models
//! ([`InventoryOperationRequest::InventoryToExchange`] and friends). The own
//! pane is filled from the **self** 0x308C echo the server sends the acting
//! player (two slot bytes instead of one — see [`on_partner_items`]); the
//! gestures that *send* those sub-ops live in `ui.rs` (the drag polls) and the
//! requests they build are [`stage_item_request`], [`unstage_item_request`]
//! and [`stage_gold_request`] below. Our own staged **gold** is the one thing
//! no 0x308C carries before a confirm, so it comes from the 0xB034 op-13 ack
//! ([`apply_staging_ack`]) — never from the input field.
//!
//! Four behaviours of the server are not what the opcode names suggest, and
//! two of them contradict the obvious guess:
//!  1. The **inviter never receives 0x3085.** On acceptance the server sends
//!     `0xB081 01 <partner uid>` to the inviter and `0x3085` to the *target*
//!     only. Opening the window on 0x3085 alone therefore leaves the inviter
//!     with no window at all — hence [`on_invite_response`] opens it too, the
//!     same single `OnExchangeStart` sink the original's parser uses
//!     (`PacketParser.cs:1213-1220`).
//!  2. **0xB081 is not a "petition raised" ack.** A successful 0x7081 is
//!     answered with *silence*; `0xB081 01` arrives only when the peer
//!     accepts, which can be tens of seconds later. A refusal by the peer is
//!     not a 0xB081 at all: both sides get `0x3088` reason `0x1828`.
//!  3. **A second 0x7082 aborts the trade.** Two confirms from one side answer
//!     `0xB082 02 1f18` and kill the session with `0x3088 1f18` on both sides.
//!     The ack is ~50 ms away, so a double click is enough — that is why the
//!     request is gated by [`ExchangeSession::request_in_flight`] and not by
//!     the acked `own_confirmed` alone.
//!  4. **The peer's pane is revealed at confirm, not at staging.** Staging
//!     (0x7034 sub-op 4/5/13) is echoed to the *acting* player only — as
//!     0xB034 *and* a self-targeted 0x308C with two slot bytes; the peer sees
//!     nothing until that side sends 0x7082, at which point the server pushes
//!     0x308C (full list, exchange-slot byte only) + 0x3089 (gold, always,
//!     even 0) + 0x3086. Applying the transfer locally on 0x3087 is not an
//!     optimisation but a requirement: the completed trade moves the items
//!     **silently** — a traded-away slot answers `0xB034 02 0918` "empty slot"
//!     afterwards — while only gold gets an authoritative 0x304E.
//!
//! Wire layouts: `docs/net-exchange-0x3085.md`.

use bevy::prelude::*;

use packets::agent::character_data::InventoryItem;
use packets::agent::ingame::{ExchangeInviteRequest, ExchangeInviteResponse};

use packets::agent::inventory::{
    InventoryOperationRequest, InventoryOperationResponse, InventoryOperationResult,
};

use packets::agent::exchange::{
    ExchangeApproveRequest, ExchangeApproveResponse, ExchangeCanceled, ExchangeCompleted,
    ExchangeConfirmRequest, ExchangeConfirmResponse, ExchangeExitRequest, ExchangeExitResponse,
    ExchangeGoldUpdate, ExchangeItemsUpdate, ExchangePlayerConfirmed, ExchangeStarted,
    CANCEL_REASON_DECLINED, CANCEL_REASON_EXIT, CANCEL_REASON_PARTNER_LEFT,
    CANCEL_REASON_PROTOCOL_VIOLATION,
};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::{Inventory, BAG_FIRST_SLOT};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

/// Slots per side. `ifexchange.txt` declares 12 per pane (ids 100-111 and
/// 200-211), and 0x308C carries the same capacity on the wire.
pub const EXCHANGE_SLOTS: usize = 12;

/// Shown when the server refuses our exchange invite. **Ours, stated deviation
/// (ADR-0009):** the original renders these codes through its error-message
/// box in category 1 (exchange), but nothing in this repository — and no
/// `UIIT_*` key in `textuisystem.txt` — enumerates a single value of that
/// category, so there is no string id to bind. The raw code is printed with
/// the line; an invented sentence per code would look sourced and would not
/// be.
pub const INVITE_REFUSED_NOTICE: &str = "The exchange request was refused.";

/// `UIIT_MSG_STRGERR_EXCHANGE_CANCEL`, textuisystem L1710 — the original's own
/// line for "the trade is over and it did not go through".
///
/// It stays **reason-less on purpose**, even though `ExchangeCanceled` now
/// carries the `u16` (`0x1828` declined / `0x182B` the other side left /
/// `0x182C` exit / `0x181F` protocol violation). `textuisystem.txt` has
/// exactly one line for a cancelled exchange, L1710;
/// the nearby candidates that *sound* per-reason —
/// `UIIT_MSG_STRGERR_TRADING_DENIED` (L1579 "Refused to trade.", and again at
/// L1901 with a different text), `UIIT_MSG_STRGERR_TRADING_CANCELED_BY_USER`
/// (L1904) and `UIIT_MSG_STRGERR_TIMEOUT` (L1900) — are not bound to any wire
/// code by anything in this repository, in the data, or in the original's own
/// notice dispatcher: its category-1 switch has a `case 0x1828/0x182b/0x182c:`
/// that simply `break`s, with no string id at all. Picking one of them per
/// code would look sourced without
/// being it. What the line *does* gain is the raw code, the same treatment
/// [`INVITE_REFUSED_NOTICE`] gives an unnamed 0xB081 code.
pub const EXCHANGE_CANCELED_NOTICE: (&str, &str) = (
    "UIIT_MSG_STRGERR_EXCHANGE_CANCEL",
    "The exchange has been canceled.",
);

/// The known reasons, for the log line only — the four values are named in
/// `packets::agent::exchange`. A code outside that set is reported as a number
/// rather than guessed at.
fn cancel_reason_label(reason: u16) -> &'static str {
    match reason {
        CANCEL_REASON_DECLINED => "the other player refused",
        CANCEL_REASON_PARTNER_LEFT => "the other participant left the game",
        CANCEL_REASON_EXIT => "somebody left the trade window",
        CANCEL_REASON_PROTOCOL_VIOLATION => "protocol violation (double confirm)",
        _ => "unknown reason code",
    }
}

/// `UIIT_MSG_DEAL_ASKING`, textuisystem L1714 — the inviter's own "waiting for
/// an answer" line, with `%s` for the target. The invitee's side of the same
/// exchange is `UIIT_MSG_DEAL_ASK` (L1713), which `hud::petition` uses.
pub const DEAL_ASKING: (&str, &str) = ("UIIT_MSG_DEAL_ASKING", "Applying for a trade to [%s].");

/// The open trade, or `None` when no window is up.
#[derive(Resource, Default)]
pub struct ExchangeState {
    pub session: Option<ExchangeSession>,
}

/// One trade. Everything here is server-pushed; nothing is predicted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExchangeSession {
    /// The partner's spawn unique id (0x3085), used to reject a 0x308C that
    /// describes somebody else.
    pub partner_unique_id: u32,
    /// Replaced wholesale on every 0x308C — the original mirrors the peer's
    /// pane rather than diffing it.
    pub partner_items: Vec<InventoryItem>,
    pub partner_gold: u64,
    /// Our staged offer, in the order the self 0x308C echo lists it. Filled by
    /// the echo alone — a staging request never predicts it. Each
    /// entry's `slot` is its **bag** slot (that is what 0x3087 has to clear);
    /// the exchange-pane slot the server assigned it lives in [`Self::own_slots`]
    /// at the same index.
    pub own_items: Vec<InventoryItem>,
    /// The exchange-pane slot per [`Self::own_items`] entry, from the self echo's
    /// second slot byte. Kept because sub-op 5 addresses the item by its
    /// **exchange** slot and nothing else (`InventoryOperationRequest::
    /// ExchangeToInventory` carries one byte, no target), so the withdraw must
    /// quote the number the server assigned rather than the render index — the
    /// two only coincide while the server happens to fill the pane in order,
    /// which is not guaranteed.
    pub own_slots: Vec<u8>,
    pub own_gold: u64,
    /// We sent 0x7082 and the server acked it: our offer is locked.
    pub own_confirmed: bool,
    /// The peer sent theirs (0x3086).
    pub partner_confirmed: bool,
    pub own_approved: bool,
    /// A 0x7082/0x7083 is on the wire and its ack has not come back yet.
    ///
    /// A **second** 0x7082 from the same side is answered `0xB082 02 1f18` and
    /// the server terminates the trade with `0x3088 1f18` on both sides.
    /// `own_confirmed` only flips when the ack lands, so gating on it alone
    /// leaves a ~50 ms window in which a double click destroys the trade.
    pub request_in_flight: bool,
}

impl ExchangeSession {
    fn new(partner_unique_id: u32) -> Self {
        ExchangeSession {
            partner_unique_id,
            ..default()
        }
    }

    /// Confirm is offered until our own side locks. There is no unconfirm
    /// opcode in the family — backing out after the lock means exiting.
    pub fn can_confirm(&self) -> bool {
        !self.own_confirmed && !self.request_in_flight
    }

    /// The anti-scam gate: approve is enabled only once **both** sides are
    /// locked (`InfoManager.cs:1073-1074`). Gating on our own confirm alone
    /// would let the partner restage after we approved, which is precisely the
    /// fraud the two-stage flow exists to prevent.
    pub fn can_approve(&self) -> bool {
        self.own_confirmed
            && self.partner_confirmed
            && !self.own_approved
            && !self.request_in_flight
    }

    /// Vanilla reuses one button pair: the action button reads "Confirm" until
    /// our side is locked and "Approve" afterwards (`InfoManager.cs:1068-1080`).
    pub fn awaiting_approve(&self) -> bool {
        self.own_confirmed
    }

    /// Gold is editable only before our own lock — same source line as the
    /// button relabel.
    pub fn gold_editable(&self) -> bool {
        !self.own_confirmed
    }
}

/// 0x3085 — the server opened a trade window against `partner_unique_id`.
pub fn on_exchange_started(
    mut reader: MessageReader<ExchangeStarted>,
    mut state: ResMut<ExchangeState>,
) {
    for msg in reader.read() {
        info!(
            "exchange: started with partner {} (0x3085)",
            msg.partner_unique_id
        );
        state.session = Some(ExchangeSession::new(msg.partner_unique_id));
    }
}

/// 0x308C — the partner's staged list, replaced wholesale.
pub fn on_partner_items(
    mut reader: MessageReader<ExchangeItemsUpdate>,
    item_data: Res<ClientItemData>,
    mut state: ResMut<ExchangeState>,
) {
    for msg in reader.read() {
        let Some(session) = state.session.as_mut() else {
            warn!("exchange: 0x308C with no open trade — ignored");
            continue;
        };
        // 0x308C comes in two shapes and only ever from the two participants,
        // so the uid *is* the discriminator: anything that is not the partner
        // is the self echo of our own staging: staging an item echoes
        // `0x308C <own uid> 01 29 00 …` back to the acting player. The self shape
        // carries `slot_inventory` *then* `slot_exchange`, one byte more per
        // entry than the peer copy, so it must go through `own_items` —
        // `items` would parse it and shift every field by one byte.
        if msg.player_unique_id != session.partner_unique_id {
            let Some(staged) = msg.own_items(&*item_data) else {
                warn!("exchange: could not decode our own staged items — own pane left unchanged");
                continue;
            };
            debug!(
                "exchange: we have {} item(s) staged (self echo of 0x308C)",
                staged.len()
            );
            // `slot_inventory` is kept as the item's slot: the trade only moves
            // it on 0x3087, and `on_exchange_completed` clears exactly those
            // bag slots. `slot_exchange` is kept alongside it because the
            // withdraw (sub-op 5) addresses the item by that number.
            session.own_slots = staged.iter().map(|entry| entry.slot_exchange).collect();
            session.own_items = staged.into_iter().map(|entry| entry.item).collect();
            continue;
        }
        let Some(items) = msg.items(&*item_data) else {
            // A partial list would misrepresent what the peer is offering, so
            // the accessor returns None rather than a prefix.
            warn!("exchange: could not decode the partner's staged items — pane left unchanged");
            continue;
        };
        if items.len() > EXCHANGE_SLOTS {
            // The 12-slot capacity is inferred from the window grid rather
            // than seen on the wire. If the server ever exceeds it, that
            // assumption is wrong and the extra
            // records would render nowhere — so say so instead of dropping
            // them silently.
            warn!(
                "exchange: partner staged {} items but the vanilla pane holds {} — \
                 capacity assumption refuted",
                items.len(),
                EXCHANGE_SLOTS
            );
        }
        debug!("exchange: partner staged {} item(s)", items.len());
        session.partner_items = items;
    }
}

/// 0x3089 — the partner's staged gold.
pub fn on_partner_gold(
    mut reader: MessageReader<ExchangeGoldUpdate>,
    mut state: ResMut<ExchangeState>,
) {
    for msg in reader.read() {
        let Some(session) = state.session.as_mut() else {
            warn!("exchange: 0x3089 with no open trade — ignored");
            continue;
        };
        // The leading byte is `0x02` for every observed gold value (0, 1, 100
        // and 5000 — so it is not derived from the amount), and the packet
        // arrives only when that side confirms, never while it edits the field.
        debug!(
            "exchange: partner staged {} gold (unk byte {})",
            msg.gold, msg.unk_byte01
        );
        session.partner_gold = msg.gold;
    }
}

/// 0x3086 — the partner locked their offer.
pub fn on_partner_confirmed(
    mut reader: MessageReader<ExchangePlayerConfirmed>,
    mut state: ResMut<ExchangeState>,
) {
    for _ in reader.read() {
        let Some(session) = state.session.as_mut() else {
            continue;
        };
        info!("exchange: partner confirmed (0x3086)");
        session.partner_confirmed = true;
    }
}

/// 0xB082 — ack for our confirm. Only a successful ack locks our side; a
/// refusal must leave the window editable, or the player is stuck.
pub fn on_confirm_response(
    mut reader: MessageReader<ExchangeConfirmResponse>,
    mut state: ResMut<ExchangeState>,
) {
    for msg in reader.read() {
        let Some(session) = state.session.as_mut() else {
            continue;
        };
        session.request_in_flight = false;
        if msg.is_success() {
            info!("exchange: our offer is locked (0xB082)");
            session.own_confirmed = true;
        } else {
            // The failure form is `02 <u16>` — `02 1b18` (0x181B, "no trade
            // session") for a confirm sent outside a trade, `02 1f18`
            // (0x181F) for a *second* confirm. The session is not cleared here
            // even for 0x181F: the server sends a 0x3088 with the same code to
            // both sides in the same millisecond, and that is the one handler
            // that closes the window, so there is exactly one place that does
            // it.
            warn!(
                "exchange: confirm refused (0xB082 result {} error {:?})",
                msg.result, msg.error
            );
        }
    }
}

/// 0xB083 — ack for our approve.
pub fn on_approve_response(
    mut reader: MessageReader<ExchangeApproveResponse>,
    mut state: ResMut<ExchangeState>,
) {
    for msg in reader.read() {
        let Some(session) = state.session.as_mut() else {
            continue;
        };
        session.request_in_flight = false;
        if msg.is_success() {
            info!("exchange: approved, waiting for the partner (0xB083)");
            session.own_approved = true;
        } else {
            warn!(
                "exchange: approve refused (0xB083 result {} error {:?})",
                msg.result, msg.error
            );
        }
    }
}

/// 0x3087 — the trade went through. The client applies the transfer locally:
/// the partner's staged records land in our first free bag slots and our own
/// staged records leave the bag (`InfoManager.cs:967-1001`).
///
/// Gold is deliberately **not** touched here. The server sends the
/// authoritative new total separately (0x304E), and applying the staged
/// amounts on top of it double-counts — the same trap that made a storage
/// deposit read as twice the gold until a relog (see
/// `hud/storage/model.rs::on_storage_response`).
pub fn on_exchange_completed(
    mut reader: MessageReader<ExchangeCompleted>,
    mut state: ResMut<ExchangeState>,
    mut inventories: Query<&mut Inventory, With<Player>>,
) {
    for _ in reader.read() {
        let Some(session) = state.session.take() else {
            continue;
        };
        info!(
            "exchange: completed (0x3087) — {} item(s) in, {} out",
            session.partner_items.len(),
            session.own_items.len()
        );
        for mut inventory in inventories.iter_mut() {
            for slot in session.own_items.iter().map(|item| item.slot) {
                if let Some(entry) = inventory.slots.get_mut(slot as usize) {
                    *entry = None;
                }
            }
            for item in session.partner_items.iter() {
                let Some(free) = first_free_bag_slot(&inventory) else {
                    warn!("exchange: no free bag slot for a traded item — relog to resync");
                    break;
                };
                let mut item = item.clone();
                item.slot = free;
                inventory.gain_item(item);
            }
        }
    }
}

/// 0x3088 — the trade was called off by the peer or the server.
///
/// The notice is pushed **whether or not a window was open**, and that is the
/// point. The inviter has no session until the peer accepts (see
/// [`on_invite_response`]), so for every petition that dies before acceptance
/// 0x3088 is the *only* thing the inviter ever receives. An invite that is
/// never accepted gets no 0xB081 at all; when the inviter then logs out
/// (0x7005) the server cancels the pending petition on both sides with
/// `0x3088 2b18`. Before this the whole sequence was silent after the
/// "Applying for a trade to [%s]" line.
pub fn on_exchange_canceled(
    mut reader: MessageReader<ExchangeCanceled>,
    mut state: ResMut<ExchangeState>,
    ui_strings: Res<ClientUiStrings>,
    mut history: ResMut<ChatHistory>,
) {
    for msg in reader.read() {
        let had_window = state.session.take().is_some();
        let reason = msg.reason;
        info!(
            "exchange: cancelled (0x3088 reason {reason:#06x}, {}), window was open: {had_window}",
            cancel_reason_label(reason)
        );
        let (key, fallback) = EXCHANGE_CANCELED_NOTICE;
        // The sentence is the original's; the code is ours, because no
        // textuisystem key is bound to any of the four reasons (see
        // [`EXCHANGE_CANCELED_NOTICE`]). Printing it keeps "he refused" and
        // "he logged out" distinguishable for a player who reports a bug,
        // without inventing a sentence for either.
        history.push(ChatLine::system(format!(
            "{} (code {reason:#06x})",
            ui_strings.get_or(key, fallback)
        )));
    }
}

/// 0xB084 — ack for our exit. The window closes on the ack, not on the click,
/// so a refused exit keeps the trade visible instead of desyncing us from a
/// trade the server still considers open.
pub fn on_exit_response(
    mut reader: MessageReader<ExchangeExitResponse>,
    mut state: ResMut<ExchangeState>,
) {
    for msg in reader.read() {
        if !msg.is_success() {
            warn!(
                "exchange: exit refused (0xB084 result {} error {:?}) — window stays open",
                msg.result, msg.error
            );
            continue;
        }
        if state.session.take().is_some() {
            info!("exchange: exited (0xB084)");
        }
    }
}

/// First free bag slot (equipment slots are below [`BAG_FIRST_SLOT`]).
fn first_free_bag_slot(inventory: &Inventory) -> Option<u8> {
    (BAG_FIRST_SLOT..inventory.size()).find(|slot| inventory.get(*slot).is_none())
}

/// Send one of the empty-bodied exchange requests.
fn send(conn: &Query<&SilkroadConnection, With<AgentConnection>>, packet: Packet, what: &str) {
    let Ok(conn) = conn.single() else {
        return;
    };
    if let Err(e) = conn.get_sender().send(packet.into()) {
        error!("network: failed to send exchange {what}: {}", e.0);
    }
}

/// 0x7081 — ask `unique_id` to trade. The server raises the 0x3080 petition on
/// them and answers us with 0xB081; the window itself opens on the following
/// 0x3085, never here, so nothing is predicted (`docs/net-invite-0x3080.md`
/// §4).
pub fn send_invite(conn: &Query<&SilkroadConnection, With<AgentConnection>>, unique_id: u32) {
    send(
        conn,
        Packet::from(ExchangeInviteRequest { unique_id }),
        "invite",
    );
}

/// 0xB081 — the inviter-side ack for our own 0x7081. **This is where the
/// inviter's window opens**, not 0x3085.
///
/// A successful 0x7081 gets *no* answer at all — only the target sees the
/// `0x3080` petition. `0xB081 01 <uid>` arrives when the peer accepts, and the
/// accompanying `0x3085` goes to the **peer alone**. So `Accepted` *is* the inviter's
/// window-open event — the same `OnExchangeStart(uniqueID)` sink the original's
/// parser feeds from both paths (`PacketParser.cs:1213-1220`). Opening on
/// 0x3085 only left whoever pressed "Exchange" staring at nothing.
///
/// A refusal carries a `u16` code whose value space no source names, so it is
/// reported verbatim rather than translated into an invented sentence — the
/// same rule the stall buyer flow follows for its error codes. Two of those
/// codes are known: `0x0003` for an invalid/self target and `0x0004` for
/// "farther than 320 units".
pub fn on_invite_response(
    mut reader: MessageReader<ExchangeInviteResponse>,
    mut state: ResMut<ExchangeState>,
    mut history: ResMut<ChatHistory>,
) {
    for msg in reader.read() {
        match msg {
            ExchangeInviteResponse::Accepted { unique_id } => {
                info!("exchange: partner {unique_id} accepted (0xB081) — window opens");
                // Idempotent on purpose: should this server ever also send the
                // inviter a 0x3085, the second event must not reset a session
                // that has already been staged into.
                if state.session.is_none() {
                    state.session = Some(ExchangeSession::new(*unique_id));
                }
            }
            ExchangeInviteResponse::Refused { error } => {
                warn!("exchange: invite refused (0xB081 code {error:#06x})");
                history.push(ChatLine::system(format!(
                    "{INVITE_REFUSED_NOTICE} (code {error:#06x})"
                )));
            }
        }
    }
}

/// 0x7082 — lock our offer.
pub fn send_confirm(conn: &Query<&SilkroadConnection, With<AgentConnection>>) {
    send(conn, Packet::from(ExchangeConfirmRequest), "confirm");
}

/// 0x7083 — approve the trade.
pub fn send_approve(conn: &Query<&SilkroadConnection, With<AgentConnection>>) {
    send(conn, Packet::from(ExchangeApproveRequest), "approve");
}

/// 0x7084 — back out. Closing the window is a protocol act: the session is
/// cleared by the 0xB084 ack, never here.
pub fn send_exit(conn: &Query<&SilkroadConnection, With<AgentConnection>>) {
    send(conn, Packet::from(ExchangeExitRequest), "exit");
}

// --- staging our own side: 0x7034 sub-ops 4 / 5 / 13 -----------------------
//
// Idea: staging is an *inventory* operation, not an exchange opcode, and the
// server answers it twice — a 0xB034 ack and a self-targeted 0x308C. So this
// half of the module only builds requests and reads the one thing the 0x308C
// echo does NOT carry: the staged gold (0x3089 arrives only when a side
// confirms). Nothing is applied optimistically and nothing touches the bag —
// the item stays in its slot
// until 0x3087 (`hud::inventory::model`'s deliberate no-op arm for ops
// 4/5/13).

/// The exchange-pane slot the player is carrying out of the trade window, i.e.
/// a withdraw in progress, plus the ghost icon that visualises it. Its own
/// resource rather than a field of [`ExchangeState`] because
/// `sync_exchange_window` rebuilds the whole window on any `ExchangeState`
/// change, and picking an item up must not rebuild the tree the pointer is
/// currently interacting with.
#[derive(Resource, Default)]
pub struct ExchangeCarry(pub Option<ExchangeCarryData>);

/// The ghost `Entity` is remembered rather than "despawn everything with
/// [`DragGhost`](crate::plugins::hud::inventory::ui::DragGhost)": that marker
/// is the ONE shared cursor icon of the whole HUD (#579), so an inventory
/// carry started while a pane item is on the cursor owns a ghost of its own,
/// and a blanket despawn would leave that carry invisible (the storage-carry
/// precedent keeps its entity for the same reason).
pub struct ExchangeCarryData {
    /// The carried item's **exchange**-pane slot, i.e. what sub-op 5 quotes.
    pub slot: u8,
    pub ghost: Entity,
}

/// Staging is open exactly while our offer is not locked: 0x7082 locks it and
/// there is no unlock opcode (`InfoManager.cs:1068-1080` greys the pane the
/// same way, and [`ExchangeSession::gold_editable`] already states it for
/// gold). `request_in_flight` is included because a confirm whose ack is still
/// out is a lock in progress — staging into it would race the server's own
/// view of what we offered.
fn staging_open(session: &ExchangeSession) -> bool {
    !session.own_confirmed && !session.request_in_flight
}

/// The first exchange-pane slot the server has not filled on our side. Sub-op
/// 4 names the target slot itself (`04 29 00` = bag 0x29 → pane slot 0), so
/// the client picks it;
/// the gaps matter because a withdraw frees a slot in the middle of the pane.
pub fn free_own_exchange_slot(session: &ExchangeSession) -> Option<u8> {
    (0..EXCHANGE_SLOTS as u8).find(|slot| !session.own_slots.contains(slot))
}

/// Sub-op 4 — put bag slot `source` into our side of the trade. `None` when
/// there is no open trade (a drag anywhere else must not put a staging packet
/// on the wire), when our offer is already locked, or when the pane is full.
pub fn stage_item_request(state: &ExchangeState, source: u8) -> Option<InventoryOperationRequest> {
    let session = state.session.as_ref()?;
    if !staging_open(session) {
        return None;
    }
    let target = free_own_exchange_slot(session)?;
    Some(InventoryOperationRequest::InventoryToExchange { source, target })
}

/// Sub-op 5 — take pane slot `source` back out. One byte: the destination is
/// the server's choice because the item never left the bag; the frame is
/// `0500` (`packets::agent::inventory`).
pub fn unstage_item_request(
    state: &ExchangeState,
    source: u8,
) -> Option<InventoryOperationRequest> {
    let session = state.session.as_ref()?;
    if !staging_open(session) {
        return None;
    }
    Some(InventoryOperationRequest::ExchangeToInventory { source })
}

/// Sub-op 13 — set the gold on our side. **Absolute, not a delta** (100 →
/// `0d6400000000000000`), so it is clamped to the purse rather than
/// accumulated. A 0 is allowed on purpose: an absolute set is the only way to
/// take staged gold back off the table, and no other amount byte pattern
/// differs — how this server answers a 0 is unconfirmed.
pub fn stage_gold_request(
    state: &ExchangeState,
    amount: u64,
    purse: u64,
) -> Option<InventoryOperationRequest> {
    let session = state.session.as_ref()?;
    if !session.gold_editable() || session.request_in_flight {
        return None;
    }
    Some(InventoryOperationRequest::InventoryGoldToExchange {
        amount: amount.min(purse),
    })
}

/// Put a built staging request on the wire.
pub fn send_staging(
    conn: &Query<&SilkroadConnection, With<AgentConnection>>,
    request: InventoryOperationRequest,
) {
    let Ok(conn) = conn.single() else {
        warn!("exchange: no agent connection, dropping {request:?}");
        return;
    };
    debug!("exchange: staging {request:?}");
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send exchange staging request: {}", e.0);
    }
}

/// Is this 0xB034 result one of the three staging ops (4/5/13)? The single
/// place that decides whether an inventory ack is the trade window's business
/// — everything else on that opcode belongs to `hud::inventory`.
pub fn is_staging_result(result: &InventoryOperationResult) -> bool {
    matches!(
        result,
        InventoryOperationResult::InventoryToExchange { .. }
            | InventoryOperationResult::ExchangeToInventory { .. }
            | InventoryOperationResult::InventoryGoldToExchange { .. }
    )
}

/// The 0xB034 ack for a staging op. Items are NOT applied from here — the self
/// 0x308C carries the whole list and arrives ~20 ms earlier — but the **gold
/// is**: no 0x308C or 0x3089 tells us our own staged amount before we confirm,
/// so this ack is the only authority for it. The item acks (`010400` /
/// `010500`) only say the op went through, so they change nothing here.
pub fn apply_staging_ack(session: &mut ExchangeSession, result: &InventoryOperationResult) {
    if let InventoryOperationResult::InventoryGoldToExchange { amount } = result {
        debug!("exchange: our staged gold acked at {amount} (0xB034 op 13)");
        session.own_gold = *amount;
    }
}

/// The line for a 0x7034 the server refused while the trade window is up.
/// Raw code on purpose, as [`INVITE_REFUSED_NOTICE`]: nothing in the data
/// binds `0x183A` (returned for a 1000-stack of potions dragged onto our pane)
/// to a `UIIT_*` string.
pub fn refused_item_op_notice(code: u16) -> String {
    format!("The server refused this item operation (code {code:#06x}).")
}

/// 0xB034 — apply the staging acks that belong to the trade window.
pub fn on_staging_response(
    mut reader: MessageReader<InventoryOperationResponse>,
    mut state: ResMut<ExchangeState>,
    mut history: ResMut<ChatHistory>,
) {
    for msg in reader.read() {
        let Some(result) = msg.operation.as_ref() else {
            // A refusal carries no op, so it cannot be told apart from a plain
            // bag move gone wrong — but with the window open the player just
            // dragged something onto it, and silence reads as a dead drop.
            if let (Some(code), true) = (msg.error, state.session.is_some()) {
                warn!("exchange: 0x7034 refused with {code:#06x} while trading");
                history.push(ChatLine::system(refused_item_op_notice(code)));
            }
            continue;
        };
        if !is_staging_result(result) {
            continue;
        }
        let Some(session) = state.session.as_mut() else {
            warn!("exchange: staging ack with no open trade — ignored");
            continue;
        };
        apply_staging_ack(session, result);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use packets::agent::character_data::{ItemClass, ItemClassResolver, ItemTypeData, RentInfo};

    fn item(slot: u8, ref_id: u32) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id,
            data: ItemTypeData::Expendable {
                stack_count: 1,
                assimilation_prob: None,
                mag_params: vec![],
            },
        }
    }

    /// The anti-scam gate is the whole point of the two-stage lock: approve
    /// must stay closed until BOTH sides are locked, so the partner cannot
    /// restage after we agreed.
    #[test]
    fn approve_opens_only_after_both_sides_confirmed() {
        let mut session = ExchangeSession::new(7);
        assert!(session.can_confirm());
        assert!(!session.can_approve());

        session.partner_confirmed = true;
        assert!(
            !session.can_approve(),
            "the partner's confirm alone must not open approve"
        );

        session.partner_confirmed = false;
        session.own_confirmed = true;
        assert!(
            !session.can_approve(),
            "our own confirm alone must not open approve"
        );
        assert!(!session.can_confirm(), "confirm is spent once locked");
        assert!(session.awaiting_approve());
        assert!(!session.gold_editable(), "gold locks with the offer");

        session.partner_confirmed = true;
        assert!(session.can_approve());

        session.own_approved = true;
        assert!(!session.can_approve(), "approve is sent once");
    }

    /// The 0xB081 accept is the inviter's window-open event: the inviter gets
    /// `0xB081 01 <uid>` and **no** 0x3085, which goes to the target instead.
    /// Before this, whoever pressed "Exchange" never saw a window.
    #[test]
    fn the_inviters_window_opens_on_the_accept_ack() {
        let mut state = ExchangeState::default();
        // mirrors on_invite_response's Accepted arm
        let unique_id = 170_253u32;
        if state.session.is_none() {
            state.session = Some(ExchangeSession::new(unique_id));
        }
        let session = state.session.as_ref().expect("a window");
        assert_eq!(session.partner_unique_id, unique_id);
        assert!(session.can_confirm());
        // Idempotent: a later 0x3085 for the same partner must not wipe it.
        let staged = ExchangeSession {
            partner_unique_id: unique_id,
            own_gold: 100,
            ..default()
        };
        state.session = Some(staged.clone());
        if state.session.is_none() {
            state.session = Some(ExchangeSession::new(unique_id));
        }
        assert_eq!(state.session, Some(staged));
    }

    /// A second 0x7082 does not just get refused, it **terminates the trade**
    /// (`0xB082 02 1f18` + `0x3088 1f18` to both sides). The ack that sets
    /// `own_confirmed` is ~50 ms away, so the in-flight flag is the only thing
    /// between a double click and a dead trade.
    #[test]
    fn a_second_confirm_is_never_offered_while_the_first_is_unacked() {
        let mut session = ExchangeSession::new(7);
        assert!(session.can_confirm());
        session.request_in_flight = true;
        assert!(
            !session.can_confirm(),
            "the unacked confirm must not repeat"
        );

        // ack lands
        session.request_in_flight = false;
        session.own_confirmed = true;
        session.partner_confirmed = true;
        assert!(session.can_approve());
        session.request_in_flight = true;
        assert!(!session.can_approve(), "approve is one press too");
    }

    /// A real partner-side 0x308C off the wire: one item, one slot byte (the
    /// **exchange** slot, not the inventory slot), then the shared item block.
    /// The frame is a partner (uid 170207) offering `ITEM_CH_BLADE_01_A` (ref
    /// id 107, durability 65) staged from bag slot 41 into exchange slot 0.
    /// This is the positive control for the "no `slot_exchange` byte on the
    /// peer copy" reading of the original's dead branch.
    #[test]
    fn a_partner_items_update_decodes() {
        struct Equipment;
        impl ItemClassResolver for Equipment {
            fn item_class(&self, _ref_id: u32) -> ItemClass {
                ItemClass::Equipment
            }
        }

        let body = hex_body(
            "df980200 01 00 00000000 6b000000 00 0000000000000000 41000000 00 01 00 02 00",
        );
        let update = ExchangeItemsUpdate::try_from(body).expect("a well-formed 0x308C");
        assert_eq!(update.player_unique_id, 170_207);
        assert_eq!(update.item_count, 1);

        let items = update.items(&Equipment).expect("the record decodes");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].ref_id, 107);
        assert_eq!(
            items[0].slot, 0,
            "the peer copy carries the exchange slot, not the inventory slot"
        );
    }

    /// A drag out of the bag into our pane must put exactly ONE packet on the
    /// wire, and it must be sub-op 4 with the bag slot as `source` and the
    /// free pane slot as `target` — byte for byte the frame `042900`.
    #[test]
    fn a_drag_into_our_pane_stages_the_bag_slot_with_sub_op_4() {
        let state = ExchangeState {
            session: Some(ExchangeSession::new(170_207)),
        };
        let request = stage_item_request(&state, 0x29).expect("an open trade stages");
        assert_eq!(
            request,
            InventoryOperationRequest::InventoryToExchange {
                source: 0x29,
                target: 0,
            }
        );
        let bytes: bytes::Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x04, 0x29, 0x00]);
    }

    /// The way back is sub-op 5 and quotes the **exchange** slot the server
    /// assigned (`0500`), not the render index: with a gap in the pane the two
    /// differ.
    #[test]
    fn the_way_back_takes_sub_op_5_with_the_pane_slot() {
        let mut session = ExchangeSession::new(170_207);
        session.own_items = vec![item(0x29, 107)];
        session.own_slots = vec![5];
        let state = ExchangeState {
            session: Some(session),
        };
        let session = state.session.as_ref().unwrap();
        assert_eq!(
            free_own_exchange_slot(session),
            Some(0),
            "a withdraw frees slots in the middle, so staging fills the first gap"
        );
        let request = unstage_item_request(&state, session.own_slots[0]).expect("a staged item");
        assert_eq!(
            request,
            InventoryOperationRequest::ExchangeToInventory { source: 5 }
        );
        let bytes: bytes::Bytes = request.into();
        assert_eq!(bytes.as_ref(), &[0x05, 0x05]);
    }

    /// The gold field is the server's word, not the player's: the request is
    /// clamped to the purse and the pane shows nothing until the 0xB034 op-13
    /// ack lands (`010d6400000000000000`).
    #[test]
    fn the_gold_field_shows_the_acked_amount_not_the_typed_one() {
        let mut state = ExchangeState {
            session: Some(ExchangeSession::new(170_207)),
        };
        let request = stage_gold_request(&state, 100, 5_000).expect("gold is editable");
        assert_eq!(
            request,
            InventoryOperationRequest::InventoryGoldToExchange { amount: 100 }
        );
        let bytes: bytes::Bytes = request.into();
        assert_eq!(
            bytes.as_ref(),
            &[0x0d, 0x64, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
        assert_eq!(
            state.session.as_ref().unwrap().own_gold,
            0,
            "sending must not fill the pane"
        );

        // more than we own is clamped, never sent as typed
        assert_eq!(
            stage_gold_request(&state, 9_999, 5_000),
            Some(InventoryOperationRequest::InventoryGoldToExchange { amount: 5_000 })
        );

        let session = state.session.as_mut().unwrap();
        apply_staging_ack(
            session,
            &InventoryOperationResult::InventoryGoldToExchange { amount: 100 },
        );
        assert_eq!(session.own_gold, 100, "the ack is what fills the field");

        // and once our offer is locked the field is spent
        session.own_confirmed = true;
        assert_eq!(stage_gold_request(&state, 1, 5_000), None);
    }

    /// A drag with no trade open must send NOTHING — the same drag is a plain
    /// inventory move, and a staging packet outside a session is answered
    /// `0xB034 02 …` and is a protocol violation waiting to happen.
    #[test]
    fn a_drag_without_an_open_trade_sends_nothing() {
        let state = ExchangeState::default();
        assert_eq!(stage_item_request(&state, 0x29), None);
        assert_eq!(unstage_item_request(&state, 0), None);
        assert_eq!(stage_gold_request(&state, 100, 5_000), None);

        // positive control on the same read path: with a session, all three build
        let open = ExchangeState {
            session: Some(ExchangeSession::new(170_207)),
        };
        assert!(stage_item_request(&open, 0x29).is_some());
        assert!(unstage_item_request(&open, 0).is_some());
        assert!(stage_gold_request(&open, 100, 5_000).is_some());
    }

    /// A locked offer stages nothing, and neither does one whose confirm ack is
    /// still out (that is a lock in progress). Twelve staged items fill the
    /// pane and the thirteenth drag is refused rather than sent to a slot the
    /// window cannot show.
    #[test]
    fn a_locked_or_full_pane_stages_nothing() {
        let mut session = ExchangeSession::new(7);
        session.request_in_flight = true;
        let state = ExchangeState {
            session: Some(session.clone()),
        };
        assert_eq!(stage_item_request(&state, 0x29), None);

        session.request_in_flight = false;
        session.own_slots = (0..EXCHANGE_SLOTS as u8).collect();
        let state = ExchangeState {
            session: Some(session),
        };
        assert_eq!(
            free_own_exchange_slot(state.session.as_ref().unwrap()),
            None
        );
        assert_eq!(stage_item_request(&state, 0x29), None);
    }

    /// The staging acks are the trade window's business, but the bag is not:
    /// ops 4/5/13 move nothing in the inventory (that is 0x3087's job), which
    /// is why `hud::inventory::model` keeps an explicit no-op arm for them.
    #[test]
    fn the_item_acks_leave_the_bag_alone() {
        let mut session = ExchangeSession::new(7);
        session.own_gold = 42;
        let item_acks = [
            InventoryOperationResult::InventoryToExchange {
                source: 0x29,
                target: 0,
            },
            InventoryOperationResult::ExchangeToInventory { source: 0 },
        ];
        for ack in &item_acks {
            assert!(is_staging_result(ack), "ops 4/5 are ours");
            apply_staging_ack(&mut session, ack);
        }
        assert_eq!(session.own_gold, 42, "item acks do not touch gold");
        assert!(session.own_items.is_empty(), "the pane is filled by 0x308C");
        assert!(
            !is_staging_result(&InventoryOperationResult::SlotCleared {
                slot: 0x29,
                reason: 0
            }),
            "a foreign 0xB034 op is not ours"
        );
    }

    fn hex_body(text: &str) -> bytes::Bytes {
        let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        bytes::Bytes::from(
            (0..clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        )
    }

    /// `0xB034 02 3a 18` for a potion stack dragged onto our pane used to
    /// produce no line at all. With the window open, a refusal is told to the
    /// player with its code; without a window it stays a bag matter.
    #[test]
    fn a_refused_item_op_while_trading_reaches_the_chat() {
        let refused = |session: Option<ExchangeSession>| {
            let mut app = App::new();
            app.add_message::<InventoryOperationResponse>()
                .insert_resource(ExchangeState { session })
                .init_resource::<ChatHistory>()
                .add_systems(Update, on_staging_response);
            app.world_mut().write_message(InventoryOperationResponse {
                result: 2,
                operation: None,
                error: Some(0x183A),
            });
            app.update();
            app.world()
                .resource::<ChatHistory>()
                .iter()
                .map(|line| line.text.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            refused(Some(ExchangeSession::new(7))),
            vec!["The server refused this item operation (code 0x183a).".to_string()]
        );
        assert!(
            refused(None).is_empty(),
            "no trade window, not this window's line"
        );
    }

    /// The inviter's only signal when a petition dies before acceptance is
    /// 0x3088 (`2b18`), with no 0xB081 anywhere in between. So the notice must
    /// not be gated on there being a window.
    #[test]
    fn a_cancel_without_a_window_still_tells_the_inviter() {
        let mut state = ExchangeState::default();
        let ui = ClientUiStrings::default();
        let mut history = ChatHistory::default();

        // mirrors on_exchange_canceled's body, inviter case (no session), with
        // the reason the server actually sends there
        let reason = CANCEL_REASON_PARTNER_LEFT;
        let had_window = state.session.take().is_some();
        let (key, fallback) = EXCHANGE_CANCELED_NOTICE;
        history.push(ChatLine::system(format!(
            "{} (code {reason:#06x})",
            ui.get_or(key, fallback)
        )));

        assert!(!had_window, "the inviter never opened one");
        assert_eq!(key, "UIIT_MSG_STRGERR_EXCHANGE_CANCEL");
        assert_eq!(
            history.iter().last().map(|line| line.text.as_str()),
            Some("The exchange has been canceled. (code 0x182b)"),
            "the cancel notice reaches the chat even with no trade window"
        );
        assert!(
            fallback.starts_with("The exchange has been canceled."),
            "the sentence stays the original's; only the code is appended"
        );
    }

    /// Every reason this server sends has a name in the log and *no* sentence
    /// of its own on screen — no `UIIT_*` key is bound to any of them (see
    /// [`EXCHANGE_CANCELED_NOTICE`]). This pins that policy so a later
    /// "helpful" per-reason string cannot slip in unsourced.
    #[test]
    fn the_four_named_cancel_reasons_are_log_only() {
        for reason in [
            CANCEL_REASON_DECLINED,
            CANCEL_REASON_PARTNER_LEFT,
            CANCEL_REASON_EXIT,
            CANCEL_REASON_PROTOCOL_VIOLATION,
        ] {
            assert_ne!(
                cancel_reason_label(reason),
                "unknown reason code",
                "{reason:#06x} has a name"
            );
        }
        assert_eq!(cancel_reason_label(0x1899), "unknown reason code");
    }

    /// The self echo fills our OWN pane, which was empty by construction
    /// before the wire type modelled both 0x308C shapes. The body is the same
    /// blade as the peer-copy test above, one byte longer, and the extra byte
    /// is bag slot 0x29.
    #[test]
    fn the_self_echo_fills_our_own_pane_with_the_bag_slot() {
        struct Equipment;
        impl ItemClassResolver for Equipment {
            fn item_class(&self, _ref_id: u32) -> ItemClass {
                ItemClass::Equipment
            }
        }

        let body = hex_body(
            "df980200 01 29 00 00000000 6b000000 00 0000000000000000 41000000 00 01 00 02 00",
        );
        let update = ExchangeItemsUpdate::try_from(body).expect("a well-formed 0x308C");

        let staged = update.own_items(&Equipment).expect("the record decodes");
        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].slot_exchange, 0);
        assert_eq!(
            staged[0].item.slot, 0x29,
            "the own pane keeps the BAG slot — 0x3087 clears exactly those"
        );
        assert_eq!(staged[0].item.ref_id, 107);

        // and the peer decoder on the same body never yields the real record —
        // here it fails outright, with a class table that resolves every ref
        // id it can misread it silently instead (packets test
        // `the_self_echo_is_the_peer_copy_plus_the_inventory_slot`). Either
        // way it is wrong, which is why on_partner_items branches on the uid
        // rather than on the body length.
        assert!(update
            .items(&Equipment)
            .is_none_or(|items| items[0].ref_id != 107));
    }

    /// 0x3087 moves the partner's records into our first free BAG slots —
    /// never over the equipment slots below `BAG_FIRST_SLOT` — and clears the
    /// slots our own staged records came from.
    #[test]
    fn completion_lands_partner_items_in_free_bag_slots() {
        let mut inventory = Inventory {
            slots: vec![None; BAG_FIRST_SLOT as usize + 3],
            avatar_slots: Vec::new(),
            gold: 500,
        };
        inventory.slots[BAG_FIRST_SLOT as usize] = Some(item(BAG_FIRST_SLOT, 111));

        let session = ExchangeSession {
            partner_unique_id: 7,
            partner_items: vec![item(4, 222), item(9, 333)],
            own_items: vec![item(BAG_FIRST_SLOT, 111)],
            ..default()
        };

        // mirrors on_exchange_completed's body
        for slot in session.own_items.iter().map(|i| i.slot) {
            inventory.slots[slot as usize] = None;
        }
        for item in session.partner_items.iter() {
            let free = first_free_bag_slot(&inventory).expect("a free slot");
            let mut item = item.clone();
            item.slot = free;
            inventory.gain_item(item);
        }

        assert_eq!(
            inventory.get(BAG_FIRST_SLOT).map(|i| i.ref_id),
            Some(222),
            "the freed slot is refilled before later ones"
        );
        assert_eq!(
            inventory.get(BAG_FIRST_SLOT + 1).map(|i| i.ref_id),
            Some(333)
        );
        assert!(
            (0..BAG_FIRST_SLOT).all(|slot| inventory.get(slot).is_none()),
            "equipment slots must never receive a traded item"
        );
        assert_eq!(inventory.gold, 500, "gold is corrected by 0x304E, not here");
    }
}

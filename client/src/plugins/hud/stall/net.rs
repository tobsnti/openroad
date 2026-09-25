//! Buyer half of the player stall (#780): entering a stall, its listing, and
//! buying a row.
//!
//! Idea: the stall is **server-driven end to end**, exactly like the exchange
//! window (`hud/exchange/mod.rs` — "opens on the server's word, never on a
//! click"). Nothing here predicts: the window opens when the server says we
//! entered a stall (`0x30B7` action 2 carrying *our* unique id), the grid is
//! replaced wholesale from the rows the server re-sends after every purchase
//! (`0x30B7` action 3), a buy is a `0x70B4` whose effect is only ever applied
//! from the `0xB0B4` ack, and the window closes on the `0xB0B5` leave ack.
//!
//! The enter **snapshot is wired**: `0xB0B3` is modelled since #759
//! (`StallTalkResponse`), so entering a stall fills the grid, the greeting and
//! the trading badge from the server instead of showing ten empty plates.
//!
//! Two honest gaps, both deliberate and neither invented around:
//!
//! * **Asking to enter a stall** is `0x70B3`, a single `u32` unique id read out
//!   of the original's stall-talk builder
//!   (`packets/.../stall.rs::StallTalkRequest`).
//!   The visitor path therefore has its first step: clicking a player who is
//!   running a stall walks up and sends it
//!   (`cursor/interactions/npcs.rs::approach_talk_target`, which is where the
//!   original's own world-click branch lands), and this module's `0xB0B3`
//!   handler opens the window on the answer.
//! * **Buy carries no quantity.** `StallBuyRequest` is `{stall_slot: u8}` and
//!   nothing else (`docs/net-stall-0x30B7.md` §0x70B4) — the row's own
//!   quantity is what is bought. The original's price/quantity message box is
//!   therefore **not** wired here: its binding to this window is unconfirmed
//!   and it has no field to fill on the wire.
//!
//! Error feedback: go-sro carries a complete
//! 17-entry `StallErrorCode` table, which is the *enum names* — not localized
//! sentences. We show the name and the code, and a code we do not have a name
//! for shows the code alone. Writing an English sentence for an unnamed code
//! would be an invented string in the user's UI.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use packets::agent::stall::{
    StallBuyRequest, StallBuyResponse, StallEntityAction, StallItemRow, StallLeaveRequest,
    StallLeaveResponse, StallTalkResponse,
};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::stall::model::{StallRow, StallState, StallTradingState};
use crate::plugins::hud::stall::ui::{StallSlot, STALL_SLOTS};
use crate::plugins::hud::toast::{ShowToast, ToastKind};
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::entities::NetworkId;
use crate::plugins::net::stall::StallOwner;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientTextNames};

/// go-sro's `StallErrorCode` table, complete
/// (`handler/stall/stall_handler.go:19-37`). These are enum identifiers, not UI
/// strings — see the module note.
const STALL_ERRORS: [(u16, &str); 17] = [
    (0x0005, "InvalidOperation"),
    (0x3C08, "InvalidPrice"),
    (0x3C0C, "NothingToSell"),
    (0x3C0E, "MarketClosed"),
    (0x3C11, "NotEnoughGold"),
    (0x3C12, "InventoryFull"),
    (0x3C15, "HostLeft"),
    (0x3C16, "ImBusy"),
    (0x3C17, "ClosedByHost"),
    (0x3C18, "MarketFull"),
    (0x3C2B, "InvalidHostState"),
    (0x3C2C, "CustomerBanned"),
    (0x3C34, "CannotOpenFromHorse"),
    (0x3C38, "MarketnameNotAllowed"),
    (0x3C39, "CannotOpenMarketMurderer"),
    (0x3C3B, "NotUseJob"),
    (0x3C41, "WareNetworkFail"),
];

/// What the UI says about an error code. A known code shows its name **and**
/// the code; an unknown one shows only the code, never a guessed sentence
/// (#780 acceptance point 4).
pub fn stall_error_text(code: u16) -> String {
    match STALL_ERRORS.iter().find(|(value, _)| *value == code) {
        Some((_, name)) => format!("Stall: {name} (0x{code:04X})"),
        None => format!("Stall: error 0x{code:04X}"),
    }
}

/// What a `0xB0B4` means, decoded once so the system stays a thin shell and
/// the branching is testable headless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuyOutcome {
    /// `result == 1` — the server sold us the item in this stall slot.
    Bought(u8),
    /// Any other result, with the server's error code.
    Failed(u16),
    /// A body the model could not complete — neither tail was present.
    Malformed,
}

/// Split on `result == 1` / `result != 1`, not on `== 2` like the other acks: an
/// unexpected result must read as a failure rather than as a successful slot.
pub fn buy_outcome(response: &StallBuyResponse) -> BuyOutcome {
    match (response.result, response.stall_slot, response.error_code) {
        (1, Some(slot), _) => BuyOutcome::Bought(slot),
        (1, None, _) => BuyOutcome::Malformed,
        (_, _, Some(code)) => BuyOutcome::Failed(code),
        _ => BuyOutcome::Malformed,
    }
}

/// What a `0x30B7` means *for us*, given our own spawn id.
///
/// The enter/exit tail is read as the viewer's id. The broadcast reaches every
/// viewer, so without that id we would open the window for a stranger's footstep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerTransition {
    /// We entered the stall — open the window.
    WeEntered,
    /// We left it — close the window.
    WeLeft,
    /// Another viewer came or went; the window is unaffected.
    OtherViewer,
}

pub fn viewer_transition(
    action: &StallEntityAction,
    own_unique_id: u32,
) -> Option<ViewerTransition> {
    match action {
        StallEntityAction::Enter { unique_id } if *unique_id == own_unique_id => {
            Some(ViewerTransition::WeEntered)
        }
        StallEntityAction::Exit { unique_id } if *unique_id == own_unique_id => {
            Some(ViewerTransition::WeLeft)
        }
        StallEntityAction::Enter { .. } | StallEntityAction::Exit { .. } => {
            Some(ViewerTransition::OtherViewer)
        }
        _ => None,
    }
}

/// Place wire rows into the ten grid slots by their own stall slot.
///
/// Replaced wholesale, never diffed: type-2/3 and action-3 both re-send the
/// **whole** list, so a diff would keep a row
/// the server just dropped. A row outside the ten-slot capacity is reported
/// rather than dropped silently — it would mean the capacity fact is wrong.
pub fn rows_to_slots(
    rows: &[StallItemRow],
    name_of: impl Fn(u32) -> String,
) -> Vec<Option<StallRow>> {
    let mut slots = vec![None; STALL_SLOTS];
    for row in rows {
        let index = row.item.slot as usize;
        let Some(cell) = slots.get_mut(index) else {
            warn!(
                "stall: row in slot {} but the window holds {} — capacity refuted",
                row.item.slot, STALL_SLOTS
            );
            continue;
        };
        *cell = Some(StallRow {
            name: name_of(row.item.ref_id),
            ref_id: row.item.ref_id,
            quantity: row.quantity,
            price: row.price,
        });
    }
    slots
}

/// Display name for an item ref id, via itemdata's `SN_*` key. Falls back to
/// the ref id so an item missing from the user's tables still has a handle.
pub(crate) fn item_name(
    item_data: &ClientItemData,
    names: &ClientTextNames,
    ref_id: u32,
) -> String {
    item_data
        .get(&(ref_id as i32))
        .and_then(|row| row.name_key())
        .and_then(|key| names.name(key))
        .map(str::to_string)
        .unwrap_or_else(|| format!("#{ref_id}"))
}

/// What a `0xB0B3` snapshot does to the window, decoded once so the branching
/// is testable headless. `slots` is the caller's already-placed grid (the row
/// decode needs an item resolver, which a pure function must not carry) and
/// `title` is what the world already knows about this stall from `0x30B8` /
/// the spawn embed — the snapshot itself carries no title, only the owner's
/// uid, note and rows.
///
/// Returns the error code of a refused talk.
pub fn apply_talk_response(
    state: &mut StallState,
    response: &StallTalkResponse,
    slots: Option<Vec<Option<StallRow>>>,
    title: String,
) -> Option<u16> {
    match response {
        StallTalkResponse::Failure { error_code, .. } => Some(*error_code),
        StallTalkResponse::Success {
            message, is_open, ..
        } => {
            state.open = true;
            // Never ours: `0xB0B3` is the answer to entering somebody else's
            // stall. Our own opens on `0xB0B1` (`owner.rs`).
            state.owner = false;
            state.title = title;
            state.greeting = message.clone();
            state.trading = if *is_open {
                StallTradingState::Open
            } else {
                StallTradingState::Modifying
            };
            // A tail that did not decode leaves the grid honestly empty rather
            // than showing the previous stall's rows.
            state.slots = slots.unwrap_or_else(|| vec![None; STALL_SLOTS]);
            None
        }
    }
}

/// `0xB0B3` — the snapshot that opens a visitor's stall window.
pub fn on_stall_talk_response(
    mut reader: MessageReader<StallTalkResponse>,
    item_data: Res<ClientItemData>,
    names: Res<ClientTextNames>,
    stalls: Query<(&NetworkId, &StallOwner)>,
    mut state: ResMut<StallState>,
    mut toasts: MessageWriter<ShowToast>,
) {
    for response in reader.read() {
        let (slots, title) = match response {
            StallTalkResponse::Success { unique_id, .. } => {
                let slots = response.snapshot(&*item_data).map(|snapshot| {
                    rows_to_slots(&snapshot.rows, |ref_id| {
                        item_name(&item_data, &names, ref_id)
                    })
                });
                if slots.is_none() {
                    warn!("stall: 0xB0B3 tail did not decode — grid left empty");
                }
                let title = stalls
                    .iter()
                    .find(|(id, _)| id.0 == *unique_id)
                    .map(|(_, owner)| owner.title.clone())
                    .unwrap_or_default();
                (slots, title)
            }
            StallTalkResponse::Failure { .. } => (None, String::new()),
        };
        if let Some(code) = apply_talk_response(&mut state, response, slots, title) {
            let text = stall_error_text(code);
            warn!("stall: could not enter the stall — {text}");
            toasts.write(ShowToast::new(ToastKind::Warning, text));
            continue;
        }
        info!(
            "stall: entered a stall, {} row(s) listed (0xB0B3)",
            state.slots.iter().filter(|s| s.is_some()).count()
        );
    }
}

/// `0x30B7` — a viewer entered or left, or a purchase went through.
pub fn on_stall_entity_action(
    mut reader: MessageReader<StallEntityAction>,
    item_data: Res<ClientItemData>,
    names: Res<ClientTextNames>,
    own: Query<&NetworkId, With<Player>>,
    mut state: ResMut<StallState>,
    mut toasts: MessageWriter<ShowToast>,
) {
    let own_unique_id = own.iter().next().map(|id| id.0);
    for action in reader.read() {
        if let StallEntityAction::Buy {
            stall_slot,
            buyer_name,
            ..
        } = action
        {
            // The refreshed rows are the stall's whole remaining listing.
            let Some(rows) = action.rows(&*item_data) else {
                warn!("stall: 0x30B7 buy rows did not decode — grid left unchanged");
                continue;
            };
            info!(
                "stall: {buyer_name} bought slot {stall_slot}; {} row(s) left",
                rows.len()
            );
            state.slots = rows_to_slots(&rows, |ref_id| item_name(&item_data, &names, ref_id));
            continue;
        }
        let Some(own_unique_id) = own_unique_id else {
            continue;
        };
        match viewer_transition(action, own_unique_id) {
            Some(ViewerTransition::WeEntered) => {
                info!("stall: entered a stall (0x30B7 action 2)");
                state.open = true;
                // The listing belongs to the 0xB0B3 snapshot, which is the
                // other half of the same entry and may arrive either side of
                // this broadcast. Clearing the grid here would blank a
                // snapshot that already landed, so the broadcast only opens
                // the window; an entry whose snapshot never came shows the
                // empty plates it honestly has.
            }
            Some(ViewerTransition::WeLeft) => {
                info!("stall: left the stall (0x30B7 action 1)");
                state.open = false;
            }
            Some(ViewerTransition::OtherViewer) => {}
            None => {
                warn!("stall: 0x30B7 with an action neither source describes — ignored");
                toasts.write(ShowToast::new(
                    ToastKind::Warning,
                    "Stall: unknown stall action",
                ));
            }
        }
    }
}

/// `0xB0B4` — the buy ack. Success is *not* applied to the grid here: the
/// server re-sends the whole listing on `0x30B7` action 3, and the item itself
/// arrives over the normal inventory paths.
pub fn on_stall_buy_response(
    mut reader: MessageReader<StallBuyResponse>,
    mut toasts: MessageWriter<ShowToast>,
) {
    for response in reader.read() {
        match buy_outcome(response) {
            BuyOutcome::Bought(slot) => {
                info!("stall: bought slot {slot} (0xB0B4)");
            }
            BuyOutcome::Failed(code) => {
                let text = stall_error_text(code);
                warn!("stall: buy refused — {text}");
                toasts.write(ShowToast::new(ToastKind::Warning, text));
            }
            BuyOutcome::Malformed => {
                warn!("stall: 0xB0B4 carried neither a slot nor an error code — ignored");
            }
        }
    }
}

/// `0xB0B5` — the leave ack. The window closes on the ack, never on the click,
/// so a refused leave keeps us in the stall the server still has us in (the
/// same rule as the exchange window's exit).
pub fn on_stall_leave_response(
    mut reader: MessageReader<StallLeaveResponse>,
    mut state: ResMut<StallState>,
    mut toasts: MessageWriter<ShowToast>,
) {
    for response in reader.read() {
        if response.result == 1 {
            info!("stall: left the stall (0xB0B5)");
            state.open = false;
            continue;
        }
        match response.error_code {
            Some(code) => {
                let text = stall_error_text(code);
                warn!("stall: leave refused — {text}");
                toasts.write(ShowToast::new(ToastKind::Warning, text));
            }
            None => warn!("stall: leave refused with no error code (0xB0B5) — window stays open"),
        }
    }
}

/// A click on a listed row buys it — or, in our own stall, edits it: left
/// takes the row off sale (type 3), right re-prices it (type 1), see below.
/// There is no confirm dialog for the BUY: the original's price/quantity
/// message box has no confirmed binding to this window and
/// `0x70B4` has no field it could fill (see the module note), so inventing one
/// would put a guessed dialog between the player and a modelled request.
///
/// Deviation, stated: the original's buy path goes through that dialog. Ours
/// sends the row directly and reports the server's verdict; the dialog lands
/// with the snapshot opcode (#759), which is what would give it a price to
/// show in the first place.
pub fn on_stall_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&StallSlot>,
    state: Res<StallState>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    item_data: Res<ClientItemData>,
    names: Res<ClientTextNames>,
    mut modal: ResMut<super::stock::StockModal>,
) {
    let Ok(cell) = cells.get(press.entity) else {
        return;
    };
    if state.slots.get(cell.0).and_then(Option::as_ref).is_none() {
        return;
    }
    let Ok(slot) = u8::try_from(cell.0) else {
        return;
    };
    // Owner, right button: re-price the row (`0x70BA` type 1). It reopens the
    // SAME price box the drop path uses, on the row's current numbers
    // (`stock::reprice_prompt`).
    //
    // Stated deviation (ADR-0009): the original's gesture for this is unknown,
    // which is also why the box's binding to this window is unconfirmed.
    // Left-click on our own row is already the take-off-sale path (type 3) and
    // is not moved; the second action therefore takes the tree's existing
    // secondary gesture (`hud/magic_state_board.rs:239`,
    // `skill_window/ui.rs:1786`).
    if state.owner && press.event.button == PointerButton::Secondary {
        if let Some(prompt) = super::stock::reprice_prompt(&state, slot, |ref_id| {
            item_name(&item_data, &names, ref_id)
        }) {
            info!("stall: re-pricing slot {slot} (0x70BA type 1)");
            modal.prompt = Some(prompt);
        }
        return;
    }
    if press.event.button != PointerButton::Primary {
        return;
    }
    // You cannot buy from your own stall: for the owner the same click is the
    // stocking path's other half — it takes the row back off sale (`0x70BA`
    // type 3, `stall/stock.rs`).
    if state.owner {
        info!("stall: taking slot {slot} off sale (0x70BA type 3)");
        super::stock::send_remove(&conn, slot);
        return;
    }
    info!("stall: buying slot {slot} (0x70B4)");
    send_buy(&conn, slot);
}

/// The close button is a **protocol act**, not a UI hide: leaving a stall is
/// `0x70B5` and the window goes away on the `0xB0B5` ack, so a refused leave
/// cannot desync us from a stall the server still has us in (the exchange
/// window's exit rule, `hud/exchange/model.rs::send_exit`).
///
/// With no agent connection there is nobody to ack, so the offline preview
/// scenes close the window locally instead of hanging it open.
pub fn on_stall_close_button(
    _: On<Activate>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut state: ResMut<StallState>,
) {
    if conn.single().is_err() {
        state.open = false;
        return;
    }
    // Closing OUR stall is a destroy (0x70B2); closing somebody else's window
    // is a leave (0x70B5). Same button, two protocol acts (#781).
    if state.owner {
        super::owner::send_destroy(&conn);
    } else {
        send_leave(&conn);
    }
}

pub(crate) fn send(
    conn: &Query<&SilkroadConnection, With<AgentConnection>>,
    packet: Packet,
    what: &str,
) {
    let Ok(conn) = conn.single() else {
        return;
    };
    if let Err(e) = conn.get_sender().send(packet.into()) {
        error!("network: failed to send stall {what}: {}", e.0);
    }
}

/// `0x70B4` — buy the item in `stall_slot`.
pub fn send_buy(conn: &Query<&SilkroadConnection, With<AgentConnection>>, stall_slot: u8) {
    send(conn, Packet::from(StallBuyRequest { stall_slot }), "buy");
}

/// `0x70B5` — leave the stall. Closing the window is a protocol act.
pub fn send_leave(conn: &Query<&SilkroadConnection, With<AgentConnection>>) {
    send(conn, Packet::from(StallLeaveRequest), "leave");
}

#[cfg(test)]
mod test {
    use super::*;
    use packets::agent::character_data::{InventoryItem, ItemTypeData, RentInfo};

    fn row(stall_slot: u8, ref_id: u32, quantity: u16, price: u64) -> StallItemRow {
        StallItemRow {
            item: InventoryItem {
                slot: stall_slot,
                rent: RentInfo::default(),
                ref_id,
                data: ItemTypeData::Expendable {
                    inscription: None,
                    stack_count: quantity as u16,
                    assimilation_prob: None,
                    mag_params: vec![],
                },
            },
            inventory_slot: 13,
            quantity,
            price,
        }
    }

    /// The listing is placed by the row's OWN stall slot, not by arrival
    /// order: a stall with a gap renders that gap, and a wholesale replace
    /// drops what the server dropped.
    #[test]
    fn rows_land_in_their_own_stall_slots_and_replace_the_grid() {
        let slots = rows_to_slots(&[row(3, 111, 2, 900), row(0, 222, 1, 12)], |id| {
            format!("item{id}")
        });
        assert_eq!(slots.len(), STALL_SLOTS);
        assert_eq!(slots[0].as_ref().map(|r| r.price), Some(12));
        assert_eq!(
            slots[3].as_ref().map(|r| r.name.clone()),
            Some("item111".into())
        );
        assert_eq!(slots[3].as_ref().map(|r| r.quantity), Some(2));
        assert!(slots[1].is_none(), "an unlisted slot stays empty");
        assert_eq!(slots.iter().filter(|s| s.is_some()).count(), 2);
    }

    /// A row past the ten-slot capacity must not panic or land in slot 0.
    #[test]
    fn a_row_past_the_capacity_is_reported_and_not_wrapped() {
        let slots = rows_to_slots(&[row(200, 111, 1, 5)], |id| format!("item{id}"));
        assert!(slots.iter().all(Option::is_none));
    }

    /// `buy_outcome` branches on `result == 1`, not `== 2`: the two differ for
    /// every other result value, which is exactly where a copy-paste from the
    /// create/destroy acks would go wrong.
    #[test]
    fn buy_outcome_branches_on_result_one() {
        assert_eq!(
            buy_outcome(&StallBuyResponse {
                result: 1,
                stall_slot: Some(4),
                error_code: None,
            }),
            BuyOutcome::Bought(4)
        );
        assert_eq!(
            buy_outcome(&StallBuyResponse {
                result: 2,
                stall_slot: None,
                error_code: Some(0x3C11),
            }),
            BuyOutcome::Failed(0x3C11)
        );
        // result 3 is not `== 2`; it must still read as a failure
        assert_eq!(
            buy_outcome(&StallBuyResponse {
                result: 3,
                stall_slot: None,
                error_code: Some(0x3C0E),
            }),
            BuyOutcome::Failed(0x3C0E)
        );
        assert_eq!(
            buy_outcome(&StallBuyResponse {
                result: 1,
                stall_slot: None,
                error_code: None,
            }),
            BuyOutcome::Malformed
        );
    }

    /// A code we have a name for shows the name; one we do not shows the code
    /// and nothing else — never an invented sentence.
    #[test]
    fn an_unknown_error_code_is_shown_as_a_code_not_a_sentence() {
        assert_eq!(stall_error_text(0x3C11), "Stall: NotEnoughGold (0x3C11)");
        assert_eq!(stall_error_text(0x3C12), "Stall: InventoryFull (0x3C12)");
        let unknown = stall_error_text(0x1234);
        assert_eq!(unknown, "Stall: error 0x1234");
        assert!(
            !unknown.contains(' ') || unknown.split(' ').count() == 3,
            "the unknown branch must carry no prose: {unknown}"
        );
    }

    /// The broadcast reaches every viewer, so only OUR uid may open or close
    /// the window.
    #[test]
    fn only_our_own_uid_opens_or_closes_the_stall_window() {
        assert_eq!(
            viewer_transition(&StallEntityAction::Enter { unique_id: 7 }, 7),
            Some(ViewerTransition::WeEntered)
        );
        assert_eq!(
            viewer_transition(&StallEntityAction::Exit { unique_id: 7 }, 7),
            Some(ViewerTransition::WeLeft)
        );
        assert_eq!(
            viewer_transition(&StallEntityAction::Enter { unique_id: 9 }, 7),
            Some(ViewerTransition::OtherViewer)
        );
        assert_eq!(
            viewer_transition(&StallEntityAction::Exit { unique_id: 9 }, 7),
            Some(ViewerTransition::OtherViewer)
        );
        assert_eq!(
            viewer_transition(
                &StallEntityAction::Unknown {
                    action: 42,
                    tail: Default::default(),
                },
                7
            ),
            None
        );
    }

    /// The request the buy button builds: opcode 0x70B4, slot only — there is
    /// no quantity field on it (see the module note).
    #[test]
    fn a_buy_request_leaves_as_0x70b4_carrying_only_the_stall_slot() {
        let (opcode, body) = Packet::from(StallBuyRequest { stall_slot: 9 }).into_serialize();
        assert_eq!(opcode, 0x70B4);
        assert_eq!(&body[..], &[9u8][..]);
        // and the leave the close button sends is the empty-bodied 0x70B5
        let (opcode, body) = Packet::from(StallLeaveRequest).into_serialize();
        assert_eq!(opcode, 0x70B5);
        assert!(body.is_empty());
    }
}

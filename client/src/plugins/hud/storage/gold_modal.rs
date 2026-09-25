//! The gold deposit/withdraw popup — **one** implementation, two warehouses.
//!
//! Idea: `GDR_STORAGE_BTN_MONEY` (`resinfo/ifstorageroom.txt:645`) exists once
//! in the data because the personal warehouse (`GDR_STORAGEROOM`, id 19) and
//! the guild warehouse (`GDR_GUILDSTORAGEROOM`, id 145) are the *same*
//! `CIFStorageRoom` class with the same interior layout file
//! (`ginterface.txt:569` / `:1362`, and resinfo has no
//! `ifguildstorageroom.txt` — see `guild_storage/ui.rs`). So the popup is
//! parameterised over its **target container** rather than duplicated: one
//! resource ([`GoldModal`], carrying [`GoldTarget`]), one builder, one press
//! handler, and each window's money button only says which side it means.
//! The alternative — a second copy in `guild_storage/ui.rs` — is what
//! `exchange/ui.rs` did, and that copy is the reason this comment exists.
//!
//! The wire half differs only in the op numbers: personal 12/11
//! (`InventoryGoldToStorage` / `StorageGoldToInventory`), guild 32/33
//! (`InventoryGoldToGuildStorage` / `GuildStorageGoldToInventory`). All four
//! are the bare `u64` amount with **no NPC id** — the gold arm of the
//! original's request builder writes the `u64` and returns
//! (the original's gold builder and inventory-op serializer, quoted at
//! `packets/src/agent/inventory.rs:168-208`) — so the session the server
//! books against is the one the open packet established, and this popup only
//! has to know *which* warehouse is open.
//!
//! Double-booking trap (already paid for once): a gold ack applies to the
//! **warehouse side only**. The player's own total is corrected by the
//! 0x304E the server sends alongside, and subtracting it here as well read a
//! 1234 deposit as 2468 gone until relog (`storage/model.rs`,
//! `guild_storage/model.rs::on_guild_storage_operation`). This file therefore
//! writes no gold anywhere — it only sends.
//!
//! The layout is unknown: `GDR_STORAGE_BTN_MONEY` fires `CommandID=11` with no
//! documented target and `ifstorageroom.txt` has no popup section, so the
//! panel's own rects are ours, not sourced. Only the *button* rect below is
//! vanilla.

use bevy::input_focus::{FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle};
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::prelude::InventoryOperationRequest;
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::guild_storage::model::GuildStorageState;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::storage::model::{PendingStorageOp, StorageOp, StorageState};
use crate::plugins::hud::storage::ui::StorageClosing;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// Which warehouse the popup is bound to. Not a bool: the two sides differ in
/// their session, their stored total and their op numbers, and a bool at the
/// call site would say nothing about which is which.
#[derive(Resource, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum GoldTarget {
    /// `GDR_STORAGEROOM`, ops 12 (deposit) / 11 (withdraw).
    #[default]
    PersonalStorage,
    /// `GDR_GUILDSTORAGEROOM`, ops 32 (deposit) / 33 (withdraw).
    GuildStorage,
}

/// The popup behind either money button.
#[derive(Resource, Default)]
pub struct GoldModal {
    pub open: bool,
    /// Which warehouse the open popup pays into. Only meaningful while
    /// `open`; a money button sets both in one go via [`Self::open_for`].
    pub target: GoldTarget,
}

impl GoldModal {
    /// Open the popup for one warehouse. Both fields are written together so
    /// a press can never open the popup on the *other* window's target.
    pub fn open_for(&mut self, target: GoldTarget) {
        self.open = true;
        self.target = target;
    }
}

/// The typed amount, parsed from the input every frame (empty = 0).
#[derive(Resource, Default)]
pub struct GoldAmount(pub u64);

#[derive(Component)]
pub struct GoldModalRoot;

#[derive(Component)]
pub struct GoldAmountInput;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum GoldModalButton {
    Deposit,
    Withdraw,
    Cancel,
}

/// What a press on the popup does. Split out of the observer so the rules
/// (clamping, the empty amount, the closed session) are testable without a
/// window, a connection or a picking backend — the `durability.rs:167`
/// precedent of isolating a decision from its query soup.
#[derive(Clone, Debug, PartialEq)]
enum GoldPress {
    /// Send this request, then close.
    Send(InventoryOperationRequest),
    /// Close without sending (Cancel, or the warehouse closed under us).
    Close,
    /// Stay open, send nothing (an empty/zero amount).
    Nothing,
}

/// The gold press rules, in one place:
/// * Cancel closes.
/// * No open session closes — a request would be booked against nothing.
/// * The amount is clamped to the side it comes FROM (deposit: the player's
///   purse, withdraw: the warehouse's total), so a typed 999999 moves what
///   exists instead of being refused server-side.
/// * A zero amount sends nothing and leaves the popup open (the player is
///   mid-typing).
fn gold_press(
    target: GoldTarget,
    button: GoldModalButton,
    typed: u64,
    player_gold: u64,
    stored_gold: u64,
    session_open: bool,
) -> GoldPress {
    if matches!(button, GoldModalButton::Cancel) || !session_open {
        return GoldPress::Close;
    }
    let amount = match button {
        GoldModalButton::Deposit => typed.min(player_gold),
        GoldModalButton::Withdraw => typed.min(stored_gold),
        GoldModalButton::Cancel => unreachable!("returned above"),
    };
    if amount == 0 {
        return GoldPress::Nothing;
    }
    GoldPress::Send(match (target, button) {
        (GoldTarget::PersonalStorage, GoldModalButton::Deposit) => {
            InventoryOperationRequest::InventoryGoldToStorage { amount }
        }
        (GoldTarget::PersonalStorage, GoldModalButton::Withdraw) => {
            InventoryOperationRequest::StorageGoldToInventory { amount }
        }
        (GoldTarget::GuildStorage, GoldModalButton::Deposit) => {
            InventoryOperationRequest::InventoryGoldToGuildStorage { amount }
        }
        (GoldTarget::GuildStorage, GoldModalButton::Withdraw) => {
            InventoryOperationRequest::GuildStorageGoldToInventory { amount }
        }
        (_, GoldModalButton::Cancel) => unreachable!("returned above"),
    })
}

/// The stored total and whether that warehouse is open, for the active target.
fn side(
    target: GoldTarget,
    storage: &StorageState,
    guild: &GuildStorageState,
) -> (u64, bool, &'static str) {
    match target {
        GoldTarget::PersonalStorage => (
            storage.items.gold,
            storage.session.is_some(),
            "UIIT_STT_STORAGEROOM",
        ),
        GoldTarget::GuildStorage => (
            guild.items.gold,
            guild.session.is_some(),
            "UIIT_STT_GUILD_WAREHOUSE",
        ),
    }
}

/// Rebuild the popup when it opens/closes. Reads both warehouses because it
/// serves both; only the one named by [`GoldModal::target`] is shown.
#[allow(clippy::too_many_arguments)]
pub fn sync_gold_modal(
    modal: Res<GoldModal>,
    storage: Res<StorageState>,
    guild: Res<GuildStorageState>,
    existing: Query<Entity, With<GoldModalRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut focus: ResMut<InputFocus>,
    mut amount: ResMut<GoldAmount>,
    mut commands: Commands,
) {
    if !modal.is_changed() {
        return;
    }
    for entity in existing.iter() {
        commands.entity(entity).insert(StorageClosing);
    }
    if !modal.open {
        if !existing.is_empty() {
            focus.clear();
        }
        return;
    }
    let Ok(camera) = cam_query.single() else {
        return;
    };
    amount.0 = 0;
    let player_gold = inventories.single().map(|inv| inv.gold).unwrap_or(0);
    let (stored_gold, _, side_key) = side(modal.target, &storage, &guild);
    let side_label = match modal.target {
        GoldTarget::PersonalStorage => ui_strings.get_or(side_key, "Storage"),
        GoldTarget::GuildStorage => ui_strings.get_or(side_key, "Guild storage"),
    }
    .to_string();
    let s = hud_scale();
    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };
    let button_style = ImageButtonStyle {
        normal: asset_server.load("media://interface/ifcommon/com_button.ddj"),
        hover: asset_server.load("media://interface/ifcommon/com_button_focus.ddj"),
        press: asset_server.load("media://interface/ifcommon/com_button_press.ddj"),
        ..Default::default()
    };

    let mut input_entity = None;
    commands
        .spawn((
            GoldModalRoot,
            Name::from("Storage Gold Modal"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(65),
            UiTargetCamera(camera),
        ))
        .with_children(|scrim| {
            scrim
                .spawn((
                    Node {
                        width: Val::Px(240.0 * s),
                        height: Val::Px(120.0 * s),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.09, 0.08, 0.06)),
                    Outline {
                        width: Val::Px(1.0),
                        color: Color::srgb(0.55, 0.45, 0.25),
                        ..default()
                    },
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(
                            ui_strings
                                .get_or("UIIT_STT_DEPOSITMONEY", "Deposit Amount")
                                .to_string(),
                        ),
                        text_font(8.5),
                        TextColor(NAME_COLOR),
                        TextLayout::justify(Justify::Center),
                        abs_node((0.0, 10.0, 240.0, 14.0), s),
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        Text::new(format!(
                            "Inventory: {player_gold}   {side_label}: {stored_gold}"
                        )),
                        text_font(7.5),
                        TextColor(PRICE_COLOR),
                        TextLayout::justify(Justify::Center),
                        abs_node((0.0, 28.0, 240.0, 12.0), s),
                        Pickable::IGNORE,
                    ));
                    let mut input_box = abs_node((60.0, 44.0, 120.0, 18.0), s);
                    input_box.padding = UiRect::top(Val::Px(2.0 * s));
                    input_entity = Some(
                        panel
                            .spawn((
                                GoldAmountInput,
                                EditableText {
                                    visible_lines: Some(1.0),
                                    allow_newlines: false,
                                    max_characters: Some(12),
                                    ..default()
                                },
                                EditableTextFilter::new(|c: char| c.is_ascii_digit()),
                                input_box,
                                text_font(9.0),
                                TextColor(Color::WHITE),
                                TextLayout::justify(Justify::Center),
                                TextCursorStyle {
                                    color: Color::WHITE,
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.16, 0.14, 0.1)),
                            ))
                            .id(),
                    );
                    for (button, key, fallback, x) in [
                        (GoldModalButton::Deposit, "UIIT_STT_DEPOSIT", "Store", 10.0),
                        (GoldModalButton::Withdraw, "UIIT_STT_WITHDRAW", "Take", 85.0),
                        (GoldModalButton::Cancel, "UIIT_CTL_CANCEL", "Cancel", 160.0),
                    ] {
                        panel
                            .spawn((
                                button,
                                Button,
                                Hovered::default(),
                                abs_node((x, 76.0, 70.0, 24.0), s),
                                ImageNode {
                                    image: button_style.normal.clone(),
                                    image_mode: NodeImageMode::Stretch,
                                    ..default()
                                },
                                button_style.clone(),
                            ))
                            .observe(on_gold_modal_button)
                            .with_children(|b| {
                                b.spawn((
                                    Text::new(ui_strings.get_or(key, fallback).to_string()),
                                    text_font(8.0),
                                    TextColor(Color::WHITE),
                                    TextLayout::justify(Justify::Center),
                                    Node {
                                        position_type: PositionType::Absolute,
                                        top: Val::Px(6.0 * s),
                                        width: Val::Percent(100.0),
                                        ..default()
                                    },
                                    Pickable::IGNORE,
                                ));
                            });
                    }
                });
        });
    if let Some(input) = input_entity {
        focus.set(input, FocusCause::Navigated);
    }
}

/// Parse the typed gold amount into [`GoldAmount`] (empty = 0).
pub fn sync_gold_amount(
    modal: Res<GoldModal>,
    inputs: Query<&EditableText, With<GoldAmountInput>>,
    mut amount: ResMut<GoldAmount>,
) {
    if !modal.open {
        return;
    }
    let Ok(editable) = inputs.single() else {
        return;
    };
    let parsed = editable
        .value()
        .to_string()
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    if amount.0 != parsed {
        amount.0 = parsed;
    }
}

/// Store/Take send the 0x7034 gold op of the popup's target; Cancel closes.
/// The decision itself is [`gold_press`] — this is the query half around it.
#[allow(clippy::too_many_arguments)]
fn on_gold_modal_button(
    activate: On<Activate>,
    buttons: Query<&GoldModalButton>,
    storage: Res<StorageState>,
    guild: Res<GuildStorageState>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    amount: Res<GoldAmount>,
    mut modal: ResMut<GoldModal>,
    mut pending: ResMut<PendingStorageOp>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    let player_gold = inventories.single().map(|inv| inv.gold).unwrap_or(0);
    let (stored_gold, session_open, _) = side(modal.target, &storage, &guild);
    let request = match gold_press(
        modal.target,
        *button,
        amount.0,
        player_gold,
        stored_gold,
        session_open,
    ) {
        GoldPress::Close => {
            modal.open = false;
            return;
        }
        GoldPress::Nothing => {
            info!("storage: gold amount is 0, nothing to send");
            return;
        }
        GoldPress::Send(request) => request,
    };
    modal.open = false;
    let Ok(conn) = conn.single() else {
        warn!("storage: no agent connection, dropping gold request");
        return;
    };
    // Only the personal warehouse gates its acks on a pending op; the guild
    // consumer applies the 0xB034 it gets (`guild_storage/model.rs`).
    if modal.target == GoldTarget::PersonalStorage {
        pending.0 = Some(match button {
            GoldModalButton::Deposit => StorageOp::GoldDeposit,
            GoldModalButton::Withdraw => StorageOp::GoldWithdraw,
            GoldModalButton::Cancel => unreachable!("Cancel closed above"),
        });
    }
    info!("storage: sending {:?}", request);
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send storage gold request: {}", e.0);
    }
}

/// The popup is bound to the warehouse it pays into: when that session ends
/// (walked away from the NPC, guild lock released), the popup goes with it.
pub fn close_modal_with_session(
    storage: Res<StorageState>,
    guild: Res<GuildStorageState>,
    mut modal: ResMut<GoldModal>,
) {
    if !modal.open {
        return;
    }
    let (_, session_open, _) = side(modal.target, &storage, &guild);
    if !session_open {
        modal.open = false;
    }
}

/// The two text colours of the `CIFStorageRoom` money row, kept identical to
/// the windows behind the popup (`storage/ui.rs`).
const NAME_COLOR: Color = Color::srgb_u8(255, 226, 123);
const PRICE_COLOR: Color = Color::srgb_u8(255, 217, 83);

#[cfg(test)]
mod test {
    use super::*;

    use bytes::Bytes;

    /// One press = one packet, and the packet is the op the doc names. The
    /// assertion is on the ENCODED bytes, not on the enum variant: op number
    /// and byte order are what the server sees, and both are pinned in
    /// `packets/src/agent/inventory.rs` (`0x0C`/`0x0B` personal, `0x20`/`0x21`
    /// guild from the gold arm of the original's builder).
    fn encoded(target: GoldTarget, button: GoldModalButton, typed: u64) -> Vec<u8> {
        match gold_press(target, button, typed, 5_000, 9_000, true) {
            GoldPress::Send(request) => Bytes::from(request).to_vec(),
            other => panic!("expected one request, got {other:?}"),
        }
    }

    /// Deposit — personal op 12, guild op 32, both a bare little-endian u64.
    #[test]
    fn a_deposit_press_is_exactly_one_gold_op_with_the_typed_amount() {
        assert_eq!(
            encoded(GoldTarget::PersonalStorage, GoldModalButton::Deposit, 1_000),
            vec![0x0C, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            encoded(GoldTarget::GuildStorage, GoldModalButton::Deposit, 1_000),
            vec![0x20, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]
        );
    }

    /// Withdraw — personal op 11, guild op 33.
    #[test]
    fn a_withdraw_press_is_exactly_one_gold_op_with_the_typed_amount() {
        assert_eq!(
            encoded(
                GoldTarget::PersonalStorage,
                GoldModalButton::Withdraw,
                1_000
            ),
            vec![0x0B, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            encoded(GoldTarget::GuildStorage, GoldModalButton::Withdraw, 1_000),
            vec![0x21, 0xE8, 0x03, 0, 0, 0, 0, 0, 0]
        );
    }

    /// The amount is clamped to the side it comes from — a typed 999999 with
    /// 5000 in the purse deposits 5000, and withdrawing more than the
    /// warehouse holds takes what it holds.
    #[test]
    fn the_amount_is_clamped_to_the_side_it_comes_from() {
        for target in [GoldTarget::PersonalStorage, GoldTarget::GuildStorage] {
            let deposit = encoded(target, GoldModalButton::Deposit, 999_999);
            assert_eq!(
                u64::from_le_bytes(deposit[1..9].try_into().expect("8 amount bytes")),
                5_000,
                "deposit clamped to the player's purse ({target:?})"
            );
            let withdraw = encoded(target, GoldModalButton::Withdraw, 999_999);
            assert_eq!(
                u64::from_le_bytes(withdraw[1..9].try_into().expect("8 amount bytes")),
                9_000,
                "withdraw clamped to the warehouse total ({target:?})"
            );
        }
    }

    /// Without an open session NOTHING goes out. The popup outlives its
    /// window by a frame (it closes in `close_modal_with_session`), and a
    /// gold op booked against a closed session is the one request that cannot
    /// be undone by an ack.
    #[test]
    fn no_session_sends_nothing() {
        for target in [GoldTarget::PersonalStorage, GoldTarget::GuildStorage] {
            for button in [GoldModalButton::Deposit, GoldModalButton::Withdraw] {
                assert_eq!(
                    gold_press(target, button, 1_000, 5_000, 9_000, false),
                    GoldPress::Close,
                    "{target:?}/{button:?} sent without a session"
                );
            }
        }
    }

    /// A zero amount is mid-typing, not a request: nothing is sent and the
    /// popup stays open. Cancel closes it, on both targets.
    #[test]
    fn zero_stays_open_and_cancel_closes() {
        for target in [GoldTarget::PersonalStorage, GoldTarget::GuildStorage] {
            assert_eq!(
                gold_press(target, GoldModalButton::Deposit, 0, 5_000, 9_000, true),
                GoldPress::Nothing
            );
            assert_eq!(
                gold_press(target, GoldModalButton::Cancel, 1_000, 5_000, 9_000, true),
                GoldPress::Close
            );
        }
    }

    /// An empty purse cannot deposit and an empty warehouse cannot pay out —
    /// the clamp turns both into "nothing", not into a zero-amount packet the
    /// server has to refuse.
    #[test]
    fn an_empty_side_sends_nothing() {
        assert_eq!(
            gold_press(
                GoldTarget::GuildStorage,
                GoldModalButton::Deposit,
                1_000,
                0,
                9_000,
                true
            ),
            GoldPress::Nothing
        );
        assert_eq!(
            gold_press(
                GoldTarget::GuildStorage,
                GoldModalButton::Withdraw,
                1_000,
                5_000,
                0,
                true
            ),
            GoldPress::Nothing
        );
    }
}

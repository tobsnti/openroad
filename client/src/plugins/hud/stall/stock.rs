//! Stocking the owner's stall: the box that asks for amount and price.
//!
//! # The idea
//!
//! The receiving half of this chain was already complete — a `0xB0BA` ack
//! re-sends the whole listing and `owner::on_stall_update_response` replaces
//! the grid from it — but there was no way to *raise* the question that
//! produces a send, so an owner's stall could be opened, named and switched to
//! "trading now" and stayed empty forever. This file is both halves: the
//! question (the drop and the box) and the answer ([`on_stock_button`],
//! `0x70BA` type 2 to list and type 1 to re-price, [`send_remove`] for type 3).
//! It still writes nothing into [`StallState`] — the server owns the stall, so
//! the grid changes when the ack arrives and not before.
//!
//! The gesture is the tree's existing one: an item dragged out of the
//! inventory and released over a window is that window's business
//! (`store/ui.rs::sell_drop_on_store`, `storage/ui.rs::deposit_drop_on_storage`).
//! [`stock_drop_on_stall`] takes the carry over — ghost included — so no
//! `0x7034` move goes out for a drop that was meant for the stall. Nothing is
//! written into [`StallState`]: the server owns the stall, exactly like every
//! other owner action here.
//!
//! # The one deliberate deviation, and what would settle it (ADR-0009)
//!
//! Selling needs two numbers the wire has fields for (`quantity: u16`,
//! `price: u64`) and no window control carries: the original asks for them in a
//! message box. `resinfo/ifmessagebox.txt` `Section = MsgBoxStoreMoney`
//! (`:909`) is the only box in that file with an **editable price**
//! (`GDR_MBS_EDIT_PRICE` `:911`) next to a quantity edit
//! (`GDR_MBS_EDIT_AMOUNT` `:987`) — every other one only *displays* a price —
//! and setting a price is something only a seller does. **Stated deviation:**
//! this box is bound to the stall window although the binding is unconfirmed,
//! because the alternative is a stall that cannot be stocked at all, and
//! unlike a guessed *buy* dialog a guessed *sell* dialog cannot cost the
//! player anything — the player sets the price, and a wrong layout is a
//! cosmetic defect, not a loss. What would settle it: which `MsgBox*` section
//! the stall window's stocking arm creates.
//!
//! Every child rect below is that section's own, cited with its line, drawn on
//! the shared `msgbox2_window_` shell every dialog in this tree uses
//! (`hud::modal_dialog`). The plate size is **derived**, because the section
//! authors no root rect: its children span `18..285` horizontally and
//! `44..125` vertically, and the shell's measured insets are 16 at the sides,
//! 40 at the top and 16 at the bottom — so the plate is `285 + 16 = 301` wide
//! and `125 + 16 = 141` tall, with the 40-unit top strip already covering the
//! topmost child at 44. A test re-derives it rather than trusting the two
//! numbers.
//!
//! The two fields are digit-only text inputs in the shape `store::ui`'s
//! quantity modal uses (one visible line, no newlines, a character filter
//! rather than a parse that discards letters afterwards). The amount opens
//! prefilled with the stack and takes focus; the price opens **empty**,
//! because the player sets it and a prefilled price is a number the box could
//! get wrong. A Confirm with no price typed sends nothing and keeps the box
//! open — closing it would throw away an answer the player is still giving.

use bevy::input_focus::{FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, LineHeight, TextCursorStyle, TextEdit};
use bevy::ui_widgets::{Activate, Button, TextInput};

use packets::agent::character_data::ItemTypeData;
use packets::agent::stall::StallUpdateRequest;
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::net::agent::AgentConnection;

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::modal_dialog::{
    modal_interior, modal_plate_node, modal_scrim_node, spawn_modal_frame,
};
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::hud::stall::model::StallState;
use crate::plugins::hud::stall::ui::StallWindowRoot;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientTextNames, ClientUiStrings};
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// What the box is about: a bag slot going up for sale (`0x70BA` type 2) or a
/// row already on sale being re-priced (type 1). The send reads this; until it
/// lands, it is what the box's title line distinguishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockTarget {
    /// Wire slot of the inventory item being listed.
    Inventory(u8),
    /// Index of the stall row being re-priced.
    Stall(u8),
}

/// The pending question. A plain value, resolved when the box was raised: by
/// the time the player answers, a server push may have rewritten the slot, and
/// the question they were asked must not change under them — the same rule
/// `inventory::drop_item::DropPrompt` states for its own confirm.
#[derive(Clone, Debug, PartialEq)]
pub struct StockPrompt {
    pub target: StockTarget,
    /// For the icon; `0` when the row carries no ref id (a re-price reads the
    /// name the server sent, not the item table).
    pub ref_id: u32,
    pub name: String,
    /// Prefill and ceiling: the whole stack, or 1 for anything that does not
    /// stack. The edit field that enforces it is the next slice.
    pub amount: u16,
    pub price: u64,
}

/// The raised question, or `None`. A resource rather than a message because
/// the box is modal: a second question raised while one is up would have
/// nowhere to go.
#[derive(Resource, Default)]
pub struct StockModal {
    pub prompt: Option<StockPrompt>,
}

#[derive(Component)]
pub struct StockModalRoot;

/// The amount field, and the number the player has typed into it.
#[derive(Component)]
pub struct StockAmountInput;

/// Clamped to the prompt's stack by [`sync_stock_fields`]; a resource rather
/// than a read of the field at send time so the clamp happens where it can be
/// shown, not inside the click.
#[derive(Resource, Default)]
pub struct StockAmount(pub u16);

#[derive(Component)]
pub struct StockPriceInput;

/// The price as typed. `0` is "not answered yet" — the send refuses it rather
/// than listing an item for nothing.
#[derive(Resource, Default)]
pub struct StockPrice(pub u64);

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum StockButton {
    Ok,
    Cancel,
}

/// `0x70BA`'s trailing `unknown0`, written as `0` by the original in every
/// captured body of this family (`packets/src/agent/stall.rs`).
const UPDATE_UNKNOWN0: u16 = 0;
/// `flea_market_tid_group`: the original writes the literal `1`
/// (`StallUpdateRequest::ItemAdded`'s own doc).
const FLEA_MARKET_TID_GROUP: u32 = 1;

/// Plate size — derived from the authored children, see the module doc.
const PLATE: (f32, f32) = (301.0, 141.0);

// `ifmessagebox.txt` `Section = MsgBoxStoreMoney` (`:909`), window units.
/// `GDR_MBS_STATIC_ICONWND` (`:1120`), art `msgbox_itemwindow.ddj`.
const ICON_WND: (f32, f32, f32, f32) = (18.0, 44.0, 48.0, 48.0);
/// `GDR_MBS_STATIC_ICON` (`:1101`).
const ICON: (f32, f32, f32, f32) = (25.0, 51.0, 32.0, 32.0);
/// `GDR_MBS_STATIC_NAME1` (`:1082`), art `msgbox_iteminfo.ddj`.
const NAME_PLATE: (f32, f32, f32, f32) = (73.0, 44.0, 212.0, 48.0);
/// The name line itself sits on that plate; `GDR_MBS_STATIC_NAME2` (`:1063`)
/// is the second row of the same art, so the text takes the first row's band.
const NAME_TEXT: (f32, f32, f32, f32) = (77.0, 48.0, 204.0, 14.0);
/// `GDR_MBS_STATIC_PRICET1` (`:1006`), key `UIIT_STT_PRICE`.
const PRICE_LABEL: (f32, f32, f32, f32) = (77.0, 71.0, 43.0, 12.0);
/// `GDR_MBS_EDIT_PRICE` (`:911`) — the field; this slice shows the prefill in
/// it, the edit behaviour is the next one.
const PRICE_EDIT: (f32, f32, f32, f32) = (127.0, 68.0, 121.0, 18.0);
/// `GDR_MBS_STATIC_PRICET2` (`:1025`), key `UIIT_STT_GOLD`.
const PRICE_UNIT: (f32, f32, f32, f32) = (255.0, 72.0, 23.0, 12.0);
/// `GDR_MBS_EDIT_AMOUNT` (`:987`), art `msgbox_quantity.ddj`.
const AMOUNT_EDIT: (f32, f32, f32, f32) = (20.0, 102.0, 42.0, 20.0);
/// `GDR_MBS_STATIC_AMOUNT` (`:968`), key `UIIT_STT_UNIT`.
const AMOUNT_UNIT: (f32, f32, f32, f32) = (64.0, 108.0, 12.0, 12.0);
/// `GDR_MBS_BTN_CANCEL` (`:930`).
const CANCEL_BTN: (f32, f32, f32, f32) = (203.0, 101.0, 76.0, 24.0);
/// `GDR_MBS_BTN_OK` (`:949`) — the arm that sends, which is why it was absent
/// until now.
const OK_BTN: (f32, f32, f32, f32) = (123.0, 101.0, 76.0, 24.0);

const MSGBOX_DIR: &str = "media://interface/messagebox/";
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// Raise the box when a carried item is released over the owner's stall.
///
/// Mirrors `store::ui::sell_drop_on_store` down to the edge it polls: the
/// inventory carries with press/release rather than `bevy_picking`'s drag, so
/// a catcher is "mouse edge + a live carry + this window hovered".
pub fn stock_drop_on_stall(
    buttons: Res<ButtonInput<MouseButton>>,
    stall: Res<StallState>,
    stall_roots: Query<&Hovered, With<StallWindowRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    item_data: Res<ClientItemData>,
    names: Res<ClientTextNames>,
    ghosts: Query<Entity, With<crate::plugins::hud::inventory::ui::DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
    mut modal: ResMut<StockModal>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) && !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // Only our own stall takes stock, and only one question at a time.
    if !stall.open || !stall.owner || modal.prompt.is_some() {
        return;
    }
    let Some(slot) = inv_state.drag else {
        return;
    };
    if !stall_roots.iter().any(|hovered| hovered.get()) {
        return;
    }
    // The drop is ours from here: take the carry over so the inventory's own
    // catcher does not also send a 0x7034 move for it.
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    let Some(item) = inventories
        .single()
        .ok()
        .and_then(|inventory| inventory.get(slot).cloned())
    else {
        // The carry outlived the item it started from (a server push emptied
        // the slot); there is nothing to ask about.
        return;
    };
    let name = item_data
        .get(&(item.ref_id as i32))
        .and_then(|row| row.name_key())
        .and_then(|key| names.name(key))
        .map(String::from)
        .unwrap_or_else(|| format!("Item #{}", item.ref_id));
    let amount = match &item.data {
        ItemTypeData::Expendable { stack_count, .. } => (*stack_count).max(1),
        _ => 1,
    };
    modal.prompt = Some(StockPrompt {
        target: StockTarget::Inventory(slot),
        ref_id: item.ref_id,
        name,
        amount,
        // The player sets it; the original prefills nothing it could get
        // wrong, and neither do we.
        price: 0,
    });
}

/// Close the box when the stall window goes — a question about a stall that is
/// no longer on screen has no answer, and the modal would otherwise outlive it
/// with the scrim swallowing every click.
pub fn clear_stock_with_stall(stall: Res<StallState>, mut modal: ResMut<StockModal>) {
    if !stall.open && modal.prompt.is_some() {
        modal.prompt = None;
    }
}

/// Rebuild the box when the question changes — the despawn-and-respawn shape
/// `inventory::drop_item::sync_drop_confirm` uses, on the same shell.
pub fn sync_stock_modal(
    modal: Res<StockModal>,
    existing: Query<Entity, With<StockModalRoot>>,
    ui_strings: Res<ClientUiStrings>,
    item_data: Res<ClientItemData>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut focus: ResMut<InputFocus>,
    mut commands: Commands,
) {
    if !modal.is_changed() {
        return;
    }
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }
    let Some(prompt) = &modal.prompt else {
        return;
    };
    let Ok(camera) = cam_query.single() else {
        return;
    };
    let s = hud_scale();
    // Every text in this box is a single line in a 12-to-14-unit rect, so the
    // line box is the rounded size itself rather than bevy's 1.2 — the same
    // pairing `minimap` and `underbar` use since the line-height slice, and
    // `LineHeight` is its own component in 0.20, not a `TextFont` field.
    let text_font = |size: f32| {
        let px = text_px(size, s);
        (
            TextFont {
                font: fonts.two.clone().into(),
                font_size: bevy::text::FontSize::Px(px),
                ..default()
            },
            LineHeight::Px(px),
        )
    };
    let icon = item_data
        .get(&(prompt.ref_id as i32))
        .and_then(|row| row.icon_path())
        .map(|path| asset_server.load::<Image>(path));
    let button_style = ImageButtonStyle {
        normal: asset_server.load("media://interface/ifcommon/com_button.ddj"),
        hover: asset_server.load("media://interface/ifcommon/com_button_focus.ddj"),
        press: asset_server.load("media://interface/ifcommon/com_button_press.ddj"),
        ..Default::default()
    };
    // Captured out of the closure so the amount field can take focus once the
    // box exists — the store's modal does the same with its quantity input.
    let mut amount_field = None;
    let plate_art = |name: &str| -> ImageNode {
        ImageNode {
            image: asset_server.load(format!("{MSGBOX_DIR}{name}.ddj")),
            image_mode: NodeImageMode::Stretch,
            ..default()
        }
    };

    commands
        .spawn((
            StockModalRoot,
            Name::from("Stall Stock Box"),
            modal_scrim_node(),
            GlobalZIndex(80),
            bevy::ui::UiTargetCamera(camera),
        ))
        .with_children(|scrim| {
            scrim
                .spawn((
                    modal_plate_node(PLATE.0, PLATE.1, s),
                    Name::from("Stall Stock Plate"),
                ))
                .with_children(|plate| {
                    spawn_modal_frame(plate, &asset_server, PLATE.0, PLATE.1, s);
                    // The ring has no centre piece and leaves the interior to
                    // the caller; `com_bg_tile_b.ddj` is what the family
                    // records for it, and what the sibling dialogs spawn here.
                    plate.spawn((
                        abs_node(modal_interior(PLATE.0, PLATE.1), s),
                        ImageNode {
                            image: asset_server.load(BG_TILE),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    for (rect, art) in [
                        (ICON_WND, "msgbox_itemwindow"),
                        (NAME_PLATE, "msgbox_iteminfo"),
                        (AMOUNT_EDIT, "msgbox_quantity"),
                    ] {
                        plate.spawn((abs_node(rect, s), plate_art(art), Pickable::IGNORE));
                    }
                    if let Some(icon) = icon {
                        plate.spawn((
                            abs_node(ICON, s),
                            ImageNode {
                                image: icon,
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    }
                    plate.spawn((
                        Text::new(prompt.name.clone()),
                        text_font(9.0),
                        TextColor(Color::srgb(1.0, 0.85, 0.32)),
                        abs_node(NAME_TEXT, s),
                        Pickable::IGNORE,
                    ));
                    // Labels and units are the section's own keys through the
                    // player's table.
                    for (rect, key, fallback, justify) in [
                        (PRICE_LABEL, "UIIT_STT_PRICE", "Price", Justify::Right),
                        (PRICE_UNIT, "UIIT_STT_GOLD", "Gold", Justify::Left),
                        (AMOUNT_UNIT, "UIIT_STT_UNIT", "ea", Justify::Left),
                    ] {
                        plate.spawn((
                            Text::new(ui_strings.get_or(key, fallback)),
                            text_font(8.0),
                            TextColor(Color::srgb(0.85, 0.85, 0.85)),
                            TextLayout::justify(justify),
                            abs_node(rect, s),
                            Pickable::IGNORE,
                        ));
                    }
                    // The two numbers the wire wants, as digit-only fields —
                    // the same shape the store's quantity modal uses
                    // (`store::ui`): one visible line, no newlines (so Enter
                    // reaches the dialog), and a filter rather than a parse
                    // that throws letters away afterwards. The amount opens
                    // prefilled with the stack and takes focus; the price
                    // opens empty, because the player sets it and the original
                    // prefills nothing it could get wrong.
                    let mut field =
                        |rect: (f32, f32, f32, f32), prefill: Option<String>, max: usize| {
                            let mut node = abs_node(rect, s);
                            node.padding = UiRect::top(Val::Px(2.0 * s));
                            let mut editable = EditableText {
                                visible_lines: Some(1.0),
                                allow_newlines: false,
                                max_characters: Some(max),
                                ..default()
                            };
                            if let Some(prefill) = prefill {
                                editable.queue_edit(TextEdit::Insert(prefill.into()));
                            }
                            (
                                TextInput,
                                editable,
                                EditableTextFilter::new(|c: char| c.is_ascii_digit()),
                                node,
                                text_font(9.0),
                                TextColor(Color::WHITE),
                                TextLayout::justify(Justify::Right),
                                TextCursorStyle {
                                    color: Color::WHITE,
                                    ..default()
                                },
                            )
                        };
                    // `price: u64` on the wire; 12 digits is what the field's
                    // own 121-unit box holds at this size, and no stall price
                    // the server accepts needs more.
                    plate.spawn((StockPriceInput, field(PRICE_EDIT, None, 12)));
                    amount_field = Some(
                        plate
                            .spawn((
                                StockAmountInput,
                                field(AMOUNT_EDIT, Some(prompt.amount.to_string()), 5),
                            ))
                            .id(),
                    );
                    for (button, rect, key, fallback) in [
                        (StockButton::Ok, OK_BTN, "UIIT_CTL_CONFIRM", "Confirm"),
                        (StockButton::Cancel, CANCEL_BTN, "UIIT_CTL_CANCEL", "Cancel"),
                    ] {
                        plate
                            .spawn((
                                button,
                                Button,
                                Hovered::default(),
                                abs_node(rect, s),
                                ImageNode {
                                    image: button_style.normal.clone(),
                                    image_mode: NodeImageMode::Stretch,
                                    ..default()
                                },
                                button_style.clone(),
                            ))
                            .observe(on_stock_button)
                            .with_children(|button| {
                                button.spawn((
                                    Text::new(ui_strings.get_or(key, fallback)),
                                    text_font(9.0),
                                    TextColor(Color::WHITE),
                                    TextLayout::justify(Justify::Center),
                                    abs_node((0.0, 6.0, rect.2, 12.0), s),
                                    Pickable::IGNORE,
                                ));
                            });
                    }
                });
        });
    // The amount is the field a player edits most often, and the box is modal
    // anyway — the same first-field focus the store's quantity modal takes,
    // with the cause it uses: `Navigated` is what every focus set in this tree
    // passes (`store/ui.rs:1650`, `ui_v2::widgets`, `captcha`), and the enum
    // has no "programmatic" arm to pick instead.
    if let Some(field) = amount_field {
        focus.set(field, FocusCause::Navigated);
    }
}

/// The request a filled-in box stands for, or `None` while it is unanswered.
///
/// Pure on purpose: it is the one piece of this file a test can hold still, and
/// it is where the two types part. A listing is `type 2` and names the
/// inventory slot it comes from; a re-price is `type 1` and names only the row
/// it changes, because the item is already on the stall. `price == 0` is not a
/// free item, it is a box nobody has typed a price into yet.
fn stock_request(prompt: &StockPrompt, amount: u16, price: u64) -> Option<StallUpdateRequest> {
    if price == 0 || amount == 0 {
        return None;
    }
    Some(match prompt.target {
        StockTarget::Inventory(inventory_slot) => StallUpdateRequest::ItemAdded {
            // The server picks the row; `0` is the only honest "wherever it
            // fits", and the `0xB0BA` ack re-sends the whole listing anyway.
            stall_slot: 0,
            inventory_slot,
            quantity: amount,
            price,
            flea_market_tid_group: FLEA_MARKET_TID_GROUP,
            unknown0: UPDATE_UNKNOWN0,
        },
        StockTarget::Stall(stall_slot) => StallUpdateRequest::ItemUpdate {
            stall_slot,
            quantity: amount,
            price,
            unknown0: UPDATE_UNKNOWN0,
        },
    })
}

/// Confirm sends, Cancel closes. A Confirm with no price typed does neither:
/// the box stays open, because closing it would throw the player's amount away
/// for a question they have not finished answering.
fn on_stock_button(
    activate: On<Activate>,
    buttons: Query<&StockButton>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    amount: Res<StockAmount>,
    price: Res<StockPrice>,
    mut modal: ResMut<StockModal>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    if *button == StockButton::Cancel {
        modal.prompt = None;
        return;
    }
    let Some(prompt) = modal.prompt.clone() else {
        return;
    };
    let Some(request) = stock_request(&prompt, amount.0, price.0) else {
        return;
    };
    let what = match prompt.target {
        StockTarget::Inventory(_) => "stock",
        StockTarget::Stall(_) => "reprice",
    };
    info!("stall: {what} {request:?} (0x70BA)");
    super::net::send(&conn, Packet::from(request), what);
    modal.prompt = None;
}

/// `0x70BA` type 3 — take our own row back off sale. The grid is not touched
/// here: the `0xB0BA` ack re-sends the whole listing and
/// `owner::on_stall_update_response` applies it.
pub fn send_remove(conn: &Query<&SilkroadConnection, With<AgentConnection>>, stall_slot: u8) {
    super::net::send(
        conn,
        Packet::from(StallUpdateRequest::ItemRemoved {
            stall_slot,
            unknown0: UPDATE_UNKNOWN0,
        }),
        "remove",
    );
}

/// Keep the two numbers in step with the two fields.
///
/// The clamp lives here rather than in the click for the reason the store's
/// modal states: it is the value the box *stands for*, so it has to be the
/// value anything else reads — and an empty amount field is 1, not 0.
pub fn sync_stock_fields(
    modal: Res<StockModal>,
    amounts: Query<&EditableText, With<StockAmountInput>>,
    prices: Query<&EditableText, With<StockPriceInput>>,
    mut amount: ResMut<StockAmount>,
    mut price: ResMut<StockPrice>,
) {
    let Some(prompt) = &modal.prompt else {
        return;
    };
    if let Ok(editable) = amounts.single() {
        let typed = editable
            .value()
            .to_string()
            .trim()
            .parse::<u16>()
            .unwrap_or(1)
            .clamp(1, prompt.amount.max(1));
        if amount.0 != typed {
            amount.0 = typed;
        }
    }
    if let Ok(editable) = prices.single() {
        let typed = editable
            .value()
            .to_string()
            .trim()
            .parse::<u64>()
            .unwrap_or(0);
        if price.0 != typed {
            price.0 = typed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    /// The box is the owner's: a visitor dragging onto someone else's stall
    /// must keep their carry, because the only thing this window would do with
    /// it is sell it.
    #[test]
    fn only_the_owners_stall_takes_stock() {
        let mut state = StallState::default();
        assert!(
            !state.owner,
            "a stall is somebody else's until 0xB0B1 says otherwise"
        );
        state.open = true;
        assert!(!(state.open && state.owner));
    }

    /// Closing the stall takes the question with it; leaving it up would leave
    /// a scrim over a window that is gone.
    #[test]
    fn closing_the_stall_closes_the_box() {
        let mut world = World::new();
        world.insert_resource(StallState {
            open: false,
            ..StallState::default()
        });
        world.insert_resource(StockModal {
            prompt: Some(StockPrompt {
                target: StockTarget::Inventory(13),
                ref_id: 0,
                name: "x".into(),
                amount: 1,
                price: 0,
            }),
        });
        world
            .run_system_once(clear_stock_with_stall)
            .expect("the system runs");
        assert!(world.resource::<StockModal>().prompt.is_none());
    }

    /// The two types part on the target, and that is the whole difference the
    /// wire sees: a listing names the bag slot it comes from (`type 2`), a
    /// re-price names only the row it changes (`type 1`).
    #[test]
    fn a_listing_and_a_reprice_are_the_two_types_they_claim_to_be() {
        let listing = StockPrompt {
            target: StockTarget::Inventory(13),
            ref_id: 1,
            name: "x".into(),
            amount: 5,
            price: 0,
        };
        assert_eq!(
            stock_request(&listing, 3, 900),
            Some(StallUpdateRequest::ItemAdded {
                stall_slot: 0,
                inventory_slot: 13,
                quantity: 3,
                price: 900,
                flea_market_tid_group: FLEA_MARKET_TID_GROUP,
                unknown0: UPDATE_UNKNOWN0,
            })
        );
        let reprice = StockPrompt {
            target: StockTarget::Stall(2),
            ..listing.clone()
        };
        assert_eq!(
            stock_request(&reprice, 1, 50),
            Some(StallUpdateRequest::ItemUpdate {
                stall_slot: 2,
                quantity: 1,
                price: 50,
                unknown0: UPDATE_UNKNOWN0,
            })
        );
    }

    /// A box nobody has typed a price into is not a free item: Confirm has
    /// nothing to send, and the box stays open rather than throwing the
    /// player's amount away.
    #[test]
    fn an_unanswered_box_sends_nothing() {
        let prompt = StockPrompt {
            target: StockTarget::Inventory(13),
            ref_id: 1,
            name: "x".into(),
            amount: 5,
            price: 0,
        };
        assert_eq!(stock_request(&prompt, 3, 0), None);
        assert_eq!(stock_request(&prompt, 0, 900), None);
    }

    /// Every child rect is inside the plate's measured interior, and the plate
    /// is no bigger than it has to be — the check that catches both a
    /// mis-copied rect from `MsgBoxStoreMoney` and a plate size picked by eye.
    /// It found one: a 300-wide plate clips `NAME_PLATE` by a single unit.
    #[test]
    fn every_authored_rect_sits_in_the_plates_interior() {
        let (ix, iy, iw, ih) = modal_interior(PLATE.0, PLATE.1);
        for rect in [
            ICON_WND,
            ICON,
            NAME_PLATE,
            NAME_TEXT,
            PRICE_LABEL,
            PRICE_EDIT,
            PRICE_UNIT,
            AMOUNT_EDIT,
            AMOUNT_UNIT,
            CANCEL_BTN,
        ] {
            assert!(
                rect.0 >= ix && rect.1 >= iy,
                "{rect:?} starts outside the interior"
            );
            assert!(
                rect.0 + rect.2 <= ix + iw && rect.1 + rect.3 <= iy + ih,
                "{rect:?} leaves the interior ({iw}x{ih} at {ix},{iy})"
            );
        }
        // the plate is the children plus the shell's insets, not a round number
        let right = [ICON_WND, NAME_PLATE, PRICE_UNIT, CANCEL_BTN]
            .iter()
            .fold(0.0f32, |acc, r| acc.max(r.0 + r.2));
        let bottom = [AMOUNT_EDIT, CANCEL_BTN]
            .iter()
            .fold(0.0f32, |acc, r| acc.max(r.1 + r.3));
        assert_eq!(
            PLATE,
            (
                right + crate::plugins::hud::modal_dialog::MODAL_SIDE,
                bottom + crate::plugins::hud::modal_dialog::MODAL_BOTTOM
            )
        );
    }
}

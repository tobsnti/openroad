//! Stocking the owner's stall: putting an item up for sale (`0x70BA` type 2),
//! re-pricing a row that is already on sale (`type 1`) and taking one back off
//! (`type 3`).
//!
//! # The idea
//!
//! The receiving half of this chain has been complete since #781 — a `0xB0BA`
//! ack for type 2/3 re-sends the **whole** listing and `owner::on_stall_update_response`
//! replaces the grid from it. What was missing was any way to *send* those two
//! types, so the owner's stall could be opened, named and switched to "trading
//! now", and stayed empty forever: `net::on_stall_slot_press` answered the
//! owner's own click with "not wired yet". This file is that missing half and
//! nothing else — it writes no row into [`StallState`], exactly like every
//! other owner action here (the server owns the stall).
//!
//! The drop gesture is the tree's existing one: an item dragged out of the
//! inventory and released over a window is that window's business
//! (`storage/ui.rs::deposit_drop_on_storage`, `store/ui.rs::sell_drop_on_store`).
//! We take the carry over — ghost included — so no `0x7034` op-0 move goes out
//! for a drop that was meant for the stall.
//!
//! # The one deliberate deviation, and what would settle it (ADR-0009)
//!
//! Selling needs two numbers the wire has fields for (`quantity: u16`,
//! `price: u64`) and no window control carries: the original asks for them in a
//! message box. `resinfo/ifmessagebox.txt` `Section = MsgBoxStoreMoney` (:909)
//! is the only message box in the whole file with an **editable price**
//! (`GDR_MBS_EDIT_PRICE` `127,68,121,18`) next to a quantity edit
//! (`GDR_MBS_EDIT_AMOUNT` `20,102,42,20`) — every other one only *displays* a
//! price — and setting a price is something only a seller does. **Stated
//! deviation:** that box is bound to this window although the binding is
//! unconfirmed, because the alternative is a stall that cannot be stocked at
//! all, and unlike a guessed *buy* dialog a guessed *sell* dialog cannot cost
//! the player anything — the player sets the price, and a wrong layout is a
//! cosmetic defect, not a loss. What would settle it is which `MsgBox*`
//! section id the stall window's stocking arm creates.
//!
//! Every rect below is the vanilla rect from that section minus the shared
//! msgbox origin, through `store::ui::msgbox_rect` — the same helper and the
//! same background plate the store's own quantity box uses, so this is the
//! second user of that geometry, not a second implementation of it.

use bevy::input_focus::{FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle};
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::character_data::ItemTypeData;
use packets::agent::stall::StallUpdateRequest;
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::stall::model::StallState;
use crate::plugins::hud::stall::ui::{StallSlot, StallWindowRoot};
use crate::plugins::hud::store::ui::{
    msgbox_rect, MODAL_ITEMWINDOW_DDJ, MODAL_QUANTITY_DDJ, MSGBOX_BG, MSGBOX_TILE,
};
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientTextNames, ClientUiStrings};
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// `GDR_MBS_STATIC_ICONWND` `18,44,48,48` / `GDR_MBS_STATIC_ICON` `25,51,32,32`.
const STOCK_ICON_WND: (f32, f32, f32, f32) = msgbox_rect((18.0, 44.0, 48.0, 48.0));
const STOCK_ICON: (f32, f32, f32, f32) = msgbox_rect((25.0, 51.0, 32.0, 32.0));
/// `GDR_MBS_STATIC_NAME1` `73,44,212,48` — the name board; the name text sits
/// on its first line like the store's box does.
const STOCK_NAME_TEXT: (f32, f32, f32, f32) = msgbox_rect((73.0, 51.0, 212.0, 12.0));
/// `GDR_MBS_STATIC_PRICET1` `77,71,43,12` (`UIIT_STT_PRICE`),
/// `GDR_MBS_EDIT_PRICE` `127,68,121,18`, `GDR_MBS_STATIC_PRICET2`
/// `255,72,23,12` (`UIIT_STT_GOLD`).
const STOCK_PRICE_LABEL: (f32, f32, f32, f32) = msgbox_rect((77.0, 71.0, 43.0, 12.0));
const STOCK_PRICE_EDIT: (f32, f32, f32, f32) = msgbox_rect((127.0, 68.0, 121.0, 18.0));
const STOCK_PRICE_UNIT: (f32, f32, f32, f32) = msgbox_rect((255.0, 72.0, 23.0, 12.0));
/// `GDR_MBS_EDIT_AMOUNT` `20,102,42,20` on `msgbox_quantity.ddj` and the
/// `UIIT_STT_UNIT` static `64,108,12,12` right of it.
const STOCK_AMOUNT_EDIT: (f32, f32, f32, f32) = msgbox_rect((20.0, 102.0, 42.0, 20.0));
const STOCK_AMOUNT_TEXT: (f32, f32, f32, f32) = msgbox_rect((27.0, 105.0, 28.0, 14.0));
const STOCK_AMOUNT_UNIT: (f32, f32, f32, f32) = msgbox_rect((64.0, 108.0, 12.0, 12.0));
/// `GDR_MBS_BTN_OK` `123,101,76,24` / `GDR_MBS_BTN_CANCEL` `203,101,76,24`.
const STOCK_OK_X: f32 = msgbox_rect((123.0, 0.0, 0.0, 0.0)).0;
const STOCK_CANCEL_X: f32 = msgbox_rect((203.0, 0.0, 0.0, 0.0)).0;
const STOCK_BUTTON_Y: f32 = msgbox_rect((0.0, 101.0, 0.0, 0.0)).1;

/// `flea_market_tid_group` is "written as a literal `1` by the original"
/// (`packets/src/agent/stall.rs:115`), and the trailing `unknown0` is the
/// field no source names — it is sent as 0, the value every transcribed
/// `0x70BA` frame carries.
const FLEA_MARKET_TID_GROUP: u32 = 1;
const UPDATE_UNKNOWN0: u16 = 0;

/// Which of the owner's two price-setting edits the box is about.
///
/// One box, two requests — not two dialogs (Regel 3): `MsgBoxStoreMoney` asks
/// for a price and a quantity, and that is exactly what `0x70BA` type 2
/// (list from the inventory) and type 1 (re-price a listed row) both need. The
/// difference is a wire fact, not a layout one: type 2 carries the
/// `inventory_slot` the item comes from, type 1 does not carry one at all
/// (`packets/src/agent/stall.rs::StallUpdateRequest`), which is why this is an
/// enum and not a bool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockTarget {
    /// A new listing: the item still sits in `inventory_slot` (type 2).
    List { inventory_slot: u8 },
    /// A row that is already on sale: re-price it in place (type 1).
    Reprice,
}

/// What the price box is asking about: one item, headed for one stall slot.
#[derive(Clone, Debug, PartialEq)]
pub struct StockPrompt {
    pub target: StockTarget,
    pub stall_slot: u8,
    pub ref_id: u32,
    pub name: String,
    /// The stack we may list at most — a partial listing is what the wire's
    /// `quantity` field is for.
    pub max: u16,
    /// What the quantity field starts out showing: 1 for a new listing, the
    /// row's own quantity for a re-price. It is prefilled rather than left
    /// empty because type 1 re-states the quantity as well — an empty field
    /// that parsed to 1 would silently shrink the listed stack to one.
    pub quantity: u16,
    /// What the price field starts out showing (0 = empty, i.e. a new
    /// listing). A re-price opens on the price the listing currently has, so
    /// the player edits a number instead of retyping it from the grid.
    pub price: u64,
}

/// The price/quantity box. `None` = closed; the whole popup rebuilds when this
/// changes, the `QuantityModal` pattern.
#[derive(Resource, Default)]
pub struct StockModal {
    pub prompt: Option<StockPrompt>,
}

/// The typed quantity, parsed from the input every frame (empty = 1).
#[derive(Resource)]
pub struct StockAmount(pub u16);

impl Default for StockAmount {
    fn default() -> Self {
        Self(1)
    }
}

/// The typed unit price, parsed from the input every frame (empty = 0).
#[derive(Resource, Default)]
pub struct StockPrice(pub u64);

#[derive(Component)]
pub struct StockModalRoot;

#[derive(Component)]
pub struct StockAmountInput;

#[derive(Component)]
pub struct StockPriceInput;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum StockButton {
    Ok,
    Cancel,
}

/// The request a confirmed prompt maps to, or `None` when it maps to nothing.
///
/// Split out of the observer so the rules are testable without a window, a
/// connection or a picking backend (`storage/gold_modal.rs::gold_press`):
/// * the quantity is clamped into `1..=max` — a typed 9999 lists the stack we
///   actually hold instead of being refused server-side,
/// * a price of 0 sends nothing and leaves the box open (the player is
///   mid-typing); the original has no "give it away" listing.
pub(crate) fn stock_request(
    prompt: &StockPrompt,
    typed_quantity: u16,
    typed_price: u64,
) -> Option<StallUpdateRequest> {
    if typed_price == 0 {
        return None;
    }
    let quantity = typed_quantity.clamp(1, prompt.max.max(1));
    Some(match prompt.target {
        StockTarget::List { inventory_slot } => StallUpdateRequest::ItemAdded {
            stall_slot: prompt.stall_slot,
            inventory_slot,
            quantity,
            price: typed_price,
            flea_market_tid_group: FLEA_MARKET_TID_GROUP,
            unknown0: UPDATE_UNKNOWN0,
        },
        // Type 1 addresses the row by its stall slot only — there is no
        // inventory slot on the wire, because the item is already in the stall.
        StockTarget::Reprice => StallUpdateRequest::ItemUpdate {
            stall_slot: prompt.stall_slot,
            quantity,
            price: typed_price,
            unknown0: UPDATE_UNKNOWN0,
        },
    })
}

/// The prompt for re-pricing the row in `stall_slot` (`0x70BA` type 1), or
/// `None` if that cell holds no row.
///
/// The effect side of type 1 has been complete since #781
/// (`owner::apply_update_ack` writes the ack's quantity and price back into the
/// row and refuses to invent a row for an empty slot); this is the send side,
/// and the only thing it needed was the row's `ref_id` for the box's icon
/// (`stall/model.rs::StallRow`).
pub(crate) fn reprice_prompt(
    state: &StallState,
    stall_slot: u8,
    name_of: impl Fn(u32) -> String,
) -> Option<StockPrompt> {
    let row = state.slots.get(stall_slot as usize)?.as_ref()?;
    Some(StockPrompt {
        target: StockTarget::Reprice,
        stall_slot,
        ref_id: row.ref_id,
        // The listing's own name is what the grid shows; itemdata is asked
        // only when the row predates a name lookup (empty).
        name: if row.name.is_empty() {
            name_of(row.ref_id)
        } else {
            row.name.clone()
        },
        // A re-price cannot raise the stack — nothing new leaves the
        // inventory on type 1, so the row's own quantity is the ceiling.
        max: row.quantity.max(1),
        quantity: row.quantity.max(1),
        price: row.price,
    })
}

/// The first free cell of the grid, preferring the one under the cursor.
pub(crate) fn stock_target_slot(state: &StallState, hovered: Option<usize>) -> Option<u8> {
    let free = |index: usize| state.slots.get(index).is_some_and(Option::is_none);
    let index = hovered
        .filter(|&index| free(index))
        .or_else(|| (0..state.slots.len()).find(|&index| free(index)))?;
    u8::try_from(index).ok()
}

/// Dropping a carried inventory item on **our own** stall opens the price box.
///
/// Mirrors `storage/ui.rs::deposit_drop_on_storage` down to the order of the
/// gates: the carry is only consumed once we know this drop is ours, so a drop
/// on a visitor's stall leaves the item on the cursor instead of eating it.
#[allow(clippy::too_many_arguments)]
pub fn stock_drop_on_stall(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<StallState>,
    roots: Query<&Hovered, With<StallWindowRoot>>,
    cells: Query<(&StallSlot, &Hovered)>,
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
    if !state.open || !state.owner {
        return;
    }
    let Some(inventory_slot) = inv_state.drag else {
        return;
    };
    if !roots.iter().any(|hovered| hovered.get()) {
        return;
    }
    let hovered_cell = cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(cell, _)| cell.0);
    let Some(stall_slot) = stock_target_slot(&state, hovered_cell) else {
        info!("stall: no free slot to list into");
        return;
    };
    let Some(item) = inventories
        .single()
        .ok()
        .and_then(|inv| inv.get(inventory_slot).cloned())
    else {
        return;
    };
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    let name = item_data
        .get(&(item.ref_id as i32))
        .and_then(|row| row.name_key())
        .and_then(|key| names.name(key))
        .unwrap_or("?")
        .to_string();
    let max = match &item.data {
        ItemTypeData::Expendable { stack_count, .. } => (*stack_count).max(1),
        _ => 1,
    };
    modal.prompt = Some(StockPrompt {
        target: StockTarget::List { inventory_slot },
        stall_slot,
        ref_id: item.ref_id,
        name,
        max,
        quantity: 1,
        price: 0,
    });
}

/// Rebuild the popup when the prompt changes (spawn on open, despawn on close).
#[allow(clippy::too_many_arguments)]
pub fn sync_stock_modal(
    modal: Res<StockModal>,
    existing: Query<Entity, With<StockModalRoot>>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    item_data: Res<ClientItemData>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut focus: ResMut<InputFocus>,
    mut amount: ResMut<StockAmount>,
    mut price: ResMut<StockPrice>,
    mut commands: Commands,
) {
    if !modal.is_changed() {
        return;
    }
    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }
    let Some(prompt) = &modal.prompt else {
        if !existing.is_empty() {
            focus.clear();
        }
        return;
    };
    let Ok(camera) = cam_query.single() else {
        return;
    };
    // The resources start out at whatever the prompt says the fields show, so
    // a confirm without touching either edit sends the prompt's own numbers
    // (a re-price that only changes the price must not change the quantity).
    amount.0 = prompt.quantity.clamp(1, prompt.max.max(1));
    price.0 = prompt.price;
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
    let icon = item_data
        .get(&(prompt.ref_id as i32))
        .and_then(|row| row.icon_path());
    let mut price_input = None;
    commands
        .spawn((
            StockModalRoot,
            Name::from("Stall Stock Modal"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            // scrim deliberately swallows clicks, like the store's box
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(65),
            UiTargetCamera(camera),
        ))
        .with_children(|scrim| {
            scrim
                .spawn((
                    Node {
                        width: Val::Px(MSGBOX_BG.2 * s),
                        height: Val::Px(MSGBOX_BG.3 * s),
                        ..default()
                    },
                    ImageNode {
                        image: asset_server.load(MSGBOX_TILE),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Outline {
                        width: Val::Px(1.0),
                        color: Color::srgb(0.55, 0.45, 0.25),
                        ..default()
                    },
                ))
                .with_children(|panel| {
                    panel.spawn((
                        abs_node(STOCK_ICON_WND, s),
                        ImageNode {
                            image: asset_server.load(MODAL_ITEMWINDOW_DDJ),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    if let Some(icon) = icon {
                        panel.spawn((
                            abs_node(STOCK_ICON, s),
                            ImageNode {
                                image: asset_server.load(icon),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    }
                    panel.spawn((
                        Text::new(prompt.name.clone()),
                        text_font(9.0),
                        TextColor(Color::srgb_u8(255, 226, 123)),
                        abs_node(STOCK_NAME_TEXT, s),
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        Text::new(ui_strings.get_or("UIIT_STT_PRICE", "Price").to_string()),
                        text_font(8.0),
                        TextColor(Color::WHITE),
                        abs_node(STOCK_PRICE_LABEL, s),
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        Text::new(ui_strings.get_or("UIIT_STT_GOLD", "Gold").to_string()),
                        text_font(8.0),
                        TextColor(Color::srgb_u8(255, 217, 83)),
                        abs_node(STOCK_PRICE_UNIT, s),
                        Pickable::IGNORE,
                    ));
                    // the price edit — `GDR_MBS_EDIT_PRICE` carries DDJ="" in
                    // the data, so it is a bare field with our own thin frame
                    let mut price_box = abs_node(STOCK_PRICE_EDIT, s);
                    price_box.padding = UiRect::all(Val::Px(2.0 * s));
                    // Prefilled for a re-price, empty for a new listing
                    // (`notice_write.rs` uses the same `EditableText::new`).
                    let mut price_edit = EditableText::new(if prompt.price == 0 {
                        String::new()
                    } else {
                        prompt.price.to_string()
                    });
                    price_edit.visible_lines = Some(1.0);
                    price_edit.allow_newlines = false;
                    // 999 999 999 999 gold is past any vSRO purse; 12 digits
                    // keep the u64 safe
                    price_edit.max_characters = Some(12);
                    price_input = Some(
                        panel
                            .spawn((
                                StockPriceInput,
                                price_edit,
                                EditableTextFilter::new(|c: char| c.is_ascii_digit()),
                                price_box,
                                text_font(9.0),
                                TextColor(Color::WHITE),
                                TextCursorStyle {
                                    color: Color::WHITE,
                                    ..default()
                                },
                                Outline {
                                    width: Val::Px(1.0),
                                    color: Color::srgb(0.35, 0.30, 0.18),
                                    ..default()
                                },
                            ))
                            .id(),
                    );
                    panel.spawn((
                        abs_node(STOCK_AMOUNT_EDIT, s),
                        ImageNode {
                            image: asset_server.load(MODAL_QUANTITY_DDJ),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        Text::new(ui_strings.get_or("UIIT_STT_UNIT", "Unit").to_string()),
                        text_font(8.0),
                        TextColor(Color::srgb_u8(255, 226, 123)),
                        abs_node(STOCK_AMOUNT_UNIT, s),
                        Pickable::IGNORE,
                    ));
                    let mut amount_box = abs_node(STOCK_AMOUNT_TEXT, s);
                    amount_box.padding = UiRect::top(Val::Px(2.0 * s));
                    let mut amount_edit = EditableText::new(amount.0.to_string());
                    amount_edit.visible_lines = Some(1.0);
                    amount_edit.allow_newlines = false;
                    amount_edit.max_characters = Some(4);
                    panel.spawn((
                        StockAmountInput,
                        amount_edit,
                        EditableTextFilter::new(|c: char| c.is_ascii_digit()),
                        amount_box,
                        text_font(9.0),
                        TextColor(Color::WHITE),
                        TextLayout::justify(Justify::Center),
                        TextCursorStyle {
                            color: Color::WHITE,
                            ..default()
                        },
                    ));
                    for (button, key, fallback, x) in [
                        (StockButton::Ok, "UIIT_CTL_CONFIRM", "Confirm", STOCK_OK_X),
                        (
                            StockButton::Cancel,
                            "UIIT_CTL_CANCEL",
                            "Cancel",
                            STOCK_CANCEL_X,
                        ),
                    ] {
                        panel
                            .spawn((
                                button,
                                Button,
                                Hovered::default(),
                                abs_node((x, STOCK_BUTTON_Y, 76.0, 24.0), s),
                                ImageNode {
                                    image: button_style.normal.clone(),
                                    image_mode: NodeImageMode::Stretch,
                                    ..default()
                                },
                                button_style.clone(),
                            ))
                            .observe(on_stock_button)
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
    // the price is the field the player must fill, so it takes the focus
    if let Some(input) = price_input {
        focus.set(input, FocusCause::Navigated);
    }
}

/// Parse both edits back into their resources every frame (the store's
/// `sync_modal_amount` rule: the text is the truth, the resource is its view).
pub fn sync_stock_fields(
    amounts: Query<&EditableText, With<StockAmountInput>>,
    prices: Query<&EditableText, With<StockPriceInput>>,
    modal: Res<StockModal>,
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
            .unwrap_or(1);
        amount.0 = typed.clamp(1, prompt.max.max(1));
    }
    if let Ok(editable) = prices.single() {
        price.0 = editable
            .value()
            .to_string()
            .trim()
            .parse::<u64>()
            .unwrap_or(0);
    }
}

/// The box belongs to the stall session: no stall, no box.
pub fn clear_stock_with_stall(state: Res<StallState>, mut modal: ResMut<StockModal>) {
    if (!state.open || !state.owner) && modal.prompt.is_some() {
        modal.prompt = None;
    }
}

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
    let Some(prompt) = modal.prompt.clone() else {
        return;
    };
    if *button == StockButton::Cancel {
        modal.prompt = None;
        return;
    }
    let Some(request) = stock_request(&prompt, amount.0, price.0) else {
        // price 0: stay open, the player has not typed one yet
        return;
    };
    let what = match prompt.target {
        StockTarget::List { .. } => "stock",
        StockTarget::Reprice => "reprice",
    };
    info!("stall: {what} {:?} (0x70BA)", request);
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

#[cfg(test)]
mod test {
    use super::*;
    use crate::plugins::hud::stall::model::StallRow;

    fn prompt(max: u16) -> StockPrompt {
        StockPrompt {
            target: StockTarget::List { inventory_slot: 13 },
            stall_slot: 2,
            ref_id: 3800,
            name: "Elixir".into(),
            max,
            quantity: 1,
            price: 0,
        }
    }

    /// A stall with one row on sale in slot 2.
    fn stall_with_a_row() -> StallState {
        let mut state = StallState {
            open: true,
            owner: true,
            ..Default::default()
        };
        state.slots[2] = Some(StallRow {
            name: "Steppe Blade".into(),
            ref_id: 3800,
            quantity: 4,
            price: 12_500,
        });
        state
    }

    /// The wire fields are the ones the packet documents: the literal `1`
    /// group, the 0 tail, and both slots the drop named.
    #[test]
    fn a_confirmed_prompt_builds_the_documented_type_2_body() {
        assert_eq!(
            stock_request(&prompt(20), 5, 900),
            Some(StallUpdateRequest::ItemAdded {
                stall_slot: 2,
                inventory_slot: 13,
                quantity: 5,
                price: 900,
                flea_market_tid_group: 1,
                unknown0: 0,
            })
        );
    }

    /// Re-pricing sends type 1 for the row's own stall slot — and no
    /// inventory slot, because the wire body has none.
    #[test]
    fn a_confirmed_reprice_builds_the_type_1_body() {
        let state = stall_with_a_row();
        let prompt =
            reprice_prompt(&state, 2, |_| "from itemdata".into()).expect("slot 2 is on sale");

        assert_eq!(prompt.target, StockTarget::Reprice);
        // the box opens on the listing's numbers, not on empty fields
        assert_eq!(prompt.price, 12_500);
        assert_eq!(prompt.quantity, 4);
        // the icon comes from the row's ref id, which is why StallRow keeps it
        assert_eq!(prompt.ref_id, 3800);
        assert_eq!(prompt.name, "Steppe Blade");

        assert_eq!(
            stock_request(&prompt, prompt.quantity, 9_999),
            Some(StallUpdateRequest::ItemUpdate {
                stall_slot: 2,
                quantity: 4,
                price: 9_999,
                unknown0: 0,
            })
        );
    }

    /// An empty cell has nothing to re-price: no prompt, and therefore no
    /// type 1 that would ask the server to price a row that is not there
    /// (`owner::apply_update_ack` refuses the mirror image of this).
    #[test]
    fn a_reprice_prompt_needs_a_row() {
        let state = stall_with_a_row();
        assert!(reprice_prompt(&state, 0, |_| "x".into()).is_none());
        assert!(reprice_prompt(&state, 99, |_| "x".into()).is_none());
    }

    /// A re-price cannot grow the listing: nothing leaves the inventory on
    /// type 1, so the row's own quantity is the ceiling.
    #[test]
    fn a_reprice_cannot_raise_the_listed_quantity() {
        let state = stall_with_a_row();
        let prompt = reprice_prompt(&state, 2, |_| "x".into()).unwrap();
        let Some(StallUpdateRequest::ItemUpdate { quantity, .. }) = stock_request(&prompt, 500, 10)
        else {
            panic!("expected a type 1 body");
        };
        assert_eq!(quantity, 4);
    }

    /// A row whose name never resolved falls back to itemdata rather than
    /// opening the box with a blank name board.
    #[test]
    fn a_nameless_row_asks_itemdata() {
        let mut state = stall_with_a_row();
        state.slots[2].as_mut().unwrap().name = String::new();
        let prompt = reprice_prompt(&state, 2, |ref_id| format!("#{ref_id}")).unwrap();
        assert_eq!(prompt.name, "#3800");
    }

    /// A typed quantity above the stack lists the stack, and 0 lists one —
    /// the clamp is here so the server never has to refuse the frame.
    #[test]
    fn the_quantity_is_clamped_into_the_stack() {
        let Some(StallUpdateRequest::ItemAdded { quantity, .. }) =
            stock_request(&prompt(20), 9999, 10)
        else {
            panic!("expected a type 2 body");
        };
        assert_eq!(quantity, 20);
        let Some(StallUpdateRequest::ItemAdded { quantity, .. }) =
            stock_request(&prompt(20), 0, 10)
        else {
            panic!("expected a type 2 body");
        };
        assert_eq!(quantity, 1);
    }

    /// Price 0 sends nothing: the player is mid-typing, and a free listing is
    /// not something the original offers.
    #[test]
    fn a_zero_price_sends_nothing() {
        assert_eq!(stock_request(&prompt(20), 5, 0), None);
    }

    /// The drop prefers the cell under the cursor and falls back to the first
    /// free one; an occupied hover does NOT overwrite that row.
    #[test]
    fn the_target_slot_prefers_the_hovered_cell_and_never_an_occupied_one() {
        let mut state = StallState::default();
        assert_eq!(stock_target_slot(&state, Some(4)), Some(4));
        assert_eq!(stock_target_slot(&state, None), Some(0));
        state.slots[0] = Some(StallRow {
            name: "taken".into(),
            ref_id: 3800,
            quantity: 1,
            price: 1,
        });
        assert_eq!(stock_target_slot(&state, Some(0)), Some(1));
        for slot in state.slots.iter_mut() {
            *slot = Some(StallRow {
                name: "taken".into(),
                ref_id: 3800,
                quantity: 1,
                price: 1,
            });
        }
        assert_eq!(stock_target_slot(&state, Some(3)), None);
    }
}

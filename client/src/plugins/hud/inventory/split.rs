//! Stack split — vanilla's `MsgBoxDivideCount` prompt (#139b).
//!
//! Idea: the original client raises a message box of kind 5 when you
//! **Shift-click** a bag item, asks how many pieces to split off, and on
//! Confirm sends the ordinary `0x7034` op-0 move into the **first free bag
//! slot** with that amount. Nothing here is invented: the gesture, the two
//! preconditions and the target rule are the original's own behaviour (see
//! below), and every rect is the vanilla `MsgBoxDivideCount` section of
//! `resinfo/ifmessagebox.txt` laid out on the msgbox family's shared
//! background, exactly as `hud/store/ui.rs` does for `MsgBoxStore`.
//!
//! What the original client does:
//! * its item-click dispatcher reacts to Shift held down **and** a clicked
//!   container id of `0x46` (inventory — the same container enumeration the
//!   storage builders use, `packets/src/agent/inventory.rs:60`), which enters
//!   the split precondition check.
//! * that check refuses a slot that holds nothing and a stack of `<= 1`, then
//!   opens msgbox kind 5 — kind 5 is `"MsgBoxDivideCount"` in the dispatcher's
//!   own string switch — and remembers the source slot.
//! * its split confirm handler looks up the first free slot: it walks the
//!   container's slot vector and takes the index of the **first empty entry**,
//!   or `-1` when there is none. On `-1` it drops the whole operation;
//!   otherwise it builds a move `0x46 -> 0x46` with the entered amount and
//!   forgets the source slot again.
//!
//! Deliberate deviations (ADR-0009), both stated because the data does not
//! decide them: the box keeps the `-`/`+` stepper this HUD's other quantity
//! prompt already has (vanilla has only the edit box), and the outer size is
//! the shared `GDR_MSGBOX_BG` rect — the section's children reach x=285/y=167,
//! so the original resizes the box in code and the true size is unknown.
//!
//! What this module does **not** do is mirror the split locally: the bag is
//! applied from the `0xB034` ack only (`Inventory::apply_move` honours the
//! echoed `amount` for a partial move into an empty slot, `net/inventory.rs`).

use bevy::input_focus::tab_navigation::TabIndex;
use bevy::input_focus::{FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle, TextEdit};
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::character_data::ItemTypeData;
use packets::agent::prelude::InventoryOperationRequest;
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::{Inventory, BAG_FIRST_SLOT};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientTextNames, ClientUiStrings};
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// `GDR_MSGBOX_BG:CIFNormalTile` — `ifmessagebox.txt` `Section = Create` :6,
/// `Rect="16,40,284,122"`, art `com_bg_tile_b.ddj`. Shared by every box in the
/// family, so the split box is placed in it like the store's quantity box is.
const MSGBOX_BG: (f32, f32, f32, f32) = (16.0, 40.0, 284.0, 122.0);
const MSGBOX_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";
const QUANTITY_DDJ: &str = "media://interface/messagebox/msgbox_quantity.ddj";
const ITEMWINDOW_DDJ: &str = "media://interface/messagebox/msgbox_itemwindow.ddj";
/// The split box's name board uses `_3`, not the store box's plain variant
/// (`GDR_MBDC_STATIC_NAMEBOARD`, `ifmessagebox.txt` `DDJ=msgbox_iteminfo_3`).
const ITEMINFO_DDJ: &str = "media://interface/messagebox/msgbox_iteminfo_3.ddj";

/// Vanilla rect -> panel-local rect (the panel *is* `GDR_MSGBOX_BG`).
const fn msgbox_rect(rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (rect.0 - MSGBOX_BG.0, rect.1 - MSGBOX_BG.1, rect.2, rect.3)
}

// All ten `Section = MsgBoxDivideCount` controls, in the file's order.
const ICON_WND: (f32, f32, f32, f32) = msgbox_rect((18.0, 44.0, 48.0, 48.0));
const ICON: (f32, f32, f32, f32) = msgbox_rect((25.0, 51.0, 32.0, 32.0));
const NAME_BOARD: (f32, f32, f32, f32) = msgbox_rect((73.0, 44.0, 212.0, 48.0));
/// The board's own client inset is `4,3,6,26`, i.e. the name row is the top
/// 19 px of the board's inner box — `73+4, 44+3, 212-4-6, 48-3-26`.
const NAME_TEXT: (f32, f32, f32, f32) = msgbox_rect((77.0, 47.0, 202.0, 19.0));
const BOXDESC: (f32, f32, f32, f32) = msgbox_rect((77.0, 68.0, 202.0, 19.0));
const CURRENT_LABEL: (f32, f32, f32, f32) = msgbox_rect((34.0, 106.0, 64.0, 19.0));
const CURRENT_VALUE: (f32, f32, f32, f32) = msgbox_rect((105.0, 106.0, 41.0, 19.0));
const DIVIDE_LABEL: (f32, f32, f32, f32) = msgbox_rect((153.0, 106.0, 64.0, 19.0));
const AMOUNT_EDIT: (f32, f32, f32, f32) = msgbox_rect((223.0, 102.0, 42.0, 24.0));
/// Text inside the edit art, by its `ClientRect="7,5,7,5"`.
const AMOUNT_TEXT: (f32, f32, f32, f32) = msgbox_rect((230.0, 107.0, 28.0, 14.0));
const OK_RECT: (f32, f32, f32, f32) = msgbox_rect((71.0, 143.0, 76.0, 24.0));
const CANCEL_RECT: (f32, f32, f32, f32) = msgbox_rect((151.0, 143.0, 76.0, 24.0));
/// Stepper: the data leaves the strip left of the "Distributed number" label
/// free (the "retained" value ends at vanilla x 146, the label starts at 153),
/// so the two buttons sit under the edit box instead — see the module note.
const STEPPER_Y: f32 = msgbox_rect((0.0, 128.0, 0.0, 0.0)).1;
const STEPPER_X: (f32, f32) = (
    msgbox_rect((223.0, 0.0, 0.0, 0.0)).0,
    msgbox_rect((245.0, 0.0, 0.0, 0.0)).0,
);

/// Label colour of `GDR_MBDC_STATIC_DIVIDE` (`FontColor="255,239,218,164"`).
const LABEL_COLOR: Color = Color::srgb(239.0 / 255.0, 218.0 / 255.0, 164.0 / 255.0);

/// The open split prompt, or `None`. One at a time, like every other msgbox.
#[derive(Resource, Default)]
pub struct SplitPrompt {
    pub prompt: Option<SplitRequest>,
}

/// What the box was opened on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitRequest {
    /// Wire slot of the stack being divided.
    pub source: u8,
    /// Its stack count at open time — the box can split off `1..=stack-1`.
    pub stack: u16,
    pub ref_id: u32,
    pub name: String,
}

/// The "distributed number" currently entered (clamped by [`sync_split_amount`]).
#[derive(Resource)]
pub struct SplitAmount(pub u16);

impl Default for SplitAmount {
    fn default() -> Self {
        Self(1)
    }
}

#[derive(Component)]
pub(crate) struct SplitModalRoot;

#[derive(Component)]
pub(crate) struct SplitAmountInput;

/// The "Retained number" value static — recomputed as the amount changes.
#[derive(Component)]
pub(crate) struct SplitRetainedText;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SplitButton {
    Minus,
    Plus,
    Ok,
    Cancel,
}

/// Despawn marker, same one-frame pattern the store/storage modals use.
#[derive(Component)]
pub struct SplitClosing;

/// The stack count of `slot`, or 0 for an empty / non-stacking slot.
/// Non-expendables carry no `stack_count` on the wire and never stack
/// (`MaxStack = 1` for all 8650 equippables in the item data), so they can
/// never open this box.
pub fn stack_of(inventory: &Inventory, slot: u8) -> u16 {
    match inventory.get(slot).map(|item| &item.data) {
        Some(ItemTypeData::Expendable { stack_count, .. }) => *stack_count,
        _ => 0,
    }
}

/// the original's first-free-slot lookup: the first empty slot of the container, `None` when full.
/// Ours starts at [`BAG_FIRST_SLOT`] because our slot numbers are the
/// server's absolute ones while the original's are bag-relative (the same
/// `+0x0D` bias the storage builders carry, `packets/src/agent/inventory.rs`).
pub fn first_free_bag_slot(inventory: &Inventory) -> Option<u8> {
    (BAG_FIRST_SLOT..inventory.size()).find(|slot| inventory.get(*slot).is_none())
}

/// Can this slot be divided at all? The original refuses twice: the slot must
/// hold something, and its stack must be `> 1`.
pub fn splittable(inventory: &Inventory, slot: u8) -> bool {
    slot >= BAG_FIRST_SLOT && stack_of(inventory, slot) > 1
}

/// Open the box on `slot` if it may be divided. Called from the slot press
/// observer through `commands.queue` — that observer is already at Bevy's
/// 16-parameter ceiling, so the keyboard read and this resource write happen
/// in an exclusive closure instead of two more system params.
/// Returns whether the box was opened (the caller undoes the carry it
/// started when it was).
pub fn open_split_prompt(world: &mut World, slot: u8) -> bool {
    let Some(inventory) = world
        .query_filtered::<&Inventory, With<Player>>()
        .iter(world)
        .next()
        .cloned()
    else {
        return false;
    };
    if !splittable(&inventory, slot) {
        return false;
    }
    let stack = stack_of(&inventory, slot);
    let Some(item) = inventory.get(slot) else {
        return false;
    };
    let ref_id = item.ref_id;
    let name = world
        .get_resource::<ClientItemData>()
        .and_then(|data| data.get(&(ref_id as i32)))
        .and_then(|row| row.name_key())
        .and_then(|key| {
            world
                .get_resource::<ClientTextNames>()
                .and_then(|names| names.name(key))
        })
        .map(str::to_string)
        .unwrap_or_default();
    let Some(mut prompt) = world.get_resource_mut::<SplitPrompt>() else {
        return false;
    };
    prompt.prompt = Some(SplitRequest {
        source: slot,
        stack,
        ref_id,
        name,
    });
    true
}

/// Rebuild the box when the prompt changes (spawn on open, despawn on close).
#[allow(clippy::too_many_arguments)]
pub fn sync_split_modal(
    split: Res<SplitPrompt>,
    existing: Query<Entity, With<SplitModalRoot>>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    item_data: Res<ClientItemData>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut focus: ResMut<InputFocus>,
    mut amount: ResMut<SplitAmount>,
    mut commands: Commands,
) {
    if !split.is_changed() {
        return;
    }
    for entity in existing.iter() {
        commands.entity(entity).insert(SplitClosing);
    }
    let Some(prompt) = &split.prompt else {
        if !existing.is_empty() {
            focus.clear();
        }
        return;
    };
    let Ok(camera) = cam_query.single() else {
        return;
    };
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
    // vanilla opens at one piece; the retained value is the rest
    amount.0 = 1;
    let icon = item_data
        .get(&(prompt.ref_id as i32))
        .and_then(|row| row.icon_path());
    let retained = prompt.stack.saturating_sub(1);

    let mut input_entity = None;
    let mut confirm_button = None;
    let mut cancel_button = None;
    let root = commands
        .spawn((
            SplitModalRoot,
            Name::from("Inventory Split Modal"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            // the scrim swallows clicks, like the store's quantity box
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
                    // item window + icon (`_ICONWND` 18,44,48,48 / `_ICON` 25,51,32,32)
                    panel.spawn((
                        abs_node(ICON_WND, s),
                        ImageNode {
                            image: asset_server.load(ITEMWINDOW_DDJ),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    if let Some(icon) = &icon {
                        panel.spawn((
                            abs_node(ICON, s),
                            ImageNode {
                                image: asset_server.load(icon.clone()),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    }
                    // name board (`_NAMEBOARD` 73,44,212,48 on msgbox_iteminfo_3)
                    panel.spawn((
                        abs_node(NAME_BOARD, s),
                        ImageNode {
                            image: asset_server.load(ITEMINFO_DDJ),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        Text::new(prompt.name.clone()),
                        text_font(8.5),
                        TextColor(LABEL_COLOR),
                        TextLayout::justify(Justify::Center),
                        abs_node(NAME_TEXT, s),
                        Pickable::IGNORE,
                    ));
                    // `_BOXDESC` = UIIT_MSG_MSGBOX_DIVIDE_CURRENT_ITEM (L2093)
                    panel.spawn((
                        Text::new(
                            ui_strings
                                .get_or(
                                    "UIIT_MSG_MSGBOX_DIVIDE_CURRENT_ITEM",
                                    "Seperating the current item.",
                                )
                                .to_string(),
                        ),
                        text_font(8.0),
                        TextColor(Color::WHITE),
                        TextLayout::justify(Justify::Center),
                        abs_node(BOXDESC, s),
                        Pickable::IGNORE,
                    ));
                    // "Retained number" (L2094) + its value static
                    panel.spawn((
                        Text::new(
                            ui_strings
                                .get_or("UIIT_MSG_MSGBOX_CURRENT_COUNT", "Retained number")
                                .to_string(),
                        ),
                        text_font(8.0),
                        TextColor(Color::WHITE),
                        abs_node(CURRENT_LABEL, s),
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        SplitRetainedText,
                        Text::new(retained.to_string()),
                        text_font(8.5),
                        TextColor(Color::WHITE),
                        TextLayout::justify(Justify::Center),
                        abs_node(CURRENT_VALUE, s),
                        Pickable::IGNORE,
                    ));
                    // "Distributed number" (L2095) + the edit art
                    panel.spawn((
                        Text::new(
                            ui_strings
                                .get_or("UIIT_MSG_MSGBOX_DIVIDE_COUNT", "Distributed number")
                                .to_string(),
                        ),
                        text_font(8.0),
                        TextColor(LABEL_COLOR),
                        abs_node(DIVIDE_LABEL, s),
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        abs_node(AMOUNT_EDIT, s),
                        ImageNode {
                            image: asset_server.load(QUANTITY_DDJ),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    for (button, label, x) in [
                        (SplitButton::Minus, "-", STEPPER_X.0),
                        (SplitButton::Plus, "+", STEPPER_X.1),
                    ] {
                        panel
                            .spawn((
                                button,
                                Button,
                                Hovered::default(),
                                Text::new(label),
                                text_font(11.0),
                                TextColor(Color::srgb(0.9, 0.9, 0.9)),
                                abs_node((x, STEPPER_Y, 20.0, 18.0), s),
                            ))
                            .observe(on_split_button);
                    }
                    let mut input_box = abs_node(AMOUNT_TEXT, s);
                    input_box.padding = UiRect::top(Val::Px(2.0 * s));
                    let editable = EditableText {
                        visible_lines: Some(1.0),
                        allow_newlines: false,
                        max_characters: Some(5),
                        ..default()
                    };
                    input_entity = Some(
                        panel
                            .spawn((
                                SplitAmountInput,
                                editable,
                                EditableTextFilter::new(|c: char| c.is_ascii_digit()),
                                // A press on any node without a `TabIndex`
                                // ancestor bubbles `AcquireFocus` to the window
                                // and CLEARS the focus (bevy_input_focus
                                // `click_to_focus`), so clicking into the box
                                // used to make it deaf. The chat row carries
                                // the same marker.
                                TabIndex(0),
                                input_box,
                                text_font(9.0),
                                TextColor(Color::WHITE),
                                TextLayout::justify(Justify::Center),
                                TextCursorStyle {
                                    color: Color::WHITE,
                                    ..default()
                                },
                            ))
                            .id(),
                    );
                    // Confirm (`_BTN_OK` 71,143,76,24) / Cancel (:151,143)
                    for (button, key, fallback, rect) in [
                        (SplitButton::Ok, "UIIT_CTL_CONFIRM", "Confirm", OK_RECT),
                        (
                            SplitButton::Cancel,
                            "UIIT_CTL_CANCEL",
                            "Cancel",
                            CANCEL_RECT,
                        ),
                    ] {
                        let mut entity = panel.spawn((
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
                        ));
                        match button {
                            SplitButton::Ok => confirm_button = Some(entity.id()),
                            _ => cancel_button = Some(entity.id()),
                        }
                        entity.observe(on_split_button).with_children(|b| {
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
        })
        .id();

    // Enter = Confirm, Escape = Cancel, through the HUD's own key chain
    // (`hud::focus`): the input disallows newlines, so Enter reaches the chain.
    if let Some(confirm_button) = confirm_button {
        commands
            .entity(root)
            .insert(crate::plugins::hud::focus::HudDialog { confirm_button });
    }
    if let Some(close_button) = cancel_button {
        commands
            .entity(root)
            .insert(crate::plugins::hud::focus::HudWindow { close_button });
    }
    if let Some(input) = input_entity {
        focus.set(input, FocusCause::Navigated);
    }
}

/// Parse the edit box back into [`SplitAmount`], clamped to `1..=stack-1`
/// (splitting off the whole stack is not a split), and keep the "Retained
/// number" static in step.
pub fn sync_split_amount(
    split: Res<SplitPrompt>,
    inputs: Query<&EditableText, With<SplitAmountInput>>,
    mut amount: ResMut<SplitAmount>,
    mut retained: Query<&mut Text, With<SplitRetainedText>>,
) {
    let Some(prompt) = &split.prompt else {
        return;
    };
    let Ok(editable) = inputs.single() else {
        return;
    };
    let parsed = editable
        .value()
        .to_string()
        .trim()
        .parse::<u16>()
        .unwrap_or(1)
        .clamp(1, prompt.stack.saturating_sub(1).max(1));
    if amount.0 != parsed {
        amount.0 = parsed;
    }
    let line = prompt.stack.saturating_sub(parsed).to_string();
    for mut text in retained.iter_mut() {
        if text.0 != line {
            line.clone_into(&mut text.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn on_split_button(
    activate: On<Activate>,
    buttons: Query<&SplitButton>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    amount: Res<SplitAmount>,
    mut inputs: Query<(Entity, &mut EditableText), With<SplitAmountInput>>,
    mut focus: ResMut<InputFocus>,
    mut split: ResMut<SplitPrompt>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    let Some(prompt) = split.prompt.clone() else {
        return;
    };
    // +/- steer the text input; `sync_split_amount` parses it back.
    // SelectAll + Insert rather than clear(): see the store modal's note —
    // clear() leaves parley's selection at a stale byte index.
    // The press on the button already cleared the focus (see the `TabIndex`
    // note at the input), so typing stays possible after a stepper click.
    let mut set_amount = |value: u16| {
        let Ok((entity, mut editable)) = inputs.single_mut() else {
            return;
        };
        editable.queue_edit(TextEdit::SelectAll);
        editable.queue_edit(TextEdit::Insert(value.to_string().into()));
        focus.set(entity, FocusCause::Navigated);
    };
    let max = prompt.stack.saturating_sub(1).max(1);
    match button {
        SplitButton::Minus => set_amount(amount.0.saturating_sub(1).max(1)),
        SplitButton::Plus => set_amount((amount.0 + 1).min(max)),
        SplitButton::Cancel => split.prompt = None,
        SplitButton::Ok => {
            let quantity = amount.0.clamp(1, max);
            split.prompt = None;
            let Ok(inventory) = inventories.single() else {
                return;
            };
            // The original drops the whole operation when the bag is full
            // (the original's split confirm handler: `cmp eax,-1 / je`), so no request goes out.
            let Some(target) = first_free_bag_slot(inventory) else {
                warn!("inventory: no free slot to split into");
                return;
            };
            let request = InventoryOperationRequest::Move {
                source: prompt.source,
                target,
                amount: quantity,
            };
            let Ok(conn) = conn.single() else {
                warn!("inventory: not sending split, no agent connection");
                return;
            };
            debug!(
                "inventory: splitting {} off slot {} into {}",
                quantity, prompt.source, target
            );
            if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
                error!("network: failed to send split move: {}", e.0);
            }
        }
    }
}

/// The box belongs to the inventory window: closing the window (or leaving
/// the world, which closes it) takes the prompt with it, so the scrim can
/// never outlive what it was opened on.
pub fn close_split_with_window(state: Res<InventoryState>, mut split: ResMut<SplitPrompt>) {
    if !state.open && split.prompt.is_some() {
        split.prompt = None;
    }
}

pub fn despawn_closing_split(closing: Query<Entity, With<SplitClosing>>, mut commands: Commands) {
    for entity in closing.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use packets::agent::character_data::{InventoryItem, ItemTypeData, RentInfo};

    fn stack_item(slot: u8, count: u16) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id: 4,
            data: ItemTypeData::Expendable {
                stack_count: count,
                assimilation_prob: None,
                mag_params: vec![],
            },
        }
    }

    /// Anything that is not an expendable carries no stack count at all —
    /// `ItemTypeData::Unknown` stands in for that class here.
    fn non_stacking(slot: u8) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id: 100,
            data: ItemTypeData::Unknown,
        }
    }

    fn inventory(size: u8, items: Vec<InventoryItem>) -> Inventory {
        let mut slots = vec![None; size as usize];
        for item in items {
            let index = item.slot as usize;
            if index >= slots.len() {
                slots.resize(index + 1, None);
            }
            slots[index] = Some(item);
        }
        Inventory {
            slots,
            avatar_slots: vec![],
            gold: 0,
        }
    }

    /// The original's two refusals, plus the container check that keeps the
    /// paper doll out of it.
    #[test]
    fn only_a_bag_stack_of_more_than_one_can_be_split() {
        let inv = inventory(
            45,
            vec![
                non_stacking(6),
                stack_item(BAG_FIRST_SLOT, 20),
                stack_item(BAG_FIRST_SLOT + 1, 1),
                non_stacking(BAG_FIRST_SLOT + 2),
            ],
        );
        assert!(splittable(&inv, BAG_FIRST_SLOT), "a 20-stack splits");
        assert!(
            !splittable(&inv, BAG_FIRST_SLOT + 1),
            "a single piece is not a stack (the original's `cmp eax,1 / jle`)"
        );
        assert!(
            !splittable(&inv, BAG_FIRST_SLOT + 2),
            "equipment never stacks (MaxStack = 1 for all 8650 rows)"
        );
        assert!(
            !splittable(&inv, 6),
            "an equipped item is not container 0x46"
        );
        assert!(
            !splittable(&inv, BAG_FIRST_SLOT + 3),
            "an empty slot has nothing to divide"
        );
    }

    /// the original's first-free-slot lookup: first empty slot, `None` when the bag is full.
    #[test]
    fn the_split_target_is_the_first_free_bag_slot() {
        let mut items: Vec<InventoryItem> = (BAG_FIRST_SLOT..BAG_FIRST_SLOT + 3)
            .map(|slot| stack_item(slot, 5))
            .collect();
        items.push(stack_item(BAG_FIRST_SLOT + 4, 5));
        let inv = inventory(BAG_FIRST_SLOT + 5, items);
        assert_eq!(first_free_bag_slot(&inv), Some(BAG_FIRST_SLOT + 3));

        let full = inventory(
            BAG_FIRST_SLOT + 2,
            (BAG_FIRST_SLOT..BAG_FIRST_SLOT + 2)
                .map(|slot| stack_item(slot, 5))
                .collect(),
        );
        assert_eq!(first_free_bag_slot(&full), None, "a full bag sends nothing");
    }

    /// The amount the box may produce: `1..=stack-1` — never the whole stack
    /// (that is a plain move) and never 0.
    #[test]
    fn the_distributed_number_stays_between_one_and_stack_minus_one() {
        let clamp = |entered: u16, stack: u16| entered.clamp(1, stack.saturating_sub(1).max(1));
        assert_eq!(clamp(0, 20), 1);
        assert_eq!(clamp(19, 20), 19);
        assert_eq!(clamp(20, 20), 19);
        assert_eq!(clamp(9999, 20), 19);
        // a 2-stack can only ever give one piece away
        assert_eq!(clamp(5, 2), 1);
    }

    /// The box as `sync_split_modal` builds it, opened on a 1000-stack.
    fn modal_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
            bevy::input_focus::tab_navigation::TabNavigationPlugin,
        ))
        .init_asset::<Image>()
        .init_resource::<InputFocus>()
        .init_resource::<bevy::input_focus::InputFocusVisible>()
        .init_resource::<ClientUiStrings>()
        .init_resource::<ClientItemData>()
        .init_resource::<SplitAmount>()
        .insert_resource(FontAssets {
            one: Handle::default(),
            two: Handle::default(),
            three: Handle::default(),
            nine: Handle::default(),
        })
        .insert_resource(SplitPrompt {
            prompt: Some(SplitRequest {
                source: 0x17,
                stack: 1000,
                ref_id: 4,
                name: "HP Recovery potion".into(),
            }),
        });
        app.world_mut().spawn(Camera2d);
        app.world_mut()
            .run_system_cached(sync_split_modal)
            .expect("sync_split_modal failed");
        app
    }

    /// Typing digits used to go nowhere: a press anywhere without a
    /// `TabIndex` ancestor bubbles `AcquireFocus` to the window and
    /// clears the focus. The amount box must therefore carry `TabIndex`, so
    /// the click into it — the very thing a player does before typing —
    /// keeps (or takes) the focus.
    #[test]
    fn the_amount_box_takes_focus_when_pressed() {
        let mut app = modal_app();
        let input = app
            .world_mut()
            .query_filtered::<Entity, With<SplitAmountInput>>()
            .single(app.world())
            .expect("one amount box");
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));

        // what `click_to_focus` raises for a press on the box itself
        let window = app.world_mut().spawn(Window::default()).id();
        app.world_mut().trigger(bevy::input_focus::AcquireFocus {
            focused_entity: input,
            window,
        });
        assert_eq!(
            app.world().resource::<InputFocus>().get(),
            Some(input),
            "a press on the amount box must leave it focused"
        );
    }

    /// Enter confirms and Escape cancels through the HUD key chain
    /// (`hud::focus`), which only knows dialogs that name their buttons.
    #[test]
    fn enter_and_escape_are_bound_to_confirm_and_cancel() {
        let mut app = modal_app();
        let (dialog, window) = app
            .world_mut()
            .query_filtered::<(
                &crate::plugins::hud::focus::HudDialog,
                &crate::plugins::hud::focus::HudWindow,
            ), With<SplitModalRoot>>()
            .single(app.world())
            .map(|(d, w)| (*d, *w))
            .expect("the modal root names both buttons");
        assert_eq!(
            app.world().get::<SplitButton>(dialog.confirm_button),
            Some(&SplitButton::Ok)
        );
        assert_eq!(
            app.world().get::<SplitButton>(window.close_button),
            Some(&SplitButton::Cancel)
        );
    }

    /// The start-trap guard (`hud/mod.rs` `hud_scenes`): every system this
    /// module adds is registered inside the inventory plugin's scene-gated
    /// tuple, and the window is cleaned up when the world ends. `cargo test`
    /// cannot see a missing `run_if`, so it is asserted as text, like
    /// `hud/job/ranking.rs` does.
    #[test]
    fn the_split_systems_are_registered_behind_the_scene_gate() {
        let registration = include_str!("mod.rs");
        for system in [
            "split::sync_split_modal",
            "split::sync_split_amount",
            "split::close_split_with_window",
        ] {
            assert!(
                registration.contains(system),
                "{system} is not registered at all"
            );
        }
        assert!(
            registration.contains(".run_if(super::hud_scenes"),
            "the inventory plugin registers Update systems without the scene gate"
        );
        assert!(
            registration.contains("OnExit(SceneState::GameWorld)"),
            "the inventory window is not cleaned up when the world scene ends"
        );
        // the modal despawn runs in PostUpdate, ungated on purpose (it only
        // reaps entities that already carry the marker) — assert it exists so
        // a rename cannot leave the scrim on screen
        assert!(
            registration.contains("split::despawn_closing_split"),
            "the split modal has no despawn system"
        );
    }
}

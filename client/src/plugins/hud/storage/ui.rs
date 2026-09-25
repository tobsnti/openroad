//! Storage (warehouse) window layout + deposit/withdraw interaction.
//!
//! Idea: transcribed from `resinfo/ifstorageroom.txt` + ginterface's
//! `GDR_STORAGEROOM` (a 254x317 mframe window): a 6x5 com_lattice slot grid
//! (32px slots on a 36px pitch — the store/inventory lattice anatomy), a page
//! spinner, and the money row ("Total Gold" tile + stored gold + the
//! com_moneybutton opening the gold deposit/withdraw popup). The window is
//! rebuilt from `StorageState` on every change (open, close, page, every
//! applied ack; a dragged position survives) inside the shared `game_window`
//! chrome. Items move with the vanilla carries: dropping an inventory carry
//! on the window deposits (slot-precise on a cell, else first free slot,
//! fee applied silently server-side), picking a storage item up and dropping
//! it on the inventory withdraws, dropping it on another storage cell moves
//! within storage. All of it is sent as 0x7034 storage ops and applied
//! server-confirmed only (model.rs).
//!
//! The gold popup behind the money button is **not** here any more: it is
//! `gold_modal.rs`, parameterised over its target warehouse, because the
//! guild warehouse is the same `CIFStorageRoom` window and needs the same
//! popup (ops 32/33 instead of 12/11). This file only says which side its own
//! button means.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::character_data::ItemTypeData;
use packets::agent::prelude::InventoryOperationRequest;
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::{DragGhost, InventoryRoot};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::storage::gold_modal::{GoldModal, GoldModalRoot, GoldTarget};
use crate::plugins::hud::storage::model::{
    PendingStorageOp, StorageOp, StorageState, STORAGE_SLOTS_PER_PAGE,
};
use crate::plugins::hud::window_positions::PersistedWindow;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::{Inventory, BAG_FIRST_SLOT};
use crate::plugins::player::Player;
use crate::plugins::settings::window_positions::WndPosSlot;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// `GDR_STORAGEROOM` (`ginterface.txt:569`, id 19), the vanilla outer window.
const VANILLA_OUTER: (f32, f32) = (254.0, 317.0);

/// Content-space origin, taken from the shell's own exports rather than
/// re-derived. A hand-derived `(12, 26)` used to live here and put every
/// element 10px low while making `CONTENT_H` 26px too tall; three windows
/// independently produced that same wrong `26`, so the rule is to import the
/// pair, never to subtract a fresh number (#313).
const ORIGIN_X: f32 = game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD;
const ORIGIN_Y: f32 = game_window::CONTENT_TOP;

/// An `ifstorageroom.txt` window-space rect in content space. Keeping the
/// vanilla numbers at the call sites means every constant below is checkable
/// against the data file by eye.
const fn content_rect(rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (rect.0 - ORIGIN_X, rect.1 - ORIGIN_Y, rect.2, rect.3)
}

// The inverse of `game_window::outer_size`; the test below pins the two
// together against `VANILLA_OUTER`.
const CONTENT_W: f32 = VANILLA_OUTER.0 - 2.0 * ORIGIN_X;
const CONTENT_H: f32 = VANILLA_OUTER.1
    - game_window::CONTENT_TOP
    - game_window::CHROME_PAD
    - game_window::FRAME_VIS_BOTTOM;
/// Left of the store window's spot, clear of the right-docked inventory.
const WINDOW_RIGHT: f32 = 560.0;
const WINDOW_TOP: f32 = 60.0;

const GRID_COLS: usize = 6;
const GRID_ROWS: usize = 5;
const CELL: f32 = 36.0;
const ICON_INSET: f32 = 2.0;
const ICON_SIZE: f32 = 32.0;
/// `GDR_STORAGE_LAT` (`ifstorageroom.txt:740`).
const LATTICE_RECT: (f32, f32, f32, f32) = content_rect((21.0, 71.0, 216.0, 180.0));
/// `GDR_STORAGE_SPIN_PAGE` (`:721`).
const SPIN_RECT: (f32, f32, f32, f32) = content_rect((102.0, 254.0, 50.0, 16.0));
/// `GDR_STORAGE_BGTILE` (`:798`).
const STRIP_RECT: (f32, f32, f32, f32) = content_rect((25.0, 250.0, 204.0, 13.0));
/// The money row — `GDR_STORAGE_STA_GOLD` and `_STA_MONEY` share the rect
/// `9,283,236,24`, whose content-space x is -3. This one keeps the pre-existing
/// clamp to `(0, CONTENT_W)` and only takes the corrected y. Not because
/// anything would clip — the shell's content node is auto-sized with default
/// `Overflow`, so children may overhang, and both this row and the vanilla
/// original do reach past the content box into the frame's bottom band. It is
/// clamped because the gold readout below is placed at a hand-chosen
/// `MONEY_RECT.0 + 96`, with no data-verified target of its own, so moving the
/// row's origin would shift text that has nothing to be verified against.
const MONEY_RECT: (f32, f32, f32, f32) = (
    0.0,
    content_rect((9.0, 283.0, 236.0, 24.0)).1,
    CONTENT_W,
    24.0,
);
/// `GDR_STORAGE_BTN_MONEY` (`:645`).
const MONEY_BTN_RECT: (f32, f32, f32, f32) = content_rect((78.0, 286.0, 20.0, 20.0));

const LATTICE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_";
const MONEY_TILE: &str = "media://interface/store/str_slot_01.ddj";

const NAME_COLOR: Color = Color::srgb_u8(255, 226, 123);
const PRICE_COLOR: Color = Color::srgb_u8(255, 217, 83);

#[derive(Component)]
pub struct StorageWindowRoot;

/// Deferred despawn marker, same trick as the store window.
#[derive(Component)]
pub struct StorageClosing;

#[derive(Component)]
pub struct StorageSlotCell {
    /// Index within the visible page grid.
    cell: usize,
}

#[derive(Component)]
enum StoragePageButton {
    Prev,
    Next,
}

#[derive(Component)]
struct MoneyButton;

/// The active storage carry (an item picked up from the storage grid) and
/// its ghost icon. Lives outside [`StorageState`] so cursor bookkeeping never
/// rebuilds the window (the store-carry precedent).
#[derive(Resource, Default)]
pub struct StorageCarry(pub Option<StorageCarryData>);

pub struct StorageCarryData {
    /// The carried item's storage wire slot.
    pub slot: u8,
    pub ghost: Entity,
    /// True until the pickup press's own just_pressed frame has passed.
    pub just_picked: bool,
}

/// The carry's cursor-following icon.
#[derive(Component)]
pub struct StorageGhost;

/// The stack size a whole-stack move reports. The original reads the source
/// item's own count and feeds it into the op-1/29 request, so a non-stackable
/// moves as one piece rather than as zero — a 0 would ask the server to move
/// nothing.
pub fn stack_at(items: &Inventory, slot: u8) -> u16 {
    match items.get(slot).map(|item| &item.data) {
        Some(ItemTypeData::Expendable { stack_count, .. }) => (*stack_count).max(1),
        _ => 1,
    }
}

/// The storage wire slot a page-grid cell shows.
fn wire_slot(state: &StorageState, cell: usize) -> u8 {
    state.active_page * STORAGE_SLOTS_PER_PAGE + cell as u8
}

/// Rebuild the storage window whenever the state changes.
#[allow(clippy::too_many_arguments)]
pub fn sync_storage_window(
    state: Res<StorageState>,
    existing: Query<(Entity, &Node), With<StorageWindowRoot>>,
    item_data: Res<ClientItemData>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !state.is_changed() {
        return;
    }
    // page flips + applied acks rebuild the window — keep a dragged position
    let mut anchor = (WINDOW_RIGHT, WINDOW_TOP);
    for (entity, node) in existing.iter() {
        if let (Val::Px(right), Val::Px(top)) = (node.right, node.top) {
            anchor = (right, top);
        }
        commands.entity(entity).insert(StorageClosing);
    }
    if state.session.is_none() {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("storage: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_STT_STORAGEROOM", "Storage"),
        (CONTENT_W, CONTENT_H),
        anchor,
        s,
    );
    commands
        .entity(window.root)
        // Hovered so deposit-drops can test "is the pointer over the storage"
        .insert((
            StorageWindowRoot,
            GlobalZIndex(58),
            Hovered::default(),
            PersistedWindow(WndPosSlot::StorageRoom),
        ));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };

    commands.entity(window.content).with_children(|content| {
        // brighter backdrop across the whole pane (store-window precedent)
        content.spawn((
            abs_node((0.0, 0.0, CONTENT_W, CONTENT_H), s),
            ImageNode {
                image: asset_server.load("media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj"),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        ));

        // "Deposit Items" caption over the grid (vanilla's sframe caption)
        content.spawn((
            Text::new(
                ui_strings
                    .get_or("UIIT_STT_DEPOSITTEDITEM", "Deposit Items")
                    .to_string(),
            ),
            text_font(8.0),
            TextColor(NAME_COLOR),
            TextLayout::justify(Justify::Center),
            abs_node((9.0, 18.0, 216.0, 14.0), s),
            Pickable::IGNORE,
        ));

        // storage grid (com_lattice anatomy, same as the store/inventory)
        content
            .spawn((
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px(GRID_COLS as u16, CELL * s),
                    grid_template_rows: RepeatedGridTrack::px(GRID_ROWS as u16, CELL * s),
                    ..abs_node(LATTICE_RECT, s)
                },
                Pickable::IGNORE,
            ))
            .with_children(|grid| {
                for cell in 0..(GRID_COLS * GRID_ROWS) {
                    let (row, col) = (cell / GRID_COLS, cell % GRID_COLS);
                    let quarter = match (row == GRID_ROWS - 1, col == GRID_COLS - 1) {
                        (false, false) => "left_up",
                        (false, true) => "right_up",
                        (true, false) => "left_down",
                        (true, true) => "right_down",
                    };
                    let item = state.items.get(wire_slot(&state, cell));
                    let mut cell_cmd = grid.spawn((
                        StorageSlotCell { cell },
                        Hovered::default(),
                        Node {
                            width: Val::Px(CELL * s),
                            height: Val::Px(CELL * s),
                            ..default()
                        },
                        ImageNode {
                            image: asset_server.load(format!("{LATTICE_DIR}{quarter}.ddj")),
                            image_mode: NodeImageMode::Stretch,
                            // dim the grid until the 0x3049 list arrived
                            color: if state.synced {
                                Color::WHITE
                            } else {
                                Color::srgb(0.55, 0.55, 0.55)
                            },
                            ..default()
                        },
                    ));
                    cell_cmd.observe(on_storage_slot_press);
                    let Some(item) = item else {
                        continue;
                    };
                    let row = item_data.get(&(item.ref_id as i32));
                    let Some(icon) = row.and_then(|row| row.icon_path()) else {
                        continue;
                    };
                    let count = match &item.data {
                        ItemTypeData::Expendable { stack_count, .. } => *stack_count as u32,
                        ItemTypeData::MagicCube { elixir_count } => *elixir_count,
                        _ => 0,
                    };
                    let opt_level = match &item.data {
                        ItemTypeData::Equipment(eq) => eq.opt_level,
                        _ => 0,
                    };
                    cell_cmd.with_children(|slot| {
                        slot.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(ICON_INSET * s),
                                top: Val::Px(ICON_INSET * s),
                                width: Val::Px(ICON_SIZE * s),
                                height: Val::Px(ICON_SIZE * s),
                                ..default()
                            },
                            ImageNode {
                                image: asset_server.load(icon),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                        if count > 1 {
                            slot.spawn((
                                Text::new(count.to_string()),
                                text_font(7.0),
                                TextColor(Color::WHITE),
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(3.0 * s),
                                    bottom: Val::Px(2.0 * s),
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                        }
                        if opt_level > 0 {
                            slot.spawn((
                                Text::new(format!("+{opt_level}")),
                                text_font(7.0),
                                TextColor(PRICE_COLOR),
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(2.0 * s),
                                    top: Val::Px(1.0 * s),
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                        }
                    });
                }
            });

        // bg tile strip behind the spinner row
        content.spawn((
            abs_node(STRIP_RECT, s),
            ImageNode {
                image: asset_server.load("media://interface/ifcommon/bg_tile/com_bg_tile_c.ddj"),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        ));

        // page spinner — CIFSpinButtonCtrl anatomy (store-window twin)
        let arrow_style = |name: &str| ImageButtonStyle {
            normal: asset_server.load(format!("media://interface/ifcommon/com_{name}_arrow.ddj")),
            hover: asset_server.load(format!(
                "media://interface/ifcommon/com_{name}_arrow_focus.ddj"
            )),
            press: asset_server.load(format!(
                "media://interface/ifcommon/com_{name}_arrow_press.ddj"
            )),
            ..Default::default()
        };
        for (button, name, x) in [
            (StoragePageButton::Prev, "left", SPIN_RECT.0),
            (StoragePageButton::Next, "right", SPIN_RECT.0 + 34.0),
        ] {
            let style = arrow_style(name);
            content
                .spawn((
                    button,
                    Button,
                    Hovered::default(),
                    abs_node((x, SPIN_RECT.1, 16.0, 16.0), s),
                    ImageNode {
                        image: style.normal.clone(),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    style,
                ))
                .observe(on_storage_page_press);
        }
        content.spawn((
            Text::new(format!("{}", state.active_page + 1)),
            text_font(8.0),
            TextColor(Color::srgb(0.85, 0.85, 0.85)),
            TextLayout::justify(Justify::Center),
            abs_node((SPIN_RECT.0 + 16.0, SPIN_RECT.1 + 2.0, 18.0, 12.0), s),
            Pickable::IGNORE,
        ));

        // money row: "Total Gold" tile, the stored gold, the money button
        content.spawn((
            abs_node(MONEY_RECT, s),
            ImageNode {
                image: asset_server.load(MONEY_TILE),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        content.spawn((
            Text::new(
                ui_strings
                    .get_or("UIIT_STT_DEPOSITTEDMONEY", "Total Gold")
                    .to_string(),
            ),
            text_font(7.5),
            TextColor(NAME_COLOR),
            abs_node((MONEY_RECT.0 + 8.0, MONEY_RECT.1 + 6.0, 60.0, 14.0), s),
            Pickable::IGNORE,
        ));
        content.spawn((
            Text::new(format!(
                "{} {}",
                state.items.gold,
                ui_strings.get_or("UIIT_STT_GOLD", "Gold")
            )),
            text_font(7.5),
            TextColor(PRICE_COLOR),
            TextLayout::justify(Justify::Right),
            abs_node((MONEY_RECT.0 + 96.0, MONEY_RECT.1 + 6.0, 126.0, 14.0), s),
            Pickable::IGNORE,
        ));
        // GDR_STORAGE_BTN_MONEY — no focus/press art variants exist for
        // com_moneybutton, so the style reuses the base texture
        let money_art: Handle<Image> =
            asset_server.load("media://interface/ifcommon/com_moneybutton.ddj");
        content
            .spawn((
                MoneyButton,
                Button,
                Hovered::default(),
                abs_node(MONEY_BTN_RECT, s),
                ImageNode {
                    image: money_art.clone(),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                ImageButtonStyle {
                    normal: money_art.clone(),
                    hover: money_art.clone(),
                    press: money_art,
                    ..Default::default()
                },
            ))
            .observe(on_money_button);
    });
}

/// The storage's X ends the whole conversation, like the shop's: the storage
/// session lives inside the talk session, so this sends the 0x704B close and
/// takes the NPC dialog down with it.
fn on_close_button(
    _: On<Activate>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut state: ResMut<StorageState>,
    mut dialog: ResMut<crate::plugins::hud::npc_dialog::model::NpcDialogState>,
) {
    if let Some(session) = state.session.take() {
        crate::plugins::hud::npc_dialog::model::send_close_to(&conn, session.npc_id);
    }
    *dialog = crate::plugins::hud::npc_dialog::model::NpcDialogState::Closed;
}

fn on_storage_page_press(
    activate: On<Activate>,
    buttons: Query<&StoragePageButton>,
    mut state: ResMut<StorageState>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    let pages = state.page_count();
    match button {
        StoragePageButton::Prev if state.active_page > 0 => state.active_page -= 1,
        StoragePageButton::Next if state.active_page + 1 < pages => state.active_page += 1,
        _ => {}
    }
}

/// The money button only names its own warehouse; the popup itself is the
/// shared `gold_modal` unit (ops 12/11 for this side).
fn on_money_button(_: On<Activate>, mut modal: ResMut<GoldModal>) {
    modal.open_for(GoldTarget::PersonalStorage);
}

/// Publish the hovered storage item for the shared item tooltip
/// (`inventory::tooltip`). Storage cells carry no Over/Out observers — hover
/// is polled from their `Hovered` components — and the result deliberately
/// lands in its own resource rather than in [`StorageState`], which would
/// rebuild the window on every mouse move. Suppressed mid-carry, matching the
/// inventory tooltip's drag behaviour.
pub fn track_storage_hover(
    state: Res<StorageState>,
    cells: Query<(&StorageSlotCell, &Hovered)>,
    carry: Res<StorageCarry>,
    inv_state: Res<InventoryState>,
    mut hovered: ResMut<crate::plugins::hud::item_cell::HoveredItem>,
) {
    let carrying = carry.0.is_some() || inv_state.drag.is_some();
    let item = (!carrying)
        .then(|| {
            cells
                .iter()
                .find(|(_, hovered)| hovered.get())
                .and_then(|(cell, _)| state.items.get(wire_slot(&state, cell.cell)))
        })
        .flatten();
    let wanted = item
        .cloned()
        .map(crate::plugins::hud::item_cell::HoveredItemKind::Owned);
    match (&hovered.0, &wanted) {
        (None, None) => {}
        _ if hovered.owned() != item => hovered.0 = wanted,
        _ => {}
    }
}

/// Press on a storage cell: begin the storage carry (a ghost sticks to the
/// cursor; [`finish_storage_carry`] routes the drop). Deposits of an active
/// INVENTORY carry are handled by the [`deposit_drop_on_storage`] poll — this
/// observer backs off so the drop isn't double-handled.
#[allow(clippy::too_many_arguments)]
fn on_storage_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&StorageSlotCell>,
    state: Res<StorageState>,
    inv_state: Res<InventoryState>,
    item_data: Res<ClientItemData>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut carry: ResMut<StorageCarry>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    if press.event.button != PointerButton::Primary {
        return;
    }
    if carry.0.is_some() || inv_state.drag.is_some() {
        return;
    }
    let Ok(cell) = cells.get(press.entity) else {
        return;
    };
    if !state.synced {
        return;
    }
    let slot = wire_slot(&state, cell.cell);
    let Some(item) = state.items.get(slot) else {
        return;
    };
    let Some(icon) = item_data
        .get(&(item.ref_id as i32))
        .and_then(|row| row.icon_path())
    else {
        return;
    };
    let mut ghost = commands.spawn((
        StorageGhost,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(-1000.0),
            top: Val::Px(-1000.0),
            width: Val::Px(ICON_SIZE * hud_scale()),
            height: Val::Px(ICON_SIZE * hud_scale()),
            ..default()
        },
        ImageNode {
            image: asset_server.load(icon),
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
        GlobalZIndex(80),
        Pickable::IGNORE,
    ));
    if let Ok(camera) = cam_query.single() {
        ghost.insert(UiTargetCamera(camera));
    }
    carry.0 = Some(StorageCarryData {
        slot,
        ghost: ghost.id(),
        just_picked: true,
    });
}

/// Cursor-follow for the storage-carry ghost.
pub fn update_storage_ghost(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut ghosts: Query<&mut Node, With<StorageGhost>>,
) {
    if ghosts.is_empty() {
        return;
    }
    let Some(cursor) = windows.single().ok().and_then(|w| w.cursor_position()) else {
        return;
    };
    let size = ICON_SIZE * hud_scale();
    for mut node in ghosts.iter_mut() {
        node.left = Val::Px(cursor.x - size / 2.0);
        node.top = Val::Px(cursor.y - size / 2.0);
    }
}

/// Route the storage carry's drop: another storage cell → within-storage
/// move; a bag slot → withdraw there; the inventory window body → withdraw
/// into the first empty bag slot; anywhere else cancels. Release over the
/// picked cell keeps the carry alive as a click-carry (vanilla feel).
#[allow(clippy::too_many_arguments)]
pub fn finish_storage_carry(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<StorageState>,
    inv_state: Res<InventoryState>,
    storage_cells: Query<(&StorageSlotCell, &Hovered)>,
    inv_roots: Query<&Hovered, With<InventoryRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut carry: ResMut<StorageCarry>,
    mut pending: ResMut<PendingStorageOp>,
    mut commands: Commands,
) {
    let Some(data) = &carry.0 else {
        return;
    };
    if buttons.just_pressed(MouseButton::Right) || state.session.is_none() {
        let data = carry.0.take().expect("checked above");
        commands.entity(data.ghost).despawn();
        return;
    }
    let clicked = buttons.just_pressed(MouseButton::Left);
    let released = buttons.just_released(MouseButton::Left);
    if !clicked && !released {
        return;
    }
    if clicked && data.just_picked {
        // the pickup press itself
        if let Some(data) = carry.0.as_mut() {
            data.just_picked = false;
        }
        return;
    }

    let hovered_cell = storage_cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(cell, _)| wire_slot(&state, cell.cell));
    let source = data.slot;
    let npc_id = state.session.as_ref().map(|s| s.npc_id).unwrap_or(0);

    // the request this drop maps to, if any; None = no valid target here
    let request = if let Some(target) = hovered_cell {
        if target == source {
            // release over the picked cell: stay a click-carry; a click
            // there cancels
            if released {
                return;
            }
            None
        } else {
            pending.0 = Some(StorageOp::MoveInStorage);
            Some(InventoryOperationRequest::StorageToStorage {
                source,
                target,
                amount: stack_at(&state.items, source),
                npc_unique_id: npc_id,
            })
        }
    } else if let Some(target) = inv_state
        .hovered_slot
        .filter(|&slot| slot >= BAG_FIRST_SLOT)
    {
        pending.0 = Some(StorageOp::Withdraw);
        Some(InventoryOperationRequest::StorageToInventory {
            source,
            target,
            npc_unique_id: npc_id,
        })
    } else if inv_roots.iter().any(|hovered| hovered.get()) {
        // over the inventory window but no slot: first empty bag slot
        let target = inventories
            .single()
            .ok()
            .and_then(|inv| (BAG_FIRST_SLOT..inv.size()).find(|&slot| inv.get(slot).is_none()));
        match target {
            Some(target) => {
                pending.0 = Some(StorageOp::Withdraw);
                Some(InventoryOperationRequest::StorageToInventory {
                    source,
                    target,
                    npc_unique_id: npc_id,
                })
            }
            None => {
                info!("storage: no free bag slot to withdraw into");
                None
            }
        }
    } else {
        None
    };

    let data = carry.0.take().expect("checked above");
    commands.entity(data.ghost).despawn();
    let Some(request) = request else {
        return;
    };
    let Ok(conn) = conn.single() else {
        warn!("storage: no agent connection, dropping request");
        pending.0 = None;
        return;
    };
    info!("storage: sending {:?}", request);
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send storage request: {}", e.0);
    }
}

/// Dropping a carried inventory item onto the storage window deposits it —
/// slot-precise on a hovered cell, else into the first free storage slot.
/// The inventory's carry state and ghost are taken over here, so no 0x7034
/// op-0 move goes out for this drop (the sell-drop precedent).
#[allow(clippy::too_many_arguments)]
pub fn deposit_drop_on_storage(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<StorageState>,
    storage_roots: Query<&Hovered, With<StorageWindowRoot>>,
    storage_cells: Query<(&StorageSlotCell, &Hovered)>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
    mut pending: ResMut<PendingStorageOp>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) && !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(session) = &state.session else {
        return;
    };
    let Some(source) = inv_state.drag else {
        return;
    };
    if !storage_roots.iter().any(|hovered| hovered.get()) {
        return;
    }
    // Gate BEFORE consuming the carry: an unsynced drop (first-load latency)
    // must leave the item on the cursor, not silently destroy it.
    if !state.synced {
        info!("storage: not synced yet, ignoring deposit drop");
        return;
    }
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    let target = storage_cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(cell, _)| wire_slot(&state, cell.cell))
        .filter(|&slot| state.items.get(slot).is_none())
        .or_else(|| state.first_free_slot());
    let Some(target) = target else {
        info!("storage: no free storage slot to deposit into");
        return;
    };
    let request = InventoryOperationRequest::InventoryToStorage {
        source,
        target,
        npc_unique_id: session.npc_id,
    };
    let Ok(conn) = conn.single() else {
        warn!("storage: no agent connection, dropping deposit");
        return;
    };
    pending.0 = Some(StorageOp::Deposit);
    info!("storage: sending {:?}", request);
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send storage deposit: {}", e.0);
    }
}

/// The carry is bound to the storage session. Closing the popup with the
/// session moved to `gold_modal::close_modal_with_session` when the popup
/// became shared — it now has to close on *whichever* warehouse it pays into,
/// which this system cannot see.
pub fn clear_carry_with_storage(
    state: Res<StorageState>,
    mut carry: ResMut<StorageCarry>,
    mut commands: Commands,
) {
    if state.session.is_none() {
        if let Some(data) = carry.0.take() {
            commands.entity(data.ghost).despawn();
        }
    }
}

/// PostUpdate: despawn windows/modals marked [`StorageClosing`].
pub fn despawn_closing_storage(
    closing: Query<Entity, With<StorageClosing>>,
    mut commands: Commands,
) {
    for entity in closing.iter() {
        commands.entity(entity).despawn();
    }
}

/// OnExit cleanup.
#[allow(clippy::type_complexity)]
pub fn cleanup_storage(
    mut commands: Commands,
    windows: Query<
        Entity,
        Or<(
            With<StorageWindowRoot>,
            With<GoldModalRoot>,
            With<StorageGhost>,
        )>,
    >,
    mut state: ResMut<StorageState>,
    mut modal: ResMut<GoldModal>,
    mut carry: ResMut<StorageCarry>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
    state.session = None;
    state.items = Inventory::default();
    state.synced = false;
    modal.open = false;
    carry.0 = None;
}

#[cfg(test)]
mod test {
    use super::*;

    /// Three windows independently hand-derived the content origin and all
    /// three landed on the same wrong `y = 26`, so the guard is not "is 36
    /// right" but "is this window still derived from the shell at all" (#313).
    #[test]
    fn outer_window_matches_the_vanilla_registry_rect() {
        assert_eq!(
            game_window::outer_size((CONTENT_W, CONTENT_H)),
            VANILLA_OUTER
        );
    }

    /// Every interior rect must be its `ifstorageroom.txt` rect minus the
    /// shell's content origin — computed here from `game_window`'s own exports
    /// rather than restated, so a change over there fails this test instead of
    /// silently desyncing the window.
    #[test]
    fn interior_rects_sit_on_the_shells_content_origin() {
        // (vanilla x, vanilla y, ours) — GDR_STORAGE_LAT :740,
        // GDR_STORAGE_SPIN_PAGE :721, GDR_STORAGE_BGTILE :798,
        // GDR_STORAGE_BTN_MONEY :645.
        let cases = [
            (21.0, 71.0, LATTICE_RECT),
            (102.0, 254.0, SPIN_RECT),
            (25.0, 250.0, STRIP_RECT),
            (78.0, 286.0, MONEY_BTN_RECT),
        ];
        for (vanilla_x, vanilla_y, ours) in cases {
            assert_eq!(
                ours.0,
                vanilla_x - (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD),
                "x of vanilla {vanilla_x}"
            );
            assert_eq!(
                ours.1,
                vanilla_y - game_window::CONTENT_TOP,
                "y of vanilla {vanilla_y}"
            );
        }
    }

    /// Boots the real window as an open storage session does, so the test sees
    /// the entities `sync_storage_window` actually spawns.
    fn storage_app() -> App {
        let mut app = App::new();
        // AssetPlugin needs the IO task pool and `App::new()` does not create
        // it (see the same note in `inventory/ui.rs`).
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<ClientItemData>()
        .init_resource::<ClientUiStrings>()
        .insert_resource(FontAssets {
            one: Handle::default(),
            two: Handle::default(),
            three: Handle::default(),
            nine: Handle::default(),
        })
        .insert_resource(StorageState {
            session: Some(crate::plugins::hud::storage::model::StorageSession {
                npc: Entity::PLACEHOLDER,
                npc_id: 0,
                title: "Storage".into(),
            }),
            ..default()
        });
        app.world_mut().spawn(Camera2d);
        app.world_mut()
            .run_system_cached(sync_storage_window)
            .expect("sync_storage_window failed");
        app
    }

    /// Twin of `inventory/ui.rs`'s `every_child_of_a_slot_cell_is_unpickable`:
    /// the cell must stay the entity the picking backend reports. Storage does
    /// not read `Pointer<Over>.entity` today — it polls `Hovered`, which
    /// bevy_picking defines as "hovering the entity **or any of its
    /// descendants**" (`bevy_picking/src/hover.rs:319-326`), so a pickable
    /// child would not break `track_storage_hover` the way it breaks the
    /// inventory's `on_slot_over`. What it does break is everything that keys
    /// off the hit entity itself: `on_storage_slot_press` is observed on the
    /// cell, and `item_cell.rs` explicitly plans the next window as "one
    /// observer" on the inventory's model. So pin the invariant here rather
    /// than after a window has silently stopped reacting.
    #[test]
    fn every_child_of_a_storage_cell_is_unpickable() {
        let mut app = storage_app();
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<StorageSlotCell>>();
        let cells: Vec<Entity> = query.iter(app.world()).collect();
        assert_eq!(
            cells.len(),
            GRID_COLS * GRID_ROWS,
            "sync_storage_window did not build the 6x5 grid"
        );

        let mut stack: Vec<Entity> = cells
            .iter()
            .filter_map(|e| app.world().get::<Children>(*e))
            .flat_map(|c| c.iter())
            .collect();
        // Vacuity guard, and the honest reason for it: a cell only gets
        // children (icon, stack count, `+opt`) once itemdata resolves an icon
        // path, and `ClientItemData`'s payload is private to
        // `plugins::textdata` — a test can only build the empty table, so the
        // grid here is legitimately childless. If that ever changes, this
        // assert fires and the walk below becomes the real check instead of a
        // test that passes on zero nodes.
        assert!(
            stack.is_empty(),
            "storage cells gained children under an empty itemdata — extend \
             this test's fixture so the Pickable walk is not vacuous"
        );
        while let Some(entity) = stack.pop() {
            assert_eq!(
                app.world().get::<Pickable>(entity).copied(),
                Some(Pickable::IGNORE),
                "{entity} under a storage cell steals the hover hit"
            );
            if let Some(children) = app.world().get::<Children>(entity) {
                stack.extend(children.iter());
            }
        }
    }

    /// The money row is the one clamped rect: `GDR_STORAGE_STA_GOLD`'s
    /// `9,283,236,24` sits at content x -3, and the gold readout is placed
    /// relative to it with a hand-chosen offset, so only its y is data-exact.
    #[test]
    fn the_money_row_keeps_the_data_y_and_clamps_only_x() {
        assert_eq!(MONEY_RECT.1, 283.0 - game_window::CONTENT_TOP);
        assert_eq!((MONEY_RECT.0, MONEY_RECT.2), (0.0, CONTENT_W));
    }
}

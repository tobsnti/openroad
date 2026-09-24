//! Guild storage (guild warehouse) window — `GDR_GUILDSTORAGEROOM`.
//!
//! Idea: this window is deliberately **not a second design**. The original
//! registers `GDR_GUILDSTORAGEROOM:CIFStorageRoom` — ID 145, `Rect 0,0,254,317`,
//! frame `mframe_wnd_`, title `UIIT_STT_GUILD_WAREHOUSE`
//! (`resinfo/ginterface.txt:1353-1371`) — which is the *same class*, the *same
//! geometry* and the *same art* as the personal warehouse
//! `GDR_STORAGEROOM` ID 19 (`:560-578`), and there is deliberately **no
//! `ifguildstorageroom.txt`** in resinfo (247 files; only `ifstorageroom.txt`
//! and `ifitemmallstorageroom.txt` exist). One `CIFStorageRoom` class with one
//! interior layout file serves both windows, so every interior rect below is
//! its `ifstorageroom.txt` rect — cited per constant — and not a fresh
//! derivation. The guild window is therefore not a new window: it is
//! `hud/storage/ui.rs` with a different title and data source.
//!
//! What it closes: the dead wire recorded in `guild-storage.md` §5. A
//! WAREHOUSE NPC offers `UIIT_CTL_GUILD_WAREHOUSE`
//! (`npc_dialog/ui.rs:356-363`), the model fetches the entire guild warehouse
//! into `GuildStorageState.items` — and until this file existed the player saw
//! nothing at all.
//!
//! **Items and gold both move now.** The item half came first; why the money
//! button was left out for one lap is recorded below. The item half is wired:
//! a carry off the guild grid, a drop on a bag slot and a drop of an
//! inventory carry on this window send `0x7034`
//! ops **29/30/31**, whose bodies are known: they are read out of
//! the original client's own request builder + serializer
//! (`packets/src/agent/inventory.rs`, `InventoryOperationRequest` doc —
//! the original's item-move builder, gold builder and inventory-op
//! serializer), which
//! settles `guild-storage.md` §9.4 for both the guild and the personal
//! family. Applied server-confirmed only, in `model.rs`.
//!
//! The money button (`GDR_STORAGE_BTN_MONEY`, ops **32/33**) was deliberately
//! absent for one lap rather than drawn dead: the encoders existed, but the
//! gold popup lived in `storage/ui.rs` bound to `StorageState`, so giving the
//! guild window one meant parameterising that popup over both warehouses
//! first. That refactor happened — the popup is now
//! `storage/gold_modal.rs`, one implementation with a [`GoldTarget`], and
//! this window draws the button and points it at
//! [`GoldTarget::GuildStorage`]. Gold acks stay on the guild side only; the
//! player's own total comes from 0x304E (`model::on_guild_storage_operation`,
//! the double-booking trap the personal warehouse already paid for).
//!
//! Not persisted in `wndpos.dat`: the original's file has ten slots
//! (`settings/window_positions.rs:36-49`, transcribed in file order) and none
//! of them is the guild warehouse, so the window keeps a dragged position for
//! the session and takes its spawn anchor otherwise.

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
use crate::plugins::hud::guild_storage::model::GuildStorageState;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::{DragGhost, InventoryRoot};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::storage::gold_modal::{GoldModal, GoldTarget};
use crate::plugins::hud::storage::model::STORAGE_SLOTS_PER_PAGE;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::{Inventory, BAG_FIRST_SLOT};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// `GDR_GUILDSTORAGEROOM` (`ginterface.txt:1362`, id 145) — byte-identical to
/// `GDR_STORAGEROOM`'s `0,0,254,317` (`:569`).
const VANILLA_OUTER: (f32, f32) = (254.0, 317.0);

/// Content-space origin imported from the shell, never re-derived — the `(12,
/// 26)` mistake three windows made independently is written up in
/// `storage/ui.rs` (#313).
const ORIGIN_X: f32 = game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD;
const ORIGIN_Y: f32 = game_window::CONTENT_TOP;

/// An `ifstorageroom.txt` window-space rect in content space, so the vanilla
/// numbers stay readable at the call sites.
const fn content_rect(rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (rect.0 - ORIGIN_X, rect.1 - ORIGIN_Y, rect.2, rect.3)
}

const CONTENT_W: f32 = VANILLA_OUTER.0 - 2.0 * ORIGIN_X;
const CONTENT_H: f32 = VANILLA_OUTER.1
    - game_window::CONTENT_TOP
    - game_window::CHROME_PAD
    - game_window::FRAME_VIS_BOTTOM;

/// Both warehouses are offered by the *same* NPC as two dialog lines, so both
/// sessions can be open at once and an identical anchor would hide one under
/// the other. Cascaded by 24 px from `storage/ui.rs`'s `(560, 60)` — a
/// placement choice, not a transcribed number: `wndpos.dat` has no slot for
/// this window (see the module note), so the original has nothing to copy here.
const WINDOW_RIGHT: f32 = 536.0;
const WINDOW_TOP: f32 = 84.0;

const GRID_COLS: usize = 6;
const GRID_ROWS: usize = 5;
/// 6x5 = 30 = `STORAGE_SLOTS_PER_PAGE`, asserted below rather than assumed.
const CELL: f32 = 36.0;
const ICON_INSET: f32 = 2.0;
const ICON_SIZE: f32 = 32.0;
/// `GDR_STORAGE_LAT` (`ifstorageroom.txt:740`).
const LATTICE_RECT: (f32, f32, f32, f32) = content_rect((21.0, 71.0, 216.0, 180.0));
/// `GDR_STORAGE_SPIN_PAGE` (`:721`).
const SPIN_RECT: (f32, f32, f32, f32) = content_rect((102.0, 254.0, 50.0, 16.0));
/// `GDR_STORAGE_BGTILE` (`:798`).
const STRIP_RECT: (f32, f32, f32, f32) = content_rect((25.0, 250.0, 204.0, 13.0));
/// The money row — `GDR_STORAGE_STA_GOLD` / `_STA_MONEY` share `9,283,236,24`
/// (`:664` / `:683`), whose content-space x is -3. Clamped to the content box
/// for the reason `storage/ui.rs` states: the readout beside it is placed at a
/// hand-chosen offset with no data target of its own, so only the y is exact.
const MONEY_RECT: (f32, f32, f32, f32) = (
    0.0,
    content_rect((9.0, 283.0, 236.0, 24.0)).1,
    CONTENT_W,
    24.0,
);
/// `GDR_STORAGE_BTN_MONEY` (`ifstorageroom.txt:645`) — the same rect the
/// personal warehouse uses, because it is the same layout file for the same
/// `CIFStorageRoom` class (module note).
const MONEY_BTN_RECT: (f32, f32, f32, f32) = content_rect((78.0, 286.0, 20.0, 20.0));

const LATTICE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_";
const MONEY_TILE: &str = "media://interface/store/str_slot_01.ddj";

/// The two text colours the personal warehouse uses, kept identical because
/// this is the same `CIFStorageRoom` window with another title.
const NAME_COLOR: Color = Color::srgb_u8(255, 226, 123);
const PRICE_COLOR: Color = Color::srgb_u8(255, 217, 83);

#[derive(Component)]
pub struct GuildStorageWindowRoot;

/// Deferred despawn marker (the storage/store precedent: despawning inside the
/// observer that closed the window tears down the entity being read).
#[derive(Component)]
pub struct GuildStorageClosing;

#[derive(Component)]
pub struct GuildStorageSlotCell {
    /// Index within the visible page grid.
    cell: usize,
}

/// `GDR_STORAGE_BTN_MONEY` on this window.
#[derive(Component)]
struct GuildMoneyButton;

#[derive(Component)]
enum GuildStoragePageButton {
    Prev,
    Next,
}

/// The guild wire slot a page-grid cell shows.
fn wire_slot(state: &GuildStorageState, cell: usize) -> u8 {
    state.active_page * STORAGE_SLOTS_PER_PAGE + cell as u8
}

/// Rebuild the window whenever [`GuildStorageState`] changes (open, close,
/// page flip, every arriving 0x3254). A dragged position survives the rebuild.
#[allow(clippy::too_many_arguments)]
pub fn sync_guild_storage_window(
    state: Res<GuildStorageState>,
    existing: Query<(Entity, &Node), With<GuildStorageWindowRoot>>,
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
    let mut anchor = (WINDOW_RIGHT, WINDOW_TOP);
    for (entity, node) in existing.iter() {
        if let (Val::Px(right), Val::Px(top)) = (node.right, node.top) {
            anchor = (right, top);
        }
        commands.entity(entity).insert(GuildStorageClosing);
    }
    if state.session.is_none() {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("guild storage: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        // `ginterface.txt:1365` names the title key; the text is
        // `textuisystem.txt:1426` "Guild storage".
        ui_strings.get_or("UIIT_STT_GUILD_WAREHOUSE", "Guild storage"),
        (CONTENT_W, CONTENT_H),
        anchor,
        s,
    );
    commands.entity(window.root).insert((
        GuildStorageWindowRoot,
        // one above the personal warehouse (z=58): the two can be open
        // together at the same NPC and the guild window is the one just opened
        GlobalZIndex(59),
        Hovered::default(),
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

        // caption over the grid. `UIIT_STT_DEPOSITTEDITEM` ("Deposit Items",
        // `textuisystem.txt:647`) is the personal window's own sframe caption
        // and the only storage-room caption the data has — there is no
        // guild-specific variant.
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
                        GuildStorageSlotCell { cell },
                        Hovered::default(),
                        Node {
                            width: Val::Px(CELL * s),
                            height: Val::Px(CELL * s),
                            ..default()
                        },
                        ImageNode {
                            image: asset_server.load(format!("{LATTICE_DIR}{quarter}.ddj")),
                            image_mode: NodeImageMode::Stretch,
                            // dim the grid until the 0x3254 end marker parsed
                            color: if state.synced {
                                Color::WHITE
                            } else {
                                Color::srgb(0.55, 0.55, 0.55)
                            },
                            ..default()
                        },
                    ));
                    cell_cmd.observe(on_guild_slot_press);
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

        // page spinner — `GDR_STORAGE_SPIN_PAGE`'s CIFSpinButtonCtrl anatomy
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
            (GuildStoragePageButton::Prev, "left", SPIN_RECT.0),
            (GuildStoragePageButton::Next, "right", SPIN_RECT.0 + 34.0),
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
                .observe(on_page_press);
        }
        content.spawn((
            Text::new(format!("{}", state.active_page + 1)),
            text_font(8.0),
            TextColor(Color::srgb(0.85, 0.85, 0.85)),
            TextLayout::justify(Justify::Center),
            abs_node((SPIN_RECT.0 + 16.0, SPIN_RECT.1 + 2.0, 18.0, 12.0), s),
            Pickable::IGNORE,
        ));

        // money row: the guild account's gold (0x3253's `u64`) plus
        // `GDR_STORAGE_BTN_MONEY` (`:645`), which now opens the shared gold
        // popup for the guild side (ops 32/33) — see the module note.
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
        // com_moneybutton has no focus/press art variants (the personal
        // warehouse's own note), so the style reuses the base texture.
        let money_art: Handle<Image> =
            asset_server.load("media://interface/ifcommon/com_moneybutton.ddj");
        content
            .spawn((
                GuildMoneyButton,
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

/// The money button only names its own warehouse — the popup is the shared
/// `storage::gold_modal` unit, which turns Store/Take into ops 32/33 for this
/// target. Nothing is applied here: the 0xB034 ack is what moves gold, and it
/// moves it on the guild side only.
fn on_money_button(_: On<Activate>, mut modal: ResMut<GoldModal>) {
    modal.open_for(GoldTarget::GuildStorage);
}

/// The (X) ends the conversation like the personal warehouse's does, with one
/// difference that matters: the session is **not** taken here. Guild storage is
/// a guild-wide exclusive lock, and `model::close_guild_storage_with_dialog` is
/// what sends the 0x7251 that releases it — it fires precisely because the
/// dialog stops naming this NPC. Clearing the session in this observer would
/// swallow the close and lock the warehouse for the whole guild.
fn on_close_button(
    _: On<Activate>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    state: Res<GuildStorageState>,
    mut dialog: ResMut<crate::plugins::hud::npc_dialog::model::NpcDialogState>,
) {
    if let Some(session) = state.session.as_ref() {
        crate::plugins::hud::npc_dialog::model::send_close_to(&conn, session.npc_id);
    }
    *dialog = crate::plugins::hud::npc_dialog::model::NpcDialogState::Closed;
}

fn on_page_press(
    activate: On<Activate>,
    buttons: Query<&GuildStoragePageButton>,
    mut state: ResMut<GuildStorageState>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    let pages = state.page_count();
    match button {
        GuildStoragePageButton::Prev if state.active_page > 0 => state.active_page -= 1,
        GuildStoragePageButton::Next if state.active_page + 1 < pages => state.active_page += 1,
        _ => {}
    }
}

/// The active guild-warehouse carry (an item picked off the guild grid) and
/// its ghost icon. Separate resource from [`super::super::storage::ui::StorageCarry`]
/// on purpose: both windows can be open at the same NPC, and one shared carry
/// would let a guild pickup finish as a personal-warehouse drop.
#[derive(Resource, Default)]
pub struct GuildStorageCarry(pub Option<GuildStorageCarryData>);

pub struct GuildStorageCarryData {
    /// The carried item's guild-warehouse wire slot.
    pub slot: u8,
    pub ghost: Entity,
    /// True until the pickup press's own just_pressed frame has passed.
    pub just_picked: bool,
}

/// The carry's cursor-following icon.
#[derive(Component)]
pub struct GuildStorageGhost;

/// The stack size a whole-stack guild move reports — borrowed from the
/// personal warehouse rather than re-derived (`storage::ui::stack_at`: the
/// original feeds the source item's own count into the op-1/29 request).
use crate::plugins::hud::storage::ui::stack_at;

/// Press on a guild cell: begin the guild carry. Mirrors
/// `storage::ui::on_storage_slot_press`, including backing off while an
/// INVENTORY drag is live so [`deposit_drop_on_guild_storage`] handles that
/// drop alone.
#[allow(clippy::too_many_arguments)]
fn on_guild_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&GuildStorageSlotCell>,
    state: Res<GuildStorageState>,
    inv_state: Res<InventoryState>,
    item_data: Res<ClientItemData>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut carry: ResMut<GuildStorageCarry>,
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
        GuildStorageGhost,
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
    carry.0 = Some(GuildStorageCarryData {
        slot,
        ghost: ghost.id(),
        just_picked: true,
    });
}

/// Cursor-follow for the guild-carry ghost.
pub fn update_guild_storage_ghost(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut ghosts: Query<&mut Node, With<GuildStorageGhost>>,
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

/// Route the guild carry's drop: another guild cell → op 29; a bag slot → op
/// 31 there; the inventory window body → op 31 into the first empty bag slot;
/// anywhere else cancels. Release over the picked cell keeps the carry alive
/// as a click-carry. Line for line the personal warehouse's
/// `finish_storage_carry` with the guild ops substituted.
#[allow(clippy::too_many_arguments)]
pub fn finish_guild_storage_carry(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<GuildStorageState>,
    inv_state: Res<InventoryState>,
    guild_cells: Query<(&GuildStorageSlotCell, &Hovered)>,
    inv_roots: Query<&Hovered, With<InventoryRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut carry: ResMut<GuildStorageCarry>,
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
        if let Some(data) = carry.0.as_mut() {
            data.just_picked = false;
        }
        return;
    }

    let hovered_cell = guild_cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(cell, _)| wire_slot(&state, cell.cell));
    let source = data.slot;
    let npc_id = state.session.as_ref().map(|s| s.npc_id).unwrap_or(0);

    let request = if let Some(target) = hovered_cell {
        if target == source {
            if released {
                return;
            }
            None
        } else {
            Some(InventoryOperationRequest::GuildStorageToGuildStorage {
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
        Some(InventoryOperationRequest::GuildStorageToInventory {
            source,
            target,
            npc_unique_id: npc_id,
        })
    } else if inv_roots.iter().any(|hovered| hovered.get()) {
        let target = inventories
            .single()
            .ok()
            .and_then(|inv| (BAG_FIRST_SLOT..inv.size()).find(|&slot| inv.get(slot).is_none()));
        match target {
            Some(target) => Some(InventoryOperationRequest::GuildStorageToInventory {
                source,
                target,
                npc_unique_id: npc_id,
            }),
            None => {
                info!("guild storage: no free bag slot to withdraw into");
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
        warn!("guild storage: no agent connection, dropping request");
        return;
    };
    info!("guild storage: sending {:?}", request);
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send guild storage request: {}", e.0);
    }
}

/// Dropping a carried inventory item onto the guild window deposits it (op 30)
/// — slot-precise on a hovered cell, else into the first free guild slot. The
/// inventory carry is consumed here so no op-0 move goes out for this drop.
#[allow(clippy::too_many_arguments)]
pub fn deposit_drop_on_guild_storage(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<GuildStorageState>,
    guild_roots: Query<&Hovered, With<GuildStorageWindowRoot>>,
    guild_cells: Query<(&GuildStorageSlotCell, &Hovered)>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
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
    if !guild_roots.iter().any(|hovered| hovered.get()) {
        return;
    }
    // Gate BEFORE consuming the carry (the personal warehouse's rule): an
    // unsynced drop must leave the item on the cursor, not destroy it.
    if !state.synced {
        info!("guild storage: not synced yet, ignoring deposit drop");
        return;
    }
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    let target = guild_cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(cell, _)| wire_slot(&state, cell.cell))
        .filter(|&slot| state.items.get(slot).is_none())
        .or_else(|| first_free_slot(&state.items));
    let Some(target) = target else {
        info!("guild storage: no free guild slot to deposit into");
        return;
    };
    let request = InventoryOperationRequest::InventoryToGuildStorage {
        source,
        target,
        npc_unique_id: session.npc_id,
    };
    let Ok(conn) = conn.single() else {
        warn!("guild storage: no agent connection, dropping deposit");
        return;
    };
    info!("guild storage: sending {:?}", request);
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send guild storage deposit: {}", e.0);
    }
}

/// First empty guild slot, or `None` when the warehouse is full.
fn first_free_slot(items: &Inventory) -> Option<u8> {
    (0..items.slots.len() as u8).find(|&slot| items.get(slot).is_none())
}

/// The carry is bound to the session: closing the warehouse drops it.
pub fn clear_carry_with_guild_storage(
    state: Res<GuildStorageState>,
    mut carry: ResMut<GuildStorageCarry>,
    mut commands: Commands,
) {
    if state.session.is_none() {
        if let Some(data) = carry.0.take() {
            commands.entity(data.ghost).despawn();
        }
    }
}

/// Publish the hovered guild item for the shared item tooltip.
///
/// Unlike `storage::ui::track_storage_hover` this **only clears what it set
/// itself** (`published`): both warehouses can be open at the same NPC, and two
/// pollers that each clear the shared resource when their own grid is unhovered
/// would fight over it every frame and blank the other window's tooltip.
pub fn track_guild_storage_hover(
    state: Res<GuildStorageState>,
    cells: Query<(&GuildStorageSlotCell, &Hovered)>,
    mut published: Local<bool>,
    mut hovered: ResMut<crate::plugins::hud::item_cell::HoveredItem>,
) {
    let item = cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .and_then(|(cell, _)| state.items.get(wire_slot(&state, cell.cell)));
    match item {
        Some(item) => {
            if hovered.owned() != Some(item) {
                hovered.0 = Some(crate::plugins::hud::item_cell::HoveredItemKind::Owned(
                    item.clone(),
                ));
            }
            *published = true;
        }
        None if *published => {
            hovered.clear();
            *published = false;
        }
        None => {}
    }
}

/// PostUpdate: despawn windows marked [`GuildStorageClosing`].
pub fn despawn_closing_guild_storage(
    closing: Query<Entity, With<GuildStorageClosing>>,
    mut commands: Commands,
) {
    for entity in closing.iter() {
        commands.entity(entity).despawn();
    }
}

/// OnExit cleanup — the window and the mirrored contents both go.
pub fn cleanup_guild_storage(
    mut commands: Commands,
    windows: Query<Entity, With<GuildStorageWindowRoot>>,
    mut state: ResMut<GuildStorageState>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
    state.session = None;
    state.items = Inventory::default();
    state.synced = false;
    state.active_page = 0;
}

#[cfg(test)]
mod test {
    use super::*;

    use packets::agent::guild::{GuildData, GuildMember};

    use crate::plugins::hud::chat::model::ChatHistory;
    use crate::plugins::hud::guild_storage::model::{
        open_guild_storage, GuildStorageDataBuffer, GuildStorageSession,
    };
    use crate::plugins::hud::npc_dialog::model::OpenGuildStorage;
    use crate::plugins::net::character_info::CharacterInfo;
    use crate::plugins::net::entities::NetworkId;
    use crate::plugins::net::guild::GuildRoster;
    use crate::plugins::player::Player;

    /// `GDR_GUILDSTORAGEROOM`'s `Rect 0,0,254,317` (`ginterface.txt:1362`) has
    /// to come back out of the shell's own size arithmetic; the guard is "is
    /// this window still derived from the shell at all" (#313).
    #[test]
    fn outer_window_matches_the_vanilla_registry_rect() {
        assert_eq!(
            game_window::outer_size((CONTENT_W, CONTENT_H)),
            VANILLA_OUTER
        );
    }

    /// Every interior rect is its `ifstorageroom.txt` rect minus the shell's
    /// content origin — the same file the personal warehouse reads, because the
    /// guild window is the same `CIFStorageRoom` class and resinfo has no
    /// `ifguildstorageroom.txt` (module note). Computed from `game_window`'s
    /// exports so a change over there fails here instead of desyncing silently.
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
        // the money row is the one clamped rect, exactly as in storage/ui.rs
        assert_eq!(MONEY_RECT.1, 283.0 - game_window::CONTENT_TOP);
        assert_eq!((MONEY_RECT.0, MONEY_RECT.2), (0.0, CONTENT_W));
    }

    /// The money button exists **and** is bound to the guild side. It was
    /// deliberately absent while the gold popup could only serve the personal
    /// warehouse (module note); now that the popup takes a [`GoldTarget`],
    /// this is the guard against both regressions: the button vanishing
    /// again, and the button opening a popup that would pay into the
    /// *personal* warehouse (ops 12/11) from the guild window.
    #[test]
    fn the_money_button_opens_the_shared_popup_on_the_guild_side() {
        let mut app = window_app();
        app.init_resource::<GoldModal>();
        app.world_mut().resource_mut::<GuildStorageState>().session = Some(GuildStorageSession {
            npc: Entity::PLACEHOLDER,
            npc_id: 4711,
        });
        app.world_mut()
            .run_system_cached(sync_guild_storage_window)
            .expect("sync_guild_storage_window failed");

        let mut buttons = app
            .world_mut()
            .query_filtered::<Entity, With<GuildMoneyButton>>();
        let button: Vec<Entity> = buttons.iter(app.world()).collect();
        assert_eq!(
            button.len(),
            1,
            "GDR_STORAGE_BTN_MONEY is not drawn on the guild warehouse"
        );
        assert!(
            !app.world().resource::<GoldModal>().open,
            "the popup must not open by itself"
        );
        app.world_mut().trigger(Activate { entity: button[0] });
        let modal = app.world().resource::<GoldModal>();
        assert!(modal.open, "the money button did not open the gold popup");
        assert_eq!(
            modal.target,
            GoldTarget::GuildStorage,
            "the guild window's money button must pay into the GUILD warehouse"
        );
    }

    /// The grid must page the way the *wire* pages: `wire_slot` multiplies the
    /// page by `STORAGE_SLOTS_PER_PAGE`, so a 6x5 grid and a 30-slot page are
    /// the same number or page 2 skips slots.
    #[test]
    fn the_page_grid_is_exactly_one_wire_page() {
        assert_eq!(GRID_COLS * GRID_ROWS, STORAGE_SLOTS_PER_PAGE as usize);
        let mut state = GuildStorageState {
            session: Some(GuildStorageSession {
                npc: Entity::PLACEHOLDER,
                npc_id: 7,
            }),
            ..default()
        };
        assert_eq!(wire_slot(&state, 0), 0);
        assert_eq!(wire_slot(&state, 29), 29);
        state.active_page = 1;
        assert_eq!(wire_slot(&state, 0), 30);
    }

    /// AssetPlugin needs the IO task pool and `App::new()` does not create it
    /// (same note as `inventory/ui.rs`).
    fn window_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<ClientItemData>()
        .init_resource::<ClientUiStrings>()
        .init_resource::<GuildStorageState>()
        .init_resource::<GuildStorageDataBuffer>()
        .init_resource::<GuildRoster>()
        .init_resource::<ChatHistory>()
        .insert_resource(FontAssets {
            one: Handle::default(),
            two: Handle::default(),
            three: Handle::default(),
            nine: Handle::default(),
        });
        app.world_mut().spawn(Camera2d);
        app
    }

    fn member(name: &str, permissions: u32) -> GuildMember {
        GuildMember {
            member_id: 1,
            name: name.into(),
            unk_u8_01: 0,
            level: 20,
            guild_points: 0,
            permissions,
            unk_u32_01: 0,
            unk_u32_02: 0,
            unk_u32_03: 0,
            nickname: String::new(),
            model_id: 0,
            is_master: false,
            is_offline: false,
        }
    }

    fn guild(level: u8, members: Vec<GuildMember>) -> GuildData {
        GuildData {
            guild_id: 42,
            name: "Testers".into(),
            level,
            guild_points: 0,
            notice: String::new(),
            message: String::new(),
            unk_u32_00: 0,
            unk_u8_00: 0,
            member_count: members.len() as u8,
            members,
        }
    }

    /// **The dead-wire test.** The `UIIT_CTL_GUILD_WAREHOUSE` dialog line fired `OpenGuildStorage`,
    /// the model filled `GuildStorageState`, and the player saw *nothing*
    /// because no window existed. So drive the real systems in registered order
    /// — message in, window out — and count the cells. If someone deletes the
    /// window again, this fails instead of the defect quietly returning.
    #[test]
    fn the_dialog_line_now_opens_a_window() {
        let mut app = window_app();
        app.add_message::<OpenGuildStorage>().add_systems(
            Update,
            (open_guild_storage, sync_guild_storage_window).chain(),
        );
        // a member of a level-2 guild holding the Storage bit — the gate the
        // original's own three strings describe (model::gate)
        app.world_mut().resource_mut::<GuildRoster>().data = Some(guild(
            2,
            vec![member(
                "Tester",
                packets::agent::guild::GuildPermissions::STORAGE,
            )],
        ));
        app.world_mut().spawn((
            Player,
            CharacterInfo {
                name: Some("Tester".into()),
                ..default()
            },
        ));
        let npc = app.world_mut().spawn(NetworkId(4711)).id();

        app.world_mut().write_message(OpenGuildStorage { npc });
        app.update();

        let state = app.world().resource::<GuildStorageState>();
        assert_eq!(
            state.session.as_ref().map(|s| s.npc_id),
            Some(4711),
            "the gate let the open through (0x7250 needs no connection to open the session)"
        );
        let mut roots = app
            .world_mut()
            .query_filtered::<Entity, With<GuildStorageWindowRoot>>();
        assert_eq!(
            roots.iter(app.world()).count(),
            1,
            "no guild storage window was built — the dead wire is back"
        );
        let mut cells = app
            .world_mut()
            .query_filtered::<Entity, With<GuildStorageSlotCell>>();
        assert_eq!(
            cells.iter(app.world()).count(),
            GRID_COLS * GRID_ROWS,
            "the 6x5 lattice did not build"
        );
    }

    /// A refused open must build no window: the refusal is a chat line, and a
    /// window over an empty state would claim the warehouse opened.
    #[test]
    fn a_refused_open_builds_no_window() {
        let mut app = window_app();
        app.add_message::<OpenGuildStorage>().add_systems(
            Update,
            (open_guild_storage, sync_guild_storage_window).chain(),
        );
        // level 1: UIIT_MSG_GUILD_WAREHOUSE_LIMIT, wire code 0x4C4A
        app.world_mut().resource_mut::<GuildRoster>().data = Some(guild(
            1,
            vec![member(
                "Tester",
                packets::agent::guild::GuildPermissions::ALL,
            )],
        ));
        app.world_mut().spawn((
            Player,
            CharacterInfo {
                name: Some("Tester".into()),
                ..default()
            },
        ));
        let npc = app.world_mut().spawn(NetworkId(4711)).id();
        app.world_mut().write_message(OpenGuildStorage { npc });
        app.update();

        assert!(app
            .world()
            .resource::<GuildStorageState>()
            .session
            .is_none());
        let mut roots = app
            .world_mut()
            .query_filtered::<Entity, With<GuildStorageWindowRoot>>();
        assert_eq!(roots.iter(app.world()).count(), 0);
    }

    /// The exclusivity rule (`npc_dialog::ui::hide_dialog_while_store_open`):
    /// an NPC service window swaps IN PLACE OF the dialog. Guild storage was
    /// deliberately absent from that list while it had no window; now it is a
    /// service window, and this pins it.
    #[test]
    fn an_open_guild_warehouse_hides_the_dialog() {
        use crate::plugins::hud::npc_dialog::model::{DialogPage, NpcDialogState};
        use crate::plugins::hud::npc_dialog::ui::{hide_dialog_while_store_open, NpcDialogRoot};

        let mut app = App::new();
        app.init_resource::<crate::plugins::hud::store::model::StoreState>()
            .init_resource::<crate::plugins::hud::storage::model::StorageState>()
            .init_resource::<GuildStorageState>();
        let npc = app.world_mut().spawn_empty().id();
        let dialog = app
            .world_mut()
            .spawn((NpcDialogRoot, Visibility::Inherited))
            .id();
        app.insert_resource(NpcDialogState::Open {
            npc,
            page: DialogPage::Options,
        });
        app.world_mut().resource_mut::<GuildStorageState>().session =
            Some(GuildStorageSession { npc, npc_id: 4711 });

        app.world_mut()
            .run_system_cached(hide_dialog_while_store_open)
            .expect("hide_dialog_while_store_open failed");
        assert_eq!(
            app.world().get::<Visibility>(dialog),
            Some(&Visibility::Hidden),
            "the guild warehouse is a service window and must swap in place of the dialog"
        );

        // and a session at a DIFFERENT npc must not hide a fresh dialog (#217)
        let other = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<GuildStorageState>().session = Some(GuildStorageSession {
            npc: other,
            npc_id: 99,
        });
        app.world_mut()
            .run_system_cached(hide_dialog_while_store_open)
            .expect("hide_dialog_while_store_open failed");
        assert_eq!(
            app.world().get::<Visibility>(dialog),
            Some(&Visibility::Inherited)
        );
    }
}

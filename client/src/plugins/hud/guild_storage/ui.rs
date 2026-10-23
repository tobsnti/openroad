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
//! derivation. `docs/re/systems/guild-storage.md` §4 draws the same
//! conclusion: "the guild window is not a new window to port: it is
//! `hud/storage/ui.rs` with a different title and data source."
//!
//! What it closes: the dead wire recorded in `guild-storage.md` §5. A
//! WAREHOUSE NPC offers `UIIT_CTL_GUILD_WAREHOUSE`
//! (`npc_dialog/ui.rs`), the model fetches the entire guild warehouse into
//! `GuildStorageState.items` — and until this file existed the player saw
//! nothing at all.
//!
//! **This lap draws and pages; it moves nothing.** The grid, the page spinner
//! and the (X) work. Carry/drop and the money button are the next two slices;
//! the wire they will use already exists (`0x7034` ops 29..=33, see
//! `packets/src/agent/inventory.rs`). Nothing here is drawn dead that the
//! player could click expecting a transfer — the money button is simply not
//! spawned yet, rather than spawned inert.
//!
//! One open question carried over from the wire slice: op 29
//! (`GuildStorageToGuildStorage`) is modelled with an `amount`, op 1
//! (`StorageToStorage`) without, and **neither has a captured frame**. A
//! capture of one warehouse move settles both at once.
//!
//! Not persisted in `wndpos.dat`: the original's file has ten slots
//! (`settings/window_positions.rs`, transcribed in file order) and none of
//! them is the guild warehouse, so the window keeps a dragged position for
//! the session and takes its spawn anchor otherwise.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::LineHeight;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::character_data::ItemTypeData;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::guild_storage::model::GuildStorageState;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::storage::model::STORAGE_SLOTS_PER_PAGE;
use crate::plugins::net::agent::AgentConnection;
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

    // Every field in this window is a single line inside a rect the data
    // authored, so the line box has to be the glyph box: without an explicit
    // `LineHeight`, bevy's default leading adds a few pixels and the text
    // drifts out of a 13..16-unit strip (PR #180).
    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        line_height: LineHeight::Px(size * s),
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
                    // No observer in this lap: picking an item up is 1c, and
                    // a cell that answers a click by doing nothing is exactly
                    // what this window set out not to ship.
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
    });
}

/// what sends the 0x7251 that releases it — it fires precisely because the
/// dialog stops naming this NPC. Clearing the session in this observer would
/// swallow the close and lock the warehouse for the whole guild
/// (`docs/re/systems/guild-storage.md` §11/§15 S4).
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

/// PostUpdate: despawn windows marked [`GuildStorageClosing`].
pub fn despawn_closing_guild_storage(
    closing: Query<Entity, With<GuildStorageClosing>>,
    mut commands: Commands,
) {
    for entity in closing.iter() {
        commands.entity(entity).despawn();
    }
}

//! COS inventory page — `resinfo/ifcosinventory.txt`, the second of the
//! shell's three pages (`GGDR_COS_INVENTORY:CIFCOSInventory`, id 122).
//!
//! Two deliberate gaps, so nothing here is invented:
//! - `GDR_COS_INV_STRETCH` (`com_lattice_outline_`) is **not** drawn: the kit
//!   ships six 4x4 pieces (2 corners short of a ring, no `mid_*` at all), so
//!   what the horizontal edges are made of is not known. The lattice tiles
//!   carry their own rim, so the grid reads correctly without it.
//! - the `Section = TradeInfo` half (trade level stars, total gold, "sell
//!   all") belongs to the NPC specialty-shop view of the same grid and has no
//!   state to show yet; it is left out rather than half-drawn here.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use bevy::ui::UiTargetCamera;

use packets::agent::character_data::ItemTypeData;
use packets::agent::prelude::InventoryOperationRequest;
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::cos::model::{CosPage, CosWindowState};
use crate::plugins::hud::cos::state::CosState;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::{DragGhost, InventoryRoot};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::{Inventory, BAG_FIRST_SLOT};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

// --- Layout constants (page space, one per GDR_* block) ---------------------

const ART: &str = "media://interface/";

/// `GDR_COS_INV_GFRAME:CIFFrame` id 2 — `frameg_wnd_` ring, same rect and same
/// measured piece sizes as the info page (corners 24x16).
const GFRAME_RECT: (f32, f32, f32, f32) = (8.0, 9.0, 315.0, 297.0);
const GFRAME_SIDE: f32 = 24.0;
const GFRAME_TOP: f32 = 16.0;
/// `GDR_COS_INV_BG:CIFNormalTile` id 3 — `com_bg_tile_b` fill.
const BG_RECT: (f32, f32, f32, f32) = (32.0, 25.0, 267.0, 265.0);
/// `GDR_COS_INV_INNER_BOX:CIFFrame` id 4 — `opt_inner_box_`, drawn as the 4px
/// ring `setup.rs` established for the same kit.
const INNER_BOX_RECT: (f32, f32, f32, f32) = (21.0, 27.0, 290.0, 235.0);
/// The sunken panel is an 8-piece kit under `interface/option/` — the same one
/// [`super::setup`] draws, and the folder the kit actually lives in (loading a
/// single `ifcommon/opt_inner_box_middle.ddj` drew nothing and logged a missing
/// asset on every open).
const INNER_BOX_DIR: &str = "media://interface/option/opt_inner_box_";
const INNER_BOX_PIECE: f32 = 4.0;
/// `GDR_COS_INV_NAME:CIFStatic` id 6, `HAlign=1` (centred) — the COS's name.
const NAME_RECT: (f32, f32, f32, f32) = (49.0, 30.0, 237.0, 12.0);
/// `GDR_COS_INV_LAT:CIFLattice` id 15.
const LATTICE_RECT: (f32, f32, f32, f32) = (41.0, 59.0, 252.0, 144.0);
/// `com_lattice_*.ddj` is 36x36; 252/36 = 7 and 144/36 = 4, so the authored
/// rect *is* a 7x4 grid of whole tiles.
const CELL: f32 = 36.0;
pub const GRID_COLS: u8 = 7;
pub const GRID_ROWS: u8 = 4;
/// Cells per page — the number the spinner pages through. Named once, in the
/// packet crate, because the original's own slot math pages by it too
/// (`COS_INVENTORY_PAGE_SLOTS`); the assertion below is what ties it to the
/// authored 7x4 rect.
pub use packets::agent::pet::COS_INVENTORY_PAGE_SLOTS as SLOTS_PER_PAGE;
const LATTICE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_";
/// The slot hole inside a lattice tile spans [2, 30) — the same inset the
/// player inventory uses for the same tiles (`hud/inventory/ui.rs`).
const ICON_INSET: f32 = 2.0;
const ICON_SIZE: f32 = 28.0;
/// `GDR_COS_INV_SPIN_PAGE:CIFSpinButtonCtrl` id 17. The block carries
/// `DDJ=STRING,""`, i.e. the original's spin control brings its own art; ours
/// is the shipped 16x16 `com_left_arrow`/`com_right_arrow` pair with the page
/// number between them, which fits the authored 52x18 rect exactly
/// (16 + 20 + 16) — the arrows are vanilla art, the split is ours.
const SPIN_RECT: (f32, f32, f32, f32) = (140.0, 271.0, 52.0, 18.0);
const ARROW: f32 = 16.0;
const ARROW_LEFT: &str = "media://interface/ifcommon/com_left_arrow.ddj";
const ARROW_RIGHT: &str = "media://interface/ifcommon/com_right_arrow.ddj";

/// `FontColor=COLOR,"255,255,255,255"` on every static of this page.
const TEXT_WHITE: Color = Color::WHITE;

// --- State ------------------------------------------------------------------

/// Which page of the cargo grid is shown. Ours: the wire has no such notion,
/// the spinner is a pure view control.
#[derive(Resource, Default)]
pub struct CosInventoryPage(pub u8);

// --- Markers ----------------------------------------------------------------

/// One lattice cell; `index` is 0..[`SLOTS_PER_PAGE`) within the current page.
#[derive(Component, Clone, Copy)]
pub struct CosInvCell {
    pub index: u8,
}
/// The lattice container — the "anywhere on the cargo area" drop target.
#[derive(Component)]
pub struct CosInvGrid;
/// The dim overlay of a cell past the COS's `inventory_size`.
#[derive(Component)]
pub struct CosInvBeyondCapacity;
/// Colour of that overlay.
const BEYOND_CAPACITY_TINT: Color = Color::srgba(0.0, 0.0, 0.0, 0.45);
#[derive(Component)]
pub struct CosInvIcon;
#[derive(Component)]
pub struct CosInvCount;
#[derive(Component)]
pub struct CosInvName;
#[derive(Component)]
pub struct CosInvPageLabel;
/// A spinner arrow: -1 = previous page, +1 = next page.
#[derive(Component, Clone, Copy)]
pub struct CosInvPageStep(i8);

// --- Spawning ---------------------------------------------------------------

/// Fill the Inventory page container from `ifcosinventory.txt`.
pub fn build_inventory_page(
    page: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    _ui_strings: &ClientUiStrings,
    s: f32,
) {
    let text_font = TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(8.0 * s),
        ..default()
    };
    let image = |rect: (f32, f32, f32, f32), path: String, mode: NodeImageMode| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path),
                image_mode: mode,
                ..default()
            },
            Pickable::IGNORE,
        )
    };

    // background tile, the frameg ring, then the inner box over it
    page.spawn(image(
        BG_RECT,
        format!("{ART}ifcommon/bg_tile/com_bg_tile_b.ddj"),
        NodeImageMode::Tiled {
            tile_x: true,
            tile_y: true,
            stretch_value: s,
        },
    ));
    let (fx, fy, fw, fh) = GFRAME_RECT;
    for ((x, y, w, h), piece) in ring(fw, fh, GFRAME_SIDE, GFRAME_TOP) {
        page.spawn(image(
            (fx + x, fy + y, w, h),
            format!("{ART}frame/frameg_wnd_{piece}.ddj"),
            NodeImageMode::Stretch,
        ));
    }
    let (bx, by, bw, bh) = INNER_BOX_RECT;
    for ((x, y, w, h), piece) in ring(bw, bh, INNER_BOX_PIECE, INNER_BOX_PIECE) {
        page.spawn(image(
            (bx + x, by + y, w, h),
            format!("{INNER_BOX_DIR}{piece}.ddj"),
            NodeImageMode::Stretch,
        ));
    }

    // the COS's name (server-filled in vanilla too: `Text=STRING,""`)
    page.spawn((
        CosInvName,
        Text::new(String::new()),
        text_font.clone(),
        TextColor(TEXT_WHITE),
        TextLayout::justify(Justify::Center),
        abs_node(NAME_RECT, s),
        Pickable::IGNORE,
    ));

    // the 7x4 cargo lattice
    page.spawn((
        Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::px(GRID_COLS as u16, CELL * s),
            grid_template_rows: RepeatedGridTrack::px(GRID_ROWS as u16, CELL * s),
            ..abs_node(LATTICE_RECT, s)
        },
        // The grid itself is the "anywhere on the cargo area" drop target for
        // an inventory drag that does not land on a specific cell.
        CosInvGrid,
        Hovered::default(),
    ))
    .with_children(|grid| {
        for index in 0..SLOTS_PER_PAGE {
            let (row, col) = (index / GRID_COLS, index % GRID_COLS);
            // `left_up` is the interior tile (separator on its right/bottom
            // edges); the `right_*`/`*_down` variants terminate the lattice.
            let quarter = match (row == GRID_ROWS - 1, col == GRID_COLS - 1) {
                (false, false) => "left_up",
                (false, true) => "right_up",
                (true, false) => "left_down",
                (true, true) => "right_down",
            };
            grid.spawn((
                CosInvCell { index },
                // Hit-testable and hover-tracked: the cell is both the source
                // of a cargo carry and the drop target of an inventory drag.
                Hovered::default(),
                Node {
                    width: Val::Px(CELL * s),
                    height: Val::Px(CELL * s),
                    ..default()
                },
                ImageNode {
                    image: asset_server.load(format!("{LATTICE_DIR}{quarter}.ddj")),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
            ))
            .observe(on_cos_slot_press)
            .with_children(|cell| {
                // A cell past the COS's `inventory_size` exists on the page but
                // not in the bag: the lattice is a fixed 7x4 and the capacity is
                // 0..177, so the page is dimmed rather than reshaped — and the
                // dim is what tells the player why a drop is refused there.
                cell.spawn((
                    CosInvBeyondCapacity,
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(BEYOND_CAPACITY_TINT),
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ));
                cell.spawn((
                    CosInvIcon,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(ICON_INSET * s),
                        top: Val::Px(ICON_INSET * s),
                        width: Val::Px(ICON_SIZE * s),
                        height: Val::Px(ICON_SIZE * s),
                        ..default()
                    },
                    ImageNode::default(),
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ));
                cell.spawn((
                    CosInvCount,
                    Text::new(String::new()),
                    TextFont {
                        font: fonts.two.clone().into(),
                        font_size: FontSize::Px(8.0 * s),
                        ..default()
                    },
                    TextColor(TEXT_WHITE),
                    TextLayout::justify(Justify::Right),
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(ICON_INSET * s),
                        top: Val::Px(20.0 * s),
                        width: Val::Px(ICON_SIZE * s),
                        ..default()
                    },
                    Visibility::Hidden,
                    Pickable::IGNORE,
                ));
            });
        }
    });

    // the page spinner: arrow, number, arrow inside the authored 52x18 rect
    let (sx, sy, sw, _sh) = SPIN_RECT;
    for (step, art, x) in [(-1i8, ARROW_LEFT, 0.0), (1, ARROW_RIGHT, sw - ARROW)] {
        page.spawn((
            CosInvPageStep(step),
            Button,
            abs_node((sx + x, sy, ARROW, ARROW), s),
            ImageNode {
                image: asset_server.load(art),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
        ))
        .observe(on_page_step);
    }
    page.spawn((
        CosInvPageLabel,
        Text::new(String::new()),
        text_font,
        TextColor(TEXT_WHITE),
        TextLayout::justify(Justify::Center),
        abs_node((sx + ARROW, sy + 2.0, sw - 2.0 * ARROW, 12.0), s),
        Pickable::IGNORE,
    ));
}

/// The eight pieces of a nine-slice ring, sides `p_side` wide and bars
/// `p_bar` tall (the `frameg_wnd_` kit is 24x16, `opt_inner_box_` 4x4).
fn ring(w: f32, h: f32, p_side: f32, p_bar: f32) -> [((f32, f32, f32, f32), &'static str); 8] {
    [
        ((0.0, 0.0, p_side, p_bar), "left_up"),
        ((p_side - 1.0, 0.0, w - 2.0 * p_side + 2.0, p_bar), "mid_up"),
        ((w - p_side, 0.0, p_side, p_bar), "right_up"),
        (
            (0.0, p_bar - 1.0, p_side, h - 2.0 * p_bar + 2.0),
            "left_side",
        ),
        (
            (w - p_side, p_bar - 1.0, p_side, h - 2.0 * p_bar + 2.0),
            "right_side",
        ),
        ((0.0, h - p_bar, p_side, p_bar), "left_down"),
        (
            (p_side - 1.0, h - p_bar, w - 2.0 * p_side + 2.0, p_bar),
            "mid_down",
        ),
        ((w - p_side, h - p_bar, p_side, p_bar), "right_down"),
    ]
}

// --- Behaviour --------------------------------------------------------------

/// How many pages a bag of `inventory_size` slots needs — `ceil(size / 28)`,
/// at least one so the empty grid still has a page to show.
pub fn page_count(inventory_size: u8) -> u8 {
    (inventory_size as u16)
        .div_ceil(SLOTS_PER_PAGE as u16)
        .max(1) as u8
}

fn on_page_step(
    activate: On<Activate>,
    steps: Query<&CosInvPageStep>,
    state: Res<CosState>,
    mut page: ResMut<CosInventoryPage>,
) {
    let Ok(step) = steps.get(activate.entity) else {
        return;
    };
    let pages = state
        .bag_owner()
        .map_or(1, |cos| page_count(cos.body.inventory_size));
    let next = page.0 as i16 + step.0 as i16;
    page.0 = next.clamp(0, pages as i16 - 1) as u8;
}

/// Repaint the grid from [`CosState`]: name, cargo icons, stack counts and the
/// page number. The bag is a list of slot-tagged records, so a cell asks for
/// its slot rather than indexing an array; the page's first slot is
/// `page * 28` (slot numbering is the wire's own, base 0 — the `0x30C8`
/// decoder's fixture carries slot 3 in a 6-slot bag, `packets/src/agent/pet.rs`,
/// and ops 25/26/27 pass the same number straight back).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn refresh_cos_inventory(
    state: Res<CosState>,
    item_data: Option<Res<ClientItemData>>,
    asset_server: Res<AssetServer>,
    mut page: ResMut<CosInventoryPage>,
    cells: Query<(&CosInvCell, &Children)>,
    mut icons: Query<(&mut ImageNode, &mut Visibility), With<CosInvIcon>>,
    mut counts: Query<(&mut Text, &mut Visibility), (With<CosInvCount>, Without<CosInvIcon>)>,
    mut dims: Query<
        &mut Visibility,
        (
            With<CosInvBeyondCapacity>,
            Without<CosInvIcon>,
            Without<CosInvCount>,
        ),
    >,
    mut names: Query<&mut Text, (With<CosInvName>, Without<CosInvCount>)>,
    mut page_labels: Query<
        &mut Text,
        (
            With<CosInvPageLabel>,
            Without<CosInvName>,
            Without<CosInvCount>,
        ),
    >,
) {
    let cos = state.bag_owner();
    let pages = cos.map_or(1, |cos| page_count(cos.body.inventory_size));
    if page.0 >= pages {
        page.0 = pages - 1;
    }
    let first_slot = page.0 as u16 * SLOTS_PER_PAGE as u16;

    for mut text in names.iter_mut() {
        let value = cos
            .and_then(|cos| cos.name().map(str::to_string))
            .unwrap_or_default();
        if text.0 != value {
            text.0 = value;
        }
    }
    for mut text in page_labels.iter_mut() {
        let value = format!("{} / {}", page.0 + 1, pages);
        if text.0 != value {
            text.0 = value;
        }
    }

    for (cell, children) in cells.iter() {
        let slot = first_slot + cell.index as u16;
        let in_bag = cos.is_some_and(|cos| slot < cos.body.inventory_size as u16);
        let item = cos
            .filter(|_| in_bag)
            .and_then(|cos| u8::try_from(slot).ok().and_then(|slot| cos.bag_get(slot)));
        let icon = item
            .zip(item_data.as_deref())
            .and_then(|(item, data)| data.get(&(item.ref_id as i32)))
            .and_then(|row| row.icon_path());
        let count = item.and_then(|item| match &item.data {
            ItemTypeData::Expendable { stack_count, .. } => Some(*stack_count as u32),
            ItemTypeData::MagicCube { elixir_count } => Some(*elixir_count),
            _ => None,
        });
        for child in children.iter() {
            if let Ok((mut image, mut visibility)) = icons.get_mut(child) {
                match &icon {
                    Some(path) => {
                        image.image = asset_server.load(path.clone());
                        *visibility = Visibility::Inherited;
                    }
                    None => *visibility = Visibility::Hidden,
                }
            }
            if let Ok((mut text, mut visibility)) = counts.get_mut(child) {
                match count {
                    Some(count) if count > 1 => {
                        let value = count.to_string();
                        if text.0 != value {
                            text.0 = value;
                        }
                        *visibility = Visibility::Inherited;
                    }
                    _ => *visibility = Visibility::Hidden,
                }
            }
            if let Ok(mut visibility) = dims.get_mut(child) {
                let wanted = if in_bag {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                };
                if *visibility != wanted {
                    *visibility = wanted;
                }
            }
        }
    }
}

// --- Cargo carry: moving goods in and out (0x7034 ops 26/27) ----------------
//
// Idea: the grid above only *showed* the cargo. Moving it is the second half,
// and it is built as the guild warehouse's carry is (`hud/guild_storage/ui.rs`),
// because that window solved exactly this problem: two containers on screen,
// one shared cursor, and two different opcodes depending on where a drag ends.
// Two directions exist and no more:
//
// * out of the bag — press a cargo cell to pick it up, release over a bag slot
//   (or the inventory window) → **op 26** `PetToInventory`;
// * into the bag — an inventory drag released over this grid → **op 27**
//   `InventoryToPet`, slot-precise on a hovered empty cell, else the first free
//   cargo slot.
//
// Both halves apply nothing locally: the authority is the `0xB034` ack, which
// `state.rs::apply_cos_bag_ops` already turns into bag + inventory changes.

/// A cargo item on the cursor (picked off this grid).
#[derive(Resource, Default)]
pub struct CosBagCarry(pub Option<CosBagCarryData>);

pub struct CosBagCarryData {
    /// The carried item's COS-bag wire slot.
    pub slot: u8,
    pub ghost: Entity,
    /// True until the pickup press's own frame has passed (the click-carry
    /// rule the inventory and both warehouses use).
    pub just_picked: bool,
}

/// The carry's cursor-following icon.
#[derive(Component)]
pub struct CosBagGhost;

/// Press on a cargo cell: begin the carry. Backs off while an inventory drag
/// is live so [`deposit_drop_on_cos_inventory`] owns that drop alone.
#[allow(clippy::too_many_arguments)]
fn on_cos_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&CosInvCell>,
    state: Res<CosState>,
    page: Res<CosInventoryPage>,
    inv_state: Res<InventoryState>,
    item_data: Res<ClientItemData>,
    cam_query: Query<Entity, With<Camera2d>>,
    asset_server: Res<AssetServer>,
    mut carry: ResMut<CosBagCarry>,
    mut commands: Commands,
) {
    if press.event.button != PointerButton::Primary {
        return;
    }
    if carry.0.is_some() || inv_state.drag.is_some() {
        return;
    }
    let (Ok(cell), Some(cos)) = (cells.get(press.entity), state.bag_owner()) else {
        return;
    };
    let Some(slot) = slot_at(page.0, cell.index) else {
        return;
    };
    let Some(item) = cos.bag_get(slot) else {
        return;
    };
    let Some(icon) = item_data
        .get(&(item.ref_id as i32))
        .and_then(|row| row.icon_path())
    else {
        return;
    };
    let mut ghost = commands.spawn((
        CosBagGhost,
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
    carry.0 = Some(CosBagCarryData {
        slot,
        ghost: ghost.id(),
        just_picked: true,
    });
}

/// Cursor-follow for the cargo ghost.
pub fn update_cos_bag_ghost(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut ghosts: Query<&mut Node, With<CosBagGhost>>,
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

/// Drop the cargo carry: a bag slot (or the inventory window) → op 26;
/// anywhere else cancels. Releasing over the cell it came from keeps the carry
/// alive, which is the vanilla click-carry.
#[allow(clippy::too_many_arguments)]
pub fn finish_cos_bag_carry(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<CosState>,
    page: Res<CosInventoryPage>,
    inv_state: Res<InventoryState>,
    cos_cells: Query<(&CosInvCell, &Hovered)>,
    inv_roots: Query<&Hovered, With<InventoryRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut carry: ResMut<CosBagCarry>,
    mut commands: Commands,
) {
    let Some(data) = &carry.0 else {
        return;
    };
    // The COS going away (unsummon, 0x30C9 type 1) drops the carry: its slots
    // no longer exist and the ack would name a bag we do not hold.
    let Some(cos_unique_id) = state.bag_owner().map(|cos| cos.unique_id) else {
        let data = carry.0.take().expect("checked above");
        commands.entity(data.ghost).despawn();
        return;
    };
    if buttons.just_pressed(MouseButton::Right) {
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

    let source = data.slot;
    let hovered_cargo_cell = cos_cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .and_then(|(cell, _)| slot_at(page.0, cell.index));
    let request = if let Some(target) = hovered_cargo_cell {
        if target == source && released {
            // release over the source = click-carry, keep it on the cursor
            return;
        }
        // cargo -> cargo is op 25, which the original never builds (see the
        // section note); cancel instead of inventing the frame.
        debug!("cos bag: cargo-to-cargo move has no request in the original (op 25) — cancelled");
        None
    } else if let Some(target) = inv_state
        .hovered_slot
        .filter(|&slot| slot >= BAG_FIRST_SLOT)
    {
        Some(InventoryOperationRequest::PetToInventory {
            cos_unique_id,
            pet_slot: source,
            inventory_slot: target,
        })
    } else if inv_roots.iter().any(|hovered| hovered.get()) {
        // dropped on the inventory window but not on a slot: first free one
        let target = inventories
            .single()
            .ok()
            .and_then(|inv| (BAG_FIRST_SLOT..inv.size()).find(|&slot| inv.get(slot).is_none()));
        match target {
            Some(target) => Some(InventoryOperationRequest::PetToInventory {
                cos_unique_id,
                pet_slot: source,
                inventory_slot: target,
            }),
            None => {
                info!("cos bag: no free bag slot to unload into");
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
    send_bag_request(&conn, request);
}

/// An inventory drag released over the cargo grid loads the item (op 27) —
/// slot-precise on a hovered empty cell, else the first free cargo slot. The
/// inventory carry is consumed here so no op-0 move goes out for this drop.
#[allow(clippy::too_many_arguments)]
pub fn deposit_drop_on_cos_inventory(
    buttons: Res<ButtonInput<MouseButton>>,
    window: Res<CosWindowState>,
    state: Res<CosState>,
    page: Res<CosInventoryPage>,
    grids: Query<&Hovered, With<CosInvGrid>>,
    cos_cells: Query<(&CosInvCell, &Hovered)>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) && !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(source) = inv_state.drag else {
        return;
    };
    if !window.open || window.page != CosPage::Inventory {
        return;
    }
    let Some(cos) = state.bag_owner() else {
        return;
    };
    if !grids.iter().any(|hovered| hovered.get()) {
        return;
    }
    // Gate BEFORE consuming the carry (the warehouses' rule): a drop we cannot
    // send must leave the item on the cursor rather than swallow it.
    if cos.body.inventory_size == 0 {
        return;
    }
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    let target = cos_cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .and_then(|(cell, _)| slot_at(page.0, cell.index))
        .filter(|&slot| slot < cos.body.inventory_size && cos.bag_get(slot).is_none())
        .or_else(|| first_free_cargo_slot(cos));
    let Some(target) = target else {
        info!("cos bag: no free cargo slot to load into");
        return;
    };
    send_bag_request(
        &conn,
        InventoryOperationRequest::InventoryToPet {
            cos_unique_id: cos.unique_id,
            inventory_slot: source,
            pet_slot: target,
        },
    );
}

/// Drop the carry when the window closes or leaves the cargo page — the ghost
/// must not outlive the grid it came from.
pub fn clear_carry_with_cos_window(
    window: Res<CosWindowState>,
    mut carry: ResMut<CosBagCarry>,
    mut commands: Commands,
) {
    if carry.0.is_none() || (window.open && window.page == CosPage::Inventory) {
        return;
    }
    let data = carry.0.take().expect("checked above");
    commands.entity(data.ghost).despawn();
}

/// The bag slot cell `index` on `page` addresses, or `None` when the page is
/// past the end of the u8 slot space.
///
/// `inventory_size` is a wire byte (up to 255), so the grid can show a last
/// page whose cells run to `9 * 28 + 27 = 279` — past what a slot byte can
/// hold, and a debug-profile `u8` multiply-add panics there rather than
/// wrapping. The arithmetic is done in `u16` (as `refresh_cos_inventory`
/// already does) and a cell outside the byte range simply addresses nothing.
fn slot_at(page: u8, index: u8) -> Option<u8> {
    u8::try_from(page as u16 * SLOTS_PER_PAGE as u16 + index as u16).ok()
}

/// First empty cargo slot within the authored capacity, or `None` when full.
fn first_free_cargo_slot(cos: &super::state::Cos) -> Option<u8> {
    (0..cos.body.inventory_size).find(|&slot| cos.bag_get(slot).is_none())
}

fn send_bag_request(
    conn: &Query<&SilkroadConnection, With<AgentConnection>>,
    request: InventoryOperationRequest,
) {
    let Ok(conn) = conn.single() else {
        warn!("cos bag: no agent connection, dropping request");
        return;
    };
    info!("cos bag: sending {:?}", request);
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send cos bag request: {}", e.0);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every rect here must fit vanilla's page rect (`ifcos.txt`
    /// `12,66,331,314`) — a copying error shows up as an overflow, not as a
    /// control drawn outside the window.
    #[test]
    fn every_rect_fits_the_page() {
        let page = (331.0, 314.0);
        for (x, y, w, h) in [
            GFRAME_RECT,
            BG_RECT,
            INNER_BOX_RECT,
            NAME_RECT,
            LATTICE_RECT,
            SPIN_RECT,
        ] {
            assert!(x + w <= page.0, "{x}+{w} overflows the page width");
            assert!(y + h <= page.1, "{y}+{h} overflows the page height");
        }
    }

    /// The authored lattice rect divided by the tile size *is* the 7x4 grid —
    /// the cell count follows from the data, it is not chosen.
    #[test]
    fn lattice_rect_is_seven_by_four_whole_tiles() {
        assert_eq!(LATTICE_RECT.2 / CELL, GRID_COLS as f32);
        assert_eq!(LATTICE_RECT.3 / CELL, GRID_ROWS as f32);
        assert_eq!(SLOTS_PER_PAGE, 28);
    }

    /// The biggest bag the wire can describe (`inventory_size = 255`) has a
    /// last page whose cells address slots past 255. Those cells must resolve
    /// to `None` instead of panicking on a `u8` overflow.
    #[test]
    fn the_last_page_of_a_255_slot_bag_addresses_no_slot_past_the_byte() {
        let last_page = page_count(255) - 1;
        assert_eq!(last_page, 9);
        assert_eq!(slot_at(last_page, 0), Some(252));
        assert_eq!(slot_at(last_page, 3), Some(255));
        for index in 4..SLOTS_PER_PAGE {
            assert_eq!(slot_at(last_page, index), None, "cell {index}");
        }
    }

    #[test]
    fn page_count_covers_the_authored_capacities() {
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(28), 1);
        assert_eq!(page_count(32), 2);
        assert_eq!(page_count(140), 5);
        assert_eq!(page_count(177), 7);
    }

    /// The two arrows and the number fit the authored 52x18 spinner rect.
    #[test]
    fn spinner_arrows_fit_the_authored_rect() {
        assert!(2.0 * ARROW < SPIN_RECT.2);
        assert!(ARROW <= SPIN_RECT.3 + 2.0);
    }
}

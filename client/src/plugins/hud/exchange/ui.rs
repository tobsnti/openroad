//! Exchange (player trade) window layout + the confirm/approve interaction.
//!
//! Idea: transcribed 1:1 from the shipped `resinfo/ifexchange.txt` and
//! `ginterface.txt:514` (`GDR_EXCHANGE`, `mframe_wnd_`, `UIIT_STT_EXCHANGE`),
//! under the branch that is actually compiled — `APPLY_EXCHANGE_UPDATE_1TH`
//! and `UI_UPDATE_2009_FIRST` are both listed in `config/define.txt:17,18`, so
//! the live shell is `ginterface.txt:525`'s **254x361**. That rect is
//! byte-identical to the unguarded fallback at `:530`, so the variant cannot
//! be told apart by size — only by the 12 elements that exist because the two
//! flags are set (see the anti-scam note below).
//!
//! The window is two identical panes stacked vertically. The **partner's** is
//! on top (`GDR_EXCHANGE_OTHER_*`, slot ids 100-111 at y 72/108) and **ours**
//! below (`GDR_EXCHANGE_MY_*`, ids 200-211 at y 215/251); each is a 6x2
//! `com_lattice_` grid of 32x32 slots on a 36px pitch over a gold row, and one
//! button pair sits at the bottom. The side split is taken from the element
//! symbols themselves (`..._OTHER_SLOT_1xx`, `ifexchange.txt:341` /
//! `..._MY_SLOT_2xx`, `:89`). Swapping the two ranges would render our own
//! goods in the partner's pane, so the id ranges decide the side, nothing
//! else.
//!
//! Rects are consumed in **window** space and hung off the shell root rather
//! than the chrome's content container, so the outer window measures the
//! vanilla 254x361 exactly and none of the `-(12,26)` content-offset drift
//! (#310/#313) applies here.
//!
//! Not built, deliberately, and each for a stated reason:
//! - the 11 anti-scam overlay elements at `x=9999` (portrait panes, the
//!   `ch_red.ddj` 9-frame warning flipbook, the floating message box). 9999 is
//!   a "positioned by code" sentinel — several are also wider than the window
//!   itself — so the original's placement is not in the data, and stays
//!   unknown.
//! - `GDR_EXCHANGE_USER_NAME` (`:68`), the only guarded element with a real
//!   rect: it is the fictitious-name warning overlay
//!   (`UIIT_MSG_EXCHANGE_NAME_WARNING`) and there is no server signal wired to
//!   raise it.
//! - the top and bottom spans of the two `com_lattice_outline_` frames: that
//!   directory ships only six pieces (four corners + two sides, all 4x4 and
//!   all distinct art), with no `mid_up`/`mid_down`, so which piece the
//!   original stretches across those spans is unknown. The six that do exist are
//!   drawn where the data places them unambiguously.

use bevy::input_focus::{FocusCause, InputFocus};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter, TextCursorStyle};
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::character_data::{InventoryItem, ItemTypeData};

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::exchange::model::{
    self, ExchangeCarry, ExchangeCarryData, ExchangeSession, ExchangeState,
};
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::{drag_ghost_bundle, format_thousands, DragGhost};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};
use crate::plugins::ui_v2::style::ImageButtonStyle;

/// `ginterface.txt:525` — `Rect="200,200,254,361"`, read as (x, y, w, h).
/// The (l, t, r, b) reading is falsified elsewhere in the same file:
/// `GDR_EXCHANGE_NORMALTILE_BG` (`:1185`, `"40,179,174,3"`) would have a
/// negative height.
const VANILLA_W: f32 = 254.0;
const VANILLA_H: f32 = 361.0;

/// Content box chosen so the chrome's own `outer_size` lands back on the
/// vanilla 254x361 rather than growing past it.
const CONTENT_W: f32 = VANILLA_W - 2.0 * (game_window::FRAME_VIS_SIDE + game_window::CHROME_PAD);
const CONTENT_H: f32 = VANILLA_H
    - (game_window::CONTENT_TOP + game_window::CHROME_PAD + game_window::FRAME_VIS_BOTTOM);

/// Opening anchor, in physical px from the viewport's right/top edge, like
/// every other HUD window (inventory 24, store 300, storage 560, character
/// info 620). This is a **placement choice, not a transcribed constant**:
/// `ginterface.txt:525`'s `Rect="200,200,…"` is a top-LEFT origin in the
/// 1024x768 space the layout is authored for, and neither the anchor corner
/// nor the resolution carries over to a right-anchored HUD at the configured
/// 1920x1080. Picked to clear the windows above; the position is draggable and
/// survives a rebuild either way.
const WINDOW_RIGHT: f32 = 900.0;
const WINDOW_TOP: f32 = 60.0;

// --- element rects, all (x, y, w, h) in window space, ifexchange.txt --------

/// `:1043` id 14 — partner pane container.
const OTHER_SUBFRAME: (f32, f32, f32, f32) = (9.0, 38.0, 236.0, 141.0);
/// `:1066` id 13 — own pane container.
const MY_SUBFRAME: (f32, f32, f32, f32) = (9.0, 182.0, 236.0, 141.0);
/// `:857` id 22 / `:837` id 23 — the lattice outlines.
const OTHER_OUTLINE: (f32, f32, f32, f32) = (18.0, 69.0, 218.0, 74.0);
const MY_OUTLINE: (f32, f32, f32, f32) = (18.0, 213.0, 218.0, 74.0);
/// `:818` id 24 / `:799` id 25 — the 6x2 slot grids.
const OTHER_LATTICE: (f32, f32, f32, f32) = (21.0, 72.0, 216.0, 72.0);
const MY_LATTICE: (f32, f32, f32, f32) = (21.0, 216.0, 216.0, 72.0);
/// `:1112` id 20 / `:1089` id 21 — gold-row backdrops.
const OTHER_GOLD_BG: (f32, f32, f32, f32) = (25.0, 143.0, 204.0, 20.0);
const MY_GOLD_BG: (f32, f32, f32, f32) = (25.0, 287.0, 204.0, 20.0);
/// `:780` id 26 / `:761` id 27 — grid/gold dividers.
const OTHER_LINE: (f32, f32, f32, f32) = (14.0, 146.0, 226.0, 4.0);
const MY_LINE: (f32, f32, f32, f32) = (14.0, 290.0, 226.0, 4.0);
/// `:612` id 16 (disabled art) / `:593` id 15 (live button).
const OTHER_MONEY_BTN: (f32, f32, f32, f32) = (40.0, 152.0, 20.0, 20.0);
const MY_MONEY_BTN: (f32, f32, f32, f32) = (40.0, 296.0, 20.0, 20.0);
/// `:650` id 18 / `:631` id 19 — `exc_box.ddj`. Their resinfo rect carries
/// `w=h=0`, i.e. "take the art's own extent"; the DDS header of
/// `interface/exchange/exc_box.ddj` reads 108x20, which is exactly the gap
/// between the money button and the "Gold" label at x=181.
const EXC_BOX_W: f32 = 108.0;
const EXC_BOX_H: f32 = 20.0;
const OTHER_MONEY_BOX: (f32, f32) = (71.0, 152.0);
const MY_MONEY_BOX: (f32, f32) = (71.0, 296.0);
/// `:688` id 13 / `:669` id 14 — the "Gold" captions.
const OTHER_GOLD_LABEL: (f32, f32, f32, f32) = (181.0, 156.0, 24.0, 12.0);
const MY_GOLD_LABEL: (f32, f32, f32, f32) = (181.0, 300.0, 24.0, 12.0);
/// `:1175` id 28 — the seam between the two panes.
const SEAM: (f32, f32, f32, f32) = (40.0, 179.0, 174.0, 3.0);
/// `:745` id 11 / `:718` id 12 — the live branch's button pair. The dead
/// branch puts them at x 96 / 193, which is how the 363-wide variant is told
/// apart from this one.
const BTN_ACTION: (f32, f32, f32, f32) = (50.0, 328.0, 76.0, 24.0);
const BTN_CANCEL: (f32, f32, f32, f32) = (130.0, 328.0, 76.0, 24.0);

/// Slot grid: 12 per pane, 32x32 art on a 36px pitch from x=21.
const SLOT_COLS: usize = 6;
const SLOT_ROWS: usize = 2;
const SLOT_SIZE: f32 = 32.0;
const SLOT_PITCH: f32 = 36.0;
const SLOT_X0: f32 = 21.0;
/// Row origins, straight from the slot blocks. Note the panes are NOT
/// symmetric: the partner's rows sit exactly on its lattice top (y=72,
/// `:572`), while ours sit one pixel above ours (y=215 against a lattice at
/// y=216, `:320` vs `:799`). That one pixel is in the data, so it is kept.
const OTHER_ROW_Y: [f32; SLOT_ROWS] = [72.0, 108.0];
const MY_ROW_Y: [f32; SLOT_ROWS] = [215.0, 251.0];

/// `sframe_wnd_` piece extents, from the DDS headers: 16px corners, a 36px top
/// band and a 16px bottom band.
const SFRAME_SIDE: f32 = 16.0;
const SFRAME_TOP: f32 = 36.0;
const SFRAME_BOTTOM: f32 = 16.0;
/// `com_lattice_outline_` pieces are 4x4.
const OUTLINE_PIECE: f32 = 4.0;

const SFRAME_DIR: &str = "media://interface/frame/sframe_wnd_";
const LATTICE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_";
const OUTLINE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_outline_";
const EXC_BOX_DDJ: &str = "media://interface/exchange/exc_box.ddj";
const EXC_LINE_DDJ: &str = "media://interface/exchange/exc_sub_window_line.ddj";
const MONEY_BTN_DDJ: &str = "media://interface/ifcommon/com_moneybutton.ddj";
const MONEY_BTN_DISABLED_DDJ: &str = "media://interface/ifcommon/com_moneybutton_disable.ddj";
const BG_TILE_A: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_a.ddj";
const BG_TILE_B: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// `FontColor="255,255,255,255"` on both gold captions (`:673`, `:692`);
/// resinfo COLOR is A,R,G,B.
const GOLD_TEXT: Color = Color::srgb_u8(255, 255, 255);
/// `FontColor="255,254,251,216"` on both buttons (`:739`, `:712`).
const BUTTON_TEXT: Color = Color::srgb_u8(254, 251, 216);
/// A locked pane is not editable; grey its action affordances the way the
/// store greys an unavailable repair button.
const DISABLED_TEXT: Color = Color::srgb(0.5, 0.5, 0.5);

#[derive(Component)]
pub struct ExchangeWindowRoot;

/// Marks the previous window for despawn after the rebuild, so the old and new
/// trees never both answer a query in the same frame.
#[derive(Component)]
pub struct ExchangeClosing;

/// Which pane a slot belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExchangeSide {
    /// `GDR_EXCHANGE_OTHER_*`, ids 100-111 — the trade partner, top pane.
    Partner,
    /// `GDR_EXCHANGE_MY_*`, ids 200-211 — us, bottom pane.
    Own,
}

impl ExchangeSide {
    /// The vanilla slot id for this pane's `index`, i.e. 100+i / 200+i.
    fn slot_id(self, index: usize) -> u16 {
        let base = match self {
            ExchangeSide::Partner => 100,
            ExchangeSide::Own => 200,
        };
        base + index as u16
    }

    fn row_origins(self) -> [f32; SLOT_ROWS] {
        match self {
            ExchangeSide::Partner => OTHER_ROW_Y,
            ExchangeSide::Own => MY_ROW_Y,
        }
    }
}

#[derive(Component)]
pub struct ExchangeSlotCell {
    pub side: ExchangeSide,
    pub index: usize,
}

/// Invisible drop catcher over our own pane, so a bag item dropped in the gaps
/// between the 32px icons on the 36px pitch still stages instead of silently
/// doing nothing. Sits under the slot cells (spawned first), which block it.
#[derive(Component)]
pub struct ExchangeOwnPaneCatch;

/// The gold-staging popup behind our own money button (the storage window's
/// deposit/withdraw popup is the precedent; `ifexchange.txt` has no rect for
/// it — the original opens `IF_MSGBOX_MONEY_INPUT` by code, so its geometry is
/// unknown and this reuses the shape we already ship).
#[derive(Resource, Default)]
pub struct ExchangeGoldModal {
    pub open: bool,
}

/// The typed amount, parsed from the input every frame (empty = 0).
#[derive(Resource, Default)]
pub struct ExchangeGoldAmount(pub u64);

#[derive(Component)]
pub struct ExchangeGoldModalRoot;

#[derive(Component)]
pub struct ExchangeGoldAmountInput;

#[derive(Component, Clone, Copy)]
enum ExchangeGoldButton {
    Set,
    Cancel,
}

/// The single action button — confirm while our offer is open, approve once
/// both sides are locked.
#[derive(Component)]
struct ExchangeActionButton;

#[derive(Component)]
struct ExchangeCancelButton;

/// The vanilla slot rect for `index` within a pane, in window space.
fn slot_rect(side: ExchangeSide, index: usize) -> (f32, f32, f32, f32) {
    let (row, col) = (index / SLOT_COLS, index % SLOT_COLS);
    (
        SLOT_X0 + col as f32 * SLOT_PITCH,
        side.row_origins()[row],
        SLOT_SIZE,
        SLOT_SIZE,
    )
}

/// Rebuild the window whenever the trade state changes.
#[allow(clippy::too_many_arguments)]
pub fn sync_exchange_window(
    state: Res<ExchangeState>,
    existing: Query<(Entity, &Node), With<ExchangeWindowRoot>>,
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
    // a dragged position survives the rebuild, as in the storage window
    let mut anchor = (WINDOW_RIGHT, WINDOW_TOP);
    for (entity, node) in existing.iter() {
        if let (Val::Px(right), Val::Px(top)) = (node.right, node.top) {
            anchor = (right, top);
        }
        commands.entity(entity).insert(ExchangeClosing);
    }
    let Some(session) = state.session.as_ref() else {
        return;
    };
    let Ok(camera) = cam_query.single() else {
        warn!("exchange: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_STT_EXCHANGE", "Exchange"),
        (CONTENT_W, CONTENT_H),
        anchor,
        s,
    );
    commands
        // Hovered so the staging/withdraw drop polls can ask "is the pointer
        // over the trade window", exactly as the storage window does for its
        // deposit drops.
        .entity(window.root)
        .insert((ExchangeWindowRoot, GlobalZIndex(59), Hovered::default()));
    // The X is a protocol act, not a hide: it sends 0x7084 like Cancel does.
    commands
        .entity(window.expect_close_button())
        .observe(on_exit_button);

    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };

    commands.entity(window.root).with_children(|root| {
        // Body spans the whole window so `ifexchange` rects can be used
        // verbatim. IGNORE so it cannot swallow the title bar's drag/close
        // picking underneath it; the interactive children opt back in.
        root.spawn((
            abs_node((0.0, 0.0, VANILLA_W, VANILLA_H), s),
            Pickable::IGNORE,
            Name::from("Exchange Body"),
        ))
        .with_children(|body| {
            for (side, items, gold) in [
                (
                    ExchangeSide::Partner,
                    &session.partner_items,
                    session.partner_gold,
                ),
                (ExchangeSide::Own, &session.own_items, session.own_gold),
            ] {
                spawn_pane(
                    body,
                    &asset_server,
                    &item_data,
                    &ui_strings,
                    &text_font,
                    session,
                    side,
                    items,
                    gold,
                    s,
                );
            }

            // seam between the two panes (`:1175` id 28)
            body.spawn((
                abs_node(SEAM, s),
                ImageNode {
                    image: asset_server.load(BG_TILE_A),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));

            let button_style = ImageButtonStyle {
                normal: asset_server.load("media://interface/ifcommon/com_button.ddj"),
                hover: asset_server.load("media://interface/ifcommon/com_button_focus.ddj"),
                press: asset_server.load("media://interface/ifcommon/com_button_press.ddj"),
                ..Default::default()
            };

            // The one action button. `ifexchange.txt:754` labels it
            // UIIT_CTL_CONFIRM statically and textdata ships no "approve"
            // caption at all, so the text does not change between the two
            // phases — only which opcode the press sends, and whether it is
            // enabled.
            let action_enabled = session.can_confirm() || session.can_approve();
            let mut action = body.spawn((
                ExchangeActionButton,
                Button,
                Hovered::default(),
                abs_node(BTN_ACTION, s),
                ImageNode {
                    image: button_style.normal.clone(),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                button_style.clone(),
            ));
            action.observe(on_action_button);
            if !action_enabled {
                action.insert(bevy::ui::InteractionDisabled);
            }
            action.with_children(|button| {
                button.spawn((
                    Text::new(ui_strings.get_or("UIIT_CTL_CONFIRM", "Confirm").to_string()),
                    text_font(8.0),
                    TextColor(if action_enabled {
                        BUTTON_TEXT
                    } else {
                        DISABLED_TEXT
                    }),
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

            let mut cancel = body.spawn((
                ExchangeCancelButton,
                Button,
                Hovered::default(),
                abs_node(BTN_CANCEL, s),
                ImageNode {
                    image: button_style.normal.clone(),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                button_style,
            ));
            cancel.observe(on_exit_button);
            cancel.with_children(|button| {
                button.spawn((
                    Text::new(ui_strings.get_or("UIIT_CTL_CANCEL", "Cancel").to_string()),
                    text_font(8.0),
                    TextColor(BUTTON_TEXT),
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

            // Waiting state: our side is locked and the trade has not
            // completed, so the only thing left is the peer
            // (UIIT_MSG_EXCHANGE_AGREEMENT_WAIT).
            if session.awaiting_approve() {
                body.spawn((
                    Text::new(ui_strings.get_plain_or(
                        "UIIT_MSG_EXCHANGE_AGREEMENT_WAIT",
                        "Waiting for other player's approval.",
                    )),
                    text_font(7.0),
                    TextColor(GOLD_TEXT),
                    TextLayout::justify(Justify::Center),
                    abs_node((9.0, 312.0, 236.0, 12.0), s),
                    Pickable::IGNORE,
                ));
            }
        });
    });
}

/// One pane: sub-frame, outline, slot grid, gold row.
#[allow(clippy::too_many_arguments)]
fn spawn_pane(
    body: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    item_data: &ClientItemData,
    ui_strings: &ClientUiStrings,
    text_font: &impl Fn(f32) -> TextFont,
    session: &ExchangeSession,
    side: ExchangeSide,
    items: &[InventoryItem],
    gold: u64,
    s: f32,
) {
    let partner = side == ExchangeSide::Partner;
    let subframe = if partner { OTHER_SUBFRAME } else { MY_SUBFRAME };
    let outline = if partner { OTHER_OUTLINE } else { MY_OUTLINE };
    let lattice = if partner { OTHER_LATTICE } else { MY_LATTICE };
    let gold_bg = if partner { OTHER_GOLD_BG } else { MY_GOLD_BG };
    let line = if partner { OTHER_LINE } else { MY_LINE };
    let money_btn = if partner {
        OTHER_MONEY_BTN
    } else {
        MY_MONEY_BTN
    };
    let money_box = if partner {
        OTHER_MONEY_BOX
    } else {
        MY_MONEY_BOX
    };
    let gold_label = if partner {
        OTHER_GOLD_LABEL
    } else {
        MY_GOLD_LABEL
    };

    spawn_subframe(body, asset_server, subframe, s);
    spawn_lattice(body, asset_server, lattice, s);
    spawn_lattice_outline(body, asset_server, outline, s);

    // Our own pane catches a dropped bag item anywhere inside it; the
    // partner's does not exist as a drop target at all — you can never put
    // something into the other side's pane.
    if !partner {
        body.spawn((
            ExchangeOwnPaneCatch,
            Hovered::default(),
            abs_node(subframe, s),
            Name::from("Exchange Own Pane Drop"),
        ));
    }

    // slots at their exact vanilla rects, over the lattice chrome
    for index in 0..(SLOT_COLS * SLOT_ROWS) {
        let mut cell = body.spawn((
            ExchangeSlotCell { side, index },
            Hovered::default(),
            abs_node(slot_rect(side, index), s),
            // the vanilla control id, so the two panes stay tellable apart in
            // an entity inspector
            Name::from(format!("Exchange Slot {}", side.slot_id(index))),
        ));
        if !partner {
            // picking a staged item back up (the withdraw half of the drag);
            // the drop itself is routed by `withdraw_drop_off_exchange`
            cell.observe(on_own_slot_press);
        }
        // The staged list is add-ordered, not slot-indexed: the partner's
        // 0x308C carries each record's *inventory* slot, which says nothing
        // about which exchange cell it occupies. So fill left to right.
        let Some(item) = items.get(index) else {
            continue;
        };
        let Some(icon) = item_data
            .get(&(item.ref_id as i32))
            .and_then(|row| row.icon_path())
        else {
            continue;
        };
        let count = match &item.data {
            ItemTypeData::Expendable { stack_count, .. } => *stack_count as u32,
            ItemTypeData::MagicCube { elixir_count } => *elixir_count,
            _ => 0,
        };
        let opt_level = match &item.data {
            ItemTypeData::Equipment(equipment) => equipment.opt_level,
            _ => 0,
        };
        cell.with_children(|slot| {
            slot.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Px(SLOT_SIZE * s),
                    height: Val::Px(SLOT_SIZE * s),
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
                    TextColor(GOLD_TEXT),
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(2.0 * s),
                        bottom: Val::Px(1.0 * s),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            if opt_level > 0 {
                slot.spawn((
                    Text::new(format!("+{opt_level}")),
                    text_font(7.0),
                    TextColor(BUTTON_TEXT),
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

    // gold row: backdrop, divider, money button, amount box, "Gold" caption
    body.spawn((
        abs_node(gold_bg, s),
        ImageNode {
            image: asset_server.load(BG_TILE_B),
            image_mode: NodeImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: s,
            },
            ..default()
        },
        Pickable::IGNORE,
    ));
    body.spawn((
        abs_node(line, s),
        ImageNode {
            image: asset_server.load(EXC_LINE_DDJ),
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
        Pickable::IGNORE,
    ));
    // The partner's money control is a CIFStatic wearing the *disabled* art
    // (`:612`), not a button — you can never edit the other side's gold. Ours
    // is a real CIFButton (`:593`): it opens the amount popup, which sends
    // 0x7034 sub-op 13, and it greys out once our offer is locked
    // (`gold_editable`, `InfoManager.cs:1068-1080`).
    let own_editable = !partner && session.gold_editable();
    let mut money = body.spawn((
        abs_node(money_btn, s),
        ImageNode {
            image: asset_server.load(if own_editable {
                MONEY_BTN_DDJ
            } else {
                MONEY_BTN_DISABLED_DDJ
            }),
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
    ));
    if own_editable {
        money.insert((Button, Hovered::default()));
        money.observe(on_money_button);
    } else {
        money.insert(Pickable::IGNORE);
    }
    body.spawn((
        abs_node((money_box.0, money_box.1, EXC_BOX_W, EXC_BOX_H), s),
        ImageNode {
            image: asset_server.load(EXC_BOX_DDJ),
            image_mode: NodeImageMode::Stretch,
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|box_node| {
        // HAlign=2 on both amount statics (`:637`, `:656`) — right aligned.
        box_node.spawn((
            Text::new(format_thousands(gold)),
            text_font(8.0),
            TextColor(GOLD_TEXT),
            TextLayout::justify(Justify::Right),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(6.0 * s),
                top: Val::Px(4.0 * s),
                ..default()
            },
            Pickable::IGNORE,
        ));
    });
    body.spawn((
        Text::new(ui_strings.get_or("UIIT_STT_GOLD", "Gold").to_string()),
        text_font(8.0),
        TextColor(GOLD_TEXT),
        abs_node(gold_label, s),
        Pickable::IGNORE,
    ));
}

/// The 8-piece `sframe_wnd_` sub-panel frame as a 3x3 grid. Border extents are
/// the pieces' own DDS dimensions: 16px sides, a 36px top band, a 16px bottom.
pub(crate) fn spawn_subframe(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    rect: (f32, f32, f32, f32),
    s: f32,
) {
    let piece = |name: &str| asset_server.load::<Image>(format!("{SFRAME_DIR}{name}.ddj"));
    parent
        .spawn((
            Node {
                display: Display::Grid,
                grid_template_columns: vec![
                    RepeatedGridTrack::px(1, SFRAME_SIDE * s),
                    RepeatedGridTrack::flex(1, 1.0),
                    RepeatedGridTrack::px(1, SFRAME_SIDE * s),
                ],
                grid_template_rows: vec![
                    RepeatedGridTrack::px(1, SFRAME_TOP * s),
                    RepeatedGridTrack::flex(1, 1.0),
                    RepeatedGridTrack::px(1, SFRAME_BOTTOM * s),
                ],
                ..abs_node(rect, s)
            },
            Pickable::IGNORE,
        ))
        .with_children(|grid| {
            // row-major; the centre stays empty (the panes draw their own)
            for name in [
                Some("left_up"),
                Some("mid_up"),
                Some("right_up"),
                Some("left_side"),
                None,
                Some("right_side"),
                Some("left_down"),
                Some("mid_down"),
                Some("right_down"),
            ] {
                let cell = Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                };
                match name {
                    Some(name) => {
                        grid.spawn((
                            cell,
                            ImageNode {
                                image: piece(name),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                            Pickable::IGNORE,
                        ));
                    }
                    None => {
                        grid.spawn((cell, Pickable::IGNORE));
                    }
                }
            }
        });
}

/// The 6x2 `com_lattice_` cell chrome. The quarters are 36x36 and are picked
/// by position exactly as the inventory/storage grids do.
fn spawn_lattice(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    rect: (f32, f32, f32, f32),
    s: f32,
) {
    parent
        .spawn((
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::px(SLOT_COLS as u16, SLOT_PITCH * s),
                grid_template_rows: RepeatedGridTrack::px(SLOT_ROWS as u16, SLOT_PITCH * s),
                ..abs_node(rect, s)
            },
            Pickable::IGNORE,
        ))
        .with_children(|grid| {
            for cell in 0..(SLOT_COLS * SLOT_ROWS) {
                let (row, col) = (cell / SLOT_COLS, cell % SLOT_COLS);
                let quarter = match (row == SLOT_ROWS - 1, col == SLOT_COLS - 1) {
                    (false, false) => "left_up",
                    (false, true) => "right_up",
                    (true, false) => "left_down",
                    (true, true) => "right_down",
                };
                grid.spawn((
                    Node {
                        width: Val::Px(SLOT_PITCH * s),
                        height: Val::Px(SLOT_PITCH * s),
                        ..default()
                    },
                    ImageNode {
                        image: asset_server.load(format!("{LATTICE_DIR}{quarter}.ddj")),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
        });
}

/// `com_lattice_outline_`: four 4x4 corners plus the two vertically stretched
/// sides. The directory ships no `mid_up`/`mid_down`, and the six pieces are
/// all distinct art, so nothing here can stand in for the missing top/bottom
/// spans — they stay undrawn rather than guessed.
fn spawn_lattice_outline(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    rect: (f32, f32, f32, f32),
    s: f32,
) {
    let (x, y, w, h) = rect;
    let c = OUTLINE_PIECE;
    for (name, piece_rect) in [
        ("left_up", (x, y, c, c)),
        ("right_up", (x + w - c, y, c, c)),
        ("left_down", (x, y + h - c, c, c)),
        ("right_down", (x + w - c, y + h - c, c, c)),
        ("left_side", (x, y + c, c, h - 2.0 * c)),
        ("right_side", (x + w - c, y + c, c, h - 2.0 * c)),
    ] {
        parent.spawn((
            abs_node(piece_rect, s),
            ImageNode {
                image: asset_server.load(format!("{OUTLINE_DIR}{name}.ddj")),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
}

/// The action button sends confirm while our offer is open and approve once
/// both sides are locked — the two-stage lock, from one control.
fn on_action_button(
    _: On<Activate>,
    mut state: ResMut<ExchangeState>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let Some(session) = state.session.as_mut() else {
        return;
    };
    // One press = one packet: the ack is ~50 ms away and a *second* 0x7082
    // makes the server terminate the trade (`0xB082 02 1f18` + `0x3088 1f18`,
    // see `hud::exchange::model` header note 3), so the flag is set here
    // rather than waiting for the ack to flip `own_confirmed`.
    if session.can_confirm() {
        model::send_confirm(&conn);
        session.request_in_flight = true;
    } else if session.can_approve() {
        model::send_approve(&conn);
        session.request_in_flight = true;
    }
}

/// Cancel and the title bar's X both back out of the trade. Neither clears the
/// session: that happens on the 0xB084 ack, so a refused exit cannot leave us
/// showing no window for a trade the server still has open.
fn on_exit_button(_: On<Activate>, conn: Query<&SilkroadConnection, With<AgentConnection>>) {
    model::send_exit(&conn);
}

// --- staging interaction: the last mile of the trade ------------------------
//
// Idea: there is exactly ONE carry mechanism in this HUD — the inventory's
// (`InventoryState.drag` plus the shared `DragGhost`) — and this window
// borrows it rather than growing a second one, exactly as the storage window
// borrows it for deposits. Two polls route the two directions of the gesture,
// because bevy_picking's DragDrop never reaches the press target (see the
// header of `hud::inventory::ui`):
//   * bag → pane: an inventory carry released or clicked over our pane sends
//     sub-op 4 and the carry is consumed HERE, so no plain 0x7034 op-0 move
//     goes out for that same drop (the storage-deposit precedent).
//   * pane → bag: a press on a staged slot lifts it onto the cursor
//     ([`ExchangeCarry`]), and letting go anywhere outside the trade window
//     sends sub-op 5. That op has no target byte at all — the item never left
//     the bag and the server picks where it lands — so "outside the window" is
//     the entire information the gesture can carry.
// Both refuse to send while no trade is open: that same drag then belongs to
// the inventory alone.

/// Press on one of OUR pane slots: lift the staged item onto the cursor. Backs
/// off while an inventory carry is live — that drop is a staging drop and
/// [`stage_drop_on_exchange`] owns it.
#[allow(clippy::too_many_arguments)]
fn on_own_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&ExchangeSlotCell>,
    state: Res<ExchangeState>,
    inv_state: Res<InventoryState>,
    item_data: Res<ClientItemData>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut carry: ResMut<ExchangeCarry>,
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
    let Some(session) = state.session.as_ref() else {
        return;
    };
    // the pane renders add-ordered, but the wire slot is the one the server
    // assigned (`ExchangeSession::own_slots`)
    let (Some(item), Some(&wire_slot)) = (
        session.own_items.get(cell.index),
        session.own_slots.get(cell.index),
    ) else {
        return;
    };
    let Some(icon) = item_data
        .get(&(item.ref_id as i32))
        .and_then(|row| row.icon_path())
    else {
        return;
    };
    let Ok(camera) = cam_query.single() else {
        return;
    };
    let s = hud_scale();
    let ghost = commands
        .spawn(drag_ghost_bundle(
            "Exchange Drag Ghost",
            asset_server.load(icon),
            press.pointer_location.position,
            SLOT_SIZE * s,
            camera,
        ))
        .id();
    carry.0 = Some(ExchangeCarryData {
        slot: wire_slot,
        ghost,
    });
}

/// Dropping a carried bag item on our pane stages it: sub-op 4, `source` the
/// bag slot, `target` the first free pane slot. The pane slot is the client's
/// choice (bag 0x29 goes into pane 0 as `042900`), and it is the
/// first *free* one because a withdraw can leave a gap in the middle.
#[allow(clippy::too_many_arguments)]
pub fn stage_drop_on_exchange(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<ExchangeState>,
    catchers: Query<&Hovered, With<ExchangeOwnPaneCatch>>,
    cells: Query<(&ExchangeSlotCell, &Hovered)>,
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
    let over_own_pane = catchers.iter().any(|hovered| hovered.get())
        || cells
            .iter()
            .any(|(cell, hovered)| cell.side == ExchangeSide::Own && hovered.get());
    if !over_own_pane {
        return;
    }
    // Gate BEFORE consuming the carry: with no session (or a locked offer)
    // this drag is not ours, and swallowing it would make the item vanish off
    // the cursor without any packet at all.
    let Some(request) = model::stage_item_request(&state, source) else {
        return;
    };
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    model::send_staging(&conn, request);
}

/// Letting the carried pane item go outside the trade window withdraws it:
/// sub-op 5 with the pane slot. Inside the window the carry survives, which is
/// the vanilla click-carry (press to lift, click the destination). The gold
/// popup counts as "inside": its scrim covers the window, so a click on OK
/// would otherwise read as a click outside and put an unasked-for sub-op 5 on
/// the wire alongside the sub-op 13.
#[allow(clippy::too_many_arguments)]
pub fn withdraw_drop_off_exchange(
    buttons: Res<ButtonInput<MouseButton>>,
    state: Res<ExchangeState>,
    modal: Res<ExchangeGoldModal>,
    roots: Query<&Hovered, With<ExchangeWindowRoot>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut carry: ResMut<ExchangeCarry>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) && !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(data) = &carry.0 else {
        return;
    };
    if modal.open || roots.iter().any(|hovered| hovered.get()) {
        return;
    }
    let request = model::unstage_item_request(&state, data.slot);
    let data = carry.0.take().expect("checked above");
    commands.entity(data.ghost).despawn();
    if let Some(request) = request {
        model::send_staging(&conn, request);
    }
}

/// Our own money button opens the amount popup.
fn on_money_button(_: On<Activate>, mut modal: ResMut<ExchangeGoldModal>) {
    modal.open = true;
}

/// Rebuild the gold popup when it opens/closes — the storage window's popup,
/// with one send button instead of two, because sub-op 13 is an absolute set:
/// there is no "withdraw", only a smaller number.
#[allow(clippy::too_many_arguments)]
pub fn sync_exchange_gold_modal(
    modal: Res<ExchangeGoldModal>,
    state: Res<ExchangeState>,
    existing: Query<Entity, With<ExchangeGoldModalRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut focus: ResMut<InputFocus>,
    mut amount: ResMut<ExchangeGoldAmount>,
    mut commands: Commands,
) {
    if !modal.is_changed() {
        return;
    }
    for entity in existing.iter() {
        commands.entity(entity).insert(ExchangeClosing);
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
    let purse = inventories.single().map(|inv| inv.gold).unwrap_or(0);
    let staged = state
        .session
        .as_ref()
        .map(|session| session.own_gold)
        .unwrap_or(0);
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
            ExchangeGoldModalRoot,
            Name::from("Exchange Gold Modal"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(66),
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
                                .get_or("UIIT_STT_EXCHANGE", "Exchange")
                                .to_string(),
                        ),
                        text_font(8.5),
                        TextColor(GOLD_TEXT),
                        TextLayout::justify(Justify::Center),
                        abs_node((0.0, 10.0, 240.0, 14.0), s),
                        Pickable::IGNORE,
                    ));
                    panel.spawn((
                        Text::new(format!(
                            "{}: {}   {}: {}",
                            ui_strings.get_or("UIIT_STT_INVENTORY", "Inventory"),
                            format_thousands(purse),
                            ui_strings.get_or("UIIT_STT_EXCHANGE", "Exchange"),
                            format_thousands(staged),
                        )),
                        text_font(7.5),
                        TextColor(BUTTON_TEXT),
                        TextLayout::justify(Justify::Center),
                        abs_node((0.0, 28.0, 240.0, 12.0), s),
                        Pickable::IGNORE,
                    ));
                    let mut input_box = abs_node((60.0, 44.0, 120.0, 18.0), s);
                    input_box.padding = UiRect::top(Val::Px(2.0 * s));
                    input_entity = Some(
                        panel
                            .spawn((
                                ExchangeGoldAmountInput,
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
                        (ExchangeGoldButton::Set, "UIIT_CTL_OK", "OK", 40.0),
                        (
                            ExchangeGoldButton::Cancel,
                            "UIIT_CTL_CANCEL",
                            "Cancel",
                            130.0,
                        ),
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
                                    TextColor(BUTTON_TEXT),
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

/// Parse the typed amount (empty = 0).
pub fn sync_exchange_gold_amount(
    modal: Res<ExchangeGoldModal>,
    inputs: Query<&EditableText, With<ExchangeGoldAmountInput>>,
    mut amount: ResMut<ExchangeGoldAmount>,
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

/// OK sends sub-op 13 with the typed amount, clamped to the purse. The pane
/// keeps showing the OLD number until the 0xB034 ack arrives — the server has
/// the last word on what we are offering, and a field that shows the typed
/// value would lie for as long as the ack takes (~60 ms) or forever if the op
/// is refused.
fn on_gold_modal_button(
    activate: On<Activate>,
    buttons: Query<&ExchangeGoldButton>,
    state: Res<ExchangeState>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    amount: Res<ExchangeGoldAmount>,
    mut modal: ResMut<ExchangeGoldModal>,
) {
    let Ok(button) = buttons.get(activate.entity) else {
        return;
    };
    modal.open = false;
    if matches!(button, ExchangeGoldButton::Cancel) {
        return;
    }
    let purse = inventories.single().map(|inv| inv.gold).unwrap_or(0);
    let Some(request) = model::stage_gold_request(&state, amount.0, purse) else {
        info!("exchange: gold not staged — no open, editable trade");
        return;
    };
    model::send_staging(&conn, request);
}

/// The popup and the pane carry live and die with the trade.
pub fn clear_exchange_extras(
    state: Res<ExchangeState>,
    mut modal: ResMut<ExchangeGoldModal>,
    mut carry: ResMut<ExchangeCarry>,
    mut commands: Commands,
) {
    if state.session.is_some() {
        return;
    }
    if modal.open {
        modal.open = false;
    }
    if let Some(data) = carry.0.take() {
        commands.entity(data.ghost).despawn();
    }
}

/// Despawn the previous window one frame after the rebuild.
pub fn despawn_closing_exchange(
    closing: Query<Entity, With<ExchangeClosing>>,
    mut commands: Commands,
) {
    for entity in closing.iter() {
        commands.entity(entity).despawn();
    }
}

/// Drop the window and the trade when the world scene is left.
#[allow(clippy::type_complexity)]
pub fn cleanup_exchange(
    roots: Query<Entity, Or<(With<ExchangeWindowRoot>, With<ExchangeGoldModalRoot>)>>,
    mut state: ResMut<ExchangeState>,
    mut modal: ResMut<ExchangeGoldModal>,
    mut carry: ResMut<ExchangeCarry>,
    mut commands: Commands,
) {
    for entity in roots.iter() {
        commands.entity(entity).despawn();
    }
    // The gold popup is a second root with its own full-screen scrim, so
    // leaving the world without it would carry a click-eating overlay into the
    // next scene; the carry's ghost is despawned with it (the storage
    // `cleanup_storage` precedent).
    if let Some(data) = carry.0.take() {
        commands.entity(data.ghost).despawn();
    }
    modal.open = false;
    state.session = None;
}

#[cfg(test)]
mod test {
    use super::*;

    /// The window must measure the vanilla `ginterface.txt:525` 254x361. The
    /// content box is derived from the chrome's own margins for exactly this
    /// reason — the storage/store windows hardcode a `-(12,26)` offset that
    /// #310/#313 show is really `CONTENT_TOP = 36`, which is why they come out
    /// 26 too tall and 10 low.
    #[test]
    fn the_outer_window_measures_the_vanilla_rect() {
        assert_eq!(
            game_window::outer_size((CONTENT_W, CONTENT_H)),
            (VANILLA_W, VANILLA_H)
        );
    }

    /// Slot geometry is the load-bearing part of this window: the partner's
    /// twelve slots must land on ids 100-111 in the TOP pane and ours on
    /// 200-211 BELOW — getting it backwards would show our own goods as the
    /// partner's offer. This is the regression guard for that.
    #[test]
    fn the_partner_pane_is_the_hundreds_range_and_sits_above_ours() {
        assert_eq!(ExchangeSide::Partner.slot_id(0), 100);
        assert_eq!(ExchangeSide::Partner.slot_id(11), 111);
        assert_eq!(ExchangeSide::Own.slot_id(0), 200);
        assert_eq!(ExchangeSide::Own.slot_id(11), 211);

        // every partner slot is above every own slot
        let lowest_partner = (0..12)
            .map(|i| slot_rect(ExchangeSide::Partner, i).1)
            .fold(f32::MIN, f32::max);
        let highest_own = (0..12)
            .map(|i| slot_rect(ExchangeSide::Own, i).1)
            .fold(f32::MAX, f32::min);
        assert!(
            lowest_partner < highest_own,
            "partner pane must render above the own pane"
        );
    }

    /// Transcribed corners, against `ifexchange.txt:572/341/320/89`. A 36px
    /// pitch from x=21 with 32px art, and the two panes' row origins are NOT
    /// symmetric — ours sit one pixel above their own lattice, which is what
    /// the data says.
    #[test]
    fn slot_rects_match_the_resinfo_corners() {
        assert_eq!(
            slot_rect(ExchangeSide::Partner, 0),
            (21.0, 72.0, 32.0, 32.0)
        );
        assert_eq!(
            slot_rect(ExchangeSide::Partner, 11),
            (201.0, 108.0, 32.0, 32.0)
        );
        assert_eq!(slot_rect(ExchangeSide::Own, 0), (21.0, 215.0, 32.0, 32.0));
        assert_eq!(slot_rect(ExchangeSide::Own, 11), (201.0, 251.0, 32.0, 32.0));

        // the grids sit inside their lattice rects (216 wide = 6 * 36)
        assert_eq!(OTHER_LATTICE.2, SLOT_COLS as f32 * SLOT_PITCH);
        assert_eq!(OTHER_LATTICE.3, SLOT_ROWS as f32 * SLOT_PITCH);
    }

    /// The gold amount box takes its extent from `exc_box.ddj` (108x20 in the
    /// DDS header) because its resinfo rect carries w=h=0. That width is what
    /// makes the row add up: button ends at x=60, box spans 71..179, caption
    /// starts at 181.
    #[test]
    fn the_money_row_tiles_without_overlap() {
        for (btn, boxx, label) in [
            (OTHER_MONEY_BTN, OTHER_MONEY_BOX, OTHER_GOLD_LABEL),
            (MY_MONEY_BTN, MY_MONEY_BOX, MY_GOLD_LABEL),
        ] {
            assert!(btn.0 + btn.2 <= boxx.0, "money button overlaps the box");
            assert!(
                boxx.0 + EXC_BOX_W <= label.0,
                "gold box overlaps the caption"
            );
            assert_eq!(btn.1, boxx.1, "button and box share a baseline");
        }
    }
}

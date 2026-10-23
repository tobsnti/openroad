//! The action / emote panel (`resinfo/ifaction.txt`) — four sub-frames on one
//! 364x357 MainPopup page.
//!
//! Idea: the panel the under-bar's "Action" row opens is not one window but a
//! page tiled by **four `CIFSubFrame` shells** on `interface\frame\sframe_wnd_`
//! (`ifaction.txt:1387/1349/1330/1368`), each holding a `com_lattice_` grid of
//! 32x32 `CIFSlotWithHelp` slots: 9x2 character controls, two 4x2 pet grids,
//! 9x2 emotes = 52 slots. Like the skill window this is a MainPopup *page*, so
//! page coordinates are our content coordinates and the shell is our own until
//! MainPopup hosting lands (#307).
//!
//! Two things the layout alone gets wrong, and both matter for the build:
//!
//! 1. **`CommandID` is not an action id.** It repeats across panels — the value
//!    `7` sits on slots 207, 307 *and* 407 — so a flat command id would
//!    mis-fire. The identity of a slot is the pair `(group, index)`.
//! 2. **The layout carries no content at all**: all 52 slots have `Text=""`,
//!    `HelpString=""` and `DDJ=""`. The content lives in
//!    `textdata/actionwnddata.txt` ([`ClientActionCommands`]), which binds
//!    `(group, index)` → icon + textuisystem key, and that table is *newer* than
//!    the layout: it assigns 1012/1014-1017 and the COS charm 5000 to slot
//!    indices whose `CommandID=` in `ifaction.txt` is still the bare index
//!    (`10..17`, `7..17`). The table wins; the stale ids are not read here.
//!
//! Geometry is derived, not transcribed, because the data is exactly regular:
//! every lattice is `cols*36 x rows*36` (9x36=324, 4x36=144, 2x36=72) with a
//! 36px pitch and **zero jitter**, and every `com_lattice_outline_` rect is its
//! lattice at `(x-3, y-3, w+2, h+2)` — identically in all four pairs, so the
//! 1px overflow on the right and bottom is authored, uniform, and kept rather
//! than "fixed" into a symmetric inset.
//!
//! The two pet grids are the same widget twice (byte-identical 4x2 blocks that
//! differ only in x), and neither has any row in `actionwnddata.txt`: their
//! content is server-supplied, so they render empty here. Slots are inert for
//! now — issuing a command needs the wire path, which is not part of this
//! change; hovering names the action so the panel is still readable.

use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::FontAssets;
use crate::plugins::combat::{AttackOrder, PickupOrder};
use crate::plugins::cursor::interactions::entity_select::SelectedEntity;
use crate::plugins::hud::exchange::ui::spawn_subframe;
use crate::plugins::hud::game_window::{abs_node, spawn_game_window};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::target_menu::{TargetAction, TargetMenuAction};
use crate::plugins::textdata::{ClientActionCommands, ClientUiStrings};

/// The page extent the four sub-frames tile: `0,240,364,117` is the last row,
/// so the page is 364 x (240+117) = 364x357.
const CONTENT_W: f32 = 364.0;
const CONTENT_H: f32 = 357.0;

/// Slot art extent and grid pitch (`32,32` on all 52 blocks; x/y steps of 36
/// with no jitter anywhere in the file).
const SLOT_SIZE: f32 = 32.0;
const SLOT_PITCH: f32 = 36.0;

/// `com_lattice_` quarters are 36x36, `com_lattice_outline_` pieces are 4x4.
const OUTLINE_PIECE: f32 = 4.0;

const LATTICE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_";
const OUTLINE_DIR: &str = "media://interface/ifcommon/lattice_window/com_lattice_outline_";
const BG_TILE_B: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// The four `CIFNormalTile` side strips, `3,65` each, flanking the two wide
/// grids only (`ifaction.txt:1254/1273/1292/1311`).
const SIDE_TILES: [(f32, f32, f32, f32); 4] = [
    (16.0, 36.0, 3.0, 65.0),
    (345.0, 36.0, 3.0, 65.0),
    (16.0, 276.0, 3.0, 65.0),
    (345.0, 276.0, 3.0, 65.0),
];

/// Caption band inside a `sframe_wnd_` shell. The 36px top band is the art's;
/// where the caption sits inside it is **ours** — the `CIFSubFrame` blocks carry
/// a `Text=` key but no text rect.
const CAPTION_INSET: (f32, f32, f32) = (10.0, 10.0, 16.0);
/// The four captions are the sub-frames' own text, and the blocks that carry
/// the keys carry the size with them: `GDR_ACTION_BG_{CTRL,SUM,BRU,ACT}`
/// (`ifaction.txt:1330,1349,1368,1387`, indices on `:1336,1355,1374,1393`) are
/// `FontIndex=0` = 9 pt = 12 px ([`font_index_px`]), as are all 68 blocks in
/// the tree. The 9 px here was that legend's *point* value used as pixels.
const CAPTION_FONT: f32 = font_index_px(0);

/// Which of the four sub-frames a slot belongs to. The pair
/// `(ActionPanel, index)` is the slot's identity — `CommandID` is not unique.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionPanel {
    /// `GDR_ACTION_BG_CTRL`, id 1 — 9x2 character controls, slot ids 100-117.
    CharacterControl,
    /// `GDR_ACTION_BG_SUM`, id 3 — 4x2, slot ids 200-207.
    CreatedPet,
    /// `GDR_ACTION_BG_BRU`, id 4 — 4x2, slot ids 300-307.
    SummonedPet,
    /// `GDR_ACTION_BG_ACT`, id 2 — 9x2 emotes, slot ids 400-417.
    Emotes,
}

/// One sub-frame: shell rect, caption key + English fallback, lattice origin,
/// grid size, the `actionwnddata` group that fills it, and the slot-id base.
struct PanelSpec {
    panel: ActionPanel,
    frame: (f32, f32, f32, f32),
    caption_key: &'static str,
    caption_fallback: &'static str,
    origin: (f32, f32),
    cols: usize,
    rows: usize,
    /// `actionwnddata.txt` group column; `None` = server-filled (the pet grids).
    group: Option<u8>,
    slot_id_base: u16,
}

/// The four blocks, in page order. Rects are `ifaction.txt` verbatim; the
/// lattice *sizes* are derived from `cols`/`rows` (see the module note).
const PANELS: [PanelSpec; 4] = [
    PanelSpec {
        panel: ActionPanel::CharacterControl,
        frame: (0.0, 0.0, 364.0, 117.0),
        caption_key: "UIIT_STT_CHARACTER_CONTROL",
        caption_fallback: "Character control",
        origin: (22.0, 36.0),
        cols: 9,
        rows: 2,
        group: Some(1),
        slot_id_base: 100,
    },
    PanelSpec {
        panel: ActionPanel::CreatedPet,
        frame: (0.0, 120.0, 178.0, 117.0),
        caption_key: "UIIT_STT_CONTROL_CREATED_MONSTER",
        caption_fallback: "Created pet",
        origin: (19.0, 156.0),
        cols: 4,
        rows: 2,
        group: None,
        slot_id_base: 200,
    },
    PanelSpec {
        panel: ActionPanel::SummonedPet,
        frame: (186.0, 120.0, 178.0, 117.0),
        caption_key: "UIIT_STT_CONTROL_SUMMONED_MONSTER",
        caption_fallback: "Summoned pet",
        origin: (205.0, 156.0),
        cols: 4,
        rows: 2,
        group: None,
        slot_id_base: 300,
    },
    PanelSpec {
        panel: ActionPanel::Emotes,
        frame: (0.0, 240.0, 364.0, 117.0),
        caption_key: "UIIT_STT_ACTION_EMOTICON",
        caption_fallback: "Emotes",
        origin: (22.0, 276.0),
        cols: 9,
        rows: 2,
        group: Some(4),
        slot_id_base: 400,
    },
];

impl PanelSpec {
    /// `cols*36 x rows*36` at the authored origin — every `CIFLattice` rect in
    /// the file equals exactly this.
    fn lattice(&self) -> (f32, f32, f32, f32) {
        (
            self.origin.0,
            self.origin.1,
            self.cols as f32 * SLOT_PITCH,
            self.rows as f32 * SLOT_PITCH,
        )
    }

    /// The `CIFStretchWnd` outline: the lattice at `(x-3, y-3, w+2, h+2)`.
    fn outline(&self) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.lattice();
        (x - 3.0, y - 3.0, w + 2.0, h + 2.0)
    }

    /// Slot `index`, row-major, at its authored 32x32.
    fn slot_rect(&self, index: usize) -> (f32, f32, f32, f32) {
        let (col, row) = (index % self.cols, index / self.cols);
        (
            self.origin.0 + col as f32 * SLOT_PITCH,
            self.origin.1 + row as f32 * SLOT_PITCH,
            SLOT_SIZE,
            SLOT_SIZE,
        )
    }
}

/// Open/closed state of the panel.
#[derive(Resource, Default)]
pub struct ActionWindowState {
    pub open: bool,
}

#[derive(Component)]
pub struct ActionWindowRoot;

/// A slot, identified the way the original cannot: by panel and grid index.
#[derive(Component, Clone, Copy, Debug)]
pub struct ActionSlot {
    pub panel: ActionPanel,
    pub index: usize,
    /// The action bound to it by `actionwnddata.txt`, if any.
    pub command_id: Option<u32>,
}

pub fn spawn_action_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    actions: Res<ClientActionCommands>,
    cameras: Query<Entity, With<Camera2d>>,
    mut state: ResMut<ActionWindowState>,
) {
    let Ok(camera) = cameras.single() else {
        warn!("action window: no 2d camera to attach to");
        return;
    };
    *state = ActionWindowState::default();
    let s = hud_scale();

    let window = spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        // The page has no title of its own (MainPopup hosting is #307); the
        // under-bar row that opens it does, so its key is what we show.
        ui_strings.get_or("UIIT_STT_TOGGLE_ACTION", "Action ( A )"),
        (CONTENT_W, CONTENT_H),
        (300.0, 90.0),
        s,
    );
    commands.entity(window.root).insert((
        ActionWindowRoot,
        GlobalZIndex(20),
        // Shares the MainPopup wndpos slot with the other pages of the
        // original's single frame — see `hud::main_popup`.
        crate::plugins::hud::window_positions::PersistedWindow(
            crate::plugins::settings::window_positions::WndPosSlot::MainPopup,
        ),
    ));
    commands.entity(window.expect_close_button()).observe(
        |_: On<Activate>, mut state: ResMut<ActionWindowState>| {
            state.open = false;
        },
    );

    commands.entity(window.content).with_children(|page| {
        for spec in PANELS.iter() {
            spawn_subframe(page, &asset_server, spec.frame, s);
            page.spawn((
                Text::new(
                    ui_strings
                        .get_or(spec.caption_key, spec.caption_fallback)
                        .to_string(),
                ),
                TextFont {
                    font: fonts.two.clone().into(),
                    font_size: FontSize::Px(CAPTION_FONT * s),
                    ..default()
                },
                TextColor(Color::WHITE),
                abs_node(
                    (
                        spec.frame.0 + CAPTION_INSET.0,
                        spec.frame.1 + CAPTION_INSET.1,
                        spec.frame.2 - 2.0 * CAPTION_INSET.0,
                        CAPTION_INSET.2,
                    ),
                    s,
                ),
                Pickable::IGNORE,
            ));
            spawn_lattice(page, &asset_server, spec, s);
            spawn_outline(page, &asset_server, spec.outline(), s);

            for index in 0..(spec.cols * spec.rows) {
                let command = spec
                    .group
                    .and_then(|group| actions.slot(group, index as u8));
                let mut slot = page.spawn((
                    ActionSlot {
                        panel: spec.panel,
                        index,
                        command_id: command.map(|c| c.command_id),
                    },
                    Name::from(format!(
                        "Action slot {}",
                        spec.slot_id_base as usize + index
                    )),
                    Button,
                    abs_node(spec.slot_rect(index), s),
                ));
                if let Some(command) = command {
                    slot.insert(ImageNode {
                        image: asset_server.load(format!("media://{}", command.icon)),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    });
                }
                slot.observe(on_slot_activate);
            }
        }

        for tile in SIDE_TILES {
            page.spawn((
                abs_node(tile, s),
                ImageNode {
                    image: asset_server.load(BG_TILE_B),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
    });
}

// `actionwnddata.txt` command ids, verified against the player's own file
// (`server_dep/silkroad/textdata/actionwnddata.txt`, 26 rows, UTF-16LE,
// `id \t panel \t name \t text key \t icon \t group \t index`). Only the ids
// whose wire path already exists are named here; the file's own text key is
// quoted beside each so a reader can find the row.
//
// The four below need **no new opcode**: two are discriminants of the `0x7074`
// envelope the combat plugin already sends, two go to senders the target menu
// already owns. Everything else in the window needs its own opcode first —
// `0x704F` (1000 sit/stand, 1001 walk/run) and `0x3091` (4000..=4006 emotes)
// are the two next wire slices, each with its own ledger row.

/// 1002 `UIIT_CTL_AUTOATTACK_TT` — `0x7074` discriminant 1, through
/// [`AttackOrder`].
pub const ACTION_COMMAND_AUTO_ATTACK: u32 = 1002;
/// 1003 `UIIT_CTL_AUTOTRACE_TT` — `0x7074` discriminant 3. Same text key the
/// target menu's Trace row uses (`target_menu.rs`), which is what ties the two
/// entry points to one verb.
pub const ACTION_COMMAND_TRACE: u32 = 1003;
/// 1006 `UIIT_CTL_EXCHANGE_TT` — `0x7081`, through the target menu's sender.
pub const ACTION_COMMAND_EXCHANGE: u32 = 1006;
/// 1007 `UIIT_STT_PARTY` — `0x7060`/`0x7062`, resolved against the roster by
/// `PartyAction`'s sender.
pub const ACTION_COMMAND_PARTY: u32 = 1007;
/// 1008 `UIIT_CTL_AUTOGET_TT` — `0x7074` discriminant 2, through
/// [`PickupOrder`].
pub const ACTION_COMMAND_AUTO_GET: u32 = 1008;

/// What a slot does when it is clicked, as a function of its command id.
///
/// Total on purpose: an id with no wire path is [`SlotVerb::Unwired`] and
/// therefore still *visible* in the log, which is what keeps the backlog
/// honest instead of silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotVerb {
    /// Needs the current selection and goes to an existing order message.
    OnTarget(TargetVerb),
    /// Named by `actionwnddata.txt`, no wire path yet.
    Unwired,
}

/// The four verbs this window can already express, each named by the sender
/// that owns its packet. **No new packet is built in this file** — that is the
/// whole point: every one of these verbs is also reachable from somewhere else
/// in the HUD, so a second builder here would be a second source of truth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetVerb {
    /// `combat::AttackOrder` -> `action_slot` -> `0x7074` discriminant 1.
    Attack,
    /// `combat::PickupOrder` -> `action_slot` -> `0x7074` discriminant 2.
    Pickup,
    /// `target_menu::send_target_menu_trace` -> `0x7074` discriminant 3.
    Trace,
    /// `target_menu::send_target_menu_exchange_invite` -> `0x7081`.
    Exchange,
    /// `target_menu::send_target_menu_party_invite` -> `PartyAction::Invite`.
    Party,
}

/// The classification above as a total function of the command id.
pub fn verb_for(command_id: u32) -> SlotVerb {
    match command_id {
        ACTION_COMMAND_AUTO_ATTACK => SlotVerb::OnTarget(TargetVerb::Attack),
        ACTION_COMMAND_TRACE => SlotVerb::OnTarget(TargetVerb::Trace),
        ACTION_COMMAND_EXCHANGE => SlotVerb::OnTarget(TargetVerb::Exchange),
        ACTION_COMMAND_PARTY => SlotVerb::OnTarget(TargetVerb::Party),
        ACTION_COMMAND_AUTO_GET => SlotVerb::OnTarget(TargetVerb::Pickup),
        _ => SlotVerb::Unwired,
    }
}

/// Run a slot's verb.
///
/// Every verb is routed to a sender that already exists, so this window adds
/// no packet builder at all: the two `0x7074` discriminants go through
/// `combat`'s order messages (the same seam a click on a monster or a dropped
/// item uses) and the three social verbs through [`TargetMenuAction`] (the
/// same seam the target right-click menu uses). One sender per verb, two entry
/// points each.
///
/// All five need the current selection, and a slot with none says so rather
/// than sending a targetless packet the server would refuse.
fn on_slot_activate(
    activate: On<Activate>,
    slots: Query<&ActionSlot>,
    selected: Res<SelectedEntity>,
    mut actions: MessageWriter<TargetMenuAction>,
    mut attacks: MessageWriter<AttackOrder>,
    mut pickups: MessageWriter<PickupOrder>,
) {
    let Ok(slot) = slots.get(activate.entity) else {
        return;
    };
    let Some(id) = slot.command_id else {
        // The two pet grids and the empty tail are server-filled capacity, not
        // a backlog item.
        debug!(
            "action window: {:?} slot {} is unassigned",
            slot.panel, slot.index
        );
        return;
    };
    let SlotVerb::OnTarget(verb) = verb_for(id) else {
        debug!(
            "action window: {:?} slot {} (command {id}) not wired yet",
            slot.panel, slot.index
        );
        return;
    };
    let Some(target) = selected.0 else {
        info!("action window: command {id} needs a selected target");
        return;
    };
    match verb {
        TargetVerb::Attack => {
            attacks.write(AttackOrder(target));
        }
        TargetVerb::Pickup => {
            pickups.write(PickupOrder(target));
        }
        TargetVerb::Trace => actions.write(TargetMenuAction {
            target,
            action: TargetAction::Trace,
        }),
        TargetVerb::Exchange => actions.write(TargetMenuAction {
            target,
            action: TargetAction::Exchange,
        }),
        TargetVerb::Party => actions.write(TargetMenuAction {
            target,
            action: TargetAction::InviteParty,
        }),
    }
}

/// The `com_lattice_` cell chrome: 36x36 quarters picked by position, exactly
/// as the inventory/exchange grids do.
fn spawn_lattice(
    parent: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    spec: &PanelSpec,
    s: f32,
) {
    parent
        .spawn((
            Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::px(spec.cols as u16, SLOT_PITCH * s),
                grid_template_rows: RepeatedGridTrack::px(spec.rows as u16, SLOT_PITCH * s),
                ..abs_node(spec.lattice(), s)
            },
            Pickable::IGNORE,
        ))
        .with_children(|grid| {
            for cell in 0..(spec.cols * spec.rows) {
                let (row, col) = (cell / spec.cols, cell % spec.cols);
                let quarter = match (row == spec.rows - 1, col == spec.cols - 1) {
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

/// The `com_lattice_outline_` ring around a grid (4x4 corner pieces, stretched
/// sides) — the `CIFStretchWnd` blocks.
fn spawn_outline(
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

pub fn apply_action_window_visibility(
    state: Res<ActionWindowState>,
    mut roots: Query<&mut Node, With<ActionWindowRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    let display = if state.open {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in roots.iter_mut() {
        if node.display != display {
            node.display = display;
        }
    }
}

pub fn cleanup_action_window(
    mut commands: Commands,
    roots: Query<Entity, With<ActionWindowRoot>>,
    mut state: ResMut<ActionWindowState>,
) {
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    *state = ActionWindowState::default();
}

#[cfg(test)]
mod test {
    use super::*;

    /// The four `CIFSubFrame` rects, verbatim from `ifaction.txt`, and the page
    /// they tile.
    #[test]
    fn the_four_subframes_tile_the_page() {
        let frames: Vec<(f32, f32, f32, f32)> = PANELS.iter().map(|p| p.frame).collect();
        assert_eq!(
            frames,
            vec![
                (0.0, 0.0, 364.0, 117.0),
                (0.0, 120.0, 178.0, 117.0),
                (186.0, 120.0, 178.0, 117.0),
                (0.0, 240.0, 364.0, 117.0),
            ]
        );
        // rows at 0/120/240 (3px gutter), the middle row split at 0/186
        assert_eq!(CONTENT_W, 364.0);
        assert_eq!(CONTENT_H, 240.0 + 117.0);
        for frame in &frames {
            assert!(frame.0 + frame.2 <= CONTENT_W);
            assert!(frame.1 + frame.3 <= CONTENT_H);
        }
    }

    /// Every authored lattice rect must fall out of `cols*36 x rows*36`, and
    /// every outline must be that lattice at `(x-3, y-3, w+2, h+2)`. These are
    /// the file's own numbers — if the derivation is wrong, they will not match.
    #[test]
    fn lattice_and_outline_rects_are_derivable() {
        let authored_lattice = [
            (22.0, 36.0, 324.0, 72.0),
            (19.0, 156.0, 144.0, 72.0),
            (205.0, 156.0, 144.0, 72.0),
            (22.0, 276.0, 324.0, 72.0),
        ];
        let authored_outline = [
            (19.0, 33.0, 326.0, 74.0),
            (16.0, 153.0, 146.0, 74.0),
            (202.0, 153.0, 146.0, 74.0),
            (19.0, 273.0, 326.0, 74.0),
        ];
        for (i, spec) in PANELS.iter().enumerate() {
            assert_eq!(spec.lattice(), authored_lattice[i]);
            assert_eq!(spec.outline(), authored_outline[i]);
        }
    }

    /// 52 slots, 32x32 at a jitter-free 36px pitch, row-major from the lattice
    /// origin — spot-checked against the authored rects at the ends of each run.
    #[test]
    fn slot_rects_match_the_authored_grid() {
        let total: usize = PANELS.iter().map(|p| p.cols * p.rows).sum();
        assert_eq!(total, 52);

        let ctrl = &PANELS[0];
        assert_eq!(ctrl.slot_rect(0), (22.0, 36.0, 32.0, 32.0)); // id 100
        assert_eq!(ctrl.slot_rect(8), (310.0, 36.0, 32.0, 32.0)); // id 108
        assert_eq!(ctrl.slot_rect(9), (22.0, 72.0, 32.0, 32.0)); // id 109
        assert_eq!(ctrl.slot_rect(17), (310.0, 72.0, 32.0, 32.0)); // id 117

        let sum = &PANELS[1];
        assert_eq!(sum.slot_rect(0), (19.0, 156.0, 32.0, 32.0)); // id 200
        assert_eq!(sum.slot_rect(7), (127.0, 192.0, 32.0, 32.0)); // id 207

        let emotes = &PANELS[3];
        assert_eq!(emotes.slot_rect(0), (22.0, 276.0, 32.0, 32.0)); // id 400
        assert_eq!(emotes.slot_rect(17), (310.0, 312.0, 32.0, 32.0)); // id 417

        // no slot leaves its lattice
        for spec in PANELS.iter() {
            let (lx, ly, lw, lh) = spec.lattice();
            for index in 0..(spec.cols * spec.rows) {
                let (x, y, w, h) = spec.slot_rect(index);
                assert!(x >= lx && x + w <= lx + lw);
                assert!(y >= ly && y + h <= ly + lh);
            }
        }
    }

    /// The two pet grids are the same widget twice: identical size, identical
    /// row, no bound content — only x differs.
    #[test]
    fn the_two_pet_grids_are_one_widget_twice() {
        let (created, summoned) = (&PANELS[1], &PANELS[2]);
        assert_eq!((created.cols, created.rows), (summoned.cols, summoned.rows));
        assert_eq!(created.frame.1, summoned.frame.1);
        assert_eq!(created.frame.2, summoned.frame.2);
        assert_eq!(created.frame.3, summoned.frame.3);
        assert_eq!(created.origin.1, summoned.origin.1);
        assert_ne!(created.origin.0, summoned.origin.0);
        assert!(created.group.is_none() && summoned.group.is_none());
    }

    /// Every wired slot routes to a sender that already exists instead of
    /// building its own packet. Pinned verb by verb because a second builder
    /// is the easy mistake here and nothing in the type system forbids it.
    fn app_with_slot(command_id: u32, select: bool) -> (App, Option<Entity>) {
        let mut app = App::new();
        app.init_resource::<SelectedEntity>()
            .add_message::<TargetMenuAction>()
            .add_message::<AttackOrder>()
            .add_message::<PickupOrder>();
        let target = if select {
            let e = app.world_mut().spawn_empty().id();
            app.world_mut().resource_mut::<SelectedEntity>().0 = Some(e);
            Some(e)
        } else {
            None
        };
        let slot = app
            .world_mut()
            .spawn(ActionSlot {
                panel: ActionPanel::CharacterControl,
                index: 0,
                command_id: Some(command_id),
            })
            .observe(on_slot_activate)
            .id();
        app.world_mut().trigger(Activate { entity: slot });
        (app, target)
    }

    fn menu_actions(app: &App) -> Vec<TargetMenuAction> {
        app.world()
            .resource::<Messages<TargetMenuAction>>()
            .iter_current_update_messages()
            .copied()
            .collect()
    }

    #[test]
    fn the_three_social_slots_go_through_the_target_menu_seam() {
        for (command_id, want) in [
            (ACTION_COMMAND_TRACE, TargetAction::Trace),
            (ACTION_COMMAND_EXCHANGE, TargetAction::Exchange),
            (ACTION_COMMAND_PARTY, TargetAction::InviteParty),
        ] {
            let (app, target) = app_with_slot(command_id, true);
            let written = menu_actions(&app);
            assert_eq!(written.len(), 1, "command {command_id}: one action");
            assert_eq!(written[0].action, want);
            assert_eq!(written[0].target, target.unwrap());
        }
    }

    /// The two `0x7074` discriminants go to combat's order messages — the same
    /// seam a click on a monster or on a dropped item uses — so the packet is
    /// still built in exactly one place (`combat::action_slot`).
    #[test]
    fn the_two_object_action_slots_go_through_combats_order_messages() {
        let (app, target) = app_with_slot(ACTION_COMMAND_AUTO_ATTACK, true);
        let orders: Vec<_> = app
            .world()
            .resource::<Messages<AttackOrder>>()
            .iter_current_update_messages()
            .map(|order| order.0)
            .collect();
        assert_eq!(orders, vec![target.unwrap()]);
        assert!(
            menu_actions(&app).is_empty(),
            "no menu action for an attack"
        );

        let (app, target) = app_with_slot(ACTION_COMMAND_AUTO_GET, true);
        let orders: Vec<_> = app
            .world()
            .resource::<Messages<PickupOrder>>()
            .iter_current_update_messages()
            .map(|order| order.0)
            .collect();
        assert_eq!(orders, vec![target.unwrap()]);
    }

    /// Without a selection every wired slot must stay silent on the wire — a
    /// targetless order is a packet the server would refuse.
    #[test]
    fn a_wired_slot_sends_nothing_without_a_selection() {
        for command_id in [
            ACTION_COMMAND_AUTO_ATTACK,
            ACTION_COMMAND_TRACE,
            ACTION_COMMAND_EXCHANGE,
            ACTION_COMMAND_PARTY,
            ACTION_COMMAND_AUTO_GET,
        ] {
            let (app, _) = app_with_slot(command_id, false);
            assert!(
                menu_actions(&app).is_empty(),
                "command {command_id} sent a menu action with no target"
            );
            assert!(app
                .world()
                .resource::<Messages<AttackOrder>>()
                .iter_current_update_messages()
                .next()
                .is_none());
            assert!(app
                .world()
                .resource::<Messages<PickupOrder>>()
                .iter_current_update_messages()
                .next()
                .is_none());
        }
    }

    /// The ids come from the player's own `actionwnddata.txt`, so a typo in one
    /// of them would silently un-wire a slot. This pins the classification, and
    /// that an id the file lists but we cannot send is reported as unwired
    /// rather than mapped to a neighbour's verb.
    #[test]
    fn the_command_ids_match_the_authored_rows() {
        for (id, want) in [
            (1002, SlotVerb::OnTarget(TargetVerb::Attack)),
            (1003, SlotVerb::OnTarget(TargetVerb::Trace)),
            (1006, SlotVerb::OnTarget(TargetVerb::Exchange)),
            (1007, SlotVerb::OnTarget(TargetVerb::Party)),
            (1008, SlotVerb::OnTarget(TargetVerb::Pickup)),
        ] {
            assert_eq!(verb_for(id), want, "command {id}");
        }
        // Authored rows with no wire path yet: 1000/1001 need 0x704F, the
        // emotes 0x3091, 1004/1014..=1017 nothing we have sourced.
        for id in [1000, 1001, 1004, 1009, 1014, 1017, 4000, 5000] {
            assert_eq!(verb_for(id), SlotVerb::Unwired, "command {id}");
        }
    }
}

/// Self-registration for the action/emote panel (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct ActionPlugin;

impl Plugin for ActionPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<ActionWindowState>()
            .add_systems(OnEnter(SceneState::GameWorld), spawn_action_window)
            .add_systems(OnExit(SceneState::GameWorld), cleanup_action_window)
            // built on enter, shown on demand
            .add_systems(
                Update,
                apply_action_window_visibility.run_if(in_state(SceneState::GameWorld)),
            );
    }
}

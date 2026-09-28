//! `ifprevjobinfo` — "Check previous job information", the answer to `0xB0E6`.
//!
//! Idea: this is the first window in the tree drawn on the **`msgbox2_window_`**
//! chrome (`game_window::MSGBOX2_WINDOW`, measured in
//! `docs/re/ui/msgbox2-chrome.md`) rather than on `mframe_wnd_`, because that is
//! the family its own registration declares:
//! `ginterface.txt:1288` `DDJ="interface\messagebox\msgbox2_window_"`, id 66,
//! `Rect="0,0,364,164"`, `Text="UIIT_STT_NPC_CHATTING_JOBINFO_OLD"`.
//!
//! **Every rect below is the authored one**, read from
//! `Media/resinfo/ifprevjobinfo.txt` (23 blocks) — not derived, not centred by
//! taste. The window is therefore spawned with its authored outer size and a
//! content origin of `(0, 0)`, so a child's `Rect=` is used verbatim; deriving
//! the outer size from a content box would have moved every one of them.
//!
//! Three things this window deliberately leaves blank, because the source is
//! not there:
//!
//! - **The EXP gauges are drawn empty.** `chr_{merchant,hunter,thief}_bar.ddj`
//!   is a 120x8 track and the fill fraction needs the per-job EXP ladder,
//!   `leveldata.txt` columns 6-8 (`docs/re/gamedata/leveldata-progression.md`).
//!   Our `LevelData` parses column 1 only, so a filled bar would be an invented
//!   ratio. The number itself is shown — the original draws it on top of the
//!   bar, which is why `_EXP` (`229,46,120,14`) and `_EXP_GAUGE` (`229,49`)
//!   overlap in the data.
//! - **The GRADENAME cells stay empty.** `assets::textdata::job::rank_name`
//!   resolves one, but only after choosing the Chinese or the European key set,
//!   and nothing in `0xB0E6` says which applies (and `MERCHANT_6` vs
//!   `MERCHANT_6_NEW` is `[U]` on top of that).
//! - **Row order is the descriptor's** MERCHANT, HUNTER, THIEF (widget ids
//!   `25,30,50,45 / 26,31,51,46 / 27,32,52,47`); the *wire* order behind
//!   `PrevJobInfo` is the `[S]` MERCHANT, THIEF, HUNTER
//!   (`docs/re/systems/job-trade-system.md` §11.3, open point W3).
//!
//! The interior background needs no special casing: the shell insets it by the
//! family's 16/40/16, so it lands on `16,40,332,108` — byte for byte the tree's
//! own `GDR_PREV_JOB_INFO_BG` rect, with the tree's tile letter
//! (`com_bg_tile_b`). That agreement is what says the chrome numbers are right.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::{self, WindowGeometry};
use crate::plugins::hud::scale::{font_px, hud_scale};
use crate::plugins::net::job::PrevJobInfo;
use crate::plugins::textdata::ClientUiStrings;

/// `ginterface.txt:1294` — the window's authored extent.
const WINDOW_SIZE: (f32, f32) = (364.0, 164.0);
/// `GDR_PREV_JOB_INFO_BG`'s tile letter (`ifprevjobinfo.txt`, block `:424`).
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";
/// `ginterface.txt:1296`.
const TITLE_KEY: (&str, &str) = (
    "UIIT_STT_NPC_CHATTING_JOBINFO_OLD",
    "Check previous job information",
);
/// `GDR_PREV_JOB_INFO_OK_BTN`, `143,124,0,0` art-sized, `UIIT_CTL_CONFIRM`.
const OK_BUTTON_AT: (f32, f32) = (143.0, 124.0);
const OK_BUTTON_KEY: (&str, &str) = ("UIIT_CTL_CONFIRM", "OK");
const OK_BUTTON_DDJ: &str = "media://interface/ifcommon/com_button.ddj";
/// `com_button.ddj` is 76x22 (DDS header), and the block is art-sized.
const OK_BUTTON_SIZE: (f32, f32) = (76.0, 22.0);
/// `com_job_*.ddj`, 16x16 (DDS header).
const ICON_SIZE: (f32, f32) = (16.0, 16.0);
/// `chr_stat_window02.ddj`, 125x12 — the plate the exp number sits on. It
/// overhangs the bg tile's right edge by 3 px in the original too (§3.4); the
/// overhang is reproduced, not clamped.
const BOARD_SIZE: (f32, f32) = (125.0, 12.0);
/// `chr_*_bar.ddj`, 120x8.
const GAUGE_SIZE: (f32, f32) = (120.0, 8.0);

/// One job's authored row: label key + the six rects the tree gives it.
struct JobRow {
    label: (&'static str, &'static str),
    icon: &'static str,
    gauge: &'static str,
    /// `_ICON`, `_GRADE_STA`, `_GRADE`, `_GRADENAME`, `_BOARD`, `_EXP_GAUGE`,
    /// `_EXP` — the y values differ per control, exactly as authored.
    icon_at: (f32, f32),
    label_at: (f32, f32, f32, f32),
    grade_at: (f32, f32, f32, f32),
    gradename_at: (f32, f32, f32, f32),
    board_at: (f32, f32),
    gauge_at: (f32, f32),
    exp_at: (f32, f32, f32, f32),
}

/// The three rows, transcribed from `ifprevjobinfo.txt` in the descriptor's own
/// top-to-bottom order (ids 15/20/25/30/10/45/50, then 16/21/26/31/11/46/51,
/// then 17/22/27/32/12/47/52).
const ROWS: [JobRow; 3] = [
    JobRow {
        label: ("UIIT_STT_MERCHANT_LEVEL", "Trader Grade"),
        icon: "media://interface/ifcommon/com_job_merchant.ddj",
        gauge: "media://interface/character/chr_merchant_bar.ddj",
        icon_at: (14.0, 47.0),
        label_at: (35.0, 49.0, 57.0, 14.0),
        grade_at: (96.0, 49.0, 32.0, 14.0),
        gradename_at: (132.0, 49.0, 89.0, 14.0),
        board_at: (226.0, 47.0),
        gauge_at: (229.0, 49.0),
        exp_at: (229.0, 46.0, 120.0, 14.0),
    },
    JobRow {
        label: ("UIIT_STT_HUNTER_LEVEL", "Hunter Grade"),
        icon: "media://interface/ifcommon/com_job_hunter.ddj",
        gauge: "media://interface/character/chr_hunter_bar.ddj",
        icon_at: (14.0, 69.0),
        label_at: (35.0, 71.0, 57.0, 14.0),
        grade_at: (96.0, 71.0, 32.0, 14.0),
        gradename_at: (132.0, 71.0, 89.0, 14.0),
        board_at: (226.0, 70.0),
        gauge_at: (229.0, 72.0),
        exp_at: (229.0, 69.0, 120.0, 14.0),
    },
    JobRow {
        label: ("UIIT_STT_THIEF_LEVEL", "Thief Grade"),
        icon: "media://interface/ifcommon/com_job_thief.ddj",
        gauge: "media://interface/character/chr_thief_bar.ddj",
        icon_at: (14.0, 91.0),
        label_at: (35.0, 93.0, 57.0, 14.0),
        grade_at: (96.0, 93.0, 32.0, 14.0),
        gradename_at: (132.0, 93.0, 89.0, 14.0),
        board_at: (226.0, 93.0),
        gauge_at: (229.0, 95.0),
        exp_at: (229.0, 92.0, 120.0, 14.0),
    },
];

/// The whole window.
#[derive(Component)]
pub struct PrevJobWindowRoot;

/// The three `(level, exp)` pairs a spawned window was built from, so a repeat
/// answer with the same numbers does not respawn it.
#[derive(Component, PartialEq, Eq)]
// `pub(crate)` because `sync_prev_job_window` below is `pub` and names this type
// in its signature: a private type in a reachable signature is
// `private_interfaces`, and that warning is a hard gate here
// (`scripts/check_warnings.py`), so it blocked every session's pre-commit check.
// Fixed from the outside on 2026-08-23 with two other sessions blocked; the
// visibility is the smallest change that clears it and touches no behaviour.
pub(crate) struct PrevJobWindowData(PrevJobInfo);

/// Opens the window when a `0xB0E6` success lands, and rebuilds it when the
/// numbers change. The request behind it is the job NPC's "check previous job
/// information" line (`hud::npc_dialog::job_menu`) — the job-neutral one, which
/// is why even the union bosses offer it.
#[allow(clippy::too_many_arguments)]
pub fn sync_prev_job_window(
    mut commands: Commands,
    info: Res<PrevJobInfo>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
    open: Query<(Entity, &PrevJobWindowData), With<PrevJobWindowRoot>>,
) {
    if !info.is_changed() {
        return;
    }
    if *info == PrevJobInfo::default() {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("prev-job window: no 2d camera to attach to");
        return;
    };
    for (entity, data) in open.iter() {
        if data.0 == *info {
            return;
        }
        commands.entity(entity).despawn();
    }
    spawn_prev_job_window(
        &mut commands,
        &asset_server,
        &fonts,
        &ui_strings,
        camera,
        info.clone(),
    );
}

/// Builds the window. Split out of the system so a test can call it.
pub fn spawn_prev_job_window(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    camera: Entity,
    info: PrevJobInfo,
) -> Entity {
    let s = hud_scale();
    let title = ui_strings.get_or(TITLE_KEY.0, TITLE_KEY.1).to_string();
    let window = game_window::spawn_game_window_with(
        commands,
        asset_server,
        fonts,
        camera,
        &title,
        WindowGeometry {
            outer: WINDOW_SIZE,
            // The tree's rects are window-absolute, so the content container
            // sits on the window origin and every child uses its `Rect=`.
            content_at: (0.0, 0.0),
        },
        None,
        (24.0, 120.0),
        s,
        game_window::GameWindowStyle {
            chrome: &game_window::MSGBOX2_WINDOW,
            bg_tile: BG_TILE,
            ..default()
        },
    );
    commands
        .entity(window.root)
        .insert((PrevJobWindowRoot, PrevJobWindowData(info.clone())));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    // The wire order behind these three fields is [S] MERCHANT, THIEF, HUNTER;
    // the rows are the descriptor's MERCHANT, HUNTER, THIEF.
    let values = [info.merchant, info.hunter, info.thief];
    let font = fonts.two.clone();
    let text_px = font_px(0);
    commands.entity(window.content).with_children(|content| {
        for (row, value) in ROWS.iter().zip(values) {
            content.spawn((
                game_window::abs_node((row.icon_at.0, row.icon_at.1, ICON_SIZE.0, ICON_SIZE.1), s),
                ImageNode::new(asset_server.load(row.icon)),
                Pickable::IGNORE,
            ));
            let label = ui_strings.get_or(row.label.0, row.label.1).to_string();
            content.spawn((
                game_window::abs_node(row.label_at, s),
                Text::new(label),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(text_px),
                    ..default()
                },
                TextColor(Color::WHITE),
                Pickable::IGNORE,
            ));
            // `HAlign=2` on `_GRADE` — right-aligned in the data.
            content.spawn((
                game_window::abs_node(row.grade_at, s),
                Text::new(
                    value
                        .map(|(level, _)| level.to_string())
                        .unwrap_or_default(),
                ),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(text_px),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Right),
                Pickable::IGNORE,
            ));
            // `_GRADENAME` — declared, and left empty on purpose (module doc).
            content.spawn((
                game_window::abs_node(row.gradename_at, s),
                Text::new(String::new()),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(text_px),
                    ..default()
                },
                TextColor(Color::WHITE),
                Pickable::IGNORE,
            ));
            content.spawn((
                game_window::abs_node(
                    (row.board_at.0, row.board_at.1, BOARD_SIZE.0, BOARD_SIZE.1),
                    s,
                ),
                ImageNode::new(
                    asset_server.load("media://interface/character/chr_stat_window02.ddj"),
                ),
                Pickable::IGNORE,
            ));
            // The gauge track, drawn empty — see the module doc.
            content.spawn((
                game_window::abs_node(
                    (row.gauge_at.0, row.gauge_at.1, GAUGE_SIZE.0, GAUGE_SIZE.1),
                    s,
                ),
                ImageNode::new(asset_server.load(row.gauge)),
                Pickable::IGNORE,
            ));
            // `HAlign=1` on `_EXP` — centred over the bar.
            content.spawn((
                game_window::abs_node(row.exp_at, s),
                Text::new(value.map(|(_, exp)| exp.to_string()).unwrap_or_default()),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(text_px),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Center),
                Pickable::IGNORE,
            ));
        }
        // The OK button, art-sized at its authored origin.
        content
            .spawn((
                game_window::abs_node(
                    (
                        OK_BUTTON_AT.0,
                        OK_BUTTON_AT.1,
                        OK_BUTTON_SIZE.0,
                        OK_BUTTON_SIZE.1,
                    ),
                    s,
                ),
                bevy::ui_widgets::Button,
                bevy::picking::hover::Hovered::default(),
                ImageNode::new(asset_server.load(OK_BUTTON_DDJ)),
                children![(
                    Text::new(
                        ui_strings
                            .get_or(OK_BUTTON_KEY.0, OK_BUTTON_KEY.1)
                            .to_string()
                    ),
                    TextFont {
                        font: font.clone().into(),
                        font_size: FontSize::Px(text_px),
                        ..default()
                    },
                    TextColor(Color::srgb_u8(255, 247, 202)),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        top: Val::Px(4.0 * s),
                        ..default()
                    },
                    TextLayout::justify(Justify::Center),
                    Pickable::IGNORE,
                )],
            ))
            .observe(on_close_button);
    });
    window.root
}

/// Both the (X) and the authored OK button close the window.
fn on_close_button(
    activate: On<Activate>,
    mut commands: Commands,
    parents: Query<&ChildOf>,
    roots: Query<Entity, With<PrevJobWindowRoot>>,
) {
    let mut node = activate.entity;
    loop {
        if roots.get(node).is_ok() {
            commands.entity(node).despawn();
            return;
        }
        match parents.get(node) {
            Ok(parent) => node = parent.parent(),
            Err(_) => return,
        }
    }
}

/// Registers the window.
pub struct PrevJobInfoPlugin;

impl Plugin for PrevJobInfoPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        // Gated on the HUD scenes, like every other HUD window: `FontAssets`
        // exists only there, and Bevy does not skip a system whose `Res` is
        // missing — it fails parameter validation, which is how an ungated
        // version of this system killed a real client start on 2026-08-23
        // (AGENTS.md: "`make ci` never starts the app"). `Option<Res<_>>` is
        // the *net* side's answer; a window has nothing to do outside the
        // scenes that own it. The condition is the tree's own
        // `hud::hud_scenes` (`hud/mod.rs`), not a second copy.
        app.add_systems(
            Update,
            sync_prev_job_window.run_if(super::super::hud_scenes),
        )
        .add_systems(OnExit(SceneState::GameWorld), cleanup_prev_job_window);
    }
}

/// Leaving the world takes the window with it.
fn cleanup_prev_job_window(
    mut commands: Commands,
    windows: Query<Entity, With<PrevJobWindowRoot>>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every rect here must be the authored one. Spot-checked against
    /// `Media/resinfo/ifprevjobinfo.txt`: the three gauges at `229,{49,72,95}`
    /// and the three boards at `226,{47,70,93}` are the pair `hud-job-system.md`
    /// §3.4 calls out, including the 3 px overhang past the bg tile's right
    /// edge (16 + 332 = 348 against the board's 226 + 125 = 351).
    #[test]
    fn the_row_rects_are_the_authored_ones() {
        let gauges: Vec<(f32, f32)> = ROWS.iter().map(|r| r.gauge_at).collect();
        assert_eq!(gauges, vec![(229.0, 49.0), (229.0, 72.0), (229.0, 95.0)]);
        let boards: Vec<(f32, f32)> = ROWS.iter().map(|r| r.board_at).collect();
        assert_eq!(boards, vec![(226.0, 47.0), (226.0, 70.0), (226.0, 93.0)]);
        for row in ROWS.iter() {
            assert_eq!(row.board_at.0 + BOARD_SIZE.0, 351.0);
            assert!(
                row.board_at.0 + BOARD_SIZE.0 < WINDOW_SIZE.0,
                "the overhang stays inside the window"
            );
        }
    }

    /// The window is drawn on the family its own registration declares, and
    /// that family's numbers are the measured ones.
    #[test]
    fn the_window_uses_the_measured_msgbox2_chrome() {
        let chrome = &game_window::MSGBOX2_WINDOW;
        assert_eq!(chrome.dir, "media://interface/messagebox/msgbox2_window_");
        assert_eq!(
            (chrome.side_w, chrome.top_h, chrome.bottom_h),
            (16.0, 40.0, 16.0)
        );
        // and it is a different shell from the one every other window uses
        assert_ne!(chrome.dir, game_window::MFRAME_WND.dir);
    }
}

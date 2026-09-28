//! The 4th-generation quest **journal window** — `res_ui/questlist.2dt`, root
//! `CNIFQuest` id 76.
//!
//! Idea: `super::model` gives the journal reception (login record + `0x30D5`)
//! and `super::mini_list` gives three of its entries a permanent line on
//! screen, but the set of tracked quests had no **operator**: `TrackedQuests`
//! could only be seeded by a heuristic and emptied by the strip's close button.
//! The original's operator is the per-row checkbox of this window (child id 8,
//! `com_checkbutton_on.ddj`, `docs/re/ui/hud-quest-windows.md` §3.4), so this
//! file is what turns "the client knows nine quests" into "the player picks
//! which three of them are on screen". Chain: reception (already there) →
//! state (`QuestJournal`/`TrackedQuests`) → **visible effect** (this list, and
//! the mini-list changing when a box is ticked). Still no send: accept,
//! abandon and hand-in have no evidenced client→server body
//! (`docs/re/systems/quest.md` §11.7 defect 6), and an invented request is
//! worse than a missing one (ADR-0009).
//!
//! **Every rect below was re-read from the user's own
//! `Media/res_ui/questlist.2dt`** for this module (81,988 B = `4 + 84 * 976`,
//! 84 entries) rather than copied out of the doc; it agrees with §3.4 field for
//! field. Positive control for the parse: the same reader on
//! `questminilist.2dt` reproduces root id 173 `(702,267,324,93)` and the three
//! slot rows, i.e. the numbers `mini_list.rs` was built from.
//!
//! ```text
//! [0] CNIFQuest        id 76 root (198,193,624,369) mframe_wnd_  UIIT_STT_QUEST_LIST
//! [1] id 2  Type 1     (209,231,603,322)  Background equip_window_
//! [2] id 3  Type 2     (236,261,547,284)  Background com_bg_tile_a.ddj
//! [3] id 6  Type 13    (789,253,16,262)   vertical scroll, no art
//! [4] CNIFQuestSlot    id 5,77,85,…,141   (214, y, 576, 32)  Background qst_sub_window.ddj
//!       y = 236,267,298,329,360,391,422,453,484,515   → pitch 31 on a 32 px row
//!     each row, identically (row-local, verified equal on all ten rows):
//!       id  7 +3,+5,444,22   qst_small_sub_window.ddj  title plate
//!       id  8 +9,+7,16,16    com_checkbutton_on.ddj    TRACK toggle
//!       id 16 +34,+8,383,16  (no art)                  title text
//!       id 11 +454,+10,77,16 / id 75 +448,+9,77,16     the status pair
//!       id 18 +527,+5,20,20  qst_world_button.ddj
//!       id 19 +548,+5,20,20  qst_contentview_button.ddj
//! ```
//!
//! One correction to the doc while re-measuring: §3.4 says "the same 8
//! children"; the file has **7** per row (the table under it lists seven, so
//! the prose is the slip, not the table).
//!
//! **The authored 1 px overlap is kept.** Pitch 31 against a 32 px row means
//! each row's bottom pixel sits under its successor — the same ledger-rule
//! artefact `docs/re/ui/hud-quest-windows.md` §Proposed offers to smooth to 32.
//! It is not smoothed here: the y values are transcribed one by one from the
//! data, so there is no pitch constant to get wrong, and the rows are spawned
//! in file order, which puts the overlap the way the original draws it.
//!
//! **What is deliberately NOT drawn, and why** (a button without an effect is
//! the defect class this lane exists to remove):
//!
//! * **The status pair, ids 11 and 75.** They carry no `Text` key at all
//!   (verified: `text` is empty on every row child), so the caption is
//!   code-populated in the original, and the wire field behind it — `state` ∈
//!   {1,7,8} — is `[U]` (`docs/re/systems/quest.md` §11.2, needs order
//!   200/202). Two invented captions in the place the original puts
//!   *in progress* / *complete* would be exactly the unsourced value ADR-0009
//!   forbids. The rects stay in this header so the slot is findable.
//! * **id 18, "show on world map".** Its input exists (`0x30D6`/`0x30D7`
//!   marks) but the pin API lives in `hud/world_map/`, another lane's files.
//! * **id 19, "open detail".** The detail pane is not ours to render: it is
//!   pre-rendered markup in `textquest_otherstring.txt` (`SN_PAYCON_*`,
//!   §11.5), a file no loader in this tree registers. Registering it means
//!   editing `plugins/textdata.rs`, outside this lane's scope.
//! * **id 6, the scroll bar.** The window shows ten rows and the largest
//!   active set in the whole capture corpus is nine (§11.2/§11.3), so a
//!   scrollbar would today be a control that can never move. Rows past ten are
//!   reported in the log rather than silently dropped.
//!
//! Opening it: the original's opener is the `KeyQuest` binding (id 3006,
//! Q by default), and [`quest_journal_hotkey`] now consumes it —
//! `settings/keymap.rs` had listed 3006 as "no quest log window yet" since the
//! Key Map tab started advertising the key, i.e. the pane offered a Q that did
//! nothing. That entry leaves `NOT_YET_WIRED` in the same change, because that
//! file's test treats a consumed-but-still-listed id as a failure. A second,
//! smaller way in stays wired as well: clicking a mini-list row's text opens
//! the journal the row came from (`mini_list::on_open_journal`).

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::hud::game_window::{self, InnerFrame, WindowGeometry};
use crate::plugins::hud::scale::{font_px, hud_scale};
use crate::plugins::net::quest::QuestJournal;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::ClientUiStrings;

use super::model::TrackedQuests;

/// Root `CNIFQuest` id 76, `Rect=(198,193,624,369)`.
const ROOT: (f32, f32) = (198.0, 193.0);
const WINDOW_SIZE: (f32, f32) = (624.0, 369.0);
/// `Text=` of the root.
const TITLE_KEY: (&str, &str) = ("UIIT_STT_QUEST_LIST", "Quest");
/// id 2 `(209,231,603,322)` → root-local; `equip_window_` pieces are 12 px
/// (`hud/inventory/ui.rs`'s `EQ_FRAME_BORDER`, measured there from the same art).
const INNER_FRAME: (f32, f32, f32, f32) = (11.0, 38.0, 603.0, 322.0);
const INNER_FRAME_DIR: &str = "media://interface/equipment/equip_window_";
const INNER_FRAME_PIECE: f32 = 12.0;
/// id 3 `(236,261,547,284)` → root-local, tiled with the letter the data names
/// (`com_bg_tile_a.ddj` — a rare case where the tile letter is not guessed).
const BG_RECT: (f32, f32, f32, f32) = (38.0, 68.0, 547.0, 284.0);
const BG_TILE_A: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_a.ddj";

/// The ten `CNIFQuestSlot` rows, root-local: `x = 214 - 198`, and the ten
/// authored `y - 193`. Transcribed one by one on purpose — see the header on
/// the 1 px overlap.
const ROW_X: f32 = 16.0;
const ROW_SIZE: (f32, f32) = (576.0, 32.0);
const ROW_Y: [f32; 10] = [
    43.0, 74.0, 105.0, 136.0, 167.0, 198.0, 229.0, 260.0, 291.0, 322.0,
];
const ROW_DDJ: &str = "media://interface/quest/qst_sub_window.ddj";
/// Row-local children (ids 7, 8, 16).
const PLATE_RECT: (f32, f32, f32, f32) = (3.0, 5.0, 444.0, 22.0);
const CHECK_RECT: (f32, f32, f32, f32) = (9.0, 7.0, 16.0, 16.0);
const TITLE_RECT: (f32, f32, f32, f32) = (34.0, 8.0, 383.0, 16.0);
const PLATE_DDJ: &str = "media://interface/quest/qst_small_sub_window.ddj";
/// The data declares the **checked** art on the widget (`Background=`); the
/// unchecked sibling is the same pair every other checkbox in the tree uses
/// (`options_game.rs`'s `CHECKBOX_OFF`/`CHECKBOX_ON`, `hud/cos/setup.rs`'s
/// `CHECK_OFF_DDJ`/`CHECK_ON_DDJ`).
const CHECK_ON_DDJ: &str = "media://interface/ifcommon/com_checkbutton_on.ddj";
const CHECK_OFF_DDJ: &str = "media://interface/ifcommon/com_checkbutton_off.ddj";

/// Where the window sits. The shell anchors right/top like every other HUD
/// window, so the authored origin `(198,193)` on the 1024x768 canvas becomes
/// `1024 - 198 - 624 = 202` from the right edge — derived from [`ROOT`] rather
/// than written out, so the authored origin stays the single source for both
/// this anchor and the root-local row rects below (`cos_status.rs:63` derives
/// its own stack anchor the same way).
const ANCHOR_RIGHT_TOP: (f32, f32) = (1024.0 - ROOT.0 - WINDOW_SIZE.0, ROOT.1);

/// Is the journal window open? Its only writer inside this lane is the
/// mini-list's row text (see the header); the (X) closes it.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QuestJournalWindow {
    pub open: bool,
}

/// The window.
#[derive(Component)]
pub struct QuestJournalRoot;

/// One row, keyed by the quest it shows — never by index, for the reason
/// `TrackedQuests` gives (an update can remove an entry between two frames).
#[derive(Component, Clone, Copy)]
pub struct QuestJournalRow(pub u32);

/// What the page was last built from, so an unchanged journal costs nothing.
#[derive(Component, PartialEq, Eq)]
pub struct QuestJournalPage(Vec<(u32, String, bool)>);

/// The rows the window would draw: `(id, title, tracked)`, journal order.
fn page_rows(journal: &QuestJournal, tracked: &TrackedQuests) -> Vec<(u32, String, bool)> {
    journal
        .entries
        .iter()
        .take(ROW_Y.len())
        .map(|entry| {
            (
                entry.id,
                entry.display_title(),
                tracked.is_tracked(entry.id),
            )
        })
        .collect()
}

/// Open/close the window and keep its rows current.
pub fn sync_quest_journal_window(
    mut commands: Commands,
    window: Res<QuestJournalWindow>,
    journal: Res<QuestJournal>,
    tracked: Res<TrackedQuests>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cameras: Query<Entity, With<Camera2d>>,
    open: Query<(Entity, &QuestJournalPage), With<QuestJournalRoot>>,
) {
    if !window.open {
        for (entity, _) in open.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }
    let rows = page_rows(&journal, &tracked);
    for (entity, page) in open.iter() {
        if page.0 == rows {
            return;
        }
        commands.entity(entity).despawn();
    }
    if journal.entries.len() > ROW_Y.len() {
        // The scroll bar is not built (header) — say what is not shown rather
        // than drop it silently.
        info!(
            "quest journal: {} quests, showing the first {}",
            journal.entries.len(),
            ROW_Y.len()
        );
    }
    let Ok(camera) = cameras.single() else {
        warn!("quest journal: no 2d camera to attach to");
        return;
    };
    spawn_quest_journal(
        &mut commands,
        &asset_server,
        &fonts,
        &ui_strings,
        camera,
        rows,
    );
}

/// Builds the window. Split out of the system so a test can call it.
pub fn spawn_quest_journal(
    commands: &mut Commands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    camera: Entity,
    rows: Vec<(u32, String, bool)>,
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
            // The 2dt rects are absolute design space, so the content
            // container sits on the window origin and every child below is
            // `authored - ROOT`.
            outer: WINDOW_SIZE,
            content_at: (0.0, 0.0),
        },
        Some(InnerFrame {
            dir: INNER_FRAME_DIR,
            rect: INNER_FRAME,
            corner: INNER_FRAME_PIECE,
        }),
        ANCHOR_RIGHT_TOP,
        s,
        game_window::GameWindowStyle::default(),
    );
    commands
        .entity(window.root)
        .insert((QuestJournalRoot, QuestJournalPage(rows.clone())));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_window);

    let font = fonts.two.clone();
    let text_px = font_px(0);
    commands.entity(window.content).with_children(|content| {
        content.spawn((
            game_window::abs_node(BG_RECT, s),
            ImageNode {
                image: asset_server.load(BG_TILE_A),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        ));
        for ((id, line, is_tracked), y) in rows.iter().zip(ROW_Y) {
            content
                .spawn((
                    QuestJournalRow(*id),
                    game_window::abs_node((ROW_X, y, ROW_SIZE.0, ROW_SIZE.1), s),
                    ImageNode::new(asset_server.load(ROW_DDJ)),
                ))
                .with_children(|row| {
                    row.spawn((
                        game_window::abs_node(PLATE_RECT, s),
                        ImageNode::new(asset_server.load(PLATE_DDJ)),
                        Pickable::IGNORE,
                    ));
                    row.spawn((
                        game_window::abs_node(CHECK_RECT, s),
                        Button,
                        Hovered::default(),
                        ImageNode::new(asset_server.load(if *is_tracked {
                            CHECK_ON_DDJ
                        } else {
                            CHECK_OFF_DDJ
                        })),
                    ))
                    .observe(on_track_checkbox);
                    row.spawn((
                        game_window::abs_node(TITLE_RECT, s),
                        Text::new(line.clone()),
                        TextFont {
                            font: font.clone().into(),
                            font_size: FontSize::Px(text_px),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Pickable::IGNORE,
                    ));
                });
        }
    });
    window.root
}

/// The per-row checkbox (id 8): tick it and the quest joins the mini-list,
/// untick it and its row there goes. That is the whole point of this window —
/// `TrackedQuests` finally has the operator the original gives it.
///
/// A full strip refuses a fourth quest ([`TrackedQuests::CAPACITY`] is the
/// data's three rows), and the box then stays unticked, which is the honest
/// rendering of what happened.
fn on_track_checkbox(
    activate: On<Activate>,
    parents: Query<&ChildOf>,
    rows: Query<&QuestJournalRow>,
    mut tracked: ResMut<TrackedQuests>,
) {
    let mut node = activate.entity;
    loop {
        if let Ok(row) = rows.get(node) {
            if tracked.is_tracked(row.0) {
                tracked.untrack(row.0);
            } else if !tracked.track(row.0) {
                info!(
                    "quest journal: the tracker already holds its {} rows",
                    TrackedQuests::CAPACITY
                );
            }
            return;
        }
        match parents.get(node) {
            Ok(parent) => node = parent.parent(),
            Err(_) => return,
        }
    }
}

fn on_close_window(_activate: On<Activate>, mut window: ResMut<QuestJournalWindow>) {
    window.open = false;
}

/// The original's opener: `KeyQuest`, Q by default.
///
/// The action id is spelled as the literal `3006` — `KEY_ACTIONS` id 3006
/// `KeyQuest`, default `KeyCode::KeyQ`, captioned by `UIIT_STT_TOGGLE_QUEST`
/// "Quest ( Q )" (`settings/keymap.rs`'s `KEY_ACTIONS`). There
/// is no `KEY_QUEST` constant, and the literal is also the needle
/// `keymap.rs`'s own "every bound key is consumed" test searches for
/// (`key_for(3006`), so writing it any other way would leave the key looking
/// dead to that test. The chat guard is the same one every other shortcut in
/// the tree uses: Q must type a Q while the chat input has the keyboard.
pub fn quest_journal_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<ChatState>,
    options: Res<GameOptions>,
    mut window: ResMut<QuestJournalWindow>,
) {
    let Some(key) = options.key_for(3006) else {
        return;
    };
    if keys.just_pressed(key) && !chat.input_open {
        window.open = !window.open;
    }
}

/// Leaving the world takes the window with it, and closes it — a re-entered
/// world must not pop a journal nobody opened.
pub fn cleanup_quest_journal(
    mut commands: Commands,
    mut window: ResMut<QuestJournalWindow>,
    open: Query<Entity, With<QuestJournalRoot>>,
) {
    window.open = false;
    for entity in open.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use packets::agent::quest::ActiveQuest;

    fn journal_of(ids: &[u32]) -> QuestJournal {
        let quests: Vec<ActiveQuest> = ids
            .iter()
            .map(|id| ActiveQuest {
                id: *id,
                achievements: 16,
                autoshare: 0,
                quest_type: 24,
                remaining_time: None,
                state: 1,
                objectives: Vec::new(),
                npcs: Vec::new(),
            })
            .collect();
        QuestJournal::from_active(&quests, None, None, None)
    }

    /// The nine live `Devi` ids (§11.2) all get a row, and the three the
    /// tracker seeded come back ticked — the state the strip shows and the
    /// state the box shows are one value, not two.
    #[test]
    fn every_journal_entry_gets_a_row_and_the_tracked_ones_are_ticked() {
        let journal = journal_of(&[3, 6, 11, 48, 53, 57, 58, 220, 399]);
        let mut tracked = TrackedQuests::default();
        tracked.seed_from(&journal);
        let rows = page_rows(&journal, &tracked);
        assert_eq!(rows.len(), 9);
        let ticked: Vec<u32> = rows
            .iter()
            .filter(|(_, _, on)| *on)
            .map(|(id, _, _)| *id)
            .collect();
        assert_eq!(ticked, vec![3, 6, 11]);
    }

    /// Ten rows is the window, not the journal: an eleventh quest is not drawn
    /// (and `sync_quest_journal_window` logs that it is not).
    #[test]
    fn the_page_stops_at_the_ten_authored_rows() {
        let journal = journal_of(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        let rows = page_rows(&journal, &TrackedQuests::default());
        assert_eq!(rows.len(), ROW_Y.len());
        assert_eq!(rows.len(), 10);
    }

    /// The geometry is the file's. `y` is transcribed row by row, so the test
    /// is what says the authored 31-pitch-on-32-rows overlap is intentional
    /// and not a typo in one of the ten numbers.
    #[test]
    fn the_row_geometry_is_the_authored_one() {
        assert_eq!(ROW_SIZE, (576.0, 32.0));
        for pair in ROW_Y.windows(2) {
            assert_eq!(pair[1] - pair[0], 31.0, "authored pitch");
        }
        assert_eq!(ROW_Y[0], 236.0 - ROOT.1);
        assert_eq!(ROW_Y[9], 515.0 - ROOT.1);
        // the row's own children, row-local, identical on all ten rows
        assert_eq!(CHECK_RECT, (9.0, 7.0, 16.0, 16.0));
        assert_eq!(TITLE_RECT, (34.0, 8.0, 383.0, 16.0));
        // the title text starts inside the plate and ends inside the row
        assert!(TITLE_RECT.0 > PLATE_RECT.0);
        assert!(TITLE_RECT.0 + TITLE_RECT.2 < ROW_SIZE.0);
        // the window's anchor is the authored origin on a 1024 canvas
        assert_eq!(
            ANCHOR_RIGHT_TOP,
            (202.0, 193.0),
            "1024 - 198 - 624 to the right edge, and the authored y"
        );
    }
}

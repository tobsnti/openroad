//! The quest **mini-list** — the on-screen strip of tracked quests.
//!
//! Idea: this is the only quest surface the original keeps on screen while
//! every window is closed, which is exactly what a journal that has no window
//! yet needs. It is the visible half of the reception chain in
//! [`super::model`]: `CHARACTER_DATA` / `0x30D5` land in `QuestJournal`,
//! `TrackedQuests` picks the rows, and this file draws them.
//!
//! **Every number below was read out of the user's own
//! `Media/res_ui/questminilist.2dt`** (9,764 B = `4 + 10 * 976`, re-measured
//! for this module rather than copied out of the doc; it agrees with
//! `docs/re/ui/hud-quest-windows.md` §3.5 field for field):
//!
//! ```text
//! [0] CNIFMiniQuestWnd   id 173  (702,267,324,93)   no Image, no Background
//! [1] CNIFMiniQuestSlot  id 115  (702,267,324,30)   qst_list_background.ddj
//! [7] CNIFMiniQuestSlot  id  17  (702,297,324,30)   qst_list_background.ddj
//! [4] CNIFMiniQuestSlot  id 121  (702,327,324,30)   qst_list_background.ddj
//!       child id 56 CNIFButton 12x12 qst_list_close_button.ddj  row-local +48,+8
//!       child id 57 CNIFStatic 251x16                           row-local +66,+7
//! ```
//!
//! Pitch is 30 against a 30 px row — this list is the one in the batch with
//! **no** 1 px overlap, so nothing has to be reconciled here.
//!
//! Two deliberate deviations, both with their reason (ADR-0009):
//!
//! 1. **Right-anchored instead of `x = 702`.** The authored root spans
//!    702..1026, which is 2 px past a 1024-wide screen; the number is not
//!    usable literally and the original must be anchoring at runtime
//!    (`hud-quest-windows.md` §3.5, `[S]`). We anchor the strip's right edge to
//!    the screen's right edge and keep the authored `y` and the authored
//!    row-local offsets, so only the one unusable number is replaced.
//! 2. **The row's single 251x16 static shows title *and* the tracked
//!    objective line**, joined by an en dash. The original's row is a header
//!    only, and its progress lives in the classic tracker (`ifquestinfo.txt`),
//!    which this lane did not build. Showing the objective is what makes a
//!    `0x30D5` counter tick *visible* — a tracker that cannot show progress
//!    would be a window without an effect, which is the defect class this
//!    round is closing. The row is the place to revisit when `ifquestinfo`
//!    lands.
//!
//! The id-121 row's static is authored 1 px further left than the other two
//! (`+65` vs `+66`); that is hand-authoring jitter in the data and is **not**
//! reproduced — all three rows use `+66`, stated here so the difference is not
//! mistaken for a transcription slip.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::scale::{font_px, hud_scale};
use crate::plugins::net::quest::QuestJournal;

use super::model::TrackedQuests;

/// `CNIFMiniQuestWnd` id 173: `(702,267,324,93)`.
const STRIP_TOP: f32 = 267.0;
const STRIP_W: f32 = 324.0;
/// The authored root ends at x 1026 on a 1024 canvas — see the module header;
/// we anchor to the right edge instead of reproducing the overrun. In screen
/// pixels, not design units: an edge-anchored offset is the same at every HUD
/// scale, so this one number is deliberately not multiplied by `hud_scale()`.
const STRIP_RIGHT: f32 = 0.0;
/// `CNIFMiniQuestSlot`: `(…,324,30)`, y 267/297/327 → pitch 30, no overlap.
const ROW_SIZE: (f32, f32) = (324.0, 30.0);
const ROW_PITCH: f32 = 30.0;
/// Row-local child rects (`id 56` and `id 57`).
const CLOSE_RECT: (f32, f32, f32, f32) = (48.0, 8.0, 12.0, 12.0);
const LABEL_RECT: (f32, f32, f32, f32) = (66.0, 7.0, 251.0, 16.0);

const ROW_DDJ: &str = "media://interface/quest/qst_list_background.ddj";
const CLOSE_DDJ: &str = "media://interface/quest/qst_list_close_button.ddj";

/// The strip itself.
#[derive(Component)]
pub struct QuestMiniListRoot;

/// One row, keyed by the quest it shows — a row is untracked by id, never by
/// index (`TrackedQuests` explains why).
#[derive(Component, Clone, Copy)]
pub struct QuestMiniRow(pub u32);

/// What the strip was last built from, so an unchanged journal costs nothing.
#[derive(Component, PartialEq, Eq, Default)]
pub struct QuestMiniListState(Vec<(u32, String)>);

/// The line one row shows: the quest's title plus its first *unfinished-looking*
/// objective line, which is where the wire's counter appears
/// (`JournalObjective::line` substitutes the `%d`).
fn row_line(journal: &QuestJournal, id: u32) -> Option<String> {
    let entry = journal.get(id)?;
    let title = entry.display_title();
    match entry.objectives.first() {
        Some(objective) => Some(format!("{title} — {}", objective.line())),
        None => Some(title),
    }
}

/// Rebuild the strip when the tracked set or the journal behind it changes.
///
/// A full rebuild rather than a diff: three rows.
pub fn sync_quest_mini_list(
    mut commands: Commands,
    journal: Res<QuestJournal>,
    tracked: Res<TrackedQuests>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    cameras: Query<Entity, With<Camera2d>>,
    existing: Query<(Entity, &QuestMiniListState), With<QuestMiniListRoot>>,
) {
    let rows: Vec<(u32, String)> = tracked
        .ids()
        .iter()
        .filter_map(|id| row_line(&journal, *id).map(|line| (*id, line)))
        .collect();

    for (entity, state) in existing.iter() {
        if state.0 == rows {
            return;
        }
        commands.entity(entity).despawn();
    }
    if rows.is_empty() {
        return;
    }
    let Ok(camera) = cameras.single() else {
        warn!("quest mini-list: no 2d camera to attach to");
        return;
    };

    let s = hud_scale();
    let font = fonts.two.clone();
    let text_px = font_px(0);
    let root = commands
        .spawn((
            QuestMiniListRoot,
            QuestMiniListState(rows.clone()),
            Name::from("Quest Mini List"),
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(STRIP_TOP * s),
                right: Val::Px(STRIP_RIGHT),
                width: Val::Px(STRIP_W * s),
                height: Val::Px(rows.len() as f32 * ROW_PITCH * s),
                ..default()
            },
            GlobalZIndex(35),
            Visibility::default(),
            UiTargetCamera(camera),
        ))
        .id();

    commands.entity(root).with_children(|strip| {
        for (index, (id, line)) in rows.iter().enumerate() {
            let y = index as f32 * ROW_PITCH;
            strip
                .spawn((
                    QuestMiniRow(*id),
                    abs_node((0.0, y, ROW_SIZE.0, ROW_SIZE.1), s),
                    ImageNode::new(asset_server.load(ROW_DDJ)),
                ))
                .with_children(|row| {
                    row.spawn((
                        abs_node(CLOSE_RECT, s),
                        Button,
                        Hovered::default(),
                        ImageNode::new(asset_server.load(CLOSE_DDJ)),
                    ))
                    .observe(on_close_row);
                    row.spawn((
                        abs_node(LABEL_RECT, s),
                        Text::new(line.clone()),
                        TextFont {
                            font: font.clone().into(),
                            font_size: FontSize::Px(text_px),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Button,
                        Hovered::default(),
                    ))
                    .observe(on_open_journal);
                });
        }
    });
}

/// The row's text opens the journal window.
///
/// **Stated deviation:** the original's opener is the `KeyQuest` binding
/// (id 3006), and that is wired — `journal::quest_journal_hotkey` consumes it.
/// This is the second, smaller way in: the strip is the one quest surface that
/// is always on screen, and clicking a quest's line to open the list it came
/// from is what a player tries first. It costs one observer and no data claim;
/// the key remains the evidenced opener.
fn on_open_journal(
    _activate: On<Activate>,
    mut window: ResMut<super::journal::QuestJournalWindow>,
) {
    window.open = true;
}

/// The row's 12x12 close button: untrack that quest. The row disappears on the
/// next `sync_quest_mini_list`, which is the observable effect this button owes
/// the player — it does **not** abandon the quest (that would be `0x70D9`, and
/// no evidenced body of it exists, see `docs/planning/QUEST-audit.md`).
fn on_close_row(
    activate: On<Activate>,
    parents: Query<&ChildOf>,
    rows: Query<&QuestMiniRow>,
    mut tracked: ResMut<TrackedQuests>,
) {
    let mut node = activate.entity;
    loop {
        if let Ok(row) = rows.get(node) {
            tracked.untrack(row.0);
            return;
        }
        match parents.get(node) {
            Ok(parent) => node = parent.parent(),
            Err(_) => return,
        }
    }
}

/// Leaving the world takes the strip with it.
pub fn cleanup_quest_mini_list(
    mut commands: Commands,
    strips: Query<Entity, With<QuestMiniListRoot>>,
) {
    for entity in strips.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use packets::agent::quest::{ActiveQuest, QuestObjective};

    fn hunt_quest() -> ActiveQuest {
        // The shape of the `Devi` record for quest 57 (§11.2), reduced to the
        // one objective whose text carries a `%d`.
        ActiveQuest {
            id: 57,
            achievements: 16,
            autoshare: 0,
            quest_type: 24,
            remaining_time: None,
            state: 1,
            objectives: vec![QuestObjective {
                id: 1,
                enabled: 1,
                name_key: "SN_CON_QNO_CH_GENARAL_1_02".into(),
                tasks: vec![4],
            }],
            npcs: Vec::new(),
        }
    }

    /// The line the strip draws is the client's own sentence with the **wire**
    /// counter in it — the whole reason the tracker shows the objective.
    #[test]
    fn a_row_shows_the_wire_counter_inside_the_client_text() {
        use crate::assets::textdata::uisystem::UiSystemText;
        let mut strings = UiSystemText::default();
        strings.0.insert(
            "SN_CON_QNO_CH_GENARAL_1_02".into(),
            "Hunt 15 TIger (%d)".into(),
        );
        let journal = QuestJournal::from_active(&[hunt_quest()], None, Some(&strings), None);
        let line = row_line(&journal, 57).expect("the tracked quest has a line");
        assert!(line.contains("Hunt 15 TIger (4)"), "line was {line:?}");
    }

    /// A tracked id the journal does not have draws nothing at all rather than
    /// an empty row.
    #[test]
    fn an_untracked_or_unknown_quest_has_no_line() {
        let journal = QuestJournal::from_active(&[hunt_quest()], None, None, None);
        assert!(row_line(&journal, 999).is_none());
    }

    /// The rects are the measured ones, and the three rows tile without a gap
    /// or an overlap (pitch 30 == row height 30, unlike every other list in
    /// this batch).
    #[test]
    fn the_row_geometry_is_the_authored_one() {
        assert_eq!(ROW_SIZE, (324.0, 30.0));
        assert_eq!(ROW_PITCH, ROW_SIZE.1);
        assert_eq!(CLOSE_RECT, (48.0, 8.0, 12.0, 12.0));
        assert_eq!(LABEL_RECT, (66.0, 7.0, 251.0, 16.0));
        // the label ends 7 px short of the row's right edge, as authored
        assert_eq!(LABEL_RECT.0 + LABEL_RECT.2, 317.0);
        assert!(LABEL_RECT.0 + LABEL_RECT.2 < ROW_SIZE.0);
    }
}

//! The tracked-quest strip — `res_ui/questminilist.2dt`, root `CNIFMiniQuestWnd`.
//!
//! Idea: this is the only quest surface the original keeps on screen while every
//! window is closed, so it is what makes an objective counter visible without
//! the player opening anything. It needs no new wire and no new strings: the
//! rows are the active quests `CHARACTER_DATA` already mirrors into
//! [`CharacterInfo`], and the two texts per row are the ones the list window and
//! the detail pane already resolve — [`row_title`] and [`condition_line`] are
//! called here, not copied.
//!
//! **The layout is the descriptor's.** Re-measured in the player's own
//! `Media/res_ui/questminilist.2dt` (9,764 B = 4 + 10 x 976):
//!
//! ```text
//! [0] CNIFWnd    id 173  (702,267,324,93)  root
//! [1] CNIFSlot   id 115  (702,267,324,30)  qst_list_background.ddj
//! [7] CNIFSlot   id  17  (702,297,324,30)  qst_list_background.ddj
//! [4] CNIFSlot   id 121  (702,327,324,30)  qst_list_background.ddj
//!       CNIFButton id 56   12x12  qst_list_close_button.ddj   row-local +48,+8
//!       CNIFStatic id 57  251x16                              row-local +66,+7
//! ```
//!
//! Three rows, pitch 30 against a 30-unit row — this descriptor is the one with
//! no overlap to reconcile. **Record order is not row order** (115, 121, 17 by
//! file against y 267/327/297), so rows are sorted by their own `y`, like
//! `hud::free_pvp` sorts its.
//!
//! **One deliberate deviation (ADR-0009): the strip is docked to the right
//! edge.** The authored root spans x 702..1026, two units past a 1024-wide
//! canvas, so that number cannot be used literally and the original must place
//! it at runtime. It is not alone: 3 of the 42 descriptors in the player's
//! `res_ui` overrun 1024 (`worldmapguide.2dt` reaches 1146), which is why this
//! reads as "authored against a wider canvas" rather than as a typo. The
//! authored `y` and every row-local offset are kept.
//!
//! The id-121 row authors its static one unit further left than the other two
//! (+65 against +66) — hand-authoring jitter, carried as authored, named here so
//! it is not mistaken for a transcription slip.
//!
//! Dismissing a row is per session. `qst_list_open_button.ddj` sits beside the
//! close art in the player's archive but no descriptor entry references it, so
//! what re-opens a dismissed row in the original is [U].

use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::ui::widget::NodeImageMode;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::character_data::ActiveQuest;

use crate::assets::twodt::{Jmxv2dtType, JMXV2DT};
use crate::assets::FontAssets;
use crate::plugins::hud::art::art_path;
use crate::plugins::hud::quest::row_title;
use crate::plugins::hud::quest_detail::condition_line;
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientQuestTable, ClientSpeechText, ClientUiStrings};
use crate::scenes::SceneState;

/// `media://` + the descriptor's own path, like every other `.2dt` consumer.
const DESCRIPTOR: &str = "media://res_ui/questminilist.2dt";
/// The row static is 16 units tall; 9 is the size the other descriptor-driven
/// rows use for a one-line caption (`hud::free_pvp`).
const LABEL_FONT: f32 = 9.0;
/// Between the strip's right edge and the screen's, in screen pixels — an
/// edge-anchored gap is the same at every HUD scale, so it is not scaled.
const RIGHT_GAP: f32 = 0.0;
/// Title and objective in one static, joined: the authored row has one text
/// control and the objective is the half that moves.
const LABEL_JOIN: &str = " — ";

/// The quests the player closed on the strip this session, by `ref_id`.
#[derive(Resource, Default, Debug)]
pub struct DismissedQuests(pub Vec<u32>);

#[derive(Component)]
struct QuestStrip;

/// Holds the descriptor handle so the strip can rebuild when the asset lands.
#[derive(Resource)]
struct QuestStripDescriptor(Handle<JMXV2DT>);

/// Which active quests the strip shows: the first `rows` that are not
/// dismissed, in the order the server sent them.
fn shown_quests(active: &[ActiveQuest], dismissed: &[u32], rows: usize) -> Vec<usize> {
    active
        .iter()
        .enumerate()
        .filter(|(_, quest)| !dismissed.contains(&quest.ref_id))
        .map(|(index, _)| index)
        .take(rows)
        .collect()
}

/// One row's text. The objective is dropped when it adds nothing — an empty or
/// repeated line would only eat the row's 251 units.
fn row_label(title: &str, objective: &str) -> String {
    if objective.is_empty() || objective == title {
        return title.to_string();
    }
    format!("{title}{LABEL_JOIN}{objective}")
}

/// The slot rows, top to bottom, each with its own art and its two children.
fn rows(descriptor: &JMXV2DT) -> Vec<&crate::assets::twodt::Jmxv2dtEntry> {
    let mut rows: Vec<_> = descriptor
        .entries()
        .iter()
        .filter(|entry| entry.ni_type() == Some(Jmxv2dtType::CNIFSlot))
        .collect();
    rows.sort_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y));
    rows
}

fn load_descriptor(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(QuestStripDescriptor(asset_server.load(DESCRIPTOR)));
}

#[allow(clippy::too_many_arguments)]
fn rebuild_quest_strip(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    descriptors: Res<Assets<JMXV2DT>>,
    handle: Option<Res<QuestStripDescriptor>>,
    dismissed: Res<DismissedQuests>,
    quests: Res<ClientQuestTable>,
    speech: Res<ClientSpeechText>,
    ui_strings: Res<ClientUiStrings>,
    players: Query<&CharacterInfo, (With<Player>, Changed<CharacterInfo>)>,
    infos: Query<&CharacterInfo, With<Player>>,
    strips: Query<Entity, With<QuestStrip>>,
    cameras: Query<Entity, With<Camera2d>>,
) {
    let changed = !players.is_empty() || dismissed.is_changed() || descriptors.is_changed();
    if !changed {
        return;
    }
    for strip in strips.iter() {
        commands.entity(strip).despawn();
    }
    let (Some(handle), Ok(info), Ok(camera)) = (handle, infos.single(), cameras.single()) else {
        return;
    };
    // Without the descriptor there is no strip: the layout is the data's and we
    // carry no transcribed fallback of it.
    let Some(descriptor) = descriptors.get(&handle.0) else {
        return;
    };
    let Some(root) = descriptor.root() else {
        return;
    };
    let slots = rows(descriptor);
    let shown = shown_quests(&info.active_quests, &dismissed.0, slots.len());
    if shown.is_empty() {
        return;
    }

    let s = hud_scale();
    let node = |rect: Rect| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(rect.min.x * s),
        top: Val::Px(rect.min.y * s),
        width: Val::Px(rect.width() * s),
        height: Val::Px(rect.height() * s),
        ..default()
    };
    let image = |path: &str| ImageNode {
        image: asset_server.load(path.to_string()),
        image_mode: NodeImageMode::Stretch,
        ..default()
    };
    let root_size = root.rect().size();
    let strip = commands
        .spawn((
            QuestStrip,
            Name::from("Quest Strip"),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(RIGHT_GAP),
                top: Val::Px(root.rect().min.y * s),
                width: Val::Px(root_size.x * s),
                height: Val::Px(root_size.y * s),
                ..default()
            },
            GlobalZIndex(30),
            UiTargetCamera(camera),
        ))
        .id();

    for (slot, index) in slots.iter().zip(shown) {
        let quest = &info.active_quests[index];
        let ref_id = quest.ref_id;
        let title = row_title(ref_id, quests.table(), &ui_strings);
        let objective = condition_line(quest, |key| speech.get(key).map(str::to_string));
        commands.entity(strip).with_child((
            node(descriptor.local_rect(slot)),
            image(&art_path(slot.background())),
            Pickable::IGNORE,
        ));
        for child in descriptor.children_of(slot.id()) {
            let rect = descriptor.local_rect(child);
            match child.ni_type() {
                Some(Jmxv2dtType::CNIFButton) => {
                    commands
                        .entity(strip)
                        .with_child((Button, node(rect), image(&art_path(child.background()))))
                        .observe(
                            move |_: On<Activate>, mut dismissed: ResMut<DismissedQuests>| {
                                if !dismissed.0.contains(&ref_id) {
                                    dismissed.0.push(ref_id);
                                }
                            },
                        );
                }
                Some(Jmxv2dtType::CNIFStatic) => {
                    commands.entity(strip).with_child((
                        node(rect),
                        Pickable::IGNORE,
                        children![(
                            Text::new(row_label(&title, &objective)),
                            TextFont {
                                font: fonts.nine.clone().into(),
                                font_size: FontSize::Px(text_px(LABEL_FONT, s)),
                                ..default()
                            },
                            TextColor(child.color()),
                            Pickable::IGNORE,
                        )],
                    ));
                }
                _ => {}
            }
        }
    }
}

fn cleanup_quest_strip(
    mut commands: Commands,
    strips: Query<Entity, With<QuestStrip>>,
    mut dismissed: ResMut<DismissedQuests>,
) {
    for strip in strips.iter() {
        commands.entity(strip).despawn();
    }
    dismissed.0.clear();
}

pub struct QuestStripPlugin;

impl Plugin for QuestStripPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DismissedQuests>()
            .add_systems(OnEnter(SceneState::GameWorld), load_descriptor)
            .add_systems(OnExit(SceneState::GameWorld), cleanup_quest_strip)
            .add_systems(
                Update,
                rebuild_quest_strip.run_if(in_state(SceneState::GameWorld)),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// A record with nothing but its id: `shown_quests` keys on `ref_id` alone,
    /// and `quest_type` 0 is the plain form that carries neither a remaining
    /// time nor an NPC list (`character_data::QUEST_TYPE_*`).
    fn quest(ref_id: u32) -> ActiveQuest {
        ActiveQuest {
            ref_id,
            achievements: 0,
            auto_share_required: 0,
            quest_type: 0,
            time_remain: None,
            state: 1,
            objectives: Vec::new(),
            target_npcs: Vec::new(),
        }
    }

    /// The strip shows the first rows it has slots for, skipping what the player
    /// closed — so dismissing the top quest promotes the next one instead of
    /// leaving a hole.
    #[test]
    fn dismissed_quests_are_skipped_and_the_rest_moves_up() {
        let active = [quest(10), quest(11), quest(12), quest(13)];
        assert_eq!(shown_quests(&active, &[], 3), vec![0, 1, 2]);
        assert_eq!(shown_quests(&active, &[10], 3), vec![1, 2, 3]);
        assert_eq!(shown_quests(&active, &[11, 12], 3), vec![0, 3]);
        assert!(shown_quests(&[], &[], 3).is_empty());
    }

    /// The row has one text control, so title and objective share it — but only
    /// when the objective says something the title does not.
    #[test]
    fn the_row_label_joins_only_a_useful_objective() {
        assert_eq!(
            row_label("Hunt", "Hunt 15 Water Ghost (0)"),
            "Hunt — Hunt 15 Water Ghost (0)"
        );
        assert_eq!(row_label("Hunt", ""), "Hunt");
        assert_eq!(row_label("Hunt", "Hunt"), "Hunt");
    }
}

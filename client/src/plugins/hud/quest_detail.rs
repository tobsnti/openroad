//! The quest detail pane (`GDR_QUESTINFO`): what one active quest wants, in
//! the player's own words.
//!
//! Idea: the list window (`hud::quest`) says *which* quests are open; this
//! pane says what one of them asks for. Every string comes from the player's
//! own data through tables the client already loads — the objective sentence
//! from `textquest_speech&name.txt`, the NPC name through `characterdata` ->
//! `textdata_object.txt`, the reward from `refqusetreward.txt`.
//!
//! **The objective chain, measured on our own captures.** The wire gives each
//! objective a text key (`QuestObjective::name`, documented as `SN_CON_*` in
//! `character_data.rs` before this slice). All 11 keys in the proxy recording
//! of the *original* client (`packet_dump/proxy/0x3013.log`, frame 2, 2745 B)
//! resolve in the player's `textquest_speech&name.txt` — e.g.
//! `SN_CON_QNO_CH_POTION_1` -> "Hunt 15 Water Ghost (%d)" (`:2717`). The
//! required count is part of the sentence; the `(%d)` is the live counter.
//!
//! The hypothesis for this chain came from outside (OpenSRO, AGPL-3.0, v1.150
//! — behaviour read, **no code or text taken**) and named `textquest.txt`. In
//! the player's v1.188 data that file is a **164-byte index** of its three
//! members; the 1383 `SN_CON_*` rows live in the `Speech&Name` member, which
//! is the one the loader already reads.
//!
//! **What fills `%d` is [S], not [V].** `task_count` is 1 in all 374 objective
//! lines of our three `0x3013` dumps and the sentence carries exactly one
//! `%d`, so the arity matches with nothing left over. But no capture shows a
//! `%d` objective with a counter above zero: all 34 non-zero counters belong
//! to one talk objective whose text has no `%d` (and which is also the only
//! one with `enabled == false`). A capture of a part-hunted quest would settle
//! it.
//!
//! **The reward row is ours, and why.** The authored slot has three rows —
//! title, NPC, condition — and no reward control. The original's reward text
//! is a `SN_PAYCON_*` paragraph in `textquest_otherstring.txt` (994 rows) of
//! `<sml2><strong><font color=...>` markup we have no renderer for. Its
//! numbers are the ones we already hold structured: `refqusetreward.txt`
//! columns 10/11 were *named* by matching that very paragraph (399 of 402 and
//! 502 of 610 rows, `textdata::questreward`). So the pane shows the same fact
//! from the structured table, in one more row at the authored pitch.

use bevy::prelude::*;
use bevy::text::LineHeight;
use bevy::ui::widget::NodeImageMode;
use bevy::ui_widgets::Activate;

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::textdata::questreward::QuestRewardValue;
use crate::plugins::hud::game_window::{abs_node, spawn_game_window};
use crate::plugins::hud::quest::{row_title, QuestRow};
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::player::Player;
use crate::plugins::textdata::{
    ClientCharacterData, ClientQuestRewards, ClientQuestTable, ClientSpeechText, ClientTextNames,
    ClientUiStrings,
};
use crate::scenes::intro_v2::fill_placeholders;
use crate::scenes::SceneState;
use packets::agent::character_data::ActiveQuest;
#[cfg(test)]
use packets::agent::character_data::QuestObjective;

/// Host width (`GDR_QUESTINFO_SCROLLMGR` `9,37,298,96`, `ifquestinfo.txt:6`)
/// plus one row pitch for the reward row. The authored outer `0,0,315,142`
/// (`ginterface.txt:867`, `DDJ="interface\frame\mframe_wnd_"` — the family
/// this shell draws) is not transcribed: our ring is 3 units wider per side,
/// so the interior is what carries over.
const CONTENT: (f32, f32) = (298.0, SLOT_H + ROW_PITCH);
/// `GDR_QUESTINFO_SLOT` `0,0,280,96` (`ifquestinfoslot.txt:101`), its art
/// `qst_info.ddj` measuring 280x96 in the player's archive. The x is derived:
/// `(298 - 280) / 2`.
const SLOT_W: f32 = 280.0;
const SLOT_H: f32 = 96.0;
const SLOT_X: f32 = (CONTENT.0 - SLOT_W) / 2.0;
const SLOT_ART: &str = "media://interface/questinfo/qst_info.ddj";
/// `GDR_QUESTINFO_SLOT_TITLE` `16,7,248,13` (`:82`).
const TITLE_RECT: (f32, f32, f32, f32) = (16.0, 7.0, 248.0, 13.0);
/// The label/value columns both label rows share: `8,_,51,13` (`:25`, `:63`)
/// and `66,_,200,13` (`:6`, `:44`).
const LABEL_X: f32 = 8.0;
const LABEL_W: f32 = 51.0;
const VALUE_X: f32 = 66.0;
const VALUE_W: f32 = 200.0;
const LINE_H: f32 = 13.0;
/// `GDR_QUESTINFO_SLOT_NPC` y (`:44`) and the condition row's y (`:6`); their
/// difference is the pitch the reward row reuses.
const NPC_Y: f32 = 38.0;
const COND_Y: f32 = 67.0;
const ROW_PITCH: f32 = COND_Y - NPC_Y;
/// Ours: the reward row, one pitch below the condition row.
const REWARD_Y: f32 = COND_Y + ROW_PITCH;
/// Every block in both trees carries `FontIndex=0`.
const TEXT_FONT_INDEX: usize = 0;
/// `FontColor="255,239,218,164"` on the two `_STATIC` labels (`:25`, `:63`);
/// resinfo COLOR is A,R,G,B. Same value as `party_matching`'s `LABEL_COLOR`.
const LABEL_COLOR: Color = Color::srgb_u8(239, 218, 164);
/// `FontColor="255,255,255,255"` on the three value blocks.
const VALUE_COLOR: Color = Color::WHITE;
/// Ours: the anchor. `Section = QuestInfo,"0,0,0,0","0"` (`ginterface.txt:865`)
/// places the window at all-zero, so the client positions it at runtime.
const ANCHOR: (f32, f32) = (560.0, 120.0);
/// Ours: the word the player's own reward paragraph uses for experience
/// (`SN_PAYCON_QNO_CH_SMITH_1`: "Exp 270 / GOLD 205"); `textuisystem.txt` has
/// no `UIIT_*` key for it, while gold has one.
const EXP_WORD: &str = "Exp";
/// Draw order: y, the label's own `Text` key, our fallback, the value cell it
/// feeds. `UIIT_STT_NPC` (`textuisystem.txt:844`) and
/// `UIIT_STT_QUEST_CONDITION` (`:551`) are the keys the `_STATIC` blocks
/// declare; the reward row reuses `UIIT_STT_QUEST_REWARD` (`:544`).
const LABEL_ROWS: [(f32, &str, &str, DetailCell); 3] = [
    (NPC_Y, "UIIT_STT_NPC", "NPC", DetailCell::Npc),
    (
        COND_Y,
        "UIIT_STT_QUEST_CONDITION",
        "Condition",
        DetailCell::Condition,
    ),
    (
        REWARD_Y,
        "UIIT_STT_QUEST_REWARD",
        "Reward",
        DetailCell::Reward,
    ),
];
/// Shown where a quest has no target NPC — 595 of the 765 records in our
/// captures are the type that carries none.
const NO_VALUE: &str = "-";

/// Which active quest the pane shows, as an index into
/// `CharacterInfo::active_quests` — the same index the list rows carry.
#[derive(Resource, Default, Debug)]
pub struct QuestDetailState {
    pub open: bool,
    pub selected: usize,
}

#[derive(Component)]
pub struct QuestDetailRoot;

/// The three value cells, in the order the slot draws them.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum DetailCell {
    Title,
    Npc,
    Condition,
    Reward,
}

pub fn spawn_quest_detail(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<crate::assets::FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cameras: Query<Entity, With<Camera2d>>,
    mut state: ResMut<QuestDetailState>,
) {
    let Ok(camera) = cameras.single() else {
        warn!("quest detail: no 2d camera to attach to");
        return;
    };
    *state = QuestDetailState::default();
    let s = hud_scale();
    let window = spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        // the authored window declares an empty `Text`, so the caption key is
        // ours from the same table
        ui_strings.get_or("UIIT_STT_QUEST", "Quest"),
        CONTENT,
        ANCHOR,
        s,
    );
    commands
        .entity(window.root)
        .insert((QuestDetailRoot, GlobalZIndex(21)));
    commands.entity(window.expect_close_button()).observe(
        |_: On<Activate>, mut state: ResMut<QuestDetailState>| {
            state.open = false;
        },
    );

    let size = text_px(font_index_px(TEXT_FONT_INDEX), s);
    // Every text box in this tree is 13 tall against a 12 px line, so all of
    // them are one-line boxes (`font_index_px`'s "One-line boxes").
    let cell = move |rect: (f32, f32, f32, f32), color: Color, text: &str| {
        (
            Text::new(text.to_string()),
            TextFont {
                font: fonts.two.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            LineHeight::Px(size),
            TextColor(color),
            TextLayout::justify(Justify::Left),
            abs_node(rect, s),
            Pickable::IGNORE,
        )
    };
    commands.entity(window.content).with_children(|page| {
        page.spawn((
            abs_node((SLOT_X, 0.0, SLOT_W, CONTENT.1), s),
            ImageNode {
                image: asset_server.load(SLOT_ART),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        page.spawn((DetailCell::Title, cell(TITLE_RECT, VALUE_COLOR, "")));
        for (y, key, fallback, which) in LABEL_ROWS {
            page.spawn(cell(
                (SLOT_X + LABEL_X, y, LABEL_W, LINE_H),
                LABEL_COLOR,
                ui_strings.get_or(key, fallback),
            ));
            page.spawn((
                which,
                cell((SLOT_X + VALUE_X, y, VALUE_W, LINE_H), VALUE_COLOR, ""),
            ));
        }
    });
}

pub fn cleanup_quest_detail(
    mut commands: Commands,
    roots: Query<Entity, With<QuestDetailRoot>>,
    mut state: ResMut<QuestDetailState>,
) {
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    *state = QuestDetailState::default();
}

pub fn apply_quest_detail_visibility(
    state: Res<QuestDetailState>,
    mut roots: Query<&mut Node, With<QuestDetailRoot>>,
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

/// The objective sentence for one quest: the first enabled objective's key
/// resolved through the player's speech strings, with its counter filled in.
///
/// Records carry 1 or 3 objectives in our captures and the slot authors one
/// condition row, so a three-objective quest shows the first enabled line.
fn condition_line(quest: &ActiveQuest, speech: impl Fn(&str) -> Option<String>) -> String {
    let objective = quest
        .objectives
        .iter()
        .find(|o| o.enabled)
        .or_else(|| quest.objectives.first());
    let Some(objective) = objective else {
        return NO_VALUE.to_string();
    };
    match speech(&objective.name) {
        Some(text) => fill_placeholders(&text, &objective.tasks),
        // the key itself is more useful than nothing: it names the row the
        // player's own data is missing
        None => objective.name.clone(),
    }
}

/// The reward line: the structured gold/exp the tables hold, in the order the
/// original's own paragraph writes them.
fn reward_line(reward: Option<QuestRewardValue>, ui_strings: &ClientUiStrings) -> String {
    match reward {
        Some(value) => format!(
            "{} {} / {} {}",
            EXP_WORD,
            value.exp,
            ui_strings.get_or("UIIT_STT_GOLD", "Gold"),
            value.gold
        ),
        None => NO_VALUE.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn refresh_quest_detail(
    state: Res<QuestDetailState>,
    info: Query<&CharacterInfo, With<Player>>,
    quests: Res<ClientQuestTable>,
    speech: Res<ClientSpeechText>,
    characters: Res<ClientCharacterData>,
    names: Res<ClientTextNames>,
    rewards: Res<ClientQuestRewards>,
    ui_strings: Res<ClientUiStrings>,
    mut cells: Query<(&DetailCell, &mut Text)>,
) {
    if !state.open {
        return;
    }
    let Ok(info) = info.single() else {
        return;
    };
    let Some(quest) = info.active_quests.get(state.selected) else {
        return;
    };
    for (which, mut text) in cells.iter_mut() {
        text.0 = match which {
            DetailCell::Title => row_title(quest.ref_id, quests.table(), &ui_strings),
            DetailCell::Npc => quest
                .target_npcs
                .first()
                .and_then(|id| characters.get(&(*id as i32)))
                .and_then(|row| row.name_key())
                .and_then(|key| names.name(key))
                .unwrap_or(NO_VALUE)
                .to_string(),
            DetailCell::Condition => {
                condition_line(quest, |key| speech.get(key).map(str::to_string))
            }
            DetailCell::Reward => reward_line(rewards.gold_exp(quest.ref_id), &ui_strings),
        };
    }
}

/// Clicking a list row opens the pane on that quest — the only way in, since
/// the authored window carries no button of its own.
pub fn on_quest_row_activate(
    activate: On<Activate>,
    rows: Query<&QuestRow>,
    mut state: ResMut<QuestDetailState>,
) {
    if let Ok(row) = rows.get(activate.entity) {
        state.selected = row.0;
        state.open = true;
    }
}

pub struct QuestDetailPlugin;

impl Plugin for QuestDetailPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QuestDetailState>()
            .add_systems(
                OnEnter(SceneState::GameWorld),
                spawn_quest_detail.after(crate::plugins::hud::quest::spawn_quest_window),
            )
            .add_systems(OnExit(SceneState::GameWorld), cleanup_quest_detail)
            .add_systems(
                Update,
                (apply_quest_detail_visibility, refresh_quest_detail).run_if(
                    in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting)),
                ),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn objective(name: &str, enabled: bool, tasks: Vec<u32>) -> QuestObjective {
        QuestObjective {
            id: 1,
            enabled,
            name: name.to_string(),
            tasks,
        }
    }

    fn quest(objectives: Vec<QuestObjective>) -> ActiveQuest {
        ActiveQuest {
            ref_id: 11,
            achievements: 16,
            auto_share_required: 0,
            quest_type: 24,
            time_remain: None,
            state: 1,
            objectives,
            target_npcs: Vec::new(),
        }
    }

    /// The reward row is the one piece of geometry that is ours, and the
    /// authored slot is why: three rows at the authored pitch end at 80, a
    /// fourth ends at 109, and the slot is 96 tall.
    #[test]
    fn the_reward_row_is_why_the_pane_is_one_pitch_taller() {
        assert_eq!(ROW_PITCH, 29.0);
        assert_eq!(REWARD_Y, COND_Y + ROW_PITCH);
        assert!(COND_Y + LINE_H <= SLOT_H);
        assert!(REWARD_Y + LINE_H > SLOT_H);
        assert_eq!(CONTENT.1, SLOT_H + ROW_PITCH);
        assert!(REWARD_Y + LINE_H <= CONTENT.1);
        // the slot is centred in the host, not placed at an authored x
        assert_eq!(SLOT_X, 9.0);
    }

    /// Captured objective lines with the text the player's own
    /// `textquest_speech&name.txt` holds for their keys: the counter fills the
    /// single `%d`, the one captured record with a non-zero counter has no
    /// `%d` at all, a three-objective record (quest 57) shows its first
    /// enabled line, and a key the data lacks shows as the key.
    #[test]
    fn the_condition_row_reads_the_players_own_sentence() {
        let hunt = quest(vec![objective("SN_CON_QNO_CH_POTION_1", true, vec![0])]);
        assert_eq!(
            condition_line(&hunt, |_| Some("Hunt 15 Water Ghost (%d)".to_string())),
            "Hunt 15 Water Ghost (0)"
        );
        let talk = quest(vec![objective("SN_CON_QTUTORIAL2_CH_1", false, vec![1])]);
        let line = "Teleport to the [South Gate] and speak with Jingyo";
        assert_eq!(condition_line(&talk, |_| Some(line.to_string())), line);
        assert_eq!(condition_line(&talk, |_| None), "SN_CON_QTUTORIAL2_CH_1");
        assert_eq!(condition_line(&quest(Vec::new()), |_| None), NO_VALUE);

        let three = quest(vec![
            objective("SN_CON_QNO_CH_GENARAL_1_01", false, vec![1]),
            objective("SN_CON_QNO_CH_GENARAL_1_02", true, vec![0]),
        ]);
        assert_eq!(
            condition_line(&three, |key| Some(key.to_string())),
            "SN_CON_QNO_CH_GENARAL_1_02"
        );
    }

    /// The reward line reads the structured table, in the order the original's
    /// own paragraph writes it (`SN_PAYCON_QNO_CH_SMITH_1`: "Exp 270 / GOLD
    /// 205"), and says nothing where the table has no row.
    #[test]
    fn the_reward_line_follows_the_originals_own_order() {
        let strings = ClientUiStrings::default();
        let value = QuestRewardValue {
            gold: 205,
            exp: 270,
        };
        assert_eq!(reward_line(Some(value), &strings), "Exp 270 / Gold 205");
        assert_eq!(reward_line(None, &strings), NO_VALUE);
    }
}

//! The quest list window — `GDR_QUEST_FRAME` of `resinfo/ifquest.txt`.
//!
//! Idea: the window is transcribed from the player's own `ifquest.txt` (14
//! blocks) and its row prototype `ifquestslot.txt` (1 block), and it shows the
//! **active** quests the server sent with `CHARACTER_DATA`, decorated by
//! `questdata.txt`. Every rect below carries the `ifquest.txt:NNN` line it came
//! from.
//!
//! **Where the window sits is ours, because the data does not say.**
//! `GDR_QUEST_FRAME` is a `CIFSubFrame` of `365x355` (`:253`) and
//! `ginterface.txt` places no `GDR_QUEST` at all — only `GDR_QUESTINFO`
//! (`:867`, the detail pane). So the list gets the shared `game_window` chrome
//! at the subframe's own extent, like `hud::action`'s unplaced page, and the
//! anchor is a choice.
//!
//! **The twelve `..._NORMALTILE_BG_*` blocks are not rows.** Eleven are `327x2`
//! hairlines on `com_bg_tile_c.ddj` at y = 52, 79, … 322 (`:215` … `:25`) — a
//! 27-unit pitch — and `BG_0` (`:234`) is a `1x303` vertical rule at x=343. The
//! row height comes from the prototype: `GDR_QUESTSLOT_EMPTY_WND` is
//! `0,0,340,28` (`ifquestslot.txt:6`), and its art `qst_blankwindo.ddj`
//! measures 340x28 in the player's archive. Eleven gaps at 27 plus that row is
//! what makes [`VISIBLE_ROWS`] twelve, which the list host `4,27,356,323`
//! (`:6`) holds exactly.
//!
//! **What it shows.** `CharacterInfo::active_quests` — the quest section of
//! `CHARACTER_DATA`, parsed since `read_active_quests` landed and mirrored into
//! the client record by this slice. A title is `questdata.txt` column `[5]`
//! (`SN_<codename>`) through `ClientUiStrings`, falling back to the codename
//! and then to the raw id: `assets::textdata::quest` measured that 2 of the 9
//! ids our own server sends have no `questdata.txt` row at all.
//!
//! Not here: a quest's objectives, its detail pane (`ifquestinfo.txt`) and the
//! completed-quest *titles* — completed quests are a count, from the same
//! record. Resolving a thousand ids is the detail pane's job.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::LineHeight;
use bevy::ui_widgets::Activate;

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::textdata::quest::QuestTable;
use crate::plugins::hud::exchange::ui::spawn_subframe;
use crate::plugins::hud::game_window::{abs_node, spawn_game_window};
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::player::Player;
use crate::plugins::settings::keymap::KEY_QUEST;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::{ClientQuestTable, ClientUiStrings};
use crate::scenes::SceneState;

/// `GDR_QUEST_FRAME:CIFSubFrame` (`ifquest.txt:253`, Rect `0,0,365,355`,
/// `Text="UIIT_STT_QUEST_LIST"`) — the page the window's content holds.
const CONTENT_W: f32 = 365.0;
const CONTENT_H: f32 = 355.0;
/// Ours: the data places no quest list (see the module note). Right/top like
/// the other read-only windows, clear of the minimap.
const ANCHOR: (f32, f32) = (180.0, 120.0);

/// `GDR_QUEST_SCROLLMGR:CIFScrollManager` (`:6`, Rect `4,27,356,323`).
const LIST_RECT: (f32, f32, f32, f32) = (4.0, 27.0, 356.0, 323.0);
/// `GDR_QUESTSLOT_EMPTY_WND` (`ifquestslot.txt:6`, Rect `0,0,340,28`) on
/// `qst_blankwindo.ddj`, whose DDS header measures 340x28.
const ROW_RECT: (f32, f32, f32, f32) = (12.0, 25.0, 340.0, 28.0);
const ROW_ART: &str = "media://interface/quest/qst_blankwindo.ddj";
/// The hairlines' own spacing: y = 52, 79, … 322 (`:215` … `:25`).
const ROW_PITCH: f32 = 27.0;
/// Eleven hairlines separate twelve rows, and twelve rows at the pitch are
/// what the list host's 323 of height holds.
const VISIBLE_ROWS: usize = 12;
/// `GDR_QUEST_NORMALTILE_BG_1..11` (`:215` … `:25`): `327x2` on
/// `com_bg_tile_c.ddj`, first at y=52.
const SEPARATOR: (f32, f32, f32) = (16.0, 327.0, 2.0);
const SEPARATOR_FIRST_Y: f32 = 52.0;
/// `GDR_QUEST_NORMALTILE_BG_0` (`:234`, Rect `343,36,1,303`) — the vertical
/// rule down the list's right edge, where the original's scrollbar sits.
const RIGHT_RULE: (f32, f32, f32, f32) = (343.0, 36.0, 1.0, 303.0);
const TILE_ART: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_c.ddj";

/// The completed-quest readout, in the band above the list that the subframe's
/// 27-unit top leaves free (`:6` starts the list at y=27). **Ours** — the tree
/// has no control for it; the count is the only thing this window can say about
/// completed quests (see the module note).
const COUNT_RECT: (f32, f32, f32, f32) = (12.0, 9.0, 340.0, 12.0);

/// Every block of both trees carries `FontIndex=0` = 12 px.
const TEXT_FONT_INDEX: usize = 0;
/// `FontColor=255,255,255,255` on all 14 blocks of `ifquest.txt`.
const TEXT_COLOR: Color = Color::WHITE;
/// Ours: a dimmed row for the line that says the list is empty.
const EMPTY_COLOR: Color = Color::srgb(0.62, 0.60, 0.56);

/// Whether the window is showing.
#[derive(Resource, Default, Debug)]
pub struct QuestWindowState {
    pub open: bool,
}

#[derive(Component)]
pub struct QuestWindowRoot;

/// A list row, by index into the active-quest vector.
#[derive(Component, Clone, Copy)]
pub struct QuestRow(pub usize);

/// The completed-quest readout.
#[derive(Component)]
pub struct QuestCountText;

/// `KeyQuest` (id 3006, `Q` by default) toggles the window.
///
/// `Q` is in [`DEV_HOTKEY_COLLISIONS`](crate::plugins::dev::DEV_HOTKEY_COLLISIONS)
/// — `dev::toggle_wireframe` reads the same key — which is the list's whole
/// purpose: the clash exists only in a session that turned `dev_tools` on, and
/// that system is in fact not registered today (its line in the gated block is
/// commented out). Naming it here is what keeps the next person from
/// rediscovering it.
pub fn toggle_quest_window(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<crate::plugins::hud::chat::model::ChatState>,
    options: Res<GameOptions>,
    mut state: ResMut<QuestWindowState>,
) {
    let Some(key) = options.key_for(KEY_QUEST) else {
        return;
    };
    if keys.just_pressed(key) && !chat.input_open {
        state.open = !state.open;
    }
}

pub fn spawn_quest_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<crate::assets::FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cameras: Query<Entity, With<Camera2d>>,
    mut state: ResMut<QuestWindowState>,
) {
    let Ok(camera) = cameras.single() else {
        warn!("quest: no 2d camera to attach to");
        return;
    };
    *state = QuestWindowState::default();
    let s = hud_scale();
    let window = spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        // the subframe's own `Text` key
        ui_strings.get_or("UIIT_STT_QUEST_LIST", "Quest list"),
        (CONTENT_W, CONTENT_H),
        ANCHOR,
        s,
    );
    commands
        .entity(window.root)
        .insert((QuestWindowRoot, GlobalZIndex(20)));
    commands.entity(window.expect_close_button()).observe(
        |_: On<Activate>, mut state: ResMut<QuestWindowState>| {
            state.open = false;
        },
    );

    // One size for the window, from the trees' `FontIndex` through the one
    // rounding rule.
    let size = text_px(font_index_px(TEXT_FONT_INDEX), s);
    commands.entity(window.content).with_children(|page| {
        // GDR_QUEST_FRAME — the page's own `sframe_wnd_` ring
        spawn_subframe(page, &asset_server, (0.0, 0.0, CONTENT_W, CONTENT_H), s);

        let tile = |rect: (f32, f32, f32, f32)| {
            (
                abs_node(rect, s),
                ImageNode {
                    image: asset_server.load(TILE_ART),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            )
        };
        // the eleven hairlines and the right-hand rule
        for row in 0..VISIBLE_ROWS - 1 {
            let y = SEPARATOR_FIRST_Y + row as f32 * ROW_PITCH;
            page.spawn(tile((SEPARATOR.0, y, SEPARATOR.1, SEPARATOR.2)));
        }
        page.spawn(tile(RIGHT_RULE));

        // The completed count. Its box is one line tall, so the line box is the
        // same value as the size (`font_index_px`'s "One-line boxes").
        page.spawn((
            QuestCountText,
            Text::new(String::new()),
            TextFont {
                font: fonts.two.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            LineHeight::Px(size),
            TextColor(TEXT_COLOR),
            TextLayout::justify(Justify::Left),
            abs_node(COUNT_RECT, s),
            Pickable::IGNORE,
        ));

        // The twelve rows: the prototype's art with its title cell on top.
        for row in 0..VISIBLE_ROWS {
            let top = ROW_RECT.1 + row as f32 * ROW_PITCH;
            let mut node = abs_node((ROW_RECT.0, top, ROW_RECT.2, ROW_RECT.3), s);
            node.display = Display::None;
            page.spawn((
                QuestRow(row),
                node,
                ImageNode {
                    image: asset_server.load(ROW_ART),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Hovered::default(),
                Pickable::default(),
                children![(
                    Text::new(String::new()),
                    TextFont {
                        font: fonts.two.clone().into(),
                        font_size: FontSize::Px(size),
                        ..default()
                    },
                    TextColor(TEXT_COLOR),
                    TextLayout::justify(Justify::Left),
                    Node {
                        position_type: PositionType::Absolute,
                        // centred by hand in the 28-tall row art: the cell is a
                        // child of an image node, so there is no box to align in
                        left: Val::Px(text_px(8.0, s)),
                        top: Val::Px(text_px(8.0, s)),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                )],
            ));
        }
    });
}

pub fn cleanup_quest_window(
    mut commands: Commands,
    roots: Query<Entity, With<QuestWindowRoot>>,
    mut state: ResMut<QuestWindowState>,
) {
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    *state = QuestWindowState::default();
}

pub fn apply_quest_window_visibility(
    state: Res<QuestWindowState>,
    mut roots: Query<&mut Node, With<QuestWindowRoot>>,
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

/// What a row shows for one active quest: the table's title where it has one,
/// the codename where the title key is missing, and the raw id where the table
/// has no row at all — which 2 of the 9 ids our own server sends really are.
fn row_title(ref_id: u32, table: Option<&QuestTable>, ui_strings: &ClientUiStrings) -> String {
    match table.and_then(|table| table.quest(ref_id)) {
        Some(quest) => match quest.title_key.as_deref() {
            Some(key) => ui_strings.get_or(key, &quest.codename).to_string(),
            None => quest.codename.clone(),
        },
        None => format!("#{ref_id}"),
    }
}

/// Push the player's quest record onto the twelve rows and the count.
pub fn refresh_quest_window(
    info: Query<&CharacterInfo, With<Player>>,
    quests: Res<ClientQuestTable>,
    ui_strings: Res<ClientUiStrings>,
    mut rows: Query<(&QuestRow, &mut Node, &Children)>,
    mut cells: Query<&mut Text, Without<QuestCountText>>,
    mut count: Query<&mut Text, With<QuestCountText>>,
) {
    let Ok(info) = info.single() else {
        return;
    };
    let table = quests.table();
    let active = &info.active_quests;
    for (row, mut node, children) in &mut rows {
        // Row 0 of an empty list carries the "nothing active" line, so it stays
        // drawn; every other empty row is hidden.
        let shown = row.0 < active.len().max(1);
        let display = if shown { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        let Some(&cell) = children.first() else {
            continue;
        };
        let Ok(mut text) = cells.get_mut(cell) else {
            continue;
        };
        let next = match active.get(row.0) {
            Some(quest) => row_title(quest.ref_id, table, &ui_strings),
            None if row.0 == 0 => EMPTY_LINE.to_string(),
            None => String::new(),
        };
        if text.0 != next {
            text.0 = next;
        }
    }
    if let Ok(mut text) = count.single_mut() {
        let next = format!("{}: {}", COMPLETED_LABEL, info.completed_quests.len());
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// Ours, both of them: the original's own strings for these two sentences are
/// not established (`textuisystem.txt` has no "no active quest" key, and the
/// completed count has no control in `ifquest.txt` at all), so inventing a
/// *key* would be worse than owning the words.
const EMPTY_LINE: &str = "No active quest.";
const COMPLETED_LABEL: &str = "Completed";

/// Self-registration (#558): the HUD registry only names this plugin.
pub struct QuestWindowPlugin;

impl Plugin for QuestWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QuestWindowState>()
            .add_systems(OnEnter(SceneState::GameWorld), spawn_quest_window)
            .add_systems(OnExit(SceneState::GameWorld), cleanup_quest_window)
            .add_systems(
                Update,
                (
                    // The guard belongs to the toggle alone: on the tuple it
                    // would also stop the two painters below.
                    toggle_quest_window
                        .run_if(not(crate::plugins::settings::keymap::text_field_focused)),
                    apply_quest_window_visibility,
                    refresh_quest_window,
                )
                    .run_if(
                        in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting)),
                    ),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::assets::textdata::quest::QuestTable;

    /// Twelve rows at the hairlines' own 27-unit pitch have to fit the list
    /// host's 323 of height, and a thirteenth must not — that is where
    /// `VISIBLE_ROWS` comes from rather than from counting the twelve
    /// `NORMALTILE` blocks, eleven of which are separators and one a rule.
    #[test]
    fn the_rows_come_from_the_pitch_and_fit_the_list_host() {
        let used = (VISIBLE_ROWS - 1) as f32 * ROW_PITCH + ROW_RECT.3;
        assert!(
            used <= LIST_RECT.1 + LIST_RECT.3,
            "{used} must fit the host"
        );
        let one_more = VISIBLE_ROWS as f32 * ROW_PITCH + ROW_RECT.3;
        assert!(one_more > LIST_RECT.1 + LIST_RECT.3);
        // the eleven hairlines sit between the twelve rows
        assert_eq!(VISIBLE_ROWS - 1, 11);
        // and the row art is the prototype's own extent
        assert_eq!((ROW_RECT.2, ROW_RECT.3), (340.0, 28.0));
    }

    /// The three readings of a wire id, in the order the table answers them.
    /// The missing-row case is measured, not hypothetical: 2 of the 9 ids our
    /// server sends have no `questdata.txt` row.
    #[test]
    fn a_row_falls_back_from_title_to_codename_to_id() {
        // service, id, codename, level, korean, title key, then filler columns
        let questdata = "1\t7\tQNO_CH_SMITH_1\t5\t\tSN_QNO_CH_SMITH_1\txxx\txxx\txxx\txxx\txxx\t\n\
                         1\t8\tQNO_NO_TITLE\t0\t\txxx\txxx\txxx\txxx\txxx\txxx\t\n";
        let table = QuestTable::parse(questdata, "");
        let strings = ClientUiStrings::default();
        // no string table row: `get_or` answers with the fallback, the codename
        assert_eq!(row_title(7, Some(&table), &strings), "QNO_CH_SMITH_1");
        // the row exists but its title column is `xxx`
        assert_eq!(row_title(8, Some(&table), &strings), "QNO_NO_TITLE");
        // no row at all
        assert_eq!(row_title(220, Some(&table), &strings), "#220");
        // and with no table loaded yet
        assert_eq!(row_title(7, None, &strings), "#7");
    }

    /// The count readout's box is one line tall — which is why it carries a
    /// pinned line box and the 28-tall rows do not.
    #[test]
    fn only_the_count_box_is_a_one_line_box() {
        let line = font_index_px(TEXT_FONT_INDEX);
        assert_eq!(COUNT_RECT.3, line);
        assert!(ROW_RECT.3 > line * 1.2);
    }
}

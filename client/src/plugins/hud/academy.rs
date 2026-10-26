//! Academy member panel — `ifapprenticeship.txt` (27 blocks) with
//! `ifapprenticeshipslot.txt` (8 blocks) as the row template.
//!
//! Idea: the original hosts this as a **page of the MainPopup frame**
//! (`ifmainpopup.txt:7` id 77, `13,38,365,355`), not as a window of its own,
//! so the panel is a `game_window` whose content is the tree's own
//! `GDR_APPRENTICESHIP_FRAMEL` sub-frame rect — 364x303 — plus the notice strip
//! and footer buttons that sit *below* that sub-frame in the same tree.
//!
//! The leader band is **not** a slot at offset zero, and the data says so: the
//! two column sets differ by 8-11 px throughout (rank mark 53 against 44, name
//! 71/83 against 63/80, `Lv` 156 against 145, level 172 against 161, rank name
//! 201 against 199). So the band has its own constants, taken from the main
//! tree, and the row constants come from the row tree. Deriving one from the
//! other would have invented five numbers that the files already answer.
//!
//! The academy is `TC` ("Training Camp") in the string table and
//! **apprenticeship** in the descriptors — neither surface is spelled
//! "academy", which is why a path-shaped search for it finds nothing.
//!
//! No roster arrives yet: `0x3C81`'s handler reads no bytes at all
//! (`packets/src/agent/academy.rs`) and nothing in our dumps carries an academy
//! roster. The panel therefore renders whatever state it is given and an empty
//! state as empty rows, the same way the appraisal dialog renders an empty
//! option list rather than a guessed one.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::text::LineHeight;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::hud::game_window::{abs_node, spawn_game_window};
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::textdata::ClientUiStrings;

/// `GDR_APPRENTICESHIP_FRAMEL` (`ifapprenticeship.txt:422`) is 364x303; the
/// notice strip and the footer buttons extend the page to the button row at
/// y=339 plus its 24px height.
const CONTENT_W: f32 = 364.0;
const CONTENT_H: f32 = 363.0;

/// `GDR_APPRENTICESHIP_BGTILE_0` (`:403`) and `_1` (`:384`).
const BGTILE_0: (f32, f32, f32, f32) = (16.0, 36.0, 332.0, 33.0);
const BGTILE_1: (f32, f32, f32, f32) = (27.0, 300.0, 308.0, 4.0);

/// The leader band, from the main tree: portrait/status (`:137`/`:156`), rank
/// mark (`:4`), name (`:118`), the `Lv` caption (`:99`), its value (`:80`),
/// and the rank name (`:61`).
///
/// `GDR_APPRENTICESHIP_DISSOLVE_APPRENTICESHIP_BTN` (`:42`, at `316,38`) is
/// deliberately not drawn: the tree gives it no size, its art is the shared
/// `com_button.ddj`, and dissolving an academy is a protocol act with no
/// decoded opcode here. Drawing it would have meant inventing a width and
/// wiring a button to nothing.
const LEADER_PICTURE: (f32, f32, f32, f32) = (9.0, 33.0, 28.0, 28.0);
const LEADER_RANK_MARK: (f32, f32, f32, f32) = (53.0, 46.0, 16.0, 16.0);
const LEADER_NAME: (f32, f32, f32, f32) = (71.0, 46.0, 83.0, 17.0);
const LEADER_LEVEL_CAPTION: (f32, f32, f32, f32) = (156.0, 46.0, 14.0, 17.0);
const LEADER_LEVEL_DATA: (f32, f32, f32, f32) = (172.0, 46.0, 19.0, 17.0);
const LEADER_RANK_NAME: (f32, f32, f32, f32) = (201.0, 46.0, 104.0, 17.0);

/// `GDR_APPRENTICESHIP_SLOT_0..6` (`:308` down to `:194`): seven 360x36 bands
/// from y=69 on a pitch of 33, so consecutive bands overlap by 3px — the
/// authored value, not a derived one.
const SLOT_X: f32 = 2.0;
const SLOT_Y0: f32 = 69.0;
const SLOT_W: f32 = 360.0;
const SLOT_H: f32 = 36.0;
const SLOT_PITCH: f32 = 33.0;
pub const SLOT_COUNT: usize = 7;

/// Row-local columns, from `ifapprenticeshipslot.txt`: portrait/status
/// (`:20`/`:9`), rank mark (`:10`), name (`:11`), `Lv` (`:12`), level (`:13`),
/// rank name (`:14`), leave button (`:18`).
const ROW_PICTURE: (f32, f32, f32, f32) = (2.0, 4.0, 28.0, 28.0);
const ROW_RANK_MARK: (f32, f32, f32, f32) = (44.0, 14.0, 16.0, 16.0);
const ROW_NAME: (f32, f32, f32, f32) = (63.0, 14.0, 80.0, 17.0);
const ROW_LEVEL_CAPTION: (f32, f32, f32, f32) = (145.0, 14.0, 14.0, 17.0);
const ROW_LEVEL_DATA: (f32, f32, f32, f32) = (161.0, 14.0, 19.0, 17.0);
const ROW_RANK_NAME: (f32, f32, f32, f32) = (199.0, 14.0, 104.0, 17.0);

/// The notice strip (`:444` caption, `:306` area) and the two footer buttons
/// (`:346` invite, `:327` match). The buttons carry no size in the tree; 76x24
/// is `com_button.ddj`'s own, the same size the appraisal footer uses.
const NOTICE_CAPTION: (f32, f32, f32, f32) = (12.0, 315.0, 38.0, 13.0);
const NOTICE_AREA: (f32, f32, f32, f32) = (4.0, 306.0, 358.0, 28.0);
const BTN_INVITE: (f32, f32) = (56.0, 339.0);
const BTN_MATCH: (f32, f32) = (230.0, 339.0);
const BTN_SIZE: (f32, f32) = (76.0, 24.0);

const LABEL_FONT: f32 = 9.0;
const ROW_FONT: f32 = 9.0;

const NAME_COLOR: Color = Color::srgb(0.93, 0.89, 0.78);
const DIM_COLOR: Color = Color::srgb(0.65, 0.63, 0.57);

/// Slots 0-1 are sub-mentors and 2-6 apprentices. The split is in the art, not
/// in a field: the row template has one picture rect and the original swaps the
/// status overlay per role, so the role is a property of the slot index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcademyRole {
    SubMentor,
    Apprentice,
}

impl AcademyRole {
    pub fn for_slot(index: usize) -> Self {
        if index < 2 {
            AcademyRole::SubMentor
        } else {
            AcademyRole::Apprentice
        }
    }
}

/// One roster entry. Every field is what a column of the row template draws —
/// nothing here is a guess about the wire, because no academy roster packet is
/// decoded yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AcademyMember {
    pub name: String,
    pub level: u8,
    /// The rank's display name; the mark beside it is `rank` as an index.
    pub rank_name: String,
    pub rank: u8,
}

#[derive(Resource, Default)]
pub struct AcademyWindowState {
    pub open: bool,
    /// The mentor's own band, drawn pinned above the slots.
    pub leader: Option<AcademyMember>,
    /// Up to [`SLOT_COUNT`] entries; shorter lists leave empty bands.
    pub members: Vec<AcademyMember>,
}

#[derive(Component)]
pub struct AcademyWindowRoot;

/// The rebuilt part (everything but the shell).
#[derive(Component)]
pub struct AcademyBoard;

#[derive(Component, Clone, Copy)]
pub struct AcademySlot(pub usize);

pub fn slot_rect(index: usize) -> (f32, f32, f32, f32) {
    (SLOT_X, SLOT_Y0 + index as f32 * SLOT_PITCH, SLOT_W, SLOT_H)
}

pub fn spawn_academy_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cameras: Query<Entity, With<Camera2d>>,
    mut state: ResMut<AcademyWindowState>,
) {
    let Ok(camera) = cameras.single() else {
        warn!("academy panel: no 2d camera to attach to");
        return;
    };
    *state = AcademyWindowState::default();

    let window = spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_CTL_TC_TRAININGCAMP", "Academy"),
        (CONTENT_W, CONTENT_H),
        (150.0, 110.0),
        hud_scale(),
    );
    commands
        .entity(window.root)
        .insert((AcademyWindowRoot, GlobalZIndex(20)));
    commands.entity(window.expect_close_button()).observe(
        |_: On<Activate>, mut state: ResMut<AcademyWindowState>| {
            state.open = false;
        },
    );
    commands
        .entity(window.content)
        .insert(AcademyBoard)
        .despawn_related::<Children>();
}

pub fn refresh_academy_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    state: Res<AcademyWindowState>,
    boards: Query<Entity, With<AcademyBoard>>,
) {
    if !state.is_changed() {
        return;
    }
    let Ok(board) = boards.single() else {
        return;
    };
    let s = hud_scale();
    let font = |size: f32| {
        (
            TextFont {
                font: fonts.two.clone().into(),
                font_size: FontSize::Px(text_px(size, s)),
                ..default()
            },
            LineHeight::Px(text_px(size, s)),
        )
    };
    let tile = |path: &str| ImageNode {
        image: asset_server.load(path),
        image_mode: NodeImageMode::Stretch,
        ..default()
    };
    let lv_caption = ui_strings.get_or("UIIT_STT_LEVEL_LV", "Lv").to_string();

    commands
        .entity(board)
        .despawn_related::<Children>()
        .with_children(|content| {
            for rect in [BGTILE_0, BGTILE_1] {
                content.spawn((
                    abs_node(rect, s),
                    tile("media://interface/ifcommon/com_bg_tile_e.ddj"),
                    Pickable::IGNORE,
                ));
            }

            // The leader band: drawn whether or not it is filled, because the
            // original pins it above the slot list rather than folding it in.
            let leader = state.leader.clone().unwrap_or_default();
            content.spawn((
                abs_node(LEADER_PICTURE, s),
                tile("media://interface/ifcommon/com_slot_01.ddj"),
                Pickable::IGNORE,
            ));
            spawn_band(
                content,
                &leader,
                s,
                &font,
                &lv_caption,
                (
                    LEADER_RANK_MARK,
                    LEADER_NAME,
                    LEADER_LEVEL_CAPTION,
                    LEADER_LEVEL_DATA,
                    LEADER_RANK_NAME,
                ),
                LABEL_FONT,
            );
            for index in 0..SLOT_COUNT {
                let (x, y, w, h) = slot_rect(index);
                content
                    .spawn((
                        AcademySlot(index),
                        abs_node((x, y, w, h), s),
                        Pickable::IGNORE,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            abs_node(ROW_PICTURE, s),
                            tile("media://interface/ifcommon/com_slot_01.ddj"),
                            Pickable::IGNORE,
                        ));
                        let Some(member) = state.members.get(index) else {
                            return;
                        };
                        spawn_band(
                            row,
                            member,
                            s,
                            &font,
                            &lv_caption,
                            (
                                ROW_RANK_MARK,
                                ROW_NAME,
                                ROW_LEVEL_CAPTION,
                                ROW_LEVEL_DATA,
                                ROW_RANK_NAME,
                            ),
                            ROW_FONT,
                        );
                    });
            }

            content.spawn((
                abs_node(NOTICE_AREA, s),
                tile("media://interface/ifcommon/com_bg_tile_e.ddj"),
                Pickable::IGNORE,
            ));
            let (tf, lh) = font(LABEL_FONT);
            content.spawn((
                Text::new(
                    ui_strings
                        .get_or("UIIT_CTL_TC_COMMON", "Notice")
                        .to_string(),
                ),
                tf,
                lh,
                TextColor(DIM_COLOR),
                abs_node(NOTICE_CAPTION, s),
                Pickable::IGNORE,
            ));

            for (pos, key, fallback) in [
                (BTN_INVITE, "UIIT_CTL_TC_INVITE", "Invite"),
                (BTN_MATCH, "UIIT_CTL_TC_MACHING", "Matching"),
            ] {
                content
                    .spawn((
                        Button,
                        abs_node((pos.0, pos.1, BTN_SIZE.0, BTN_SIZE.1), s),
                        tile("media://interface/ifcommon/com_button.ddj"),
                    ))
                    .with_children(|button| {
                        let (tf, lh) = font(LABEL_FONT);
                        button.spawn((
                            Text::new(ui_strings.get_or(key, fallback).to_string()),
                            tf,
                            lh,
                            TextColor(NAME_COLOR),
                            TextLayout::justify(Justify::Center),
                            abs_node((0.0, 5.0, BTN_SIZE.0, 14.0), s),
                            Pickable::IGNORE,
                        ));
                    });
            }
        });
}

/// The five text columns a band draws. Leader and row pass their own rects, so
/// this shares the *drawing* without pretending the two layouts are one.
#[allow(clippy::type_complexity)]
fn spawn_band(
    parent: &mut RelatedSpawnerCommands<ChildOf>,
    member: &AcademyMember,
    s: f32,
    font: &impl Fn(f32) -> (TextFont, LineHeight),
    lv_caption: &str,
    rects: (
        (f32, f32, f32, f32),
        (f32, f32, f32, f32),
        (f32, f32, f32, f32),
        (f32, f32, f32, f32),
        (f32, f32, f32, f32),
    ),
    size: f32,
) {
    let (rank_mark, name, lv, level, rank_name) = rects;
    parent.spawn((
        abs_node(rank_mark, s),
        Pickable::IGNORE,
        BackgroundColor(DIM_COLOR.with_alpha(0.25)),
    ));
    for (rect, text, color) in [
        (name, member.name.clone(), NAME_COLOR),
        (lv, lv_caption.to_string(), DIM_COLOR),
        (level, member.level.to_string(), NAME_COLOR),
        (rank_name, member.rank_name.clone(), DIM_COLOR),
    ] {
        if text.is_empty() {
            continue;
        }
        let (tf, lh) = font(size);
        parent.spawn((
            Text::new(text),
            tf,
            lh,
            TextColor(color),
            abs_node(rect, s),
            Pickable::IGNORE,
        ));
    }
}

pub fn apply_academy_window_visibility(
    state: Res<AcademyWindowState>,
    mut roots: Query<&mut Node, With<AcademyWindowRoot>>,
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

pub fn cleanup_academy_window(
    mut commands: Commands,
    roots: Query<Entity, With<AcademyWindowRoot>>,
    mut state: ResMut<AcademyWindowState>,
) {
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    *state = AcademyWindowState::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The seven authored slot rects, `ifapprenticeship.txt:308` up to `:194`.
    #[test]
    fn slot_rects_match_the_authored_bands() {
        let authored = [69.0, 102.0, 135.0, 168.0, 201.0, 234.0, 267.0];
        for (index, y) in authored.iter().enumerate() {
            assert_eq!(slot_rect(index), (2.0, *y, 360.0, 36.0));
        }
    }

    /// Pitch 33 under height 36 means the bands overlap by 3px. That is the
    /// tree's own arithmetic; a test pins it so a later "tidy" spacing of 36
    /// has to argue with the data.
    #[test]
    fn bands_overlap_by_three_pixels() {
        let (_, first_y, _, h) = slot_rect(0);
        let (_, second_y, ..) = slot_rect(1);
        assert_eq!(second_y - first_y, SLOT_PITCH);
        assert_eq!(first_y + h - second_y, 3.0);
    }

    /// The role split is positional (§3.1): two sub-mentors, five apprentices.
    #[test]
    fn the_first_two_slots_are_sub_mentors() {
        let roles: Vec<_> = (0..SLOT_COUNT).map(AcademyRole::for_slot).collect();
        assert_eq!(
            roles
                .iter()
                .filter(|r| **r == AcademyRole::SubMentor)
                .count(),
            2
        );
        assert_eq!(roles[0], AcademyRole::SubMentor);
        assert_eq!(roles[1], AcademyRole::SubMentor);
        assert_eq!(roles[2], AcademyRole::Apprentice);
    }

    /// The leader band is not a slot at offset zero — every column differs.
    #[test]
    fn the_leader_band_is_not_the_row_template() {
        for (leader, row) in [
            (LEADER_RANK_MARK, ROW_RANK_MARK),
            (LEADER_NAME, ROW_NAME),
            (LEADER_LEVEL_CAPTION, ROW_LEVEL_CAPTION),
            (LEADER_LEVEL_DATA, ROW_LEVEL_DATA),
        ] {
            assert_ne!(leader.0, row.0, "a column would have been derived");
        }
    }
}

pub struct AcademyPlugin;

impl Plugin for AcademyPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<AcademyWindowState>()
            .add_systems(OnEnter(SceneState::GameWorld), spawn_academy_window)
            .add_systems(OnExit(SceneState::GameWorld), cleanup_academy_window)
            .add_systems(
                Update,
                (apply_academy_window_visibility, refresh_academy_window)
                    .chain()
                    .run_if(in_state(SceneState::GameWorld)),
            );
    }
}

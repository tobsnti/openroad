//! Region-select board — the original step between character select and
//! character creation.
//!
//! Idea: in v1.188 the Create button does not open the creation screen
//! directly; it swaps the select screen into a "Region Select" sub-mode
//! (resinfo/pscharacterselect.txt, section `Select`): the title image changes
//! to `text-region.ddj`, and translucent race plates (`china.ddj` /
//! `europe.ddj`, 440x152, sections `China`/`Europe`) hover over the 3D stage.
//! Picking a plate cuts to a full-screen race-themed loading screen
//! (`loading_charactercustom[_europe].ddj` + `loading_form`/`nowloading`) and
//! then enters the per-race creation screen.
//!
//! The select→board camera is known: the original walks the camera ~100 units
//! across the same stage, to the three race idols (`interface_idol_*.bsr`) it
//! keeps at `x 155..157, z 651..654`. That flight lives in
//! [`super::race_stage`]. Still UNKNOWN: the plates' SCREEN POSITIONS — they sit
//! here as a centered pair (slot order from the baked tab art: europe center,
//! china right).

use bevy::asset::UntypedHandle;
use bevy::color::ColorToComponents;
use bevy::prelude::*;
use bevy::ui::Overflow;
use bevy::ui_widgets::Activate;
use bevy::window::PrimaryWindow;
use bevy_tweening::{EaseMethod, Tween, TweenAnim};

use crate::assets::bsr::resource::SroResource;
use crate::assets::FontAssets;
use crate::plugins::camera::CinematicCamera2;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::{
    ClientCharacterData, ClientItemData, ClientItemIndex, ClientUiStrings,
};
use crate::plugins::ui_v2::style::ButtonSound;
use crate::plugins::ui_v2::widgets::{image_button, label};
use crate::scenes::intro_v2::character_create::{
    figure_variants, CharCreateSelection, Gender, Race,
};
use crate::scenes::loading_screen::{set_gauge_fraction, spawn_loading_surface, LoadingProgress};

use crate::plugins::world_origin::WorldOrigin;

use super::assets::IntroV2Assets;
use super::chrome::InfoTextV2Update;
use super::login_form::main_button_style;
use super::race_catalog::{available_races, KNOWN_RACES};
use super::race_stage::RaceBoardIdol;
use super::scene_data::ActiveCharSelectSceneV2;
use super::{intro_font_px, unescape_newlines, IntroV2State, IntroV2Ui};

/// Root marker of the region-select UI.
#[derive(Component, Default, Clone)]
pub struct RegionSelectRoot;

/// A clickable race plate.
#[derive(Component, Clone, Copy)]
pub struct RegionPlate(pub Race);

/// Text inside a plate — the tab label (`GDR_STATIC1`) and the description body
/// (`GDR_TEXT_CHINA` / `GDR_TEXT_EUROPE`, `CIFTextBox` at the plate-local rect
/// `19,47,401,87`) — carrying its authored colour so the plate fade can scale
/// its alpha along with the plate art.
///
/// Why the *art* and not only the text fades: `china.ddj`/`europe.ddj` **are**
/// the description boxes — 440x152 gold-framed dark panels with a tab — so
/// hiding the text child leaves two empty frames standing. The original ramps
/// the *whole control's* alpha and starts both plates at alpha 0.
#[derive(Component, Clone, Copy)]
pub struct RegionPlateInk {
    pub race: Race,
    pub base: Color,
}

/// How visible a plate currently is, 0..1 — the state the original keeps as a
/// byte alpha on the control itself.
#[derive(Component, Clone, Copy, Default)]
pub struct RegionPlateFade(pub f32);

/// Which race's idol the pointer is over, or `None` — our `g_region`
/// (set to `-1` at the top of every state-8 tick and to the pick index on a
/// hit).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct HoveredRegion(pub Option<Race>);

/// Alpha per second of the plate fade: the original multiplies the frame delta
/// by `3.0` and by `255.0` before adding it
/// to the byte alpha, i.e. a full fade in one third of a second.
const PLATE_FADE_PER_SEC: f32 = 3.0;

/// Alpha per second while the click's confirm is running — a **different**
/// number from the hover ramp: the confirm fades *both* plates out over
/// `0.5 s`, so the rate is `1 / 0.5`. Using the hover rate here would be a
/// third of a second — faster than the original and, worse, indistinguishable
/// from "the pointer left the idol".
const CONFIRM_PLATE_FADE_PER_SEC: f32 = 2.0;

/// Left-edge clamp of a plate, in the original's own pixels: after centring the
/// plate on its idol's projected x it does `if (x < 5) x = 5`, so a
/// figure near the left edge cannot push the panel off screen.
const PLATE_LEFT_CLAMP_PX: f32 = 5.0;

/// Alpha of a plate after `dt` seconds, given whether its own idol is hovered.
/// Split out because it is the one half of the hover behaviour a test can pin
/// without a window: a `Visibility` flip cannot produce the intermediate values
/// this asserts.
pub(crate) fn plate_alpha_step(current: f32, hovered: bool, dt: f32) -> f32 {
    let delta = PLATE_FADE_PER_SEC * dt;
    if hovered {
        (current + delta).min(1.0)
    } else {
        (current - delta).max(0.0)
    }
}

/// Alpha of a plate `dt` seconds into the confirm: both plates leave, at
/// [`CONFIRM_PLATE_FADE_PER_SEC`], no matter what is hovered.
pub(crate) fn plate_alpha_dismiss(current: f32, dt: f32) -> f32 {
    (current - CONFIRM_PLATE_FADE_PER_SEC * dt).max(0.0)
}

/// Left edge of a plate whose idol projects to `proj_x`, for a plate `width` px
/// wide: centred on the figure, clamped like the original.
pub(crate) fn plate_left_px(proj_x: f32, width: f32) -> f32 {
    (proj_x - width * 0.5).max(PLATE_LEFT_CLAMP_PX)
}

/// The full-screen loading cut shown between the board and the creation
/// screen (original: `GDR_LOADING_CHINA` id `0x16` / `GDR_LOADING_EUROPE` id
/// `0x1c`, plus the `0x17`/`0x1b`/`0x18` chrome).
///
/// Its timer is **no longer the progress source**: the gauge counts
/// the creation screen's own asset load, and this timer only starts once that
/// load is full — it is the dwell that lets `OnEnter(CharacterCreate)` build the
/// screen behind a finished loading picture instead of showing one empty frame
/// of stage. Ticked in `CharacterCreate` by [`tick_loading_cut`].
#[derive(Component)]
pub struct CreationLoadingCut(pub Timer);

/// The click's confirm, in flight: **the ORDER is the point**.
///
/// Idea: the original does click → 1.0 s camera move → *then* the race loading
/// art with its gauge at zero → *then* the creation screen, and state 9 is
/// literally a wait for the camera move to land before it shows anything.
/// We used to spawn the loading art inside the click handler, i.e.
/// two of those steps in the wrong order and one missing altogether. Modelling
/// the sequence as one resource with one [`RegionConfirm::advance`] step — rather
/// than as three systems that each guess where they are — is what makes the
/// order testable without a window.
#[derive(Resource)]
pub struct RegionConfirm {
    /// The confirmed race; also decides the loading art and the flight target.
    pub race: Race,
    /// The camera move (`race_stage::CONFIRM_FLIGHT` = 1.0 s).
    flight: Timer,
    /// Assets the gauge counts. Empty until the art is on screen, because the
    /// original resets its gauge to `0.0` at that very moment — requesting them
    /// earlier would make the bar start somewhere in the middle.
    watched: Vec<UntypedHandle>,
    /// Whether the loading art has been spawned yet.
    cut_shown: bool,
}

/// What [`RegionConfirm::advance`] wants done this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ConfirmStep {
    /// The camera is still moving; nothing else may appear yet.
    Flying,
    /// The move has landed: show the race art with the gauge at `0.0` and start
    /// the assets it will count.
    ShowCut,
    /// The art is up; this is the real fraction of the creation screen's load.
    Loading(f32),
    /// The load is complete — hand over to `CharacterCreate`.
    Done,
}

impl RegionConfirm {
    pub(crate) fn new(race: Race) -> Self {
        Self {
            race,
            flight: Timer::new(super::race_stage::CONFIRM_FLIGHT, TimerMode::Once),
            watched: Vec::new(),
            cut_shown: false,
        }
    }

    /// One step of the sequence. `settled` / `total` are the load counter of the
    /// watched assets; they are only consulted once the art is up, so the gauge
    /// cannot start above zero.
    pub(crate) fn advance(
        &mut self,
        dt: std::time::Duration,
        settled: usize,
        total: usize,
    ) -> ConfirmStep {
        if !self.flight.tick(dt).is_finished() {
            return ConfirmStep::Flying;
        }
        if !self.cut_shown {
            self.cut_shown = true;
            return ConfirmStep::ShowCut;
        }
        // A selection whose assets do not resolve (nothing to count) must not
        // hang on a gauge that can never fill.
        if total == 0 || settled >= total {
            return ConfirmStep::Done;
        }
        ConfirmStep::Loading(settled as f32 / total as f32)
    }
}

/// Plate and loading textures are authored **per race** and therefore live
/// with the race they belong to, in [`super::race_catalog::KNOWN_RACES`] —
/// this screen no longer holds a constant per race, because it no longer
/// knows how many races there are.
const TITLE_REGION_DDJ: &str = "media://interface/outer/text-region.ddj";

/// Every rect this screen takes from the data, in one place so a test can pin
/// them.
///
/// **What the data does and does not author.** All three plates —
/// `GDR_STA_CHINA` (9), `GDR_STA_ISLAM` (10), `GDR_STA_EUROPE` (11) — declare
/// `Rect="0,0,440,152"`: the **size** is authored, the **screen position is
/// not**, exactly like the twelve other `0,0,w,h` controls in section
/// `Select`. So the plate placement below is openroad's (a centered pair), and
/// no amount of re-reading the resinfo will change that; it is unauthored, not
/// a gap. The title, in contrast, *is* placed by the data at `47,110`.
///
/// Each plate's insides come from its own race section, and the label x
/// genuinely differs per race — China `318`, Europe `250`.
const TITLE_RECT: (f32, f32, f32, f32) = (47.0, 110.0, 292.0, 36.0);
/// The virtual canvas the `Select` rects are authored on — the shared
/// [`crate::plugins::ui_v2::RESINFO_CANVAS`], not a local copy.
const DESIGN: (f32, f32) = crate::plugins::ui_v2::RESINFO_CANVAS;
const PLATE_SIZE: (f32, f32) = (440.0, 152.0);

/// Where a plate sits: **no longer ours, and no longer a row.** The original
/// projects the two idols' world points in one call and then places each plate
/// at `x = projX - plateWidth/2` (clamped to `x >= 5`), `y = projY` — the panel
/// hangs on its own figure. The centred pair that stood here (a `15%` row with
/// a `40 px` gap) was invented while the placement was unknown.
/// The screen offsets therefore live in [`plate_left_px`], not in a constant.

/// `GDR_STATIC1` in sections `China`/`Europe`, plate-local; only x differs.
const PLATE_LABEL_RECT: (f32, f32, f32, f32) = (0.0, 9.0, 92.0, 15.0);
/// (the two label x values moved into the per-race presentation table)
/// `GDR_TEXT_CHINA`/`GDR_TEXT_EUROPE` `CIFTextBox`, plate-local.
const PLATE_BODY_RECT: (f32, f32, f32, f32) = (19.0, 47.0, 401.0, 87.0);
/// Text sizes of the two plate children and of Cancel, from the `FontIndex` of
/// the very controls the rects above are transcribed from: the plate label
/// `GDR_STATIC1` is `FontIndex=2` -> 16 px
/// (`pscharacterselect_europe.txt:67` Europe, `:25` China), the body
/// `GDR_TEXT_EUROPE`/`GDR_TEXT_CHINA` `CIFTextBox` is `FontIndex=0` -> 12 px
/// (`:48`, `:6`), and `GDR_BTN_CANCEL` is `FontIndex=2` (`:706`). The 12/10 that
/// stood here were picked by eye while the ladder was UNKNOWN.
const PLATE_LABEL_FONT_INDEX: usize = 2;
const PLATE_BODY_FONT_INDEX: usize = 0;
const CANCEL_FONT_INDEX: usize = 2;

/// How long the finished loading picture stays up after its gauge is full, so
/// `OnEnter(CharacterCreate)` can build the screen behind it. **Our number**
/// (timing UNKNOWN — not data-derived); what it is *not* any more is the
/// progress source, which is now the real asset load.
const LOADING_CUT_SECS: f32 = 1.2;

/// A race the data has no playable body for is not on the board at all; it
/// replaces the dimmed "Out of service area." plate. A data set may carry
/// `CHAR_CH_*` body rows across the `characterdata*.txt` shards and **zero**
/// `CHAR_EU_*`, while `Data/res/char/europe/*.bsr` and the European starter
/// weapons ship — the models are there, the row a creation packet needs is not.
///
/// Player-facing reason on a data-blocked plate: the ORIGINAL's own key for
/// "this region cannot be played here", `UIO_MSG_ERROR_ REGION_SUPPORT`
/// (`textuisystem.txt`, English column: *"Out of service area."*; the stray
/// space in the key is the original's). `CPSCharacterSelect::Create` shows that
/// very message when `g_region` is not 0 or 1, so it is the sentence the
/// v1.188 client itself puts in front of a player who cannot have this race.
///
/// Why this replaced our own text: the body rect used to
/// carry the *diagnosis* — "your PK2 ships the models but no playable body row
/// `CHAR_EU_*` … and a creation packet needs that row's ref id". That is a note
/// to us, not a message to a player, and it was on screen. The diagnosis is not
/// lost: it moved to a log line at the click, where the person who can act on it
/// will read it.
const PLATE_DISABLED_KEY: &str = "UIO_MSG_ERROR_ REGION_SUPPORT";
const PLATE_DISABLED_FALLBACK: &str = "Out of service area.";

/// The same gap in *our* words, for the log — kept because it is what makes the
/// state fixable: the models ship (`Data/res/char/europe/europeman_*.bsr`), the
/// European monsters, NPCs and starter weapons ship, and what is missing is the
/// one row a creation packet needs — the playable body in `characterdata`.
const PLATE_DISABLED_DIAGNOSIS: &str = "no playable body row (CHAR_CH_*/CHAR_EU_*) in \
     server_dep/silkroad/textdata/characterdata*.txt for this race — the models ship, \
     but a creation packet needs that row's ref id";

/// Whether `race` can be created at all on this corpus: the race board and the
/// plate click must agree, so both ask this one predicate. A race is offerable
/// exactly when characterdata holds at least one body row for either gender
/// (`CHAR_{CH,EU}_{MAN,WOMAN}_*`, see `Race::body_prefix`).
pub(crate) fn race_available(char_data: &ClientCharacterData, race: Race) -> bool {
    !figure_variants(char_data, race, Gender::Male).is_empty()
        || !figure_variants(char_data, race, Gender::Female).is_empty()
}

/// `OnEnter(RegionSelect)`: clears the connection keep-alive marker (its job
/// ended with the CharacterList exit) and spawns title + plates + Cancel.
pub fn enter_region_select(
    mut commands: Commands,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    char_data: Res<ClientCharacterData>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    commands.remove_resource::<super::character_create::EnteringCharacterCreate>();
    // Before the camera gate, not after it: `update_region_hover` takes
    // `ResMut<HoveredRegion>` and Bevy does not skip a system whose resource is
    // missing — it fails parameter validation and panics the schedule
    // (AGENTS.md). Leaving the insert behind the early return meant a frame
    // without a 2d camera took the whole state down instead of just drawing
    // nothing.
    commands.init_resource::<HoveredRegion>();

    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    // Title image at the original anchor (47,110 on the 1600x1200 virtual
    // canvas → percent so it tracks the window).
    commands.spawn((
        RegionSelectRoot,
        IntroV2Ui,
        UiTargetCamera(camera),
        Name::from("Region Select Title"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(100.0 * TITLE_RECT.0 / DESIGN.0),
            top: Val::Percent(100.0 * TITLE_RECT.1 / DESIGN.1),
            width: Val::Px(TITLE_RECT.2),
            height: Val::Px(TITLE_RECT.3),
            ..default()
        },
        ImageNode::new(asset_server.load(TITLE_REGION_DDJ)),
        Pickable::IGNORE,
    ));

    // Each plate is its own absolutely placed root, because it is hung on its
    // own idol every frame rather than laid out next to its sibling: the
    // two used to be children of a centred row, which is precisely the
    // invented placement the original's rule replaced. They start invisible
    // (`RegionPlateFade(0.0)`, alpha 0 like the original) and are
    // ramped in by [`update_region_plates`] while the pointer is on their idol.
    // The board is as long as the corpus says: one plate per race the
    // character table can build, plus the plates the interface data itself
    // declares (those stay, dimmed, with the original's reason).
    for race in available_races(&char_data) {
        let presentation = race.presentation();
        if presentation.is_none() {
            info!(
                "region plate {race:?}: corpus has bodies for this race but our \
                 interface data names no plate art or caption for it; showing \
                 the data code"
            );
        }
        let (label_key, label_fallback) = race.label();
        spawn_plate(
            &mut commands,
            camera,
            &asset_server,
            &ui_strings,
            &fonts,
            race,
            presentation.map(|p| p.plate_ddj),
            // label tab rect x from the race's own section (China 318,
            // Europe 250); an uncited race gets the plate-local left edge
            // of the shared rect, which is the only x we can justify.
            presentation.map(|p| p.plate_label_x).unwrap_or(0.0),
            label_key,
            &label_fallback,
            presentation.map(|p| p.desc_key).unwrap_or(""),
        );
    }

    // Cancel back to the list (original: one of the GDR_BTN_CANCEL trio —
    // which of the three belongs to the board sub-mode is UNKNOWN).
    let cancel_font = fonts.nine.clone();
    let cancel_px = intro_font_px(CANCEL_FONT_INDEX);
    let cancel_sound = assets.sound_button_sound_a.clone();
    let cancel_text = ui_strings
        .get_or("UIO_COMMON_CTL_CANCEL", "Cancel")
        .to_string();
    commands
        .spawn_scene(bsn! {
            RegionSelectRoot
            Name("Region Select Controls")
            Node {
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Row,
                bottom: percent(7.5),
                right: percent(1),
            }
            Children [
                (
                    image_button(main_button_style(&assets), 91.0, 41.0)
                    ButtonSound({cancel_sound})
                    Children [ (label(&cancel_text, cancel_font, cancel_px) TextColor(Color::WHITE)) ]
                    // Cancel flies the camera home instead of cutting; the
                    // state follows when the flight lands
                    // (`race_stage::start_return_flight`). If
                    // there is no camera or no scene to fly to — a headless or
                    // half-built stage — it falls back to the immediate switch
                    // rather than leaving the player on a dead button.
                    on(|_a: On<Activate>,
                        cam: Query<(Entity, &Transform), With<CinematicCamera2>>,
                        scene: Option<Res<ActiveCharSelectSceneV2>>,
                        origin: Res<WorldOrigin>,
                        mut commands: Commands,
                        mut next_state: ResMut<NextState<IntroV2State>>| {
                        let flying = scene.is_some_and(|scene| {
                            super::race_stage::start_return_flight(
                                &cam, &scene, &origin, &mut commands,
                            )
                        });
                        if !flying {
                            next_state.set(IntroV2State::CharacterList);
                        }
                    })
                ),
            ]
        })
        .insert((UiTargetCamera(camera), IntroV2Ui));
}

/// One 440x152 race plate: baked art + label text on its tab + the vanilla race
/// description in the body rect (19,47,401x87, plate-local — it was never a
/// screen rect).
///
/// It is art, not a button: the original confirms a region with a plain
/// `WM_LBUTTONDOWN` while an idol is hovered, so the plate
/// carries no `Button`/`Activate` and stays `Pickable::IGNORE` — a fully
/// transparent 440x152 button would otherwise sit in front of the stage and
/// swallow clicks meant for the figures.
#[allow(clippy::too_many_arguments)]
fn spawn_plate(
    commands: &mut Commands,
    camera: Entity,
    asset_server: &AssetServer,
    ui_strings: &ClientUiStrings,
    fonts: &FontAssets,
    race: Race,
    // `plate_ddj = None` = a race the corpus can build but our interface data
    // names no plate texture for: it gets an untextured (tinted) plate rather
    // than a guessed filename, and stays clickable.
    plate_ddj: Option<&str>,
    label_x: f32,
    label_key: &str,
    label_fallback: &str,
    desc_key: &str,
) {
    let tint = Color::WHITE;
    let mut plate = commands.spawn((
        RegionSelectRoot,
        IntroV2Ui,
        UiTargetCamera(camera),
        RegionPlate(race),
        RegionPlateFade(0.0),
        Name::from(format!("Region Plate {race:?}")),
        Node {
            position_type: PositionType::Absolute,
            // Off screen until the first projection lands, so a frame between
            // spawn and the first `update_region_plates` pass cannot flash the
            // panel at the window corner.
            left: Val::Px(-PLATE_SIZE.0),
            top: Val::Px(-PLATE_SIZE.1),
            width: Val::Px(PLATE_SIZE.0),
            height: Val::Px(PLATE_SIZE.1),
            ..default()
        },
        ImageNode {
            image: plate_ddj
                .map(|ddj| asset_server.load(ddj.to_string()))
                .unwrap_or_default(),
            color: tint.with_alpha(0.0),
            ..default()
        },
        Pickable::IGNORE,
    ));
    plate.with_children(|p| {
        let label_colour = Color::WHITE;
        p.spawn((
            RegionPlateInk {
                race,
                base: label_colour,
            },
            Text::new(ui_strings.get_or(label_key, label_fallback).to_string()),
            TextFont {
                font: fonts.nine.clone().into(),
                font_size: FontSize::Px(intro_font_px(PLATE_LABEL_FONT_INDEX)),
                ..default()
            },
            TextColor(label_colour.with_alpha(0.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(label_x),
                top: Val::Px(PLATE_LABEL_RECT.1),
                width: Val::Px(PLATE_LABEL_RECT.2),
                height: Val::Px(PLATE_LABEL_RECT.3),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ));
        // `UIO_NEWCHAR_CTL_*_TT` carries literal `\n` (textuisystem convention)
        let body_text = unescape_newlines(ui_strings.get_or(desc_key, ""));
        let body_colour = Color::srgba(0.9, 0.9, 0.9, 0.9);
        p.spawn((
            RegionPlateInk {
                race,
                base: body_colour,
            },
            Text::new(body_text),
            TextFont {
                font: fonts.nine.clone().into(),
                font_size: FontSize::Px(intro_font_px(PLATE_BODY_FONT_INDEX)),
                ..default()
            },
            TextColor(body_colour.with_alpha(0.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PLATE_BODY_RECT.0),
                top: Val::Px(PLATE_BODY_RECT.1),
                width: Val::Px(PLATE_BODY_RECT.2),
                height: Val::Px(PLATE_BODY_RECT.3),
                overflow: Overflow::clip(),
                ..default()
            },
            Pickable::IGNORE,
        ));
    });
}

/// `Update` in `RegionSelect`: which idol is under the pointer.
///
/// This is the original's state-8 tick and nothing else: the
/// cursor is turned into a ray through the stage camera and tested against the
/// two idols' own boxes. It deliberately does **not** go through bevy's picking
/// backends — the plates sit in front of the figures as UI, and the original's
/// own test is a geometric one against exactly two props, with the lizard
/// excluded.
///
/// `OPENROAD_REGION_HOVER=chinese|european` forces the result. Same dev-only
/// hook class as `OPENROAD_SKILL_TOOLTIP_ROW`: an automated run has no pointer,
/// so without it the hover state — the whole point of this screen — could not be
/// reached. Inert unless set, no counterpart in the original.
pub fn update_region_hover(
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<CinematicCamera2>>,
    idols: Query<&RaceBoardIdol>,
    mut hovered: ResMut<HoveredRegion>,
) {
    if let Some(forced) = forced_hover_race() {
        if hovered.0 != Some(forced) {
            *hovered = HoveredRegion(Some(forced));
        }
        return;
    }
    let picked = pick_idol(&windows, &cameras, &idols);
    if hovered.0 != picked {
        *hovered = HoveredRegion(picked);
    }
}

/// The ray cast itself, so [`update_region_hover`] reads as the tick it mirrors.
fn pick_idol(
    windows: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform), With<CinematicCamera2>>,
    idols: &Query<&RaceBoardIdol>,
) -> Option<Race> {
    let cursor = windows.iter().next()?.cursor_position()?;
    let (camera, cam_gt) = cameras.iter().find(|(camera, _)| camera.is_active)?;
    let ray = camera.viewport_to_world(cam_gt, cursor).ok()?;
    idols
        .iter()
        .filter_map(|idol| {
            idol.ray_hit(ray.origin, *ray.direction)
                .map(|t| (t, idol.race))
        })
        // nearest hit wins, exactly like a pick that returns a distance
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, race)| race)
}

/// `OPENROAD_REGION_CONFIRM_AT=<seconds>`: fire the confirm click at that point
/// on the app clock. Same dev-only hook class as `OPENROAD_REGION_HOVER` above
/// and for the same reason — an automated run has no pointer, so the click, the
/// camera move it starts and the loading step that follows could not be
/// reached at all. Inert unless set, no counterpart in the original.
fn forced_confirm_at() -> Option<f32> {
    let raw = std::env::var("OPENROAD_REGION_CONFIRM_AT").ok()?;
    match raw.trim().parse::<f32>() {
        Ok(secs) => Some(secs),
        Err(_) => {
            warn!("OPENROAD_REGION_CONFIRM_AT: '{raw}' is not a number of seconds");
            None
        }
    }
}

fn forced_hover_race() -> Option<Race> {
    // Takes the data code (`CH`, `EU`, …) as well as the two long names, so a
    // data set with a race we have no name for is still reachable.
    let raw = std::env::var("OPENROAD_REGION_HOVER").ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "chinese" | "china" => Some(Race::CHINESE),
        "european" | "europe" => Some(Race::EUROPEAN),
        other => {
            let bytes = other.to_ascii_uppercase().into_bytes();
            if bytes.len() == 2 {
                return Some(Race::from_ascii([bytes[0], bytes[1]]));
            }
            warn!("OPENROAD_REGION_HOVER: unknown race '{raw}' (chinese|european|<CODE>)");
            None
        }
    }
}

/// `Update` in `RegionSelect`: ramp each plate's alpha towards its idol's hover
/// state and hang it on that idol's projected screen point.
///
/// One system for both halves because they are one loop in the original and
/// because they share the per-race lookup; splitting them would mean projecting
/// the same two points twice.
pub fn update_region_plates(
    time: Res<Time>,
    hovered: Res<HoveredRegion>,
    // Once the region is confirmed the plates leave regardless of the pointer
    // — the same ramp, a different target and a different rate.
    confirm: Option<Res<RegionConfirm>>,
    cameras: Query<(&Camera, &GlobalTransform), With<CinematicCamera2>>,
    idols: Query<&RaceBoardIdol>,
    mut plates: Query<(
        &RegionPlate,
        &mut RegionPlateFade,
        &mut Node,
        &mut ImageNode,
    )>,
    mut ink: Query<(&RegionPlateInk, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    let projector = cameras.iter().find(|(camera, _)| camera.is_active);
    for (plate, mut fade, mut node, mut image) in plates.iter_mut() {
        fade.0 = if confirm.is_some() {
            plate_alpha_dismiss(fade.0, dt)
        } else {
            plate_alpha_step(fade.0, hovered.0 == Some(plate.0), dt)
        };
        image.color = image.color.with_alpha(fade.0);
        let Some((camera, cam_gt)) = projector else {
            continue;
        };
        let Some(idol) = idols.iter().find(|idol| idol.race == plate.0) else {
            continue;
        };
        if let Ok(px) = camera.world_to_viewport(cam_gt, idol.world_centre()) {
            node.left = Val::Px(plate_left_px(px.x, PLATE_SIZE.0));
            node.top = Val::Px(px.y);
        }
    }
    for (ink, mut colour) in ink.iter_mut() {
        let alpha = plates
            .iter()
            .find(|(plate, ..)| plate.0 == ink.race)
            .map(|(_, fade, ..)| fade.0)
            .unwrap_or(0.0);
        colour.0 = ink.base.with_alpha(ink.base.alpha() * alpha);
    }
}

/// `Update` in `RegionSelect`: a left click while an idol is hovered confirms
/// that race. A click on nothing does nothing,
/// which is why the original's own `g_region != -1` guard is the condition here
/// too.
#[allow(clippy::too_many_arguments)]
/// The three resources a click sound needs, as one system param.
///
/// `confirm_hovered_region` already carries twelve parameters; three more would
/// put it one short of the arity Bevy implements, and the error that arity
/// failure produces names no type at all (AGENTS.md warns about exactly this).
/// Bundling them also says what they are for in one place.
#[derive(bevy::ecs::system::SystemParam)]
pub struct ClickSound<'w> {
    options: Res<'w, GameOptions>,
    assets: Res<'w, IntroV2Assets>,
}

impl ClickSound<'_> {
    /// The sound the original plays for every activating button click
    /// (`snd_button_click`, raised by its generic button class). Our race
    /// plates are picked by a ray against the 3D idols rather than by a
    /// `ui_v2` button, so they never reach `play_button_click_sound`'s
    /// `On<Activate>` observer and the confirm click was silent.
    fn play_click(&self, commands: &mut Commands) {
        if let Some(playback) = self.options.audio.fx_playback() {
            commands.spawn((
                AudioPlayer::new(self.assets.sound_button_sound_a.clone()),
                playback,
            ));
        }
    }
}

pub fn confirm_hovered_region(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredRegion>,
    char_data: Res<ClientCharacterData>,
    ui_strings: Res<ClientUiStrings>,
    confirm: Option<Res<RegionConfirm>>,
    stage_cam: Query<(Entity, &Transform), With<CinematicCamera2>>,
    scene: Option<Res<ActiveCharSelectSceneV2>>,
    origin: Res<WorldOrigin>,
    fade: Query<Entity, With<super::fade::FadeScreenV2>>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    click_sound: ClickSound,
    mut commands: Commands,
) {
    // A second click during the confirm move must not restart it (the original
    // sets a flag for exactly this and skips its own tick).
    let forced = forced_confirm_at().is_some_and(|at| time.elapsed_secs() >= at);
    if confirm.is_some() || !(buttons.just_pressed(MouseButton::Left) || forced) {
        return;
    }
    let Some(race) = hovered.0 else {
        return;
    };
    click_sound.play_click(&mut commands);
    enter_creation_for(
        race,
        &char_data,
        &ui_strings,
        &stage_cam,
        scene.as_deref(),
        &origin,
        &fade,
        &mut info_text_writer,
        &mut commands,
    );
}

/// Region confirmed: data-blocked races only report the gap; otherwise seed the
/// creation selection with the race and **start the click's camera move**. What
/// used to stand here — spawn the loading art and switch the state, both inside
/// the click — was the wrong order and the missing move in one: the
/// original's confirm starts a 1.0 s flight and only its *state 9*,
/// after the move has landed, shows the loading picture. Everything after
/// the move now lives in [`tick_region_confirm`].
///
/// A free function rather than an observer, because the trigger moved from a
/// plate click to a pointer-plus-click on the figure, and this half — what
/// *happens* once a race is chosen — is unchanged by that.
#[allow(clippy::too_many_arguments)]
fn enter_creation_for(
    race: Race,
    char_data: &ClientCharacterData,
    ui_strings: &ClientUiStrings,
    stage_cam: &Query<(Entity, &Transform), With<CinematicCamera2>>,
    scene: Option<&ActiveCharSelectSceneV2>,
    origin: &WorldOrigin,
    fade: &Query<Entity, With<super::fade::FadeScreenV2>>,
    info_text_writer: &mut MessageWriter<InfoTextV2Update>,
    commands: &mut Commands,
) {
    if !race_available(char_data, race) {
        // The label key follows the chosen race — reporting "European" for a
        // blocked Chinese plate was the original wording bug here.
        let (key, fallback) = race.label();
        info_text_writer.write(InfoTextV2Update(
            ui_strings
                .get_or(PLATE_DISABLED_KEY, PLATE_DISABLED_FALLBACK)
                .to_string(),
        ));
        // The diagnosis goes where the person who can act on it looks; the
        // screen gets the original's sentence (see `PLATE_DISABLED_KEY`).
        warn!(
            "region plate {:?} is blocked: {}",
            ui_strings.get_or(key, &fallback),
            PLATE_DISABLED_DIAGNOSIS
        );
        return;
    }

    commands.insert_resource(CharCreateSelection {
        race,
        ..Default::default()
    });

    // The move itself. Without a stage or a camera (headless, half-built stage)
    // there is nothing to fly, so the sequence starts anyway and its flight leg
    // simply runs out on the clock — a dead click would be the worse failure.
    if let Some(scene) = scene {
        super::race_stage::start_confirm_flight(race, stage_cam, scene, origin, commands);
    }
    if super::race_stage::CONFIRM_FADE_TO_BLACK {
        // The original hides its own move under this. Off by default,
        // see `race_stage::CONFIRM_FADE_TO_BLACK` for the reason.
        for entity in fade.iter() {
            commands
                .entity(entity)
                .insert(TweenAnim::new(Tween::new::<BackgroundColor, _>(
                    EaseMethod::EaseFunction(EaseFunction::Linear),
                    super::race_stage::CONFIRM_FLIGHT,
                    super::fade::BackgroundColorLens {
                        start: Color::NONE.to_srgba().to_vec4(),
                        end: Color::BLACK.to_srgba().to_vec4(),
                    },
                )));
        }
    }
    commands.insert_resource(RegionConfirm::new(race));
}

/// `Update` in `RegionSelect` while a [`RegionConfirm`] lives: the three steps
/// the original puts *after* the click, in its order.
///
/// * the flight runs;
/// * it lands → the race art appears with `GDR_LOADINGG` at `0.0`, and the
///   creation screen's own assets are requested — that is what the gauge counts;
/// * the count fills → `CharacterCreate` (state `0xc` → creation screen).
///
/// The gauge is a **real** load counter, like the original's `loaded / 1000.0`:
/// the starter body plus its four equipment
/// models, through `AssetServer::recursive_dependency_load_state`, which is the
/// same question the preview asks a frame later.
#[allow(clippy::too_many_arguments)]
pub fn tick_region_confirm(
    time: Res<Time>,
    mut confirm: ResMut<RegionConfirm>,
    selection: Option<ResMut<CharCreateSelection>>,
    char_data: Res<ClientCharacterData>,
    item_index: Res<ClientItemIndex>,
    item_data: Res<ClientItemData>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut gauges: Query<(&mut Node, &mut LoadingProgress)>,
    mut fade: Query<(Entity, &mut BackgroundColor), With<super::fade::FadeScreenV2>>,
    mut next_state: ResMut<NextState<IntroV2State>>,
    mut commands: Commands,
) {
    let settled = confirm
        .watched
        .iter()
        .filter(|handle| {
            let state = asset_server.recursive_dependency_load_state(handle.id());
            // A failed load is *settled*, not pending: a corpus missing one
            // equipment model must not park the player on a gauge at 80%.
            state.is_loaded() || state.is_failed()
        })
        .count();
    let total = confirm.watched.len();
    match confirm.advance(time.delta(), settled, total) {
        ConfirmStep::Flying => {}
        ConfirmStep::ShowCut => {
            let race = confirm.race;
            if let Some(camera) = cam_query.iter().next() {
                spawn_loading_cut(&mut commands, &asset_server, race, camera);
            }
            set_gauge_fraction(&mut gauges, 0.0);
            if let Some(selection) = selection {
                confirm.watched = super::character_create::creation_preload_paths(
                    &selection,
                    &char_data,
                    &item_index,
                    &item_data,
                )
                .into_iter()
                .map(|path| asset_server.load::<SroResource>(path).untyped())
                .collect();
            }
            info!(
                "region confirm: flight landed, loading {} creation assets",
                confirm.watched.len()
            );
        }
        ConfirmStep::Loading(fraction) => set_gauge_fraction(&mut gauges, fraction),
        ConfirmStep::Done => {
            set_gauge_fraction(&mut gauges, 1.0);
            // Re-stamp the selection as changed. Not cosmetic: the creation
            // screen spawns its figure in `update_preview`, which runs under
            // `resource_exists_and_changed::<CharCreateSelection>`. The click
            // used to switch the state in the same frame it inserted the
            // selection, so the change was fresh on the other side; with the
            // original's order the click is a second and a load earlier, and
            // the change is stale by then — the symptom was a creation screen
            // with an EMPTY podium (the jump hook, which inserts and switches
            // together, showed the figure). The hand-over
            // is the moment the selection becomes news for the creation screen,
            // so it is stamped here.
            if let Some(mut selection) = selection {
                selection.set_changed();
            }
            // Lift the confirm's fade. The original's `GDR_FADE` is a
            // control it owns and drops with the screen; ours is the shared
            // `FadeScreenV2` overlay, which keeps whatever the tween left on it
            // — the symptom was a creation screen that was entirely BLACK.
            // The loading art (z 500) is over the overlay from
            // the moment it spawns, so clearing it here is invisible and one
            // frame later than strictly needed.
            for (entity, mut colour) in fade.iter_mut() {
                commands.entity(entity).remove::<TweenAnim>();
                colour.0 = Color::NONE;
            }
            commands.remove_resource::<RegionConfirm>();
            // Board -> creation is a move on the SAME stage, so the stage camera
            // must survive this exit; the marker is what `OnExit(RegionSelect)`
            // asks (`scenes/intro_v2/mod.rs`). Without it the camera was
            // despawned here and re-spawned at the render origin on the other
            // side — a visible pop, first the new scene and only then the
            // camera move.
            commands.insert_resource(super::character_create::EnteringCharacterCreate);
            next_state.set(IntroV2State::CharacterCreate);
        }
    }
}

/// `OnExit(RegionSelect)`: a confirm that did not finish must not tick into the
/// next visit (same reason as `race_stage::clear_race_board_flight`).
pub fn clear_region_confirm(mut commands: Commands) {
    commands.remove_resource::<RegionConfirm>();
}

/// The original board→creation cut: full-screen race art + the loading-bar
/// chrome (gauge skipped — no real progress source).
fn spawn_loading_cut(
    commands: &mut Commands,
    asset_server: &AssetServer,
    race: Race,
    camera: Entity,
) {
    // Per-race loading picture from the race's presentation; a race we have
    // no authored picture for shows the un-suffixed one (the file every
    // corpus ships) instead of a guessed `_<code>` name.
    let art = race
        .presentation()
        .map(|p| p.loading_ddj)
        .unwrap_or(KNOWN_RACES[0].loading_ddj);
    commands
        .spawn((
            CreationLoadingCut(Timer::from_seconds(LOADING_CUT_SECS, TimerMode::Once)),
            UiTargetCamera(camera),
            Name::from("Creation Loading Cut"),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                // the 4:3 cover box overflows on the axis that does not fit
                overflow: bevy::ui::Overflow::clip(),
                ..default()
            },
            GlobalZIndex(500),
        ))
        .with_children(|cut| {
            // Art and chrome from the module that owns the rects: the
            // hand-copied version here mixed design spaces (percentage frame,
            // Val::Px caption), so the caption slid off the frame at any size
            // but exactly design scale. The art moved off this root into the
            // 4:3 cover box — a 1024x768 painting stretched to a 16:9 window is
            // visibly squashed (`loading_screen::DESIGN_ASPECT`).
            //
            // `with_gauge: true`: the original shows `GDR_LOADINGG` (id `0x18`)
            // here and resets it to `0.0`, and we have something real for it to
            // count — see [`tick_region_confirm`].
            spawn_loading_surface(cut, asset_server, asset_server.load(art), true);
        });
}

/// Ticks the loading cut down and removes it (runs in `CharacterCreate`).
pub fn tick_loading_cut(
    time: Res<Time>,
    mut cuts: Query<(Entity, &mut CreationLoadingCut)>,
    mut commands: Commands,
) {
    for (entity, mut cut) in cuts.iter_mut() {
        if cut.0.tick(time.delta()).just_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// `OnExit(RegionSelect)` / `OnExit(CharacterCreate)` cleanup.
pub fn despawn_region_select(roots: Query<Entity, With<RegionSelectRoot>>, mut commands: Commands) {
    for entity in roots.iter() {
        commands.entity(entity).despawn();
    }
}

/// Safety net: a lingering cut is removed when creation is left.
pub fn despawn_loading_cut(cuts: Query<Entity, With<CreationLoadingCut>>, mut commands: Commands) {
    for entity in cuts.iter() {
        commands.entity(entity).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::textdata::characterdata::{CharacterData, CharacterDataRow};
    use std::collections::HashMap;

    /// Every rect this screen takes from `resinfo/pscharacterselect.txt`
    /// section `Select` and its `China`/`Europe` sections, pinned so a later
    /// edit cannot drift them. The values are the authored ones and match the
    /// size of the art they place.
    #[test]
    fn the_board_rects_are_the_authored_ones() {
        // GDR_STA_REGIONTITLE "47,110,292,36" — text-region.ddj is 292x36
        assert_eq!(TITLE_RECT, (47.0, 110.0, 292.0, 36.0));
        // GDR_STA_CHINA / _ISLAM / _EUROPE all "0,0,440,152"; all three arts
        // are 440x152
        assert_eq!(PLATE_SIZE, (440.0, 152.0));
        // GDR_STATIC1 in each race section: 92x15 at y=9, x per race
        assert_eq!(
            (PLATE_LABEL_RECT.1, PLATE_LABEL_RECT.2, PLATE_LABEL_RECT.3),
            (9.0, 92.0, 15.0)
        );
        let china_x = Race::CHINESE.presentation().unwrap().plate_label_x;
        let europe_x = Race::EUROPEAN.presentation().unwrap().plate_label_x;
        assert_eq!(china_x, 318.0);
        assert_eq!(europe_x, 250.0);
        assert_ne!(
            china_x, europe_x,
            "the label x really does differ per race section"
        );
        // GDR_TEXT_CHINA / GDR_TEXT_EUROPE "19,47,401,87"
        assert_eq!(PLATE_BODY_RECT, (19.0, 47.0, 401.0, 87.0));
    }

    /// The plate-local rects have to fit the plate, and the title has to fit
    /// the 1600x1200 canvas its percentage placement is derived from.
    #[test]
    fn the_authored_rects_fit_their_parents() {
        assert_eq!(DESIGN, (1600.0, 1200.0));
        assert!(TITLE_RECT.0 + TITLE_RECT.2 <= DESIGN.0);
        assert!(TITLE_RECT.1 + TITLE_RECT.3 <= DESIGN.1);
        for x in super::super::race_catalog::KNOWN_RACES
            .iter()
            .map(|p| p.plate_label_x)
        {
            assert!(x + PLATE_LABEL_RECT.2 <= PLATE_SIZE.0);
        }
        assert!(PLATE_LABEL_RECT.1 + PLATE_LABEL_RECT.3 <= PLATE_SIZE.1);
        assert!(PLATE_BODY_RECT.0 + PLATE_BODY_RECT.2 <= PLATE_SIZE.0);
        assert!(PLATE_BODY_RECT.1 + PLATE_BODY_RECT.3 <= PLATE_SIZE.1);
        // label above body, both inside the plate
        assert!(PLATE_LABEL_RECT.1 + PLATE_LABEL_RECT.3 <= PLATE_BODY_RECT.1);
    }

    /// Minimal characterdata row: the parser keeps any tab row with >10
    /// columns and reads the code name from column 2, so only the id and the
    /// code name matter for the race predicate.
    fn row(id: i32, code_name: &str) -> (i32, CharacterDataRow) {
        let mut cols = vec![String::new(); 60];
        cols[1] = id.to_string();
        cols[2] = code_name.to_string();
        (id, CharacterDataRow(cols))
    }

    fn char_data(rows: Vec<(i32, CharacterDataRow)>) -> ClientCharacterData {
        ClientCharacterData::from_table(CharacterData(rows.into_iter().collect::<HashMap<_, _>>()))
    }

    /// A data set whose `characterdata_5000.txt` carries the `CHAR_CH_*`
    /// player bodies while **no** shard carries a `CHAR_EU_*` row. The
    /// `MOB_EU_*`/`NPC_EU_*` rows in the other shards show the `EU` token
    /// itself reads fine, so the absence is data, not a decode bug.
    /// Consequence: Chinese is offerable, European is not.
    #[test]
    fn a_corpus_without_european_bodies_blocks_only_the_europe_plate() {
        let data = char_data(vec![
            row(1907, "CHAR_CH_MAN_ADVENTURER"),
            row(1919, "CHAR_CH_MAN_WARRIOR"),
            row(1920, "CHAR_CH_WOMAN_ADVENTURER"),
            row(1932, "CHAR_CH_WOMAN_WARRIOR"),
            // same corpus, same read path: EU exists as monsters/NPCs only
            row(5851, "MOB_EU_MOVOI"),
            row(41769, "NPC_EU_EVENT_CARNIVAL_OBJECT_2011"),
        ]);
        assert!(race_available(&data, Race::CHINESE));
        assert!(!race_available(&data, Race::EUROPEAN));
    }

    /// The regression guard proper: when a corpus *does* ship European bodies,
    /// the predicate must offer Europe. This is what would have caught a
    /// broken race predicate, which is indistinguishable in the UI from a
    /// corpus that simply has no European rows.
    #[test]
    fn european_bodies_make_the_europe_plate_offerable() {
        let male_only = char_data(vec![row(14000, "CHAR_EU_MAN_FIGHTER")]);
        assert!(race_available(&male_only, Race::EUROPEAN));

        let female_only = char_data(vec![row(14100, "CHAR_EU_WOMAN_FIGHTER")]);
        assert!(race_available(&female_only, Race::EUROPEAN));
    }

    /// **(d) The order.** The loading picture may not appear before the click's
    /// camera move has landed, and its gauge starts at zero on a real count.
    ///
    /// The whole rule in one test: the original's state 9
    /// waits for the camera move to land and only *then* shows the art with
    /// the gauge reset to `0.0`, while ours spawned the
    /// art inside the click handler.
    ///
    /// Negative control — the old behaviour, verified failing: making
    /// [`RegionConfirm::advance`] return `ShowCut` before the timer finishes (the
    /// old order: art at click time) fails the first two assertions, and
    /// driving the gauge from the flight timer instead of the load count fails
    /// `Loading(0.0)` at the moment the art appears.
    #[test]
    fn the_loading_art_waits_for_the_flight_and_the_gauge_starts_at_zero() {
        use std::time::Duration;
        let mut confirm = RegionConfirm::new(Race::CHINESE);
        // the first second is the flight, and nothing else happens in it
        assert_eq!(
            confirm.advance(Duration::from_millis(400), 0, 0),
            ConfirmStep::Flying
        );
        assert_eq!(
            confirm.advance(Duration::from_millis(500), 3, 5),
            ConfirmStep::Flying,
            "a load that is already running must not pull the art forward"
        );
        // ... and the art appears exactly when it lands
        assert_eq!(
            confirm.advance(Duration::from_millis(100), 3, 5),
            ConfirmStep::ShowCut
        );
        // the gauge counts real assets, from zero — the load is only requested
        // now, so `settled` is 0 of 5 at that moment
        assert_eq!(
            confirm.advance(Duration::ZERO, 0, 5),
            ConfirmStep::Loading(0.0)
        );
        assert_eq!(
            confirm.advance(Duration::ZERO, 2, 5),
            ConfirmStep::Loading(0.4)
        );
        // and the creation screen is entered only when the count is full
        assert_eq!(
            confirm.advance(Duration::ZERO, 4, 5),
            ConfirmStep::Loading(0.8)
        );
        assert_eq!(confirm.advance(Duration::ZERO, 5, 5), ConfirmStep::Done);
        // the flight leg is the original's 1.0 s, not a taste value
        assert_eq!(
            super::super::race_stage::CONFIRM_FLIGHT,
            Duration::from_secs(1)
        );
    }

    /// A selection whose assets do not resolve must not park the player in front
    /// of a gauge that can never fill.
    #[test]
    fn a_confirm_with_nothing_to_load_still_reaches_the_creation_screen() {
        use std::time::Duration;
        let mut confirm = RegionConfirm::new(Race::EUROPEAN);
        assert_eq!(
            confirm.advance(Duration::from_secs(1), 0, 0),
            ConfirmStep::ShowCut
        );
        assert_eq!(confirm.advance(Duration::ZERO, 0, 0), ConfirmStep::Done);
    }

    /// The confirm takes BOTH plates away, at its own rate.
    ///
    /// Negative control built in: the confirm fade is slower than the hover fade
    /// (0.5 s against 1/3 s), so re-using [`plate_alpha_step`] for the dismissal
    /// — the obvious shortcut — fails the middle assertion.
    #[test]
    fn the_confirm_fades_both_plates_out_over_half_a_second() {
        // full plate, quarter of a second in: half gone
        let half = plate_alpha_dismiss(1.0, 0.25);
        assert!((half - 0.5).abs() < 1e-6, "{half}");
        // the hover ramp would already be at 0.25 after the same time
        assert!(half > plate_alpha_step(1.0, false, 0.25));
        // gone after 0.5 s, and never below zero
        assert_eq!(plate_alpha_dismiss(1.0, 0.5), 0.0);
        assert_eq!(plate_alpha_dismiss(0.1, 1.0), 0.0);
        assert_eq!(CONFIRM_PLATE_FADE_PER_SEC, 2.0);
    }

    /// The plate is invisible at rest and RAMPS in only for the hovered race —
    /// the original's behaviour, and the half that was wrong before.
    ///
    /// The negative control is built into the assertions rather than bolted on:
    /// the intermediate value after a sixth of a second must be strictly between
    /// 0 and 1, which **no** `Visibility` flip and no always-visible plate can
    /// produce. Verified failing against the old behaviour by replacing
    /// `plate_alpha_step` with `if hovered { 1.0 } else { 0.0 }` (the flip) and
    /// with `1.0` (an always-on plate): the flip fails on the two `< 1.0`
    /// assertions, the always-on version already fails the resting assertion.
    #[test]
    fn a_plate_is_invisible_at_rest_and_ramps_in_on_its_own_idol() {
        // resting: not hovered, and it stays gone
        assert_eq!(plate_alpha_step(0.0, false, 1.0 / 60.0), 0.0);
        // the original's rate: full alpha in one third of a second
        let sixth = plate_alpha_step(0.0, true, 1.0 / 6.0);
        assert!(
            sixth > 0.0 && sixth < 1.0,
            "a third of a second is the full fade, so half of it is halfway: {sixth}"
        );
        assert!((sixth - 0.5).abs() < 1e-6, "{sixth}");
        assert_eq!(plate_alpha_step(0.0, true, 1.0 / 3.0), 1.0);
        // and it never overshoots in either direction
        assert_eq!(plate_alpha_step(1.0, true, 1.0), 1.0);
        assert_eq!(plate_alpha_step(0.2, false, 1.0), 0.0);
        let fading = plate_alpha_step(1.0, false, 1.0 / 6.0);
        assert!(fading > 0.0 && fading < 1.0, "{fading}");
        // the rate is the original's one, not a taste
        assert_eq!(PLATE_FADE_PER_SEC, 3.0);
    }

    /// The plate follows its own figure instead of sitting at a fixed screen
    /// fraction, and it is clamped exactly like the original.
    ///
    /// Negative control: two different projected x values must give two
    /// different left edges. The centred row this replaced would give the same
    /// left edge for both — that is the assertion that fails with the old
    /// placement.
    #[test]
    fn a_plate_hangs_on_its_own_idol_and_is_clamped_like_the_original() {
        // centred on the figure
        assert_eq!(plate_left_px(1000.0, PLATE_SIZE.0), 1000.0 - 220.0);
        assert_eq!(plate_left_px(600.0, PLATE_SIZE.0), 600.0 - 220.0);
        assert_ne!(
            plate_left_px(1000.0, PLATE_SIZE.0),
            plate_left_px(600.0, PLATE_SIZE.0),
            "a plate that follows its idol cannot land in the same place for two idols"
        );
        // the original's `if (x < 5) x = 5`
        assert_eq!(plate_left_px(10.0, PLATE_SIZE.0), PLATE_LEFT_CLAMP_PX);
        assert_eq!(plate_left_px(-500.0, PLATE_SIZE.0), PLATE_LEFT_CLAMP_PX);
        assert_eq!(PLATE_LEFT_CLAMP_PX, 5.0);
    }

    /// The hover target is the idol's own box, and a ray past it is a miss —
    /// the stand-in for the original's `-1.0` return.
    #[test]
    fn the_pick_hits_the_idol_and_misses_beside_it() {
        use crate::scenes::intro_v2::race_stage::RaceBoardIdol;
        let idol = RaceBoardIdol {
            race: Race::CHINESE,
            world_min: Vec3::new(-1.0, 0.0, -1.0),
            world_max: Vec3::new(1.0, 2.0, 1.0),
        };
        // straight at it
        assert!(idol.ray_hit(Vec3::new(0.0, 1.0, -10.0), Vec3::Z).is_some());
        // parallel, beside it — the negative control
        assert!(idol.ray_hit(Vec3::new(5.0, 1.0, -10.0), Vec3::Z).is_none());
        // above it
        assert!(idol.ray_hit(Vec3::new(0.0, 9.0, -10.0), Vec3::Z).is_none());
        // behind the ray
        assert!(idol.ray_hit(Vec3::new(0.0, 1.0, 10.0), Vec3::Z).is_none());
    }

    /// A blocked plate must say something a PLAYER can read, and the cause has
    /// to survive somewhere a maintainer can read it.
    ///
    /// Both halves, because the first one used to be broken in the most
    /// embarrassing way: the plate carried our internal diagnosis ("no playable
    /// body row CHAR_EU_* … a creation packet needs that row's ref id") on
    /// screen. The sentence now comes from the original's own
    /// key, and the diagnosis is asserted to still name the file that would
    /// unblock it — just not in the player's face.
    #[test]
    fn a_blocked_plate_speaks_to_the_player_and_logs_for_the_maintainer() {
        // player-facing: the original's key, and a fallback that is one plain
        // sentence with no identifiers in it
        assert_eq!(PLATE_DISABLED_KEY, "UIO_MSG_ERROR_ REGION_SUPPORT");
        assert!(!PLATE_DISABLED_FALLBACK.contains('_'));
        assert!(!PLATE_DISABLED_FALLBACK.contains("PK2"));
        assert!(PLATE_DISABLED_FALLBACK.len() < 60);
        // maintainer-facing: still names the file that would unblock it
        assert!(PLATE_DISABLED_DIAGNOSIS.contains("characterdata"));
    }
}

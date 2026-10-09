//! Content of the options window's Video tab.
//!
//! Idea: the pane is transcribed from the **live** branch of
//! `Media.pk2/resinfo/ifoption_video.txt` — the one guarded by `APPLY_UI_4TH`,
//! which `config/define.txt:15` defines, so the `#else` half (with its quality
//! spinner and a 30px-lower list) is build-excluded and is *not* what the
//! shipped client draws. Geometry is page-local to the pane rect
//! `11,62,364,313` from `ifoption.txt`.
//!
//! Two things are deliberately **not** invented here, because the user's data
//! does not carry them:
//!
//! * **Defaults.** No `Default=`/`Min=`/`Max=` key exists anywhere in
//!   `resinfo/`; the original's shipped defaults live in the exe. What a row
//!   shows is whatever [`GameOptions`] holds, whose own baseline is openroad's
//!   (see `settings/options.rs`), not a recovered constant.
//! * **Which value list belongs to which row.** The value vocabulary is closed
//!   and verified (textuisystem 943-960), but nothing in the data binds a list
//!   to a row. Rows whose list is an inference render their raw stored value
//!   rather than a plausible-looking word.
//!
//! Only controls with a real backing feature are interactive. The rest are
//! rendered — the original has them, so hiding them would be its own drift —
//! but dimmed and marked, per the "no fake controls" rule.

use bevy::camera::{Hdr, RenderTarget};
use bevy::picking::hover::Hovered;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::plugins::config::graphics::FoliageMode;
use crate::plugins::config::preset::QualityPreset;
use crate::plugins::config::ClientConfig;
use crate::plugins::settings::options::{GameOptions, GraphicProfile};
use crate::plugins::textdata::ClientUiStrings;

/// Page-local geometry, all `[V]` from `resinfo/ifoption_video.txt`.
const BOARD_X: f32 = 12.0;
const BOARD_Y: f32 = 12.0;
/// `opt_video_control_03.ddj`, 340x64 (verified from its DDS header).
const BOARD_W: f32 = 340.0;
const BOARD_H: f32 = 64.0;
const LABEL_X: f32 = 15.0;
const LABEL_W: f32 = 156.0;
const LABEL_H: f32 = 12.0;
const VALUE_X: f32 = 171.0;
const VALUE_W: f32 = 176.0;
const VALUE_H: f32 = 20.0;
const RESOLUTION_LABEL_Y: f32 = 23.0;
const RESOLUTION_VALUE_Y: f32 = 18.0;
const BRIGHTNESS_LABEL_Y: f32 = 53.0;
const BRIGHTNESS_VALUE_Y: f32 = 48.0;
/// `GDR_OPT_VIDEO_ST_CTRL_1` / `_2`; `opt_video_tab_back.ddj` is 124x28.
const PROFILE_TAB_Y: f32 = 76.0;
const PROFILE_TAB_W: f32 = 124.0;
const PROFILE_TAB_H: f32 = 28.0;
const PROFILE_TAB_XS: [f32; 2] = [13.0, 137.0];
/// `GDR_OPT_VIDEO_DETAIL_OPT`, the scrolling detail list.
const LIST_X: f32 = 14.0;
const LIST_Y: f32 = 103.0;
const LIST_W: f32 = 336.0;
const LIST_H: f32 = 191.0;
/// Row pitch of the 4th-gen slot list; the classic tree carries no pitch, and
/// the row inventory there is `[U]`.
const ROW_H: f32 = 22.0;

const BOARD_DDJ: &str = "media://interface/option/opt_video_control_03.ddj";
const TAB_DDJ: &str = "media://interface/option/opt_video_tab_back.ddj";
const FONT: &str = crate::assets::BUNDLED_FALLBACK_FACE;

const LABEL_COLOR: Color = Color::srgb(1.0, 0.965, 0.827);
const VALUE_COLOR: Color = Color::WHITE;
/// Rows whose feature does not exist yet render dimmed, so the pane never
/// pretends a control does something.
const UNBACKED_COLOR: Color = Color::srgb(0.45, 0.45, 0.45);
const ACTIVE_PROFILE_COLOR: Color = Color::srgb(1.0, 0.816, 0.318);

/// Which of the two graphics banks the pane is editing. The original keeps two
/// complete profiles — ids `1..=15`/`501..=504` and `101..=115`/`601..=604` —
/// selected by the Graphic 1 / Graphic 2 tabs.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum GraphicProfileTab {
    #[default]
    One,
    Two,
}

/// Root of the Video pane content; carries the selected profile.
#[derive(Component, Default)]
pub(crate) struct VideoPane {
    pub(crate) profile: GraphicProfileTab,
}

/// A profile tab caption, recolored when the selection changes.
#[derive(Component)]
pub(crate) struct ProfileTabLabel(GraphicProfileTab);

/// A detail row's value text, refreshed from [`GameOptions`].
#[derive(Component)]
pub(crate) struct RowValue {
    /// SROptionSet id of the Graphic 1 bank; the Graphic 2 id is `+100`.
    id: u16,
}

/// One detail row of the quality list.
struct DetailRow {
    /// Graphic 1 id (`docs/formats/sroptionset.md`); Graphic 2 is `+100`.
    id: u16,
    /// textuisystem key, `[V]` from the resinfo tree.
    key: &'static str,
    /// English fallback, matching the user's own textuisystem line.
    english: &'static str,
    /// Whether a real feature backs this row today.
    backing: Backing,
}

/// Whether a control can actually change anything in this client.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backing {
    /// Wired to a live feature and interactive.
    Live,
    /// Steps a `graphics` config key (see [`config_row`]): the choice is
    /// saved in the profile bank and laid over `config.yaml` by
    /// [`apply_config_rows`].
    Config,
}

/// A quality row backed by a `graphics` config key.
///
/// The original stores these rows as SROptionSet values whose meanings live
/// in its code. Ours store the index of the chosen step instead, in the same
/// ids and banks, which is a deliberate deviation (ADR 0009): the steps are
/// this client's settings, not the original's. Precedence follows the
/// original's own options file, so a value saved here wins over
/// `config.yaml`, which wins over the graphics preset.
struct ConfigRow {
    /// What each step is called, lowest first.
    steps: &'static [&'static str],
    /// Why a change only shows after a restart, if it does.
    restart: Option<&'static str>,
}

/// View distances for Background Sight Range. 5760 is the original's
/// fog-bound draw distance; the others are the presets' steps.
const SIGHT_STEPS: [f32; 5] = [1920.0, 2880.0, 3840.0, 5760.0, 7680.0];
/// Character Sight Range steps; 0 = as far as the server sends them. Never
/// below `view_range::MIN_CHARACTER_DISTANCE`, which clicking reaches.
const CHARACTER_SIGHT_STEPS: [f32; 4] = [1000.0, 1500.0, 2500.0, 0.0];
/// Light Effect steps: the dungeon point-light budget. 16 is the default.
const LIGHT_STEPS: [u32; 4] = [4, 8, 16, 32];
/// Texture Filtering steps: the anisotropic filtering clamp.
const FILTERING_STEPS: [u16; 5] = [1, 2, 4, 8, 16];

/// The config key behind row `id`, if it has one.
fn config_row(id: u16) -> Option<ConfigRow> {
    let row = |steps, restart| Some(ConfigRow { steps, restart });
    let off_on: &'static [&'static str] = &["Off", "On"];
    match id {
        1 => row(off_on, Some("restart; PBR lighting only")),
        2 => row(
            &[
                "Shortest (1920)",
                "Short (2880)",
                "Medium (3840)",
                "Original (5760)",
                "Far (7680)",
            ],
            None,
        ),
        3 => row(
            &["Near (1000)", "Medium (1500)", "Far (2500)", "Unlimited"],
            None,
        ),
        4 => row(off_on, Some("next scene; high water only")),
        5 => row(&["Low", "High"], Some("restart")),
        6 => row(off_on, Some("restart")),
        7 => row(
            &["Low (4)", "Medium (8)", "High (16)", "Highest (32)"],
            None,
        ),
        8 => row(
            &[
                "Trilinear",
                "Anisotropic 2x",
                "Anisotropic 4x",
                "Anisotropic 8x",
                "Anisotropic 16x",
            ],
            Some("restart"),
        ),
        9 => row(&["Quarter", "Half", "Full"], Some("restart")),
        10 => row(off_on, None),
        12 => row(off_on, None),
        13 => row(&["Low", "Medium", "High"], None),
        _ => None,
    }
}

/// The index of the step in `steps` nearest `value`.
fn nearest_step<T: Copy + Into<f64>>(steps: &[T], value: T) -> usize {
    let value: f64 = value.into();
    steps
        .iter()
        .enumerate()
        .min_by(|a, b| {
            ((*a.1).into() - value)
                .abs()
                .total_cmp(&((*b.1).into() - value).abs())
        })
        .map_or(0, |(i, _)| i)
}

/// The step row `id` is on in `config`. Off-scale values (a hand-edited
/// view distance) read as the nearest step.
fn config_row_step(id: u16, config: &ClientConfig) -> usize {
    use crate::plugins::config::graphics::{EffectQuality, TextureDetail, WaterQuality};
    let graphics = &config.graphics;
    match id {
        1 => usize::from(graphics.shadows.enabled),
        2 => nearest_step(&SIGHT_STEPS, graphics.view.view_distance),
        3 => match graphics.view.character_distance {
            // 0 (unlimited) is its own step, not the nearest distance
            range if range <= 0.0 => CHARACTER_SIGHT_STEPS.len() - 1,
            range => nearest_step(&CHARACTER_SIGHT_STEPS[..3], range),
        },
        4 => usize::from(graphics.depth_prepass),
        5 => usize::from(graphics.water.quality == WaterQuality::High),
        6 => usize::from(graphics.sheen.enabled),
        7 => match graphics.dungeon.max_lights {
            // 0 means unlimited: the top step
            0 => LIGHT_STEPS.len() - 1,
            lights => nearest_step(&LIGHT_STEPS, lights as u32),
        },
        8 => nearest_step(&FILTERING_STEPS, graphics.anisotropy.0),
        9 => match graphics.texture_detail {
            TextureDetail::Quarter => 0,
            TextureDetail::Half => 1,
            TextureDetail::Full => 2,
        },
        10 => usize::from(graphics.lens_flare),
        12 => usize::from(graphics.objects.animate),
        13 => match graphics.effect_quality {
            EffectQuality::Low => 0,
            EffectQuality::Medium => 1,
            EffectQuality::High => 2,
        },
        _ => 0,
    }
}

/// Puts row `id` on `step` in `config`.
fn set_config_row_step(id: u16, step: usize, config: &mut ClientConfig) {
    use crate::plugins::config::graphics::{
        Anisotropy, EffectQuality, TextureDetail, WaterQuality,
    };
    let graphics = &mut config.graphics;
    let pick = |steps_len: usize| step.min(steps_len - 1);
    match id {
        1 => graphics.shadows.enabled = step != 0,
        3 => {
            graphics.view.character_distance =
                CHARACTER_SIGHT_STEPS[pick(CHARACTER_SIGHT_STEPS.len())]
        }
        4 => graphics.depth_prepass = step != 0,
        6 => graphics.sheen.enabled = step != 0,
        7 => graphics.dungeon.max_lights = LIGHT_STEPS[pick(LIGHT_STEPS.len())] as usize,
        8 => graphics.anisotropy = Anisotropy(FILTERING_STEPS[pick(FILTERING_STEPS.len())]),
        10 => graphics.lens_flare = step != 0,
        12 => graphics.objects.animate = step != 0,
        13 => {
            graphics.effect_quality = match step {
                0 => EffectQuality::Low,
                1 => EffectQuality::Medium,
                _ => EffectQuality::High,
            }
        }
        2 => {
            let distance = SIGHT_STEPS[step.min(SIGHT_STEPS.len() - 1)];
            // the fog band scales with it: opaque at the view distance and
            // starting at two thirds of it, as the default 3840..5760 does
            graphics.view.view_distance = distance;
            graphics.view.fog_end = distance;
            graphics.view.fog_start = distance * 2.0 / 3.0;
        }
        5 => {
            graphics.water.quality = if step == 0 {
                WaterQuality::Low
            } else {
                WaterQuality::High
            }
        }
        9 => {
            graphics.texture_detail = match step {
                0 => TextureDetail::Quarter,
                1 => TextureDetail::Half,
                _ => TextureDetail::Full,
            }
        }
        _ => {}
    }
}

/// A config-backed row's value text, refreshed from [`ClientConfig`].
#[derive(Component)]
pub(crate) struct ConfigRowValue {
    id: u16,
}

/// Lays the config-backed rows saved in the active profile bank over
/// `ClientConfig`, so they take effect and outrank `config.yaml`. Runs at
/// startup (the options resource is loaded then) and on every change. Writes
/// only rows whose saved step differs from what the config already says,
/// because every `ClientConfig` write re-runs the config's apply systems.
pub(crate) fn apply_config_rows(
    options: Res<GameOptions>,
    panes: Query<&VideoPane>,
    mut config: ResMut<ClientConfig>,
) {
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    // read first: a write through `ResMut` would mark the config changed
    if overlay_needed(bank, &config) {
        overlay_saved_rows(bank, &mut config);
    }
}

/// Whether any saved config-backed row in `bank` differs from `config`.
fn overlay_needed(bank: &GraphicProfile, config: &ClientConfig) -> bool {
    DETAIL_ROWS
        .iter()
        .filter(|r| r.backing == Backing::Config)
        .any(
            |row| match (bank.quality.get(&row.id), config_row(row.id)) {
                (Some(&saved), Some(def)) => {
                    config_row_step(row.id, config) != (saved as usize).min(def.steps.len() - 1)
                }
                _ => false,
            },
        )
}

/// Lays the config-backed rows saved in `bank` over `config`. Also called by
/// `main` before the app is built, with the saved Graphic 1 bank, so the rows
/// whose settings are read once at startup (Texture Filtering, Metallic
/// Sheen, Water Detail...) take effect on the next launch.
pub(crate) fn overlay_saved_rows(bank: &GraphicProfile, config: &mut ClientConfig) {
    for row in DETAIL_ROWS.iter().filter(|r| r.backing == Backing::Config) {
        let (Some(&saved), Some(def)) = (bank.quality.get(&row.id), config_row(row.id)) else {
            continue;
        };
        let step = (saved as usize).min(def.steps.len() - 1);
        if config_row_step(row.id, config) != step {
            set_config_row_step(row.id, step, config);
        }
    }
}

/// Keeps the config-backed rows showing what is in effect, preset and
/// `config.yaml` included. Every frame, like [`refresh_extra_rows`].
pub(crate) fn refresh_config_rows(
    config: Res<ClientConfig>,
    mut rows: Query<(&ConfigRowValue, &mut Text)>,
) {
    for (row, mut text) in rows.iter_mut() {
        let Some(def) = config_row(row.id) else {
            continue;
        };
        let step = def.steps[config_row_step(row.id, &config).min(def.steps.len() - 1)];
        let wanted = match def.restart {
            Some(note) => format!("{step} ({note})"),
            None => step.to_string(),
        };
        if text.0 != wanted {
            *text = Text::new(wanted);
        }
    }
}

/// The 13 named quality rows. Ids and names are `[S]` from the SROptionSet
/// table (`docs/formats/sroptionset.md:62-99`); the label keys are `[V]` from
/// the resinfo tree. Ids 14/15 exist in the id space but their CSV name cells
/// are blank, so they are left out rather than captioned by guess.
const DETAIL_ROWS: [DetailRow; 13] = [
    DetailRow {
        id: 1,
        key: "UIIT_STT_SHADOW_DETAIL",
        english: "Shadow Detail",
        backing: Backing::Config,
    },
    DetailRow {
        id: 2,
        key: "UIIT_STT_SCENERY_SIGHT_RANGE",
        english: "Background Sight Range",
        backing: Backing::Config,
    },
    DetailRow {
        id: 3,
        key: "UIIT_STT_CHAR_SIGHT_RANGE",
        english: "Character Sight Range",
        backing: Backing::Config,
    },
    DetailRow {
        id: 4,
        key: "UIIT_STT_WATER_REFLECTION",
        english: "Water Reflection",
        backing: Backing::Config,
    },
    DetailRow {
        id: 5,
        key: "UIIT_STT_WATER_DETAIL",
        english: "Water Detail",
        backing: Backing::Config,
    },
    DetailRow {
        id: 6,
        key: "UIIT_STT_METAL_DETAIL",
        english: "Metallic Sheen",
        backing: Backing::Config,
    },
    DetailRow {
        id: 7,
        key: "UIIT_STT_LIGHT_EFFECT",
        english: "Light Effect",
        backing: Backing::Config,
    },
    DetailRow {
        id: 8,
        key: "UIIT_STT_FILTERING",
        english: "Texture Filtering",
        backing: Backing::Config,
    },
    DetailRow {
        id: 9,
        key: "UIIT_STT_TEXTER_DETAIL",
        english: "Texture Detail",
        backing: Backing::Config,
    },
    DetailRow {
        id: 10,
        key: "UIIT_STT_LENS_FLAIR",
        english: "Lens Flare",
        backing: Backing::Config,
    },
    // A live toggle of its own: `Bloom` is inserted/removed on the window
    // cameras (see `apply_bloom_option`).
    DetailRow {
        id: 11,
        key: "UIIT_STT_BLOOM_EFFECT",
        english: "Bloom Effect",
        backing: Backing::Live,
    },
    DetailRow {
        id: 12,
        key: "UIIT_STT_DYNAMIC_ANIMATION",
        english: "Dynamic Animation",
        backing: Backing::Config,
    },
    DetailRow {
        id: 13,
        key: "UIIT_STT_EFFECT_QUALITY",
        english: "Effect Quality",
        backing: Backing::Config,
    },
];

/// Id of the bloom row within a profile bank.
const BLOOM_ID: u16 = 11;

/// Spawns the Video pane's content into the pane node.
pub(crate) fn spawn_video_pane(
    pane: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    ui_strings: &ClientUiStrings,
    options: &GameOptions,
) {
    let font = asset_server.load::<Font>(FONT);

    pane.spawn((
        ImageNode::new(asset_server.load(BOARD_DDJ)),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(BOARD_X),
            top: Val::Px(BOARD_Y),
            width: Val::Px(BOARD_W),
            height: Val::Px(BOARD_H),
            ..default()
        },
        Pickable::IGNORE,
    ));

    // Resolution and brightness sit inside the board's extent.
    spawn_static_label(
        pane,
        &font,
        ui_strings.get_or("UIIT_STT_RESOLUTION", "Resolution"),
        RESOLUTION_LABEL_Y,
        LABEL_COLOR,
    );
    let profile = &options.video.graphic1;
    spawn_value_text(
        pane,
        &font,
        &format!("{} x {}", profile.width, profile.height),
        RESOLUTION_VALUE_Y,
        VALUE_COLOR,
    );

    spawn_static_label(
        pane,
        &font,
        ui_strings.get_or("UIIT_STT_LUMINOSITY", "Set Brightness"),
        BRIGHTNESS_LABEL_Y,
        LABEL_COLOR,
    );
    // Brightness is a 5-step enum (textuisystem 946-950), not a 0..255 scalar
    // — but nothing in this client applies it, so the raw value is shown
    // rather than a word implying it took effect.
    spawn_value_text(
        pane,
        &font,
        &format!("{} (not applied)", profile.brightness),
        BRIGHTNESS_VALUE_Y,
        UNBACKED_COLOR,
    );

    // Graphic 1 / Graphic 2: tabs over the detail list (the 1px overlap with
    // the list top is the original's tab-over-panel idiom, not a rounding
    // slip).
    let tab_style = asset_server.load::<Image>(TAB_DDJ);
    for (index, (tab, key, english)) in [
        (
            GraphicProfileTab::One,
            "UIIT_STT_GRAPHIC_QUALITY_CONTROL1",
            "Graphic 1",
        ),
        (
            GraphicProfileTab::Two,
            "UIIT_STT_GRAPHIC_QUALITY_CONTROL2",
            "Graphic 2",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        pane.spawn((
            Button,
            Hovered::default(),
            ImageNode::new(tab_style.clone()),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(PROFILE_TAB_XS[index]),
                top: Val::Px(PROFILE_TAB_Y),
                width: Val::Px(PROFILE_TAB_W),
                height: Val::Px(PROFILE_TAB_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .observe(
            move |_activate: On<Activate>, mut panes: Query<&mut VideoPane>| {
                for mut pane in panes.iter_mut() {
                    pane.profile = tab;
                }
            },
        )
        .with_children(|b| {
            b.spawn((
                ProfileTabLabel(tab),
                Text::new(ui_strings.get_or(key, english).to_string()),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(11.0),
                    ..default()
                },
                TextColor(if index == 0 {
                    ACTIVE_PROFILE_COLOR
                } else {
                    VALUE_COLOR
                }),
                Pickable::IGNORE,
            ));
        });
    }

    // The detail list. The original scrolls it; the 12-slot pool in the
    // 4th-gen tree is a viewport height, not the row inventory, so all 13
    // named rows are laid out and the list clips.
    pane.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(LIST_X),
            top: Val::Px(LIST_Y),
            width: Val::Px(LIST_W),
            height: Val::Px(LIST_H),
            flex_direction: FlexDirection::Column,
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.2)),
        Pickable::IGNORE,
    ))
    .with_children(|list| {
        for row in &DETAIL_ROWS {
            spawn_detail_row(list, &font, ui_strings, row, options);
        }
        for row in EXTRA_ROWS {
            spawn_extra_row(list, &font, row);
        }
    });
}

// ---------------------------------------------------------------------------
// openroad extras
//
// #366 checked the user's own PK2 before assuming: there is no grass /
// vegetation / foliage control anywhere in v1.188 — not in
// `resinfo/ifoption_video.txt`, not in `ifvideooptionslot.txt`, not in
// `textuisystem`. Our foliage layer is an openroad addition built on authored-
// but-unused tile data, so per ADR-0009 these rows are presented as a *stated*
// addition (own colour, "openroad" in the caption) rather than dressed up in
// the original's vocabulary. They sit below the 13 original rows so the
// vanilla list is untouched.
//
// They write `ClientConfig` directly and are applied by
// `map::foliage::apply_foliage_settings` on the live-settings mechanism
// (`plugins::settings::live`) — no restart, no scene reload.
// ---------------------------------------------------------------------------

/// openroad-added rows, drawn below the original's detail list.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ExtraRow {
    /// `graphics.foliage.mode`.
    FoliageMode,
    /// `graphics.foliage.density`, the tuft-count multiplier.
    FoliageDensity,
    /// `graphics.foliage.view_distance` — #366's "the single biggest win".
    /// Named after the original's own *sight range* vocabulary, which is the
    /// closest thing it has to this concept.
    FoliageSightRange,
    /// The graphics preset in effect and why (`graphics.preset`). Read-only:
    /// a preset is laid under `config.yaml` while it loads, so it changes
    /// with a restart, not a click.
    Preset,
}

const EXTRA_ROWS: [ExtraRow; 4] = [
    ExtraRow::Preset,
    ExtraRow::FoliageMode,
    ExtraRow::FoliageDensity,
    ExtraRow::FoliageSightRange,
];

/// Density steps. 1.0 is the authored baseline, so the scale is centred on it
/// rather than starting there.
const DENSITY_STEPS: [f32; 4] = [0.5, 1.0, 1.5, 2.0];
/// Sight-range steps in world units. 300 is the config default and covers the
/// camera's working range (`camera.rs`: 40..400); 0 means "never cull", which
/// is what shipped before #366 and is kept reachable but is not a step anyone
/// lands on by accident.
const SIGHT_RANGE_STEPS: [f32; 4] = [150.0, 300.0, 600.0, 0.0];
/// Openroad additions are drawn in their own colour so a screenshot never
/// suggests the original had this control.
const OPENROAD_COLOR: Color = Color::srgb(0.63, 0.84, 1.0);

impl ExtraRow {
    fn label(self) -> &'static str {
        match self {
            ExtraRow::FoliageMode => "Grass (openroad)",
            ExtraRow::FoliageDensity => "Grass Density (openroad)",
            ExtraRow::FoliageSightRange => "Grass Sight Range (openroad)",
            ExtraRow::Preset => "Graphics Preset (openroad)",
        }
    }

    /// What the row shows for the current config.
    fn value_text(self, config: &ClientConfig) -> String {
        let foliage = &config.graphics.foliage;
        match self {
            ExtraRow::FoliageMode => match foliage.mode {
                FoliageMode::Off => "Off (vanilla)".to_string(),
                FoliageMode::Native => "Authored".to_string(),
                FoliageMode::Pack => "Bundled".to_string(),
                FoliageMode::Both => "Authored + Bundled".to_string(),
            },
            ExtraRow::FoliageDensity => format!("{:.1}x", foliage.density),
            ExtraRow::FoliageSightRange => {
                if foliage.view_distance > 0.0 {
                    format!("{:.0}", foliage.view_distance)
                } else {
                    "Unlimited".to_string()
                }
            }
            ExtraRow::Preset => match &config.graphics.resolved_preset {
                Some(preset) if config.graphics.preset == QualityPreset::Auto => {
                    format!("{} (auto; config.yaml)", preset.tier.name())
                }
                Some(preset) => format!("{} (config.yaml)", preset.tier.name()),
                None => "none".to_string(),
            },
        }
    }

    /// Advances the row one step, writing `ClientConfig`.
    fn cycle(self, config: &mut ClientConfig) {
        let foliage = &mut config.graphics.foliage;
        match self {
            ExtraRow::FoliageMode => {
                foliage.mode = match foliage.mode {
                    FoliageMode::Off => FoliageMode::Native,
                    FoliageMode::Native => FoliageMode::Pack,
                    FoliageMode::Pack => FoliageMode::Both,
                    FoliageMode::Both => FoliageMode::Off,
                }
            }
            ExtraRow::FoliageDensity => {
                foliage.density = next_step(&DENSITY_STEPS, foliage.density)
            }
            ExtraRow::FoliageSightRange => {
                foliage.view_distance = next_step(&SIGHT_RANGE_STEPS, foliage.view_distance)
            }
            // read-only, see the variant
            ExtraRow::Preset => {}
        }
    }
}

/// The step after `current`, wrapping. A hand-edited `config.yaml` value that
/// is not on the scale snaps to the first step rather than being preserved —
/// the row is a stepper, and silently keeping an off-scale value would make
/// the next click look like it did nothing.
fn next_step(steps: &[f32], current: f32) -> f32 {
    match steps
        .iter()
        .position(|s| (*s - current).abs() < f32::EPSILON)
    {
        Some(i) => steps[(i + 1) % steps.len()],
        None => steps[0],
    }
}

fn spawn_extra_row(list: &mut ChildSpawnerCommands, font: &Handle<Font>, row: ExtraRow) {
    let mut entity = list.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(ROW_H),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(Val::Px(4.0)),
            ..default()
        },
        Button,
        Hovered::default(),
        Pickable::default(),
    ));
    entity.observe(
        move |_activate: On<Activate>, mut config: ResMut<ClientConfig>| {
            row.cycle(&mut config);
        },
    );
    entity.with_children(|r| {
        r.spawn((
            Text::new(row.label()),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(OPENROAD_COLOR),
            Node {
                width: Val::Px(196.0),
                ..default()
            },
            Pickable::IGNORE,
        ));
        r.spawn((
            row,
            Text::new(String::new()),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(OPENROAD_COLOR),
            Pickable::IGNORE,
        ));
    });
}

/// Keeps the openroad rows showing the live `ClientConfig`.
///
/// Runs every frame like [`refresh_row_values`] rather than on
/// `resource_changed`: the rows are spawned with empty text (the pane builder
/// has no `ClientConfig`), so they need a fill on their first frame too, and a
/// handful of short string formats is not worth a second code path.
pub(crate) fn refresh_extra_rows(
    config: Res<ClientConfig>,
    mut rows: Query<(&ExtraRow, &mut Text)>,
) {
    for (row, mut text) in rows.iter_mut() {
        let wanted = row.value_text(&config);
        if text.0 != wanted {
            *text = Text::new(wanted);
        }
    }
}

fn spawn_detail_row(
    list: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    ui_strings: &ClientUiStrings,
    row: &DetailRow,
    options: &GameOptions,
) {
    let configured = row.backing == Backing::Config;
    let live = row.backing == Backing::Live || configured;
    let id = row.id;

    let mut entity = list.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(ROW_H),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(Val::Px(4.0)),
            ..default()
        },
        Pickable::IGNORE,
    ));
    if configured {
        entity.insert((Button, Hovered::default(), Pickable::default()));
        // one step up from what is in effect, saved in the bank;
        // `apply_config_rows` lays it over the config
        entity.observe(
            move |_activate: On<Activate>,
                  panes: Query<&VideoPane>,
                  config: Res<ClientConfig>,
                  mut options: ResMut<GameOptions>| {
                let Some(def) = config_row(id) else {
                    return;
                };
                let next = (config_row_step(id, &config) + 1) % def.steps.len();
                let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
                let bank = match profile {
                    GraphicProfileTab::One => &mut options.video.graphic1,
                    GraphicProfileTab::Two => &mut options.video.graphic2,
                };
                bank.quality.insert(id, next as u16);
            },
        );
    } else if live {
        entity.insert((Button, Hovered::default(), Pickable::default()));
        entity.observe(
            move |_activate: On<Activate>,
                  panes: Query<&VideoPane>,
                  mut options: ResMut<GameOptions>| {
                let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
                let bank = match profile {
                    GraphicProfileTab::One => &mut options.video.graphic1,
                    GraphicProfileTab::Two => &mut options.video.graphic2,
                };
                let current = bank.quality.get(&id).copied().unwrap_or(1);
                bank.quality.insert(id, u16::from(current == 0));
            },
        );
    }

    entity.with_children(|r| {
        r.spawn((
            Text::new(ui_strings.get_or(row.key, row.english).to_string()),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(if live { LABEL_COLOR } else { UNBACKED_COLOR }),
            Node {
                width: Val::Px(196.0),
                ..default()
            },
            Pickable::IGNORE,
        ));
        let mut value = r.spawn((
            Text::new(row_value_text(row, options, GraphicProfileTab::One)),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(if live { VALUE_COLOR } else { UNBACKED_COLOR }),
            Pickable::IGNORE,
        ));
        // config-backed rows show the config, not the bank
        if configured {
            value.insert(ConfigRowValue { id: row.id });
        } else {
            value.insert(RowValue { id: row.id });
        }
    });
}

/// What a row shows. Backed rows render their state; unbacked ones say so
/// instead of borrowing a value word from the original's vocabulary.
fn row_value_text(row: &DetailRow, options: &GameOptions, profile: GraphicProfileTab) -> String {
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    let value = bank.quality.get(&row.id).copied().unwrap_or_default();
    match row.backing {
        Backing::Live => if value == 0 { "Off" } else { "On" }.to_string(),
        // filled from the config by `refresh_config_rows`
        Backing::Config => String::new(),
    }
}

fn spawn_static_label(
    pane: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    text: &str,
    top: f32,
    color: Color,
) {
    pane.spawn((
        Text::new(text.to_string()),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(color),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(LABEL_X),
            top: Val::Px(top),
            width: Val::Px(LABEL_W),
            height: Val::Px(LABEL_H),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

fn spawn_value_text(
    pane: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    text: &str,
    top: f32,
    color: Color,
) {
    pane.spawn((
        Text::new(text.to_string()),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(color),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(VALUE_X),
            top: Val::Px(top),
            width: Val::Px(VALUE_W),
            height: Val::Px(VALUE_H),
            ..default()
        },
        Pickable::IGNORE,
    ));
}

/// Recolors the profile tabs and rewrites every row value when the selected
/// profile changes.
pub(crate) fn apply_profile_tab(
    panes: Query<&VideoPane, Changed<VideoPane>>,
    options: Res<GameOptions>,
    mut labels: Query<(&ProfileTabLabel, &mut TextColor)>,
    mut values: Query<(&RowValue, &mut Text), Without<ProfileTabLabel>>,
) {
    let Some(pane) = panes.iter().next() else {
        return;
    };
    for (label, mut color) in labels.iter_mut() {
        color.0 = if label.0 == pane.profile {
            ACTIVE_PROFILE_COLOR
        } else {
            VALUE_COLOR
        };
    }
    for (value, mut text) in values.iter_mut() {
        let Some(row) = DETAIL_ROWS.iter().find(|r| r.id == value.id) else {
            continue;
        };
        *text = Text::new(row_value_text(row, &options, pane.profile));
    }
}

/// Rewrites row values after an edit, so a click shows its effect.
pub(crate) fn refresh_row_values(
    options: Res<GameOptions>,
    panes: Query<&VideoPane>,
    mut values: Query<(&RowValue, &mut Text)>,
) {
    if !options.is_changed() {
        return;
    }
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    for (value, mut text) in values.iter_mut() {
        let Some(row) = DETAIL_ROWS.iter().find(|r| r.id == value.id) else {
            continue;
        };
        *text = Text::new(row_value_text(row, &options, profile));
    }
}

// The window itself is not applied from here any more. Both halves —
// `setup_window_settings` (the boot apply) and `apply_window_mode` (the
// options-driven one) — moved into `plugins::config::window`, which now
// resolves one `WindowIntent` from `config.yaml` plus the session-only
// `video.window_mode_override` and is the single writer. Two modules each
// computing their own answer for `window.mode` is exactly what made
// `config.yaml`'s `mode:` a dead key: whichever ran last won, and that was
// this one.

/// Applies the Bloom quality row to the window cameras.
///
/// Mirrors the dev inspector's toggle (`plugins::dev::render_debug`): `Bloom`
/// is `#[require(Hdr)]`, and a required component is not removed with the one
/// that pulled it in, so `Hdr` has to go explicitly or "bloom off" still
/// renders to a float target. Gated on the config switch for the same reason
/// the dev path is: with bloom disabled in `config.yaml` there is no intensity
/// to apply.
pub(crate) fn apply_bloom_option(
    options: Res<GameOptions>,
    config: Res<ClientConfig>,
    panes: Query<&VideoPane>,
    cameras: Query<(Entity, &RenderTarget), With<Camera3d>>,
    ui_cameras: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !options.is_changed() || !config.graphics.bloom.enabled {
        return;
    }
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    let on = bank.quality.get(&BLOOM_ID).copied().unwrap_or(1) != 0;

    for (entity, target) in cameras.iter() {
        if !matches!(target, RenderTarget::Window(_)) {
            continue;
        }
        if on {
            commands
                .entity(entity)
                .insert(config.graphics.bloom.to_bloom());
        } else {
            commands.entity(entity).remove::<(Bloom, Hdr)>();
        }
    }
    // The 2d UI camera shares the window's main texture, so its format has to
    // follow the 3d cameras' or the 3d view disappears (`camera::setup_ui_camera`).
    for entity in ui_cameras.iter() {
        if on {
            commands.entity(entity).insert(Hdr);
        } else {
            commands.entity(entity).remove::<Hdr>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_rows_cover_the_named_sroptionset_ids() {
        // Ids 1..=13 are the named Graphic 1 quality slots; 14/15 exist in the
        // id space but their CSV name cells are blank, so they are left out
        // rather than captioned by guess.
        let ids: Vec<u16> = DETAIL_ROWS.iter().map(|r| r.id).collect();
        assert_eq!(ids, (1..=13).collect::<Vec<u16>>());
    }

    #[test]
    fn every_row_shows_a_real_effect() {
        // Each row is backed by a feature and renders its state; none may
        // render a value word that the client does not act on.
        let options = GameOptions::default();
        for row in &DETAIL_ROWS {
            let text = row_value_text(row, &options, GraphicProfileTab::One);
            match row.backing {
                Backing::Live => assert!(
                    text == "On" || text == "Off",
                    "{} is wired and must show its state, got {text:?}",
                    row.english
                ),
                // shown from the config by `refresh_config_rows`
                Backing::Config => assert!(
                    config_row(row.id).is_some(),
                    "{} is config-backed but maps to no key",
                    row.english
                ),
            }
        }
    }

    #[test]
    fn the_config_backed_rows_are_exactly_these() {
        let ids: Vec<u16> = DETAIL_ROWS
            .iter()
            .filter(|r| r.backing == Backing::Config)
            .map(|r| r.id)
            .collect();
        // every row but Bloom (11), which toggles its cameras directly
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13]);
    }

    /// Every step of every config-backed row reads back as itself, so a
    /// saved step is what the row then shows and `apply_config_rows` does
    /// not rewrite the config forever.
    #[test]
    fn every_config_row_step_round_trips() {
        for row in DETAIL_ROWS.iter().filter(|r| r.backing == Backing::Config) {
            let def = config_row(row.id).unwrap();
            let mut config = test_config();
            for step in 0..def.steps.len() {
                set_config_row_step(row.id, step, &mut config);
                assert_eq!(
                    config_row_step(row.id, &config),
                    step,
                    "{} step {step}",
                    row.english
                );
            }
        }
    }

    /// Background Sight Range keeps the fog inside the view distance, in the
    /// default band's proportions.
    #[test]
    fn the_sight_range_row_scales_the_fog_band() {
        let mut config = test_config();
        set_config_row_step(2, 1, &mut config);
        let view = &config.graphics.view;
        assert_eq!(view.view_distance, 2880.0);
        assert_eq!(view.fog_end, 2880.0);
        assert_eq!(view.fog_start, 1920.0);
    }

    #[test]
    fn bloom_is_the_only_live_row() {
        // If a later change wires another feature, this test should be updated
        // deliberately — it is the guard against quietly marking rows Live.
        let live: Vec<u16> = DETAIL_ROWS
            .iter()
            .filter(|r| r.backing == Backing::Live)
            .map(|r| r.id)
            .collect();
        assert_eq!(live, vec![BLOOM_ID]);
    }

    #[test]
    fn row_value_follows_the_selected_profile() {
        let mut options = GameOptions::default();
        options.video.graphic1.quality.insert(BLOOM_ID, 1);
        options.video.graphic2.quality.insert(BLOOM_ID, 0);
        let bloom = DETAIL_ROWS.iter().find(|r| r.id == BLOOM_ID).unwrap();
        assert_eq!(
            row_value_text(bloom, &options, GraphicProfileTab::One),
            "On"
        );
        assert_eq!(
            row_value_text(bloom, &options, GraphicProfileTab::Two),
            "Off"
        );
    }

    /// The grass rows are openroad additions, and #366 is the reason: the
    /// user's own PK2 has no grass/vegetation/foliage control in
    /// `ifoption_video.txt`, `ifvideooptionslot.txt` or `textuisystem`. A row
    /// that borrowed an original caption would be claiming fidelity we do not
    /// have, so every extra row has to say "openroad" in its label.
    #[test]
    fn every_extra_row_declares_itself_an_openroad_addition() {
        for row in EXTRA_ROWS {
            assert!(
                row.label().contains("openroad"),
                "{row:?} does not mark itself as an addition: {:?}",
                row.label()
            );
        }
        // ...and none of them borrows an original row's caption.
        for row in EXTRA_ROWS {
            assert!(
                !DETAIL_ROWS.iter().any(|d| d.english == row.label()),
                "{row:?} reuses an original caption"
            );
        }
    }

    /// The steppers must return to where they started, or a user who clicks
    /// past the value they wanted can never get back to it.
    #[test]
    fn every_extra_row_cycles_back_to_its_starting_value() {
        for row in EXTRA_ROWS {
            let mut config = test_config();
            let start = row.value_text(&config);
            let steps = match row {
                ExtraRow::FoliageMode => 4,
                ExtraRow::FoliageDensity => DENSITY_STEPS.len(),
                ExtraRow::FoliageSightRange => SIGHT_RANGE_STEPS.len(),
                // read-only: one value, which a click keeps
                ExtraRow::Preset => 1,
            };
            let mut seen = vec![start.clone()];
            for _ in 0..steps {
                row.cycle(&mut config);
                seen.push(row.value_text(&config));
            }
            assert_eq!(seen.first(), seen.last(), "{row:?} does not wrap: {seen:?}");
            assert_eq!(
                seen[..steps]
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                steps,
                "{row:?} repeats a value inside one lap: {seen:?}"
            );
        }
    }

    /// `off` is the faithful v1.188 setting (#366, #646): the default must
    /// stay off, and the mode row must start there rather than at a mode that
    /// renders grass the original never had.
    #[test]
    fn the_shipped_default_is_the_vanilla_off_mode() {
        let config = test_config();
        assert_eq!(config.graphics.foliage.mode, FoliageMode::Off);
        assert_eq!(
            ExtraRow::FoliageMode.value_text(&config),
            "Off (vanilla)",
            "the off state has to name itself as the original behaviour"
        );
    }

    /// A hand-edited `config.yaml` value that is not on the stepper's scale
    /// must snap onto it, or the first click looks like it did nothing.
    #[test]
    fn an_off_scale_value_snaps_onto_the_first_step() {
        assert_eq!(next_step(&DENSITY_STEPS, 0.37), DENSITY_STEPS[0]);
        assert_eq!(next_step(&SIGHT_RANGE_STEPS, 1234.0), SIGHT_RANGE_STEPS[0]);
        // and an on-scale value advances rather than snapping
        assert_eq!(next_step(&DENSITY_STEPS, 1.0), 1.5);
    }

    /// `config.example.yaml` through the loader `main()` uses — the same file a
    /// fresh setup copies (#539) — so the rows are exercised against the real
    /// shipped defaults rather than against a struct literal.
    fn test_config() -> ClientConfig {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../config.example.yaml")
            .with_extension("");
        ClientConfig::from_file(path.to_str().expect("the example path is utf-8"))
            .expect("config.example.yaml matches ClientConfig")
    }
}

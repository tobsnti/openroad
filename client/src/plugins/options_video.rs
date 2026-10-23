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
use bevy::text::LineHeight;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::Monitor;

use crate::assets::resinfo::interface_text::font_index_px;
use crate::plugins::config::graphics::{FoliageMode, RenderMode, RenderScale, WaterQuality};
use crate::plugins::config::preset::QualityPreset;
use crate::plugins::config::ClientConfig;
use crate::plugins::settings::options::{GameOptions, GraphicProfile};
use crate::plugins::settings::tooltip::{attach_tooltip, spawn_tooltip_line};
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

/// Every text in the pane takes its size from the tree: all eight blocks of the
/// live `APPLY_UI_4TH` branch carry `FontIndex=0` — the two profile tabs
/// `GDR_OPT_VIDEO_ST_CTRL_1`/`_2` (`ifoption_video.txt:31,12`), the two row
/// labels `GDR_OPT_VIDEO_ST_SS`/`_ST_BR` (`:107,69`), their value controls
/// `GDR_OPT_VIDEO_CB_SS`/`_SP_BR` (`:88,50`) and the detail list
/// `GDR_OPT_VIDEO_DETAIL_OPT` (`:145`) — and slot 0 of the ladder is 12 px
/// ([`font_index_px`]). The file agrees with itself: [`LABEL_H`] is 12, exactly
/// one 12 px line. The 11 px this replaced appears nowhere in the data.
const TEXT_FONT_INDEX: usize = 0;

const BOARD_DDJ: &str = "media://interface/option/opt_video_control_03.ddj";
const TAB_DDJ: &str = "media://interface/option/opt_video_tab_back.ddj";
const FONT: &str = crate::assets::BUNDLED_FALLBACK_FACE;

const LABEL_COLOR: Color = Color::srgb(1.0, 0.965, 0.827);
const VALUE_COLOR: Color = Color::WHITE;
/// Rows whose feature does not exist yet render dimmed, so the pane never
/// pretends a control does something.
const UNBACKED_COLOR: Color = Color::srgb(0.45, 0.45, 0.45);
// see `options_input_tab::CAPTURING_COLOR` — authored gold, not 1/PI
#[allow(clippy::approx_constant)]
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
    /// What has to be true before the row can do anything at all.
    requires: Requirement,
}

/// A condition outside the row's own key that decides whether it can have any
/// effect.
///
/// The pane already refuses to render a control with no feature behind it
/// ("no fake controls"). This is the same rule one level down: a row can be
/// fully wired and still be inert because the client is in a mode, or on a
/// machine, that cannot honour it. Saying so in the row is the difference
/// between a setting and a lie — and keeping it as *data* beside the steps is
/// what stops the answer from being buried in a value string per row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Requirement {
    /// Nothing outside the key refuses it.
    Always,
    /// Shadow maps are drawn in PBR lighting only: `map::setup_lighting` sets
    /// `shadow_maps_enabled: is_pbr && shadows.enabled` (`map/mod.rs:342`),
    /// and `render_mode: vanilla` — the faithful default — is the mode that
    /// draws none.
    PbrLighting,
    /// Only the high water tier reflects anything: the low tier's material
    /// drops the whole reflection term (`map::water_material`), so there is
    /// nothing for the row to switch there.
    HighWater,
}

impl Requirement {
    /// Whether this config, on this machine, can honour the row.
    fn met(self, config: &ClientConfig) -> bool {
        match self {
            Requirement::Always => true,
            Requirement::PbrLighting => config.graphics.render_mode == RenderMode::Pbr,
            Requirement::HighWater => config.graphics.water.quality == WaterQuality::High,
        }
    }

    /// What the row says instead of a plain step while the condition is
    /// unmet. Names the condition, not the row: the player has to know *what*
    /// to change.
    fn unmet_note(self) -> Option<&'static str> {
        match self {
            Requirement::Always => None,
            Requirement::PbrLighting => Some("no effect in vanilla lighting"),
            Requirement::HighWater => Some("no effect on low water"),
        }
    }
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
    let row = |steps, restart| {
        Some(ConfigRow {
            steps,
            restart,
            requires: Requirement::Always,
        })
    };
    let row_requiring = |steps, restart, requires| {
        Some(ConfigRow {
            steps,
            restart,
            requires,
        })
    };
    let off_on: &'static [&'static str] = &["Off", "On"];
    match id {
        // no restart note: `environment::options::apply_shadow_detail_option`
        // follows the key into the live Sun
        1 => row_requiring(off_on, None, Requirement::PbrLighting),
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
        // no restart note: `map::water_options::apply_water_reflection_option`
        // carries the key into the live material
        4 => row_requiring(off_on, None, Requirement::HighWater),
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
    use crate::plugins::config::graphics::{EffectQuality, TextureDetail};
    let graphics = &config.graphics;
    match id {
        1 => usize::from(graphics.shadows.enabled),
        2 => nearest_step(&SIGHT_STEPS, graphics.view.view_distance),
        3 => match graphics.view.character_distance {
            // 0 (unlimited) is its own step, not the nearest distance
            range if range <= 0.0 => CHARACTER_SIGHT_STEPS.len() - 1,
            range => nearest_step(&CHARACTER_SIGHT_STEPS[..3], range),
        },
        4 => usize::from(graphics.water.reflection),
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
    use crate::plugins::config::graphics::{Anisotropy, EffectQuality, TextureDetail};
    let graphics = &mut config.graphics;
    let pick = |steps_len: usize| step.min(steps_len - 1);
    match id {
        1 => graphics.shadows.enabled = step != 0,
        3 => {
            graphics.view.character_distance =
                CHARACTER_SIGHT_STEPS[pick(CHARACTER_SIGHT_STEPS.len())]
        }
        4 => graphics.water.reflection = step != 0,
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
    mut rows: Query<(&ConfigRowValue, &mut Text, &mut TextColor)>,
) {
    for (row, mut text, mut color) in rows.iter_mut() {
        let Some(wanted) = config_row_text(row.id, &config) else {
            continue;
        };
        if text.0 != wanted {
            *text = Text::new(wanted);
        }
        // Dimmed while the condition is unmet, for the same reason the pane
        // dims a control with no feature: the state is real, its effect is not.
        let unmet = config_row(row.id).is_some_and(|def| !def.requires.met(&config));
        let wanted = if unmet { UNBACKED_COLOR } else { VALUE_COLOR };
        if color.0 != wanted {
            color.0 = wanted;
        }
    }
}

/// What a config-backed row shows: the step in effect, plus the one note that
/// applies — the condition in the way if there is one, the restart it needs
/// otherwise. An unmet condition outranks the restart note: a change that has
/// no effect in this mode will not gain one by relaunching.
fn config_row_text(id: u16, config: &ClientConfig) -> Option<String> {
    let def = config_row(id)?;
    let step = def.steps[config_row_step(id, config).min(def.steps.len() - 1)];
    let note = if def.requires.met(config) {
        def.restart
    } else {
        def.requires.unmet_note()
    };
    Some(match note {
        Some(note) => format!("{step} ({note})"),
        None => step.to_string(),
    })
}

/// The 13 named quality rows. Ids and names are `[S]` from the SROptionSet
/// table (`docs/formats/sroptionset.md:62-99`); the label keys are `[V]` from
/// the resinfo tree. Ids 14/15 exist in the id space but their CSV name cells
/// are blank, so they are left out rather than captioned by guess.
const DETAIL_ROWS: [DetailRow; 13] = [
    // Steps `graphics.shadows.enabled`, which
    // `environment::options::apply_shadow_detail_option` follows into the
    // Sun's cascaded shadow maps while the game runs. PBR mode only: vanilla
    // has no dynamic shadows, which is what `Requirement::PbrLighting` says.
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
    // Steps `graphics.water.reflection`, which
    // `map::water_options::apply_water_reflection_option` carries into the
    // high tier's reflection blend while the game runs. High tier only, which
    // is what `Requirement::HighWater` says.
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

/// The original's hover help for the quality rows: `UIIT_STT_VIDIO_TTDESC_*`
/// (textuisystem :968-984, **17** strings). The key block is spelled
/// `VIDIO`, not `VIDEO` — a typo in the original's own data, and the reason a
/// search for `VIDEO_TTDESC` comes back empty.
///
/// The mapping is by **what the English string names**, not by the block's
/// position, because the block is not in id order: `_11` says "Texture detail",
/// which is id 9, so a positional read would shift every later row by one.
///
/// `_15` is the one string that names no row, and it lands on id 8 by
/// elimination rather than by wording: `_04`..`_16` is exactly thirteen
/// strings for exactly thirteen rows, twelve of them name their row outright,
/// and the single row left over is id 8 `UIIT_STT_FILTERING`. Its subject —
/// compensating rough outlines — is what texture filtering does.
///
/// Four of the seventeen strings are deliberately absent here:
/// * `_01`/`_02` belong to the resolution and brightness controls, not to a row
///   (wired at their own call sites below).
/// * `_03` describes the graphic-quality **preset**, which exists only in the
///   `#else` (classic) branch of `ifoption_video.txt`; this pane transcribes
///   the 4th-gen branch, which has the two profile tabs instead.
/// * `_17` (large-scale combat outfit unification) belongs to ids 14/15, whose
///   name cells are blank and which this pane therefore does not render.
const ROW_TOOLTIPS: [(u16, &str, &str); 13] = [
    (
        1,
        "UIIT_STT_VIDIO_TTDESC_04",
        "Able to control the degree of shadow details.",
    ),
    (
        2,
        "UIIT_STT_VIDIO_TTDESC_05",
        "As the number of the game background increases, farther background can be seen.",
    ),
    (
        3,
        "UIIT_STT_VIDIO_TTDESC_06",
        "As the number of the character vision increases, farther characters, monsters can be seen.",
    ),
    (
        4,
        "UIIT_STT_VIDIO_TTDESC_07",
        "Water Reflection displays object reflections on water surface.",
    ),
    (
        5,
        "UIIT_STT_VIDIO_TTDESC_08",
        "Water Detail is the degree of water effect.",
    ),
    (
        6,
        "UIIT_STT_VIDIO_TTDESC_09",
        "Metallic Sheen is the display of metallic effects on metallic materials such as armors and weapons.",
    ),
    (
        7,
        "UIIT_STT_VIDIO_TTDESC_10",
        "Light Effect is indicating an effect of light when(Sun/Torch) is shown.",
    ),
    (
        8,
        "UIIT_STT_VIDIO_TTDESC_15",
        "Compensates rough outlines to produce more natural-looking objects.",
    ),
    (
        9,
        "UIIT_STT_VIDIO_TTDESC_11",
        "Texture detail is the degree of texture detail.",
    ),
    (
        10,
        "UIIT_STT_VIDIO_TTDESC_12",
        "Lens Flare is expressing shining condition like a camera looking towards the sun.",
    ),
    (
        11,
        "UIIT_STT_VIDIO_TTDESC_13",
        "Bloom Effect is known as blur effect which has a condition of mixing colors moderately so it looks blurry",
    ),
    (
        12,
        "UIIT_STT_VIDIO_TTDESC_14",
        "Dynamic Animation is a technique of expressing better animation of fabrics, texture, and hair.",
    ),
    (
        13,
        "UIIT_STT_VIDIO_TTDESC_16",
        "Toggles skill effects to improve performance.",
    ),
];

/// The resolved help text of a quality row, or `None` for the two rows the
/// original's own tooltip block does not describe (see [`ROW_TOOLTIPS`]).
fn row_tooltip(ui_strings: &ClientUiStrings, id: u16) -> Option<String> {
    ROW_TOOLTIPS
        .iter()
        .find(|(row_id, _, _)| *row_id == id)
        .map(|(_, key, english)| ui_strings.get_or(key, english).to_string())
}

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
    spawn_resolution_control(pane, &font, ui_strings, profile);

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
    let mut brightness = spawn_value_text(
        pane,
        &font,
        &format!("{} (not applied)", profile.brightness),
        BRIGHTNESS_VALUE_Y,
        UNBACKED_COLOR,
    );
    // `UIIT_STT_VIDIO_TTDESC_02` (textuisystem :969). The control is inert, so
    // the help text is the only thing it can honestly offer.
    attach_tooltip(
        &mut brightness,
        ui_strings.get_or(
            "UIIT_STT_VIDIO_TTDESC_02",
            "Able to control the game's brightness.",
        ),
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
                    font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
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

    // The pane's hover-help footer (see `settings::tooltip`), spanning the
    // detail list's width so it lines up with the rows it describes.
    spawn_tooltip_line(pane, &font, LIST_X, LIST_W);
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
    /// `graphics.render_scale` — the 3D view's resolution as a share of the
    /// window, the largest single lever in the frame (the cost tracks pixels,
    /// see `config::graphics::RenderScale`). Live.
    ///
    /// Above 1.0 it is supersampling, which is why this row is also the
    /// answer to "can I have DLSS": it needs no vendor and no SDK, and it
    /// works on every backend we run on.
    RenderScale,
    /// The graphics preset (`graphics.preset`): the one row that moves every
    /// other value at once, which is what a weak machine needs first.
    ///
    /// It writes `GameOptions`, not `ClientConfig`, and takes effect on the
    /// next start — a preset is merged *under* `config.yaml` while the file is
    /// parsed (`config::ClientConfig::from_file_with_preset`), and the values
    /// it decides include ones nothing re-reads later. Applying the tier live
    /// would mean a second precedence path beside `apply_config_rows`, so the
    /// row states the restart instead, exactly as the config-backed rows do.
    Preset,
}

const EXTRA_ROWS: [ExtraRow; 5] = [
    ExtraRow::Preset,
    ExtraRow::RenderScale,
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
/// Preset steps. `None` — "follow `config.yaml`" — is a step of its own, and
/// deliberately the first: without it a single click would take the choice
/// away from the file for good, and there would be no way back to it from the
/// window. The rest follow the order `config.example.yaml` documents.
/// Render-scale steps. 1.0 is the native default and sits in the middle: the
/// two below it buy frames, the two above spend them on supersampling.
const RENDER_SCALE_STEPS: [f32; 5] = [0.5, 0.75, 1.0, 1.5, 2.0];
const PRESET_STEPS: [Option<QualityPreset>; 6] = [
    None,
    Some(QualityPreset::Auto),
    Some(QualityPreset::Low),
    Some(QualityPreset::Medium),
    Some(QualityPreset::High),
    Some(QualityPreset::Ultra),
];
/// Openroad additions are drawn in their own colour so a screenshot never
/// suggests the original had this control.
const OPENROAD_COLOR: Color = Color::srgb(0.63, 0.84, 1.0);
/// Marks a row whose change applies instantly but does **not** survive a
/// restart — see [`ExtraRow::value_text`].
const SESSION_ONLY: &str = "(session only)";

impl ExtraRow {
    fn label(self) -> &'static str {
        match self {
            ExtraRow::FoliageMode => "Grass (openroad)",
            ExtraRow::FoliageDensity => "Grass Density (openroad)",
            ExtraRow::FoliageSightRange => "Grass Sight Range (openroad)",
            ExtraRow::Preset => "Graphics Preset (openroad)",
            ExtraRow::RenderScale => "Render Scale (openroad)",
        }
    }

    /// What the row shows, with the [`SESSION_ONLY`] suffix where the write
    /// does not survive a restart.
    ///
    /// The suffix is not decoration. A row that writes `ClientConfig` is
    /// lost on the next launch: `config.yaml` is an **author-owned,
    /// read-only input** (`settings/persistence.rs:1-6`), the player's own
    /// store is `user_settings.yaml`, and `ClientConfig` never reaches it.
    /// Saying so in the value is the same honesty idiom this pane already
    /// uses for its restart notes; mirroring these fields into
    /// [`GameOptions`] is the real fix and is a bigger change than these rows
    /// deserve. The preset row is exempt because it writes `GameOptions` and
    /// already names its own timing.
    fn value_text(self, config: &ClientConfig, options: &GameOptions) -> String {
        let text = self.raw_value_text(config, options);
        if self.session_only() {
            format!("{text} {SESSION_ONLY}")
        } else {
            text
        }
    }

    /// Whether the row's write is gone on the next launch — true for every
    /// row that writes `ClientConfig`, which is all of them but the preset.
    fn session_only(self) -> bool {
        !matches!(self, ExtraRow::Preset)
    }

    fn raw_value_text(self, config: &ClientConfig, options: &GameOptions) -> String {
        let foliage = &config.graphics.foliage;
        match self {
            ExtraRow::FoliageMode => match foliage.mode {
                FoliageMode::Off => "Off (vanilla)".to_string(),
                FoliageMode::Native => "Authored".to_string(),
                FoliageMode::Pack => "Bundled".to_string(),
                FoliageMode::Both => "Authored + Bundled".to_string(),
            },
            ExtraRow::FoliageDensity => format!("{:.1}x", foliage.density),
            // What is in effect, not what the file says: a hand-edited 3.0
            // clamps, and the row has to show where it landed.
            ExtraRow::RenderScale => {
                let factor = config.graphics.render_scale.factor();
                let percent = (factor * 100.0).round();
                if factor > 1.0 {
                    format!("{percent:.0}% (supersampled)")
                } else if factor < 1.0 {
                    format!("{percent:.0}% (upscaled)")
                } else {
                    "100% (native)".to_string()
                }
            }
            ExtraRow::FoliageSightRange => {
                if foliage.view_distance > 0.0 {
                    format!("{:.0}", foliage.view_distance)
                } else {
                    "Unlimited".to_string()
                }
            }
            ExtraRow::Preset => {
                let Some(resolved) = &config.graphics.resolved_preset else {
                    return "none".to_string();
                };
                let running = match resolved.requested {
                    // `auto` names the tier it picked, or the row would read
                    // "auto" on every machine and say nothing.
                    QualityPreset::Auto => format!("auto ({})", resolved.tier.name()),
                    tier => tier.name().to_string(),
                };
                match options.video.preset_override {
                    None => format!("{running}; config.yaml"),
                    Some(saved) if saved == resolved.requested => format!("{running}; yours"),
                    Some(saved) => format!("{} (next start; now {running})", saved.name()),
                }
            }
        }
    }

    /// Advances the row one step. The foliage rows write `ClientConfig` (they
    /// are live); the preset row writes `GameOptions`, because it is read
    /// before the next `ClientConfig` exists.
    fn cycle(self, config: &mut ClientConfig, options: &mut GameOptions) {
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
            ExtraRow::RenderScale => {
                let graphics = &mut config.graphics;
                graphics.render_scale =
                    RenderScale(next_step(&RENDER_SCALE_STEPS, graphics.render_scale.0))
            }
            ExtraRow::Preset => {
                let at = PRESET_STEPS
                    .iter()
                    .position(|step| *step == options.video.preset_override)
                    .unwrap_or(0);
                options.video.preset_override = PRESET_STEPS[(at + 1) % PRESET_STEPS.len()];
            }
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

/// openroad's own rows, so they carry no block of their own — but they are rows
/// *of the original's list*, so they take that list's `FontIndex` like the
/// thirteen vanilla rows above them ([`TEXT_FONT_INDEX`]). Their colour and
/// their "(openroad)" caption are what mark them as ours, not a second size.
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
        move |_activate: On<Activate>,
              mut config: ResMut<ClientConfig>,
              mut options: ResMut<GameOptions>| {
            row.cycle(&mut config, &mut options);
        },
    );
    entity.with_children(|r| {
        r.spawn((
            Text::new(row.label()),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
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
                font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
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
    options: Res<GameOptions>,
    mut rows: Query<(&ExtraRow, &mut Text)>,
) {
    for (row, mut text) in rows.iter_mut() {
        let wanted = row.value_text(&config, &options);
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
    // Hover help, `UIIT_STT_VIDIO_TTDESC_*` (see `ROW_TOOLTIPS`). Attached to
    // inert rows too: hover is not a click, and "what would this do" is the
    // one thing a row without a feature can still answer honestly.
    if let Some(help) = row_tooltip(ui_strings, row.id) {
        attach_tooltip(&mut entity, help);
    }

    entity.with_children(|r| {
        r.spawn((
            Text::new(ui_strings.get_or(row.key, row.english).to_string()),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
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
                font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
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
            font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
            ..default()
        },
        // `156x12` (`LABEL_H`) is one line, so the line box is the font size —
        // see `font_index_px`'s "One-line boxes". The value column next door
        // keeps 1.2: its `VALUE_H` is 20, a line in a taller box.
        LineHeight::Px(font_index_px(TEXT_FONT_INDEX)),
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

/// Returns the spawned entity so the caller can attach hover help
/// ([`attach_tooltip`]) without this helper having to know about text keys.
fn spawn_value_text<'a>(
    pane: &'a mut ChildSpawnerCommands,
    font: &Handle<Font>,
    text: &str,
    top: f32,
    color: Color,
) -> EntityCommands<'a> {
    pane.spawn((
        Text::new(text.to_string()),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(font_index_px(TEXT_FONT_INDEX)),
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
    ))
}

/// The resolution control (`GDR_OPT_VIDEO_CB_SS`, `CIFComboBox` id 11 —
/// `ifoption_video.txt:82` in the 4th-gen branch, `:257` in the classic one)
/// It is a **control in both generations**, so a plain readout would be drift,
/// not a simplification.
///
/// Two deliberate deviations, per ADR 0009:
///
/// * **No drop-down.** Clicking steps to the next mode, the same idiom every
///   other value row in this pane uses. A real popup list needs its own
///   z-order and dismissal rules inside a window whose stacking cannot be
///   checked without a live client.
/// * **The value list is the monitor's, not the original's.** v1.188 carries
///   no resolution list anywhere in `resinfo/` (the tree only declares the
///   control), and `silkcfg.dat` holds a single pair (800x600). So the modes
///   come from `bevy::window::Monitor::video_modes` — real device values,
///   which is the opposite of inventing a table.
fn spawn_resolution_control(
    pane: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    ui_strings: &ClientUiStrings,
    profile: &GraphicProfile,
) {
    let mut value = spawn_value_text(
        pane,
        font,
        &resolution_text(profile.width, profile.height),
        RESOLUTION_VALUE_Y,
        VALUE_COLOR,
    );
    value.insert((ResolutionValue, Button, Hovered::default()));
    value.observe(
        move |_activate: On<Activate>,
              panes: Query<&VideoPane>,
              monitors: Query<&Monitor>,
              mut options: ResMut<GameOptions>| {
            let modes = available_resolutions(monitors.iter());
            let selected = panes.iter().next().map(|p| p.profile).unwrap_or_default();
            let bank = match selected {
                GraphicProfileTab::One => &mut options.video.graphic1,
                GraphicProfileTab::Two => &mut options.video.graphic2,
            };
            match next_resolution(&modes, (bank.width, bank.height)) {
                Some((width, height)) => {
                    bank.width = width;
                    bank.height = height;
                }
                // Nothing to offer: keep the stored value rather than snap to
                // something the player did not pick.
                None => warn!(
                    "options: no monitor reported a video mode, resolution stays {}x{}",
                    bank.width, bank.height
                ),
            }
        },
    );
    // `UIIT_STT_VIDIO_TTDESC_01` (textuisystem :968).
    attach_tooltip(
        &mut value,
        ui_strings.get_or(
            "UIIT_STT_VIDIO_TTDESC_01",
            "Able to control the game's resolution.",
        ),
    );
}

/// Marks the resolution value text, so [`refresh_row_values`] can rewrite it.
#[derive(Component)]
pub(crate) struct ResolutionValue;

fn resolution_text(width: u32, height: u32) -> String {
    format!("{width} x {height}")
}

/// Every distinct resolution the attached monitors report, ascending.
///
/// The monitor's own `physical_size` is included because a platform may report
/// an empty `video_modes` list (winit does on some Wayland setups) — without it
/// the control would have nothing to step through on exactly those machines.
fn available_resolutions<'a>(monitors: impl Iterator<Item = &'a Monitor>) -> Vec<(u32, u32)> {
    let mut modes: Vec<(u32, u32)> = monitors
        .flat_map(|monitor| {
            let own = monitor.physical_size();
            monitor
                .video_modes
                .iter()
                .map(|mode| (mode.physical_size.x, mode.physical_size.y))
                .chain(std::iter::once((own.x, own.y)))
                .collect::<Vec<_>>()
        })
        .filter(|(w, h)| *w > 0 && *h > 0)
        .collect();
    modes.sort_unstable();
    modes.dedup();
    modes
}

/// The next mode after `current`, wrapping. A stored value that is not on the
/// list (a hand-edited `user_settings.yaml`, or a monitor that was swapped)
/// lands on the smallest available mode instead of being kept, so the first
/// click visibly does something.
fn next_resolution(modes: &[(u32, u32)], current: (u32, u32)) -> Option<(u32, u32)> {
    if modes.is_empty() {
        return None;
    }
    match modes.iter().position(|mode| *mode == current) {
        Some(index) => Some(modes[(index + 1) % modes.len()]),
        None => Some(modes[0]),
    }
}

/// Recolors the profile tabs and rewrites every row value when the selected
/// profile changes.
pub(crate) fn apply_profile_tab(
    panes: Query<&VideoPane, Changed<VideoPane>>,
    options: Res<GameOptions>,
    mut labels: Query<(&ProfileTabLabel, &mut TextColor)>,
    // `Without<ResolutionValue>` is not cosmetic: the resolution readout is a
    // `RowValue` too, so without it the two `&mut Text` queries overlap and
    // Bevy's access check panics the whole schedule at startup (B0001).
    // `refresh_row_values` below is filtered the same way.
    mut values: Query<(&RowValue, &mut Text), (Without<ProfileTabLabel>, Without<ResolutionValue>)>,
    mut resolution: Query<&mut Text, With<ResolutionValue>>,
) {
    let Some(pane) = panes.iter().next() else {
        return;
    };
    // The two banks keep their own resolution (ids 503/504 vs 603/604), so the
    // readout follows the tab like every quality row does.
    let bank = match pane.profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    for mut text in resolution.iter_mut() {
        *text = Text::new(resolution_text(bank.width, bank.height));
    }
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
    mut values: Query<(&RowValue, &mut Text), Without<ResolutionValue>>,
    mut resolution: Query<&mut Text, With<ResolutionValue>>,
) {
    if !options.is_changed() {
        return;
    }
    let profile = panes.iter().next().map(|p| p.profile).unwrap_or_default();
    let bank = match profile {
        GraphicProfileTab::One => &options.video.graphic1,
        GraphicProfileTab::Two => &options.video.graphic2,
    };
    let wanted = resolution_text(bank.width, bank.height);
    for mut text in resolution.iter_mut() {
        if text.0 != wanted {
            *text = Text::new(wanted.clone());
        }
    }
    for (value, mut text) in values.iter_mut() {
        let Some(row) = DETAIL_ROWS.iter().find(|r| r.id == value.id) else {
            continue;
        };
        *text = Text::new(row_value_text(row, &options, profile));
    }
}

// The window itself is applied by `plugins::config::window`, which resolves one
// `WindowIntent` from `config.yaml` plus the session-only
// `video.window_mode_override` and is the single writer for `window.mode`.
// The resolution control (`CIFComboBox` id 11, `spawn_resolution_control`)
// still writes ids 503/504 of the selected bank; it reaches the window through
// the same intent, as `graphic1.chosen_size()` — see `config::window`, which
// documents why only bank 1 drives the window and why "chosen" cannot simply
// be the stored pair.

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

    /// The resolution is a *control*. The pure stepper must
    /// visit every mode the monitor offers and come back — a combo box that
    /// cannot reach a value again is worse than a readout.
    #[test]
    fn the_resolution_stepper_wraps_through_every_mode() {
        let modes = [(800, 600), (1280, 720), (1920, 1080)];
        assert_eq!(next_resolution(&modes, (800, 600)), Some((1280, 720)));
        assert_eq!(next_resolution(&modes, (1280, 720)), Some((1920, 1080)));
        assert_eq!(next_resolution(&modes, (1920, 1080)), Some((800, 600)));
    }

    /// A stored value the monitor does not offer (hand-edited settings, or a
    /// swapped screen) lands on the smallest mode instead of being kept, or the
    /// first click would look like it did nothing.
    #[test]
    fn an_unavailable_stored_resolution_snaps_onto_the_list() {
        let modes = [(1280, 720), (1920, 1080)];
        assert_eq!(next_resolution(&modes, (1024, 768)), Some((1280, 720)));
    }

    /// No modes reported: the control must return nothing so its caller keeps
    /// the player's value and logs, rather than substituting one.
    #[test]
    fn no_modes_means_no_change() {
        assert_eq!(next_resolution(&[], (1920, 1080)), None);
    }

    /// The monitor's own size is part of the list, and duplicates collapse: a
    /// platform that reports an empty `video_modes` must still leave the
    /// control something to step through.
    #[test]
    fn the_mode_list_includes_the_monitor_itself_and_is_deduplicated() {
        let monitor = Monitor {
            name: None,
            physical_height: 1080,
            physical_width: 1920,
            physical_position: IVec2::ZERO,
            refresh_rate_millihertz: None,
            scale_factor: 1.0,
            video_modes: vec![
                bevy::window::VideoMode {
                    physical_size: UVec2::new(1920, 1080),
                    bit_depth: 32,
                    refresh_rate_millihertz: 60_000,
                },
                bevy::window::VideoMode {
                    physical_size: UVec2::new(1280, 720),
                    bit_depth: 32,
                    refresh_rate_millihertz: 60_000,
                },
            ],
        };
        assert_eq!(
            available_resolutions(std::iter::once(&monitor)),
            vec![(1280, 720), (1920, 1080)]
        );

        let modeless = Monitor {
            video_modes: Vec::new(),
            ..monitor
        };
        assert_eq!(
            available_resolutions(std::iter::once(&modeless)),
            vec![(1920, 1080)],
            "a monitor without modes still offers its own size"
        );
    }

    /// Every wired tooltip key has to be one textuisystem actually carries. The
    /// block is `UIIT_STT_VIDIO_TTDESC_01..17` (:968-984) — note `VIDIO`, the
    /// original's own typo — so a key outside that range never resolves.
    #[test]
    fn every_row_tooltip_key_exists_in_the_original_block() {
        for (_, key, _) in ROW_TOOLTIPS {
            let number = key
                .strip_prefix("UIIT_STT_VIDIO_TTDESC_")
                .unwrap_or_else(|| panic!("{key} is not from the VIDIO_TTDESC block"))
                .parse::<u8>()
                .expect("the suffix is a two-digit number");
            assert!((1..=17).contains(&number), "{key} is outside :968-984");
        }
    }

    /// No key twice, no row twice: a duplicate would mean one row shows another
    /// row's help, which is worse than no help.
    #[test]
    fn row_tooltips_are_one_to_one() {
        let keys: std::collections::BTreeSet<&str> =
            ROW_TOOLTIPS.iter().map(|(_, key, _)| *key).collect();
        assert_eq!(keys.len(), ROW_TOOLTIPS.len());
        let ids: std::collections::BTreeSet<u16> =
            ROW_TOOLTIPS.iter().map(|(id, _, _)| *id).collect();
        assert_eq!(ids.len(), ROW_TOOLTIPS.len());
        // Every tooltip belongs to a row this pane actually renders.
        for (id, _, _) in ROW_TOOLTIPS {
            assert!(
                DETAIL_ROWS.iter().any(|row| row.id == id),
                "tooltip for id {id}, which is not a rendered row"
            );
        }
    }

    /// Every rendered row has help. The block `_04`..`_16` holds exactly one
    /// string per row, so a row without one would mean a string was dropped.
    #[test]
    fn every_rendered_row_has_help() {
        let strings = ClientUiStrings::default();
        let missing: Vec<u16> = DETAIL_ROWS
            .iter()
            .filter(|row| row_tooltip(&strings, row.id).is_none())
            .map(|row| row.id)
            .collect();
        assert_eq!(missing, Vec::<u16>::new());
    }

    /// The elimination argument the `_15` mapping rests on, as a test: the
    /// help block covers every rendered row exactly once, so the one string
    /// that names no row belongs to the one row no string names.
    #[test]
    fn the_help_block_is_a_bijection_onto_the_rendered_rows() {
        assert_eq!(ROW_TOOLTIPS.len(), DETAIL_ROWS.len());
        for row in &DETAIL_ROWS {
            assert_eq!(
                ROW_TOOLTIPS
                    .iter()
                    .filter(|(id, _, _)| *id == row.id)
                    .count(),
                1,
                "{} must have exactly one help string",
                row.english
            );
        }
    }

    /// Every text in the pane is one size, and that size is the tree's: all
    /// eight blocks of the live branch carry `FontIndex=0`, which the ladder
    /// answers with 12 px — the same 12 the file gives `LABEL_H`, so the size
    /// and the box it sits in come out of one source. The 11 px this replaced
    /// was in neither.
    #[test]
    fn the_pane_takes_its_text_size_from_the_trees_font_index() {
        assert_eq!(TEXT_FONT_INDEX, 0);
        assert_eq!(font_index_px(TEXT_FONT_INDEX), 12.0);
        assert_eq!(LABEL_H, 12.0);
    }

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
            let mut options = GameOptions::default();
            let start = row.value_text(&config, &options);
            let steps = match row {
                ExtraRow::FoliageMode => 4,
                ExtraRow::FoliageDensity => DENSITY_STEPS.len(),
                ExtraRow::FoliageSightRange => SIGHT_RANGE_STEPS.len(),
                ExtraRow::Preset => PRESET_STEPS.len(),
                ExtraRow::RenderScale => RENDER_SCALE_STEPS.len(),
            };
            let mut seen = vec![start.clone()];
            for _ in 0..steps {
                row.cycle(&mut config, &mut options);
                seen.push(row.value_text(&config, &options));
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

    /// The preset row is the one control that moves every other value at
    /// once, so it has to say which of three states it is in: following the
    /// file, holding the tier this process actually started with, or holding a
    /// choice that only a restart can honour.
    #[test]
    fn the_preset_row_names_the_file_the_running_tier_and_a_pending_choice() {
        // `config.example.yaml` ships `preset: auto`, and the test probe
        // reports a discrete GPU, so `auto` resolved to `high`.
        let config = test_config();
        let mut options = GameOptions::default();
        assert_eq!(
            ExtraRow::Preset.value_text(&config, &options),
            "auto (high); config.yaml"
        );

        options.video.preset_override = Some(QualityPreset::Auto);
        assert_eq!(
            ExtraRow::Preset.value_text(&config, &options),
            "auto (high); yours",
            "the saved choice matches what the process started with"
        );

        options.video.preset_override = Some(QualityPreset::Low);
        assert_eq!(
            ExtraRow::Preset.value_text(&config, &options),
            "low (next start; now auto (high))",
            "a pending choice must name both halves, or the row looks broken"
        );
    }

    /// The shipped default is `render_mode: vanilla`, which draws no shadow
    /// maps at all (`map/mod.rs:342`). So the Shadow Detail row can hold "On"
    /// and change nothing, and it has to name that condition. With the
    /// condition met the row has nothing left to note: it is applied while
    /// the game runs (`environment::options::apply_shadow_detail_option`), so
    /// a restart note here would be a lie in the other direction.
    #[test]
    fn a_row_whose_condition_is_unmet_names_the_condition() {
        let mut config = test_config();
        assert_eq!(
            config.graphics.render_mode,
            RenderMode::Vanilla,
            "the faithful default is what makes this row inert"
        );
        let text = config_row_text(1, &config).expect("Shadow Detail is config-backed");
        assert!(
            text.ends_with("(no effect in vanilla lighting)"),
            "the row has to name the condition: {text:?}"
        );

        config.graphics.render_mode = RenderMode::Pbr;
        let text = config_row_text(1, &config).expect("Shadow Detail is config-backed");
        assert_eq!(
            text, "On",
            "with the condition met the row is live and has nothing to note"
        );
    }

    /// Every other row is unconditional, and should stay that way by
    /// accident of nobody adding a condition: a row that quietly grew one
    /// would start claiming an effect it does not have.
    #[test]
    fn only_shadow_detail_and_water_reflection_carry_a_condition() {
        let conditional: Vec<u16> = DETAIL_ROWS
            .iter()
            .filter(|row| config_row(row.id).is_some_and(|def| def.requires != Requirement::Always))
            .map(|row| row.id)
            .collect();
        assert_eq!(conditional, vec![1, 4]);
    }

    /// Supersampling has to be reachable from the window, or "let me have the
    /// maximum" stays a config-file secret. The row walks past 100% rather
    /// than stopping there.
    #[test]
    fn the_render_scale_row_reaches_both_sides_of_native() {
        let mut config = test_config();
        let mut options = GameOptions::default();
        let mut seen = Vec::new();
        for _ in 0..RENDER_SCALE_STEPS.len() {
            seen.push(config.graphics.render_scale.0);
            ExtraRow::RenderScale.cycle(&mut config, &mut options);
        }
        assert!(
            seen.iter().any(|scale| *scale < 1.0),
            "no step below native: {seen:?}"
        );
        assert!(
            seen.iter().any(|scale| *scale > 1.0),
            "no supersampling step: {seen:?}"
        );
        assert_eq!(
            seen.iter().filter(|scale| **scale == 1.0).count(),
            1,
            "native must be reachable exactly once per lap: {seen:?}"
        );
        // and the row names which side of native it is on
        config.graphics.render_scale = RenderScale(2.0);
        assert_eq!(
            ExtraRow::RenderScale.raw_value_text(&config, &options),
            "200% (supersampled)"
        );
        config.graphics.render_scale = RenderScale(0.5);
        assert_eq!(
            ExtraRow::RenderScale.raw_value_text(&config, &options),
            "50% (upscaled)"
        );
    }

    /// One lap hands the choice back to `config.yaml`. Without that step a
    /// single click would take the setting away from the file for good.
    #[test]
    fn the_preset_row_can_hand_the_choice_back_to_the_file() {
        let mut config = test_config();
        let mut options = GameOptions::default();
        for _ in 0..PRESET_STEPS.len() {
            ExtraRow::Preset.cycle(&mut config, &mut options);
        }
        assert_eq!(options.video.preset_override, None);
    }

    /// `off` is the faithful v1.188 setting (#366, #646): the default must
    /// stay off, and the mode row must start there rather than at a mode that
    /// renders grass the original never had.
    #[test]
    fn the_shipped_default_is_the_vanilla_off_mode() {
        let config = test_config();
        assert_eq!(config.graphics.foliage.mode, FoliageMode::Off);
        assert_eq!(
            ExtraRow::FoliageMode.raw_value_text(&config, &GameOptions::default()),
            "Off (vanilla)",
            "the off state has to name itself as the original behaviour"
        );
    }

    /// Every row that writes `ClientConfig` says so, because no
    /// player-facing store persists it — `config.yaml` is an author-owned
    /// read-only input. A change that applies instantly and is gone next
    /// launch must say so in the row. The preset row writes `GameOptions`
    /// and names its own timing, so it must *not* carry the suffix.
    #[test]
    fn every_config_writing_extra_row_declares_that_its_change_is_session_only() {
        let config = test_config();
        let options = GameOptions::default();
        for row in EXTRA_ROWS {
            let text = row.value_text(&config, &options);
            assert_eq!(
                text.contains(SESSION_ONLY),
                row.session_only(),
                "{row:?} misreports whether its change survives a restart: {text:?}"
            );
        }
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

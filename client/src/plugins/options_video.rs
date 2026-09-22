//! Content of the options window's Video tab.
//!
//! Idea: the pane is transcribed from the **live** branch of
//! `Media.pk2/resinfo/ifoption_video.txt` — the one guarded by `APPLY_UI_4TH`,
//! which `config/define.txt:15` defines, so the `#else` half (with its quality
//! spinner and a 30px-lower list) is build-excluded and is *not* what the
//! shipped client draws. Geometry is page-local to the pane rect
//! `11,62,364,313` from `ifoption.txt`.
//!
//! Two things are deliberately **not** invented here, because the game data
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
use bevy::window::Monitor;

use crate::plugins::config::graphics::FoliageMode;
use crate::plugins::config::ClientConfig;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::settings::tooltip::{attach_tooltip, spawn_tooltip_line};
use crate::plugins::textdata::ClientUiStrings;

/// Page-local geometry, all taken from `resinfo/ifoption_video.txt`.
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
/// its row inventory is unknown.
const ROW_H: f32 = 22.0;

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
    /// textuisystem key, from the resinfo tree.
    key: &'static str,
    /// English fallback, matching the textuisystem line.
    english: &'static str,
    /// Whether a real feature backs this row today.
    backing: Backing,
}

/// Whether a control can actually change anything in this client.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backing {
    /// Wired to a live feature and interactive.
    Live,
    /// The original has the control, we have no feature for it yet.
    Missing,
}

/// The 13 named quality rows. Ids and names come from the SROptionSet table
/// (`docs/formats/sroptionset.md:62-99`); the label keys from the resinfo tree.
/// Ids 14/15 exist in the id space but their CSV name cells are blank, so they
/// are left out rather than captioned by guess.
const DETAIL_ROWS: [DetailRow; 13] = [
    DetailRow {
        id: 1,
        key: "UIIT_STT_SHADOW_DETAIL",
        english: "Shadow Detail",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 2,
        key: "UIIT_STT_SCENERY_SIGHT_RANGE",
        english: "Background Sight Range",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 3,
        key: "UIIT_STT_CHAR_SIGHT_RANGE",
        english: "Character Sight Range",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 4,
        key: "UIIT_STT_WATER_REFLECTION",
        english: "Water Reflection",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 5,
        key: "UIIT_STT_WATER_DETAIL",
        english: "Water Detail",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 6,
        key: "UIIT_STT_METAL_DETAIL",
        english: "Metallic Sheen",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 7,
        key: "UIIT_STT_LIGHT_EFFECT",
        english: "Light Effect",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 8,
        key: "UIIT_STT_FILTERING",
        english: "Texture Filtering",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 9,
        key: "UIIT_STT_TEXTER_DETAIL",
        english: "Texture Detail",
        backing: Backing::Missing,
    },
    DetailRow {
        id: 10,
        key: "UIIT_STT_LENS_FLAIR",
        english: "Lens Flare",
        backing: Backing::Missing,
    },
    // The one quality row this client can actually honour: `Bloom` is
    // insert/removed on the window cameras (see `apply_bloom_option`).
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
        backing: Backing::Missing,
    },
    DetailRow {
        id: 13,
        key: "UIIT_STT_EFFECT_QUALITY",
        english: "Effect Quality",
        // Wired to the effect runtime by
        // `effects::options::apply_effect_quality_option`: off pauses the
        // whole effect schedule and hides the wrappers. Read as a toggle,
        // not a graded ramp — see that module's ADR-0009 note.
        backing: Backing::Live,
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
/// Five of the seventeen strings are deliberately absent here:
/// * `_01`/`_02` belong to the resolution and brightness controls, not to a row
///   (wired at their own call sites below).
/// * `_03` describes the graphic-quality **preset**, which exists only in the
///   `#else` (classic) branch of `ifoption_video.txt`; this pane transcribes
///   the 4th-gen branch, which has the two profile tabs instead.
/// * `_15` ("Compensates rough outlines...") describes edge smoothing. Our id 8
///   is `UIIT_STT_FILTERING`, "Texture Filtering" — near, but the string does
///   not name the row, and `_11` already proved that positional inference is
///   wrong here. Left unwired.
/// * `_17` (large-scale combat outfit unification) belongs to ids 14/15, whose
///   name cells are blank and which this pane therefore does not render.
const ROW_TOOLTIPS: [(u16, &str, &str); 12] = [
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

    // The pane's hover-help footer (see `settings::tooltip`), spanning the
    // detail list's width so it lines up with the rows it describes.
    spawn_tooltip_line(pane, &font, LIST_X, LIST_W);
}

// ---------------------------------------------------------------------------
// openroad extras
//
// There is no grass / vegetation / foliage control anywhere in v1.188 — not in
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
}

const EXTRA_ROWS: [ExtraRow; 3] = [
    ExtraRow::FoliageMode,
    ExtraRow::FoliageDensity,
    ExtraRow::FoliageSightRange,
];

/// Density steps. 1.0 is the authored baseline, so the scale is centred on it
/// rather than starting there.
const DENSITY_STEPS: [f32; 4] = [0.5, 1.0, 1.5, 2.0];
/// Sight-range steps in world units. 300 is the config default and covers the
/// camera's working range (`camera.rs`: 40..400); 0 means "never cull", which
/// is what shipped earlier and is kept reachable, but is not a step anyone
/// lands on by accident.
const SIGHT_RANGE_STEPS: [f32; 4] = [150.0, 300.0, 600.0, 0.0];
/// Openroad additions are drawn in their own colour so the pane never suggests
/// the original had this control.
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
        }
    }

    /// What the row shows for the current config, always with the
    /// [`SESSION_ONLY`] suffix.
    ///
    /// The suffix is not decoration. These three rows write `ClientConfig`,
    /// and `config.yaml` is an **author-owned, read-only input**
    /// (`settings/persistence.rs:1-6`) — the player's own store is
    /// `user_settings.yaml`, which `ClientConfig` never reaches. So a grass
    /// setting applies instantly and is then lost on the next launch, which is
    /// exactly the "I changed it and it didn't stick" complaint in a different
    /// costume. Saying so in the value is the same honesty idiom this pane
    /// already uses for `(not applied)` and `(no effect yet)`; mirroring the
    /// three fields into [`GameOptions`] is the real fix and is a bigger
    /// change than this row deserves.
    fn value_text(self, config: &ClientConfig) -> String {
        format!("{} {SESSION_ONLY}", self.raw_value_text(config))
    }

    fn raw_value_text(self, config: &ClientConfig) -> String {
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
    let live = row.backing == Backing::Live;
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
    if live {
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
        r.spawn((
            RowValue { id: row.id },
            Text::new(row_value_text(row, options, GraphicProfileTab::One)),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(if live { VALUE_COLOR } else { UNBACKED_COLOR }),
            Pickable::IGNORE,
        ));
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
        Backing::Missing => format!("{value} (no effect yet)"),
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
    profile: &crate::plugins::settings::options::GraphicProfile,
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
    // Bevy's access check panics the whole schedule at startup (B0001) — the
    // one failure class `make ci` cannot see. `refresh_row_values` below is
    // filtered the same way.
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

// The window itself is not applied from here any more. Both halves —
// `setup_window_settings` (the boot apply) and `apply_window_mode` (the
// options-driven one) — moved into `plugins::config::window`, which now
// resolves one `WindowIntent` from `config.yaml` plus the session-only
// `video.window_mode_override` and is the single writer. Two modules each
// computing their own answer for `window.mode` is exactly what made
// `config.yaml`'s `mode:` a dead key: whichever ran last won, and that was
// this one.
//
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
}

#[cfg(test)]
mod tests {
    use super::*;
    // Only the assertion below names the effect-quality row; importing it at
    // module scope made it an unused import in a normal build, and that is a
    // hard gate here (`scripts/check_warnings.py`).
    use crate::plugins::effects::options::EFFECT_QUALITY_ID;

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

    /// The two rows left unwired on purpose (`PREGAME-options-b2b3.md` §2):
    /// id 8 `UIIT_STT_FILTERING`, whose candidate string `_15` describes edge
    /// smoothing and does not name the row, and nothing else. If a later change
    /// wires id 8, this test should be updated deliberately.
    #[test]
    fn only_the_unmatched_row_is_left_without_help() {
        let strings = ClientUiStrings::default();
        let missing: Vec<u16> = DETAIL_ROWS
            .iter()
            .filter(|row| row_tooltip(&strings, row.id).is_none())
            .map(|row| row.id)
            .collect();
        assert_eq!(missing, vec![8]);
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
    fn only_backed_rows_claim_an_effect() {
        // Every row without a real feature must say so; none may render a
        // value word that implies it took effect.
        let options = GameOptions::default();
        for row in &DETAIL_ROWS {
            let text = row_value_text(row, &options, GraphicProfileTab::One);
            match row.backing {
                Backing::Missing => assert!(
                    text.contains("no effect yet"),
                    "{} must be marked unbacked, got {text:?}",
                    row.english
                ),
                Backing::Live => assert!(
                    text == "On" || text == "Off",
                    "{} is wired and must show its state, got {text:?}",
                    row.english
                ),
            }
        }
    }

    #[test]
    fn only_deliberately_wired_rows_are_live() {
        // If a later change wires another feature, this test should be updated
        // deliberately — it is the guard against quietly marking rows Live.
        // Row 13 (Effect Quality) joined Bloom later: its read site is
        // `effects::options::apply_effect_quality_option`.
        let live: Vec<u16> = DETAIL_ROWS
            .iter()
            .filter(|r| r.backing == Backing::Live)
            .map(|r| r.id)
            .collect();
        assert_eq!(live, vec![BLOOM_ID, EFFECT_QUALITY_ID]);
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

    /// The grass rows are openroad additions: v1.188 has no
    /// grass/vegetation/foliage control in `ifoption_video.txt`,
    /// `ifvideooptionslot.txt` or `textuisystem`. A row
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
            ExtraRow::FoliageMode.raw_value_text(&config),
            "Off (vanilla)",
            "the off state has to name itself as the original behaviour"
        );
    }

    /// These three rows write `ClientConfig`, which no player-facing store
    /// persists — `config.yaml` is an author-owned read-only input. A change
    /// that applies instantly and is gone next launch is the "it didn't stick"
    /// complaint in a different costume, so every row has to say so.
    #[test]
    fn every_extra_row_declares_that_its_change_is_session_only() {
        let config = test_config();
        for row in EXTRA_ROWS {
            let text = row.value_text(&config);
            assert!(
                text.contains(SESSION_ONLY),
                "{row:?} does not tell the user its change is lost on restart: {text:?}"
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

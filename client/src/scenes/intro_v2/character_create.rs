//! Character-creation sub-screen of the intro v2 flow.
//!
//! Idea: reuse the char-select 3d stage (same camera, world origin and model
//! spawn path) to show a *live* preview of the picked starter body + gear, and
//! drive the whole screen off a single [`CharCreateSelection`] resource — a
//! change to it re-spawns the preview and re-highlights the toggles, mirroring
//! how the char-select selection UI keys off `SelectedCharacterV2`. On submit it
//! sends the `Create` lobby action (with `CheckName` for a pre-submit
//! availability probe) over the still-open agent connection and, on success,
//! returns to a freshly-listed `CharacterList`.

use std::f32::consts::PI;

use bevy::camera::visibility::RenderLayers;
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::ui::{InteractionDisabled, Pressed};
use bevy::ui_widgets::Activate;
use bevy::window::PrimaryWindow;

use packets::agent::prelude::*;
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::camera::{CameraLayers, CinematicCamera2};
use crate::plugins::dynamic_resource_loader::{MirroredResource, UnloadedResource};
use crate::plugins::environment::reflections::sky_reflection_env_light;
use crate::plugins::hud::modal_dialog::{modal_plate, modal_scrim};
use crate::plugins::map::terrain::Terrain;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::{
    ClientCharacterData, ClientItemData, ClientItemIndex, ClientUiStrings,
};
use crate::plugins::ui_v2::style::{ButtonSound, ImageButtonStyle};
use crate::plugins::ui_v2::widgets::{image_button, label_sized, text_input};
use crate::plugins::world_origin::{set_world_origin, WorldOrigin};
use crate::util::mesh::needs_winding_reversal;

use super::assets::IntroV2Assets;
use super::character_select;
use super::chrome::{InfoTextV2, InfoTextV2Update};
use super::login_form::main_button_style;
use super::scene_data::ActiveCharSelectSceneV2;
use super::{intro_font_px, play_error_sound, unescape_newlines, IntroV2State, IntroV2Ui};
use crate::assets::FontAssets;

/// Marker resource set by the char-select Create button so the char-list exit
/// keeps the agent connection alive for the switch into creation.
#[derive(Resource)]
pub struct EnteringCharacterCreate;

pub use super::model::{CharCreateSelection, Garment, Gender, Race};

/// Root marker of the creation UI (left panel + bottom button row).
#[derive(Component, Default, Clone)]
pub struct CharCreateRoot;

/// The 3d body preview reflecting the current selection.
#[derive(Component, Default, Clone)]
pub struct CharCreatePreview;

/// How the preview is being looked at, driven by the original's
/// `Section = Rotate` controls: an accumulated yaw and the zoom toggle.
/// Kept as a resource (not on the preview entity) because the preview is
/// re-spawned on every selection change and the view must survive that.
#[derive(Resource, Default)]
pub struct CharCreatePreviewView {
    /// Yaw added to the podium pose, in radians.
    pub yaw: f32,
    /// `GDR_BTN_ZOOM` state: the art swaps `zoomin` <-> `zoomout`.
    pub zoomed_in: bool,
}

/// One of the two rotate buttons; the sign is the yaw direction. `Default`
/// is required by `bsn!` (every templated component must be constructible)
/// and is never the value used: both call sites pass their own step.
#[derive(Component, Clone, Default)]
pub struct PreviewRotateButton(pub f32);

/// `GDR_BTN_ZOOM` — its texture set is swapped on toggle.
#[derive(Component, Default, Clone)]
pub struct PreviewZoomButton;

/// Marker on the name `EditableText`.
#[derive(Component, Default, Clone)]
pub struct NameInput;

/// Marker on the name-availability feedback text.
#[derive(Component, Default, Clone)]
pub struct NameFeedback;

/// Marker on the Create button, used to re-enable it when the server rejects.
#[derive(Component, Default, Clone)]
pub struct CreateButton;

/// Root of the `Section = WCreate` confirmation modal.
#[derive(Component, Default, Clone)]
pub struct CreateConfirmModal;

/// Marker on a gender toggle *button*. It sits on the button and not on its
/// caption because the original expresses selection as a texture swap
/// (`man_on` <-> `man_off`), which is a property of the button.
#[derive(Component, Default, Clone)]
pub struct GenderOption(pub Gender);

/// Which option row a `CIFSliderCtrl` instance drives. The original authors
/// `Section = Slider` ONCE and instantiates it per row, so the row identity is
/// a component on the instance rather than five separate widgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Component)]
pub enum SliderRow {
    #[default]
    Figure,
    Height,
    Volume,
    Protector,
    Weapon,
}

/// A slider's prev/next arrow: which row it drives and in which direction.
#[derive(Component, Default, Clone)]
pub struct SliderStep(pub SliderRow, pub i32);

/// A slider's `GDR_BTN_THUMB`; its `left` tracks the row's current index.
#[derive(Component, Default, Clone)]
pub struct SliderThumb(pub SliderRow);

/// The row the Explain box is currently describing. The original's Explain
/// panel is context-sensitive per focused slider (24 `UIO_NEWCHAR_EXPLANATION_*`
/// keys ship for exactly that), which is also why the rows carry no value text
/// of their own: the readout IS the Explain box.
#[derive(Resource, Default)]
pub struct CharCreateFocus(pub SliderRow);

/// Marker on the figure-story title (the figure's display name).
#[derive(Component, Default, Clone)]
pub struct FigureStoryTitle;

/// Marker on the figure-story body (the vanilla `*_EXPLANATION` text).
#[derive(Component, Default, Clone)]
pub struct FigureStoryText;

// --- The screen's unit: one authored pixel of the original -------------------
//
// Every rect in this file is transcribed from the original's `resinfo` tree, and
// the original draws all of them **1:1 in device pixels** on an 800x600 client
// area; the two full-width bands are the one exception — 1600x172 art rendered
// 800x85. The sibling pre-game screens do the same: the login form, the region
// plates (`region_select::PLATE_SIZE`) and the character-select button row draw
// their art at its native size, and `intro_font_px` is unscaled for the same
// reason.
//
// History, so nobody re-derives either end: this screen was drawn 1:1, then for
// a while `cu` was `Val::Vh(v / 6)` — the art grew with the window height, 1.5x
// at 1600x900 and 2.4x at 1440 rows, while every other pre-game surface stayed
// 1:1. That scale was too large. One unit for the whole pre-game wins; if the
// pre-game is ever scaled, it is scaled in one place for all its screens, not
// here.

/// One authored pixel.
fn cu(v: f32) -> Val {
    Val::Px(v)
}

/// [`cu`] for text.
fn cu_font(v: f32) -> FontSize {
    FontSize::Px(v)
}

/// [`cu`] as a number — for the geometry tests.
///
/// Test-only: the screens themselves spawn `cu(..)` nodes, so the only callers
/// of the bare number are the pinning tests below (and `control_row_box`,
/// which exists for them). Without the gate the non-test build warns
/// `never used`, which `clippy -D warnings` turns into an error.
#[cfg(test)]
fn cu_at(v: f32) -> f32 {
    v
}

/// `GDR_STA_TITLE` art (`text-custom.ddj`), `Rect=47,111,428,36` in both the
/// CH and EU create trees.
const TITLE_CUSTOM_DDJ: &str = "media://interface/outer/text-custom.ddj";
const TITLE_ART_W: f32 = 428.0;
const TITLE_ART_H: f32 = 36.0;

// --- `GDR_STA_CUSTOM` and its `Section = Custom` children --------------------
//
// Idea: the customize panel is a fixed-size baked sprite
// (`customize_window[_europe].ddj`, 264x316) whose slots — the name field, the
// two sex buttons, the five option rows — are painted INTO the art. So the
// children are absolutely placed at the resinfo rects in the panel's own pixel
// space and nothing here is flex, padding or spacing that we get to choose.
// Line numbers below are from `resinfo/pscharactercreatechina.txt`; the
// European file is line-for-line identical except where noted.

/// `GDR_STA_CUSTOM` (`:139-157`), `customize_window[_europe].ddj` — both race
/// variants are 264x316.
const CUSTOM_W: f32 = 264.0;
const CUSTOM_H: f32 = 316.0;
/// `GDR_STATIC1` (`:561-579`) `UIO_NEWCHAR_STT_NAME` and `GDR_EDIT_NAME`
/// (`:428-446`). The edit has no DDJ: its slot is baked into the panel art.
/// The label uses the widened caption column (see [`LABEL_X`]), not the
/// authored `x=32, w=45`; only its `top` is the resinfo's.
const NAME_LABEL_RECT: (f32, f32, f32, f32) = (LABEL_X, 35.0, LABEL_W, LABEL_H);
const NAME_EDIT_RECT: (f32, f32, f32, f32) = (92.0, 32.0, 135.0, 20.0);
/// `GDR_BTN_CHECK` (`:409-427`) on `overlap.ddj` (75x25, exact fit).
const CHECK_RECT: (f32, f32, f32, f32) = (156.0, 57.0, 75.0, 25.0);
/// `GDR_STATIC2` (`:542-560`) `UIO_NEWCHAR_STT_SEX`, then `GDR_BTN_MALE`
/// (`:390-408`) and `GDR_BTN_FEMALE` (`:371-389`).
/// Same column as every other caption (see [`LABEL_X`]); authored `x=32,w=45`.
const SEX_LABEL_RECT: (f32, f32, f32, f32) = (LABEL_X, 100.0, LABEL_W, LABEL_H);
/// The art is `man_*/woman_*.ddj` at **72x28** with a 70x25 picture and
/// transparent padding on the right/bottom edges; drawing it into this 70x25
/// rect squashes the file, and that is what the original does too — see
/// `padded_art_is_drawn_at_the_authored_rect_not_the_file_size`.
/// Do not "fix" this to 72x28.
const MALE_RECT: (f32, f32, f32, f32) = (87.0, 95.0, 70.0, 25.0);
const FEMALE_RECT: (f32, f32, f32, f32) = (166.0, 95.0, 70.0, 25.0);

// --- Where the gender captions sit ------------------------------------------
//
// Idea: a gender button's art is *not* a plate. `man_*.ddj` draws the green
// gem in its own left columns and `woman_*.ddj` the grey one on the right, so
// the writable plate is only part of the control. Centring the caption on the
// control therefore parks it half a gem into the gem; the original centres it
// on the plate, and the two disagree by exactly half the gem's width.

/// The gem baked into each gender button, in the control's own pixels.
///
/// Not chosen: the four textures
/// (`man_on/off`, `woman_on/off`) are 72x28 with 70x25 of content, and the
/// alpha's full-height columns put the man plate at 20..68 and the woman plate
/// at 1..49 — both 49 wide, both with a 20-column gem on the other side.
///
/// The original agrees: with its panel left at x=78 and 1:1 drawing, its
/// "Female" ink runs 249..290, centre 269.5, against a drawn
/// plate of 245..292.6, centre 268.8 — centred on the plate, six pixels clear
/// of the gem. Ours centred on the control instead, which at 800x600 put
/// "Female"'s ink at 207..244 with the plate ending at 238.6: 5.4 px *on* the
/// gem, and "Male" 7.7 px left of its plate's centre. That is the
/// "Female runs past its button" defect.
const GENDER_GEM_W: f32 = 20.0;

/// The plate's span inside a gender button, `(left, right)` in control pixels.
///
/// The gem sits on the side the *other* button's gem does not: left for male,
/// right for female (see [`GENDER_GEM_W`]).
///
/// Test-only, like [`control_row_box`]: the caption placement itself goes
/// through [`gender_caption_margin`]; this is the span that margin is checked
/// against.
#[cfg(test)]
fn gender_plate_span(gender: Gender) -> (f32, f32) {
    let w = MALE_RECT.2;
    match gender {
        Gender::Male => (GENDER_GEM_W, w),
        Gender::Female => (0.0, w - GENDER_GEM_W),
    }
}

/// The margin that moves a centred caption from the control's centre onto the
/// plate's, in control pixels: a margin on the gem's side shifts a centred
/// child by half of it, and half the gem is exactly the offset wanted.
///
/// Returns `(left, right)`; only one is ever non-zero.
fn gender_caption_margin(gender: Gender) -> (f32, f32) {
    match gender {
        Gender::Male => (GENDER_GEM_W, 0.0),
        Gender::Female => (0.0, GENDER_GEM_W),
    }
}
/// The five `CIFSliderCtrl` rows all share `x=88, w=120, h=24` and step by 31:
/// `GDR_SLI_FIGURE :352-370`, `_HEIGHT :333-351`, `_VOLUME :314-332`, and the
/// two gear rows (see [`gear_row_tops`]).
const ROW_X: f32 = 88.0;
/// The authored slider rect's width. Only the geometry test reads it: the
/// drawn row is [`SLIDER_TEMPLATE_W`] (the arrows reach past the rect).
#[cfg(test)]
const ROW_W: f32 = 120.0;
const ROW_H: f32 = 24.0;
const ROW_FIGURE_TOP: f32 = 131.0;
const ROW_HEIGHT_TOP: f32 = 162.0;
const ROW_VOLUME_TOP: f32 = 193.0;
/// The two gear rows, in the order the *Chinese* tree authors them.
const ROW_GEAR_TOPS: (f32, f32) = (224.0, 255.0);
// --- The caption column: a deliberate deviation (ADR-0009) ------------------
//
// Idea: the caption box the original authors is *too small for its own text*,
// so we move the whole column left into the panel's empty margin and give the
// box the width its widest caption actually needs. The rows, arrows and
// buttons do not move — the box grows towards the free side.
//
// What the ORIGINAL does: all seven captions sit in `x=32, w=45` boxes
// (`GDR_STATIC1..7`) drawn in Arial at FontIndex 2 = 16 px. At that size the
// `textuisystem.txt` strings do not fit: "Weapon" advances
// **59.59 px into a 45 px box (+14.59, +32.4 %)** and "Volume" 54.25 (+9.25),
// with "Height" (46.25) and "Figure" (45.35) grazing the edge. The original
// simply lets them run out of the box and the slider's left arrow (`ROW_X`
// 88) then paints over the tail — the original's own "Weapon" is clipped
// mid-'n', exactly like ours was. `45` was never a transcription error, and it
// should not be "fixed".
//
// What WE do instead: keep the boxes' geometry-defining right side clear of
// the controls and grow leftwards. `LABEL_X` 32 -> 16 and `LABEL_W` 45 -> 66.
//
// Why: readability. A caption that is painted over by the control it labels is
// unreadable, and this is a *clone*, not a replica (ADR-0009) — a deliberate,
// stated deviation for legibility is wanted; an unexplained number is not.
//
// What it COSTS: the column no longer starts on the original's x=32, so these
// seven strings sit 16 px left of where the original puts them. The left
// margin to the panel art's ornamental frame shrinks
// (see [`LABEL_X`]) and the box's right edge moves 77 -> 82, i.e. the clear
// space to the male button (x=87) and the slider arrows (x=88) drops from 10
// to 5 px for the *box*. No caption's ink gets that close: the widest shipped
// string ends at 16+59.59 = 75.6, still 11 px short of the arrow.
//
// Checked for BOTH races, in the data: the Chinese and
// European `resinfo` trees carry the *same seven keys* at the same rects and
// differ only in the order of the two gear rows (`GDR_STATIC6/7` swap), so
// there is no longer European string to overflow. Widths at Arial 16 from
// `textuisystem.txt`: Weapon 59.59, Volume 54.25, Height 46.25,
// Figure 45.35, Name 42.68, Cloth 37.35, Sex 27.57.

/// Recorded, not consumed: the authored column of `GDR_STATIC1..7`
/// (`:447-541` and `:542-579`), `x=32, w=45, h=15`. Kept as a constant so the
/// deviation above stays measurable against the source rect.
#[allow(dead_code)]
const LABEL_AUTHORED_X: f32 = 32.0;
/// Recorded, not consumed — see [`LABEL_AUTHORED_X`].
#[allow(dead_code)]
const LABEL_AUTHORED_W: f32 = 45.0;

/// Left edge of the caption column, all seven captions (openroad, see above).
///
/// Not a round number: `customize_window[_europe].ddj` paints its ornamental
/// frame in columns **4..12** (the interior is flat from column 13 on), so 16 is the frame's
/// last column plus a **3 px** buffer. Anything smaller runs the text into the
/// border ornament.
const LABEL_X: f32 = 16.0;
/// Width of a caption box (openroad, see above).
///
/// Sized from the strings, not chosen: the widest caption the data can put
/// here is "Weapon" at **59.59 px**, which leaves **6.4 px**
/// spare; 66 also still holds "Protector" (**64.91**, the fallback literal in
/// the docs for this row) with **1.09 px** spare. A box narrower than its text
/// makes bevy wrap the caption onto a second line, which is why this grows
/// with `LABEL_X` instead of leaving the authored 45.
const LABEL_W: f32 = 66.0;
/// Right edge of the caption column; the controls it must stay clear of are
/// the male button (`MALE_RECT` x=87) and the slider arrows (`ROW_X` 88).
#[allow(dead_code)]
const LABEL_RIGHT: f32 = LABEL_X + LABEL_W;
const LABEL_H: f32 = 15.0;
const LABEL_FIGURE_TOP: f32 = 137.0;
const LABEL_HEIGHT_TOP: f32 = 167.0;
const LABEL_VOLUME_TOP: f32 = 198.0;
const LABEL_GEAR_TOPS: (f32, f32) = (229.0, 260.0);
/// Name-check feedback line. `GDR_TEXT_MESSAGE` (`:196-214`) has rect
/// `0,0,0,0` — the original places its message sink from code, so its position
/// is unknown.
/// **openroad choice:** the panel's free strip below the last row (which ends
/// at 260+15=275), so the answer appears next to the field it is about.
const FEEDBACK_TOP: f32 = 285.0;
const FEEDBACK_W: f32 = 200.0;

// --- `Section = Slider` (`:685-744`) — one template, every row ---------------
//
// Idea: the tree authors the slider ONCE and each `CIFSliderCtrl` row
// instantiates it, so we build one widget and place it per row rather than
// five bespoke pickers. All three child rects are in the template's own space.
//
// Note the width: the row's own `Rect` is 120 wide, but `GDR_BTN_NEXT` starts
// at x=120, so a drawn instance is 140 wide. That is not a contradiction to
// paper over — the panel is 264 wide and the rows start at x=88, so 88+140=228
// still lands inside it, which is what makes the 140 credible.

/// `GDR_BTN_PREV` (`:725-743`), `arrow_left.ddj`.
const SLIDER_PREV_RECT: (f32, f32, f32, f32) = (0.0, 2.0, 20.0, 20.0);
/// `GDR_BTN_THUMB` (`:687-705`), `slider.ddj` — the rect at index 0.
const SLIDER_THUMB_RECT: (f32, f32, f32, f32) = (20.0, 0.0, 16.0, 24.0);
/// `GDR_BTN_NEXT` (`:706-724`), `arrow_right.ddj`.
const SLIDER_NEXT_RECT: (f32, f32, f32, f32) = (120.0, 2.0, 20.0, 20.0);
/// Drawn width of one instance: up to the right edge of the next arrow.
const SLIDER_TEMPLATE_W: f32 = SLIDER_NEXT_RECT.0 + SLIDER_NEXT_RECT.2;

/// Where the thumb sits for `index` of `count` steps.
///
/// Derived, not invented: the thumb starts at the authored `x=20` and the
/// track ends where `GDR_BTN_NEXT` begins (`x=120`), so the thumb's travel is
/// `120 - 20 - 16 = 84` px and the steps divide it evenly. A single-step row
/// (or an empty one) parks the thumb at the start.
fn slider_thumb_left(index: usize, count: usize) -> f32 {
    let start = SLIDER_THUMB_RECT.0;
    if count <= 1 {
        return start;
    }
    let travel = SLIDER_NEXT_RECT.0 - start - SLIDER_THUMB_RECT.2;
    start + travel * index.min(count - 1) as f32 / (count - 1) as f32
}

// --- `Section = WCreate` — the create-confirm modal --------------------------
//
// Idea: the same shape char-select's delete-confirm already has — a dimming
// full-screen scrim (which is what actually makes the modal modal: it eats the
// clicks behind it) carrying the warning frame at its native size. Only the
// art and the child rects differ, so the button style is shared rather than
// re-minted.

/// `GDR_STA_WCREATE` (`:25-43`), `warning_create.ddj` at its native size.
const WCREATE_W: f32 = 248.0;
const WCREATE_H: f32 = 128.0;
/// `GDR_STA_WNAME` (`:805-823`) — the name being created, FontIndex 2.
const WCREATE_NAME_RECT: (f32, f32, f32, f32) = (4.0, 23.0, 239.0, 15.0);
/// `GDR_STATIC1` (`:786-804`), `UIO_NEWCHAR_MSG_CREATE`.
const WCREATE_MSG_RECT: (f32, f32, f32, f32) = (4.0, 47.0, 239.0, 13.0);
/// `GDR_BTN_WCREATE` (`:767-785`) `UIO_COMMON_CTL_CREATE` — this is where that
/// key belongs; the screen's own `GDR_BTN_OK` carries `UIO_NEWCHAR_CTL_CONFIRM`
/// and a stale comment used to misattribute it to this button.
const WCREATE_OK_RECT: (f32, f32, f32, f32) = (42.0, 75.0, WARNING_BUTTON_W, WARNING_BUTTON_H);
/// `GDR_BTN_WCANCEL` (`:748-766`) `UIO_COMMON_CTL_CANCEL`.
const WCREATE_CANCEL_RECT: (f32, f32, f32, f32) = (130.0, 75.0, WARNING_BUTTON_W, WARNING_BUTTON_H);
/// Size of the shared `warning_button.ddj` art, `76x32` in both modals'
/// authored rects (`GDR_BTN_WCREATE`/`WCANCEL` `..,75,76,32` here).
const WARNING_BUTTON_W: f32 = 76.0;
const WARNING_BUTTON_H: f32 = 32.0;

/// `GDR_STA_EXPLAIN` (`:101-119`) and its `Section = Explain` children
/// (`:582-622`): the description panel right of the customize window.
/// **The one rect/art mismatch in the tree**: the European tree
/// declares `0,0,212,250` but ships `explain-window_02.ddj` at 220x236. We
/// draw the ART at its own size, because a stretched 9-slice-less sprite would
/// visibly distort; the declared rect is recorded here and not silently lost.
const EXPLAIN_CH_SIZE: (f32, f32) = (212.0, 180.0);
/// Recorded, not consumed: the declared European rect. Kept as a constant so
/// the mismatch stays in the source next to the art size we actually draw.
#[allow(dead_code)]
const EXPLAIN_EU_RECT: (f32, f32) = (212.0, 250.0);
const EXPLAIN_EU_ART: (f32, f32) = (220.0, 236.0);
/// `GDR_STA_EXPLAINNAME` (`:603-621`), FontColor `255,255,239,153`, and
/// `GDR_TEXT_EXPLAIN` (`:584-602`).
const EXPLAIN_NAME_RECT: (f32, f32, f32, f32) = (15.0, 18.0, 183.0, 14.0);
const EXPLAIN_BODY_RECT: (f32, f32, f32, f32) = (15.0, 42.0, 183.0, 122.0);
/// `GDR_STA_EXPLAINNAME`'s authored `FontColor` — a warm parchment, distinct
/// from the char-select gold this panel used to borrow.
const EXPLAIN_NAME_COLOR: Color = Color::srgb(1.0, 239.0 / 255.0, 153.0 / 255.0);

// --- Text sizes -------------------------------------------------------------
//
// Idea: every size on this screen is the `FontIndex` of the *same* resinfo
// control the rect above it is transcribed from, resolved on the original's
// five-slot ladder by `intro_font_px` (12, 11, 16, 15, 20 px — see
// `plugins::hud::scale::FONT_INDEX_PX`). The 11/12 that stood at these
// call sites were chosen by eye while "FontIndex -> size" was still unknown,
// and they made the seven row captions a third too small.
/// The seven row captions `GDR_STATIC1..7` (`:561,542,523,504,485,466,447`) are
/// all `FontIndex=2` -> 16 px.
const CAPTION_FONT_INDEX: usize = 2;
/// The three panel buttons `GDR_BTN_CHECK` (`:409`), `GDR_BTN_MALE` (`:390`)
/// and `GDR_BTN_FEMALE` (`:371`) are `FontIndex=0` -> 12 px.
const PANEL_BUTTON_FONT_INDEX: usize = 0;
/// `GDR_STA_EXPLAINNAME` (`:603`) and `GDR_TEXT_EXPLAIN` (`:584`), both
/// `FontIndex=0` -> 12 px.
const EXPLAIN_FONT_INDEX: usize = 0;
/// The two bottom buttons `GDR_BTN_OK` (`:82`) and `GDR_BTN_BACK` (`:63`) are
/// `FontIndex=2` -> 16 px.
const MAIN_BUTTON_FONT_INDEX: usize = 2;
/// The create-confirm window: `GDR_STA_WNAME` (`:805`) is `FontIndex=2`,
/// its message `GDR_STATIC1` `UIO_NEWCHAR_MSG_CREATE` (`:786`) and both buttons
/// `GDR_BTN_WCREATE`/`GDR_BTN_WCANCEL` (`:767`, `:748`) are `FontIndex=0`.
const CONFIRM_NAME_FONT_INDEX: usize = 2;
const CONFIRM_BODY_FONT_INDEX: usize = 0;
/// The slider rows' value readout and the name-check feedback line have **no**
/// resinfo control at all (the `CIFSliderCtrl` carries no text, and the
/// original reports name collisions in the notice line, not in the panel), so
/// their size is ours. Index 0 is the size 3547 of the 3739 authored controls
/// use, which is the least surprising choice for a code-drawn string; the
/// alternative would be an invented number.
const CODE_DRAWN_FONT_INDEX: usize = 0;

/// The gear rows swap between the race trees — this is behavioural fidelity,
/// not decoration. Chinese authors `GDR_SLI_PROTECTOR` at y=224 and
/// `GDR_SLI_WEAPON` at y=255 (`china :295-313` / `:276-294`); European swaps
/// them (`_europe :276-294` / `:295-313`), and its captions swap with them
/// (`GDR_STATIC6`/`7` texts, `:459` / `:478` in both files). Returns
/// `(protector_row_top, weapon_row_top)`.
/// The swap itself is a per-race property of the interface data, so it is read
/// off the race's presentation entry ([`super::race_catalog::RacePresentation`]);
/// a race with no interface data of its own keeps the China order, which is the layout
/// the shared rects were transcribed from.
fn gear_row_tops(race: Race, tops: (f32, f32)) -> (f32, f32) {
    if race.presentation().is_some_and(|p| p.gear_rows_swapped) {
        (tops.1, tops.0)
    } else {
        tops
    }
}

/// `GDR_STA_ROTATE` (`pscharactercreate{china,_europe}.txt:120-138`): the
/// 152x56 `rotate_window.ddj` panel holding the preview controls. Both race
/// trees are byte-identical here, so there is no `_europe` variant to pick.
const ROTATE_WINDOW_W: f32 = 152.0;
const ROTATE_WINDOW_H: f32 = 56.0;
/// Where that window sits — **not authored.** The resinfo rect is
/// `0,0,152,56`: size only, no position (the original places all four of this
/// screen's position-less panels from code). In the original's 800x600 client
/// area the window's outer edge runs x 635..786 / y 446..501 — i.e. **13 px
/// off the right edge, 98 px off the bottom**. Cross-check: adding the three
/// authored child rects (12/57/99, y 11) onto that corner predicts 650 / 695 /
/// 737 and y 483, and that is exactly where the original's four black edges
/// sit, 0 px error.
///
/// **Edge inset, not a scaled position — deliberate, and the one open call
/// here (ADR-0009).** The original at a single resolution cannot tell whether
/// it keeps the inset constant or scales the 635,446 position; deciding that
/// needs the original at a second resolution. We anchor to the edges because
/// everything else on this screen points that way: all four position-less panels
/// draw their art 1:1 unscaled (only the two full-width bands are halved), so
/// a constant pixel inset is the model that matches the art, and it keeps the
/// buttons in the same bottom-right corner at 1600x900 and 3440x1440 instead
/// of stranding them mid-screen. Before this the window was centred under the
/// podium (`left: 50%`, `bottom: 18%`), which put it 711 px left and 64 px up
/// from where the original has it.
const ROTATE_WINDOW_RIGHT_INSET: f32 = 13.0;
const ROTATE_WINDOW_BOTTOM_INSET: f32 = 98.0;
/// `Section = Rotate` (`:624-683`), child rects as authored, in the section's
/// own space: `GDR_BTN_LROTATE` (:664-682), `GDR_BTN_ZOOM` (:645-663),
/// `GDR_BTN_RROTATE` (:626-644). `(left, top, width, height)`.
/// The three arts are padded like the sex buttons (`rotate_*.ddj` 44x36 for a
/// 42x35 picture, `zoom*.ddj` 40x36 for 39x35) and, like them, the original
/// draws the whole file into the authored rect — these two are the clearest
/// cases of it.
const ROTATE_LEFT_RECT: (f32, f32, f32, f32) = (12.0, 11.0, 42.0, 35.0);
const ROTATE_ZOOM_RECT: (f32, f32, f32, f32) = (57.0, 11.0, 39.0, 35.0);
const ROTATE_RIGHT_RECT: (f32, f32, f32, f32) = (99.0, 11.0, 42.0, 35.0);

/// Yaw applied per rotate click. **Deviation / openroad choice:** the resinfo
/// tree authors the buttons but not what they do — no step, speed or limit
/// ships anywhere in the data. 15 deg is a 24-click full turn, which reads as a deliberate step on
/// both mouse and keyboard rather than a spin.
const PREVIEW_ROTATE_STEP: f32 = PI / 12.0;
/// Dolly of the zoomed-in create camera along its own view axis, in world
/// units. **Deviation / openroad choice:** `zoomin.ddj`/`zoomout.ddj` prove
/// the control is a two-state toggle, but no distance ships. The create pose
/// stands ~44 units off the podium (`assets/char_selects/*.selection`:
/// `cam_offset.z 697.6` vs `char_*_offset.z 654/644`), so 15 units is a
/// readable close-up that cannot pass through the body.
const PREVIEW_ZOOM_DOLLY: f32 = 15.0;

/// The height/volume wire byte. It is **not** a 0..255 scale: the two axes
/// share one byte, one **nibble per axis**. An untouched screen sends `0x22`;
/// values such as `0x11`, `0x00` and `0x20` show that the two nibbles move
/// independently.
/// Each axis has five steps — `UIO_NEWCHAR_EXPLANATION_HEIGHT` ("one of 5
/// levels from the thiniest to the tallest") and `_VOLUME` ("one of 5 types")
/// in `textdata/textuisystem.txt` — so a nibble is `0..=4` and `0x22` is the
/// middle step of both.
///
/// **Which nibble is which: low nibble = Height, high nibble = Volume.** From
/// the `0x22` default, a single Height step sends `0x23` and a single Volume
/// step sends `0x32`, so each axis moves only its own nibble. That is what
/// makes the two rows safe to arm.
const SCALE_STEPS: u8 = 5;
/// The middle of `0..=SCALE_STEPS - 1`, which is what an untouched screen sends.
const SCALE_MIDDLE_STEP: u8 = SCALE_STEPS / 2;

/// Packs the two axis steps the way the original serialises them (one nibble
/// each, in the create struct): `high` is
/// Volume, `low` is Height (see [`SCALE_STEPS`]).
const fn pack_scale(high: u8, low: u8) -> u8 {
    (high << 4) | (low & 0x0f)
}

/// The wire scale byte of a selection: Volume in the high nibble, Height in the
/// low one.
fn selection_scale(selection: &CharCreateSelection) -> u8 {
    pack_scale(
        selection.volume.min(SCALE_STEPS - 1),
        selection.height.min(SCALE_STEPS - 1),
    )
}

/// What an untouched screen sends: both axes on the middle step.
pub(super) const DEFAULT_SCALE: u8 = pack_scale(SCALE_MIDDLE_STEP, SCALE_MIDDLE_STEP);

/// Name length window of the original's own pre-send validation: the CN screen
/// checks the length of the name field (`GDR_EDIT_NAME`) against
/// `(unsigned)(len - 2) > 10`, i.e. it accepts exactly `2 ..= 12`. The message
/// is `UIO_MSG_ERROR_CHARACTER_NAME_STRING`, whose shipped text ends in the
/// literal `[Min., Max.]` because the call site passes no format arguments.
const MIN_NAME_LEN: usize = 2;
const MAX_NAME_LEN: usize = 12;

/// The name charset the original tests, as ranges rather than a bitmap.
///
/// Idea: the client does not hardcode a charset — it builds an 8 KiB
/// U+0000..U+FFFF bitmap from `textdata/abusefilter.txt` and tests one bit per
/// code unit,
/// rejecting with `UIO_MSG_ERROR_CHARACTER_WRONGSTRING` and sending no packet.
/// That file is an `#ALLOW_ID_TABLE lo hi` list in six language columns; column
/// 4 has exactly these four ranges (`0x30-0x39`, `0x41-0x5A`, `0x5F`,
/// `0x61-0x7A`).
///
/// **Which column this build selects is open.** The only candidates are column
/// 4 and column 5, and they differ *only* in the Latin-1 accents recorded
/// below — so we take the strict column, for a wire reason: our
/// `#[silkroad(size = 1)]` string serializer
/// emits a `u16` *byte* count followed by the UTF-8 bytes
/// (`sro_macro_derive/src/serialize.rs:66-70`), while the original writes one
/// byte per code unit, so `é` would already leave the client as two bytes and
/// the server would read a length that does not match the characters. Column 4
/// (strict ASCII alnum + `_`) is therefore the only charset that stays
/// wire-compatible without a second encoding path.
const NAME_ALLOWED_RANGES: [(char, char); 4] = [('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z')];
/// Column 5 of the same file = column 4 plus these three Latin-1 blocks.
/// Recorded, not consumed: it is the other candidate for the charset, and
/// keeping it here is what makes the choice above reviewable.
#[allow(dead_code)]
const NAME_ALLOWED_RANGES_COLUMN5_EXTRA: [(char, char); 3] = [
    ('\u{c0}', '\u{d6}'),
    ('\u{d9}', '\u{f6}'),
    ('\u{f9}', '\u{ff}'),
];

// --- Creation data model -----------------------------------------------------
//
// Everything below is enumerated from the game's PK2 tables at runtime,
// mirroring the original creation inputs (resinfo pscharactercreate*:
// GDR_SLI_FIGURE / GDR_SLI_WEAPON / GDR_SLI_PROTECTOR):
//
// - Bodies: the *figure variant* rows (CHAR_{CH,EU}_{MAN,WOMAN}_*, ids
//   1907–1932 for CH in this data) — no bare CHAR_XX_YY rows exist, and an
//   original-made character carries variant ref 1907. Display names come from the
//   UIO_NEWCHAR_{EU_}{MAN,FEMALE}_<SUFFIX> textuisystem keys ("Ryujoyeong",
//   "Eungyo", …). Races whose prefix yields no rows (European in this data)
//   are presented disabled.
// - Weapons: the degree-1 `_DEF` creation weapons per race (ids
//   3632–3636 CH / 10887–10896 EU), resolved by codename; entries missing
//   from the data drop out of the picker.
// - Protector: the three `_DEF` garment sets (CLOTHES / LIGHT / HEAVY) per
//   race/gender, again resolved by codename.
//
// The height/volume sliders are live: the nibbles are pinned (low = Height,
// high = Volume, five steps each, default 0x22). They are not data-driven
// like the rows above: their option count comes from the wire encoding itself.
// They also do not change the preview yet: how a step maps to a body scale
// factor is not in any data we hold, and inventing a factor would
// show the user a body the server will not build.

impl Garment {
    pub const ALL: [Garment; 3] = [Garment::Clothes, Garment::Light, Garment::Heavy];

    /// Next/previous set, wrapping — the row is one cycling control, the way
    /// the original's single `GDR_SLI_PROTECTOR` slider is.
    fn cycle(self, step: isize) -> Garment {
        let len = Self::ALL.len() as isize;
        let index = Self::ALL.iter().position(|g| *g == self).unwrap_or(0) as isize;
        Self::ALL[(index + step).rem_euclid(len) as usize]
    }

    /// Codename segment in `ITEM_{race}_{gender}_<SEGMENT>_01_<slot>_A_DEF`.
    fn segment(self) -> &'static str {
        match self {
            Garment::Clothes => "CLOTHES",
            Garment::Light => "LIGHT",
            Garment::Heavy => "HEAVY",
        }
    }

    /// Caption key + fallback per race (textuisystem: STT_CLOTHES "Garment",
    /// STT_LIGHT_ARMOR, STT_HEAVY_ARMOR; EU variants incl. ROBE).
    /// A race with no strings of its own falls back to the un-prefixed China
    /// keys: they are the generic set (`STT_CLOTHES`/`LIGHT_ARMOR`/
    /// `HEAVY_ARMOR`), so the row still reads as a garment row instead of
    /// showing a raw key.
    fn label(self, race: Race) -> (&'static str, &'static str) {
        let european = race == Race::EUROPEAN;
        match (european, self) {
            (false, Garment::Clothes) => ("UIO_NEWCHAR_STT_CLOTHES", "Garment"),
            (false, Garment::Light) => ("UIO_NEWCHAR_STT_LIGHT_ARMOR", "Protector"),
            (false, Garment::Heavy) => ("UIO_NEWCHAR_STT_HEAVY_ARMOR", "Armor"),
            (true, Garment::Clothes) => ("UIO_NEWCHAR_STT_EU_ROBE", "Robe"),
            (true, Garment::Light) => ("UIO_NEWCHAR_STT_EU_LIGHT_ARMOR", "Light Armor"),
            (true, Garment::Heavy) => ("UIO_NEWCHAR_STT_EU_HEAVY_ARMOR", "Heavy Armor"),
        }
    }
}

impl Race {
    /// Built from the race's own data code, so it exists for a race this
    /// program has never seen: the table spells its body rows
    /// `CHAR_<CODE>_{MAN,WOMAN}_<SUFFIX>`.
    fn body_prefix(self, gender: Gender) -> String {
        self.body_prefix_of(gender.body_segment())
    }

    /// The item table spells the same race code as the character table
    /// (`ITEM_CH_*` next to `CHAR_CH_*`), so this token is the code too.
    fn item_tokens(self, gender: Gender) -> (String, &'static str) {
        (
            self.code().to_string(),
            match gender {
                Gender::Male => "M",
                Gender::Female => "W",
            },
        )
    }

    /// Degree-1 creation weapons: (codename, caption key, fallback). The CH
    /// five and the EU list mirror the original GDR_SLI_WEAPON options; the
    /// per-weapon caption keys are the vanilla UIO_NEWCHAR_STT_* entries.
    /// A race whose weapon row the interface data does not author for us has
    /// no cited option list; it gets none rather than an invented one, and
    /// [`weapon_choices`] then leaves the row empty.
    fn weapon_options(self) -> &'static [(&'static str, &'static str, &'static str)] {
        match self {
            Race::CHINESE => &[
                ("ITEM_CH_SWORD_01_A_DEF", "UIO_NEWCHAR_STT_SWORD", "Sword"),
                ("ITEM_CH_BLADE_01_A_DEF", "UIO_NEWCHAR_STT_BLADE", "Blade"),
                ("ITEM_CH_SPEAR_01_A_DEF", "UIO_NEWCHAR_STT_SPEAR", "Spear"),
                (
                    "ITEM_CH_TBLADE_01_A_DEF",
                    "UIO_NEWCHAR_STT_TBLADE",
                    "Glaive",
                ),
                ("ITEM_CH_BOW_01_A_DEF", "UIO_NEWCHAR_STT_BOW", "Bow"),
            ],
            Race::EUROPEAN => &[
                (
                    "ITEM_EU_SWORD_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_ONEHANDSWORD",
                    "One-hand Sword",
                ),
                (
                    "ITEM_EU_TSWORD_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_TWOHANDSWORD",
                    "Two-hand Sword",
                ),
                (
                    "ITEM_EU_AXE_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_DUELAXE",
                    "Dual Axe",
                ),
                (
                    "ITEM_EU_DAGGER_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_DAGGER",
                    "Dagger",
                ),
                (
                    "ITEM_EU_CROSSBOW_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_CROSSBOW",
                    "Crossbow",
                ),
                (
                    "ITEM_EU_DARKSTAFF_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_DARKSTAFF",
                    "Warlock Rod",
                ),
                (
                    "ITEM_EU_TSTAFF_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_TWOHANDSTAFF",
                    "Two-hand Staff",
                ),
                (
                    "ITEM_EU_STAFF_01_A_DEF",
                    "UIO_NEWCHAR_STT_EU_ONEHANDSTAFF",
                    "Cleric Rod",
                ),
                ("ITEM_EU_HARP_01_A_DEF", "UIO_NEWCHAR_STT_EU_HARP", "Harp"),
            ],
            _ => &[],
        }
    }
}

/// The figure-variant body rows for a (race, gender), ordered by ref id (the
/// data's own creation order — 1907.. = ADVENTURER first, as the original
/// sends it). Empty when the data lacks the race (EU here). `pub(crate)`:
/// the region-select board uses it to grey out data-blocked races.
pub(crate) fn figure_variants(
    char_data: &ClientCharacterData,
    race: Race,
    gender: Gender,
) -> Vec<(i32, String)> {
    let Some(table) = char_data.data() else {
        return Vec::new();
    };
    let prefix = race.body_prefix(gender);
    let mut rows: Vec<(i32, String)> = table
        .iter()
        .filter(|(_, row)| row.in_service() && row.code_name().starts_with(&prefix))
        .map(|(id, row)| (*id, row.code_name().clone()))
        .collect();
    rows.sort_by_key(|(id, _)| *id);
    rows
}

/// textuisystem key of a figure's display name: codename suffix after the
/// body prefix, keyed as UIO_NEWCHAR_{EU_}{MAN|FEMALE}_<SUFFIX>.
fn figure_label_key(race: Race, gender: Gender, code_name: &str) -> String {
    let suffix = code_name
        .strip_prefix(&race.body_prefix(gender))
        .unwrap_or(code_name);
    // The key infix is authored per race in the string table, so it comes off
    // the race's presentation entry; an uncited race has no infix.
    let race_part = race.presentation().map(|p| p.ui_key_infix).unwrap_or("");
    let gender_part = match gender {
        Gender::Male => "MAN",
        Gender::Female => "FEMALE",
    };
    format!("UIO_NEWCHAR_{race_part}{gender_part}_{suffix}")
}

/// The weapon options of `race` that actually resolve in this data:
/// (ref id, caption key, fallback).
fn weapon_choices(
    race: Race,
    item_index: &ClientItemIndex,
) -> Vec<(i32, &'static str, &'static str)> {
    race.weapon_options()
        .iter()
        .filter_map(|(code, key, fallback)| item_index.id(code).map(|id| (id, *key, *fallback)))
        .collect()
}

fn garment_codename(race: Race, gender: Gender, set: Garment, slot: &str) -> String {
    let (r, g) = race.item_tokens(gender);
    format!("ITEM_{r}_{g}_{}_01_{slot}_A_DEF", set.segment())
}

/// Ref-obj-ids for the create request + preview.
struct StarterRefs {
    body: u32,
    chest: u32,
    pants: u32,
    boots: u32,
    weapon: u32,
}

/// Resolves the current selection to ref-obj-ids; `None` if the data lacks
/// the race's bodies, the picked garment set, or every weapon option. Indices
/// are clamped so a stale figure/weapon index (e.g. after a race switch)
/// still resolves.
fn resolve_starter(
    sel: &CharCreateSelection,
    char_data: &ClientCharacterData,
    item_index: &ClientItemIndex,
) -> Option<StarterRefs> {
    let figures = figure_variants(char_data, sel.race, sel.gender);
    let (body, _) = figures.get(sel.figure.min(figures.len().checked_sub(1)?))?;
    let weapons = weapon_choices(sel.race, item_index);
    let (weapon, _, _) = weapons.get(sel.weapon.min(weapons.len().checked_sub(1)?))?;
    let garment =
        |slot: &str| item_index.id(&garment_codename(sel.race, sel.gender, sel.garment, slot));
    Some(StarterRefs {
        body: *body as u32,
        chest: garment("BA")? as u32,
        pants: garment("LA")? as u32,
        boots: garment("FA")? as u32,
        weapon: *weapon as u32,
    })
}

/// Every 3D resource the creation screen will load for `sel`, as asset paths.
///
/// Idea: the region board's loading gauge has to count **real** work, the way
/// the original's does (its gauge is `loaded / 1000.0`), and the real work between
/// the board and the creation screen is exactly what [`update_preview`] is
/// about to ask the asset server for: the starter body plus its four equipment
/// models. Naming it here — next to the resolver both sides share — is what
/// keeps the gauge honest if the preview's asset set ever changes.
pub(crate) fn creation_preload_paths(
    sel: &CharCreateSelection,
    char_data: &ClientCharacterData,
    item_index: &ClientItemIndex,
    item_data: &ClientItemData,
) -> Vec<String> {
    let Some(refs) = resolve_starter(sel, char_data, item_index) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Some(path) = char_data
        .get(&(refs.body as i32))
        .and_then(|row| char_data.model_path(row))
    {
        paths.push(path);
    }
    for id in [refs.chest, refs.pants, refs.boots, refs.weapon] {
        if let Some(path) = item_data
            .get(&(id as i32))
            .and_then(|row| row.resource_path())
        {
            paths.push(path);
        }
    }
    paths
}

/// `GDR_STA_CUSTOM`: the customize panel, drawn as the baked
/// `customize_window[_europe].ddj` sprite with every child absolutely placed
/// at its `Section = Custom` rect.
///
/// Idea: the panel art already paints the frames the children sit in, so the
/// only correct layout is the authored one — no flex, no padding, nothing to
/// tune. Two things vary by race and are read from the race's own tree rather
/// than blended: the panel texture, and the order of the two gear rows.
///
/// The five option rows keep our existing prev/value/next cycler *inside* the
/// authored 120x24 row rect. Replacing that cycler with the `Section = Slider`
/// template (`:685-745`) is the next build step and a separate change; this
/// one is the texture/rect pass.
fn create_panel(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    race: Race,
) -> impl Scene {
    // The `_europe` art is the Europe tree's own panel; an uncited race gets
    // the un-suffixed panel, which is the generic one the rects come from.
    let panel_art = if race == Race::EUROPEAN {
        assets.customize_window_europe.clone()
    } else {
        assets.customize_window.clone()
    };
    let male_style = gender_button_style(assets, Gender::Male, true);
    let female_style = gender_button_style(assets, Gender::Female, false);
    let check_style = ImageButtonStyle {
        normal: assets.overlap.clone(),
        hover: assets.overlap_focus.clone(),
        press: assets.overlap_press.clone(),
        ..Default::default()
    };
    let male_font = fonts.nine.clone();
    let female_font = fonts.nine.clone();
    let name_font = fonts.nine.clone();
    let sex_font = fonts.nine.clone();
    let input_font = fonts.nine.clone();
    let check_font = fonts.nine.clone();
    let feedback_font = fonts.nine.clone();
    let male_sound = assets.sound_button_sound_a.clone();
    let female_sound = assets.sound_button_sound_a.clone();
    let check_sound = assets.sound_button_sound_a.clone();
    let male_text = ui_strings.get_or("UIO_NEWCHAR_CTL_MALE", "Male");
    let female_text = ui_strings.get_or("UIO_NEWCHAR_CTL_FEMALE", "Female");
    let name_text = ui_strings.get_or("UIO_NEWCHAR_STT_NAME", "Name");
    let sex_text = ui_strings.get_or("UIO_NEWCHAR_STT_SEX", "Sex");
    let check_text = ui_strings.get_or("UIO_NEWCHAR_CTL_CHECK_OVERLAP", "Unique check");
    let figure_text = ui_strings.get_or("UIO_NEWCHAR_STT_FIGURE", "Figure");
    let weapon_text = ui_strings.get_or("UIO_NEWCHAR_STT_WEAPON", "Weapon");
    let protector_text = ui_strings.get_or("UIO_NEWCHAR_STT_PROTECTOR", "Cloth");
    let height_text = ui_strings.get_or("UIO_NEWCHAR_STT_HEIGHT", "Height");
    let volume_text = ui_strings.get_or("UIO_NEWCHAR_STT_VOLUME", "Volume");
    let figure_caption_font = fonts.nine.clone();
    let weapon_caption_font = fonts.nine.clone();
    let garment_caption_font = fonts.nine.clone();
    let height_font = fonts.nine.clone();
    let volume_font = fonts.nine.clone();

    let caption_px = cu_font(intro_font_px(CAPTION_FONT_INDEX));
    let panel_button_px = cu_font(intro_font_px(PANEL_BUTTON_FONT_INDEX));
    let code_drawn_px = cu_font(intro_font_px(CODE_DRAWN_FONT_INDEX));
    let (nlx, nly, nlw, nlh) = NAME_LABEL_RECT;
    let (nex, ney, new_, neh) = NAME_EDIT_RECT;
    let (cbx, cby, cbw, cbh) = CHECK_RECT;
    let (slx, sly, slw, slh) = SEX_LABEL_RECT;
    let (mx, my, mw, mh) = MALE_RECT;
    let (fx, fy, fw, fh) = FEMALE_RECT;
    // The seven panel captions are LEFT aligned, not centred. `label_sized`
    // centres (every other intro caption wants that), but the original draws
    // this column flush: all seven start at x=110/111 against a panel left edge
    // of x=78 — that is `LABEL_X` (32) exactly, on one line. Centring them
    // inside the 45px box instead spreads the column by up to 8.7 px ("Sex"
    // landed at 65 where "Weapon" sat at 56), i.e. a ragged left edge. Only the
    // long captions were ever flush, and only because a word wider than its box
    // cannot be centred.
    let male_margin = gender_caption_margin(Gender::Male);
    let female_margin = gender_caption_margin(Gender::Female);
    let (protector_row, weapon_row) = gear_row_tops(race, ROW_GEAR_TOPS);
    let (protector_label, weapon_label) = gear_row_tops(race, LABEL_GEAR_TOPS);

    bsn! {
        CharCreateRoot
        Name("Character Create Panel V2")
        Node {
            position_type: PositionType::Absolute,
            left: percent(3),
            top: percent(22),
            width: cu(CUSTOM_W),
            height: cu(CUSTOM_H),
        }
        ImageNode { image: {panel_art} }
        TabGroup::new(0)
        Children [
            (
                label_sized(name_text, name_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(nlx), top: cu(nly), width: cu(nlw), height: cu(nlh) }
            ),
            // `GDR_EDIT_NAME` has no DDJ of its own: the slot is painted into
            // the panel art, so there is no background node here any more.
            (
                Node { position_type: PositionType::Absolute, left: cu(nex), top: cu(ney), width: cu(new_), height: cu(neh) }
                // The 12-char limit is an INPUT CAP, not an error case: in the
                // original, 15 typed characters land as 12 in the field, the
                // request carries 12, and NO message is shown.
                // `max_characters` is bevy's own edit-rejecting cap, so
                // the field can never hold an over-long name in the first place.
                Children [ (
                    text_input(input_font, 0)
                    EditableText { max_characters: {Some(MAX_NAME_LEN)} }
                    NameInput
                ) ]
            ),
            (
                image_button(check_style, cbw, cbh)
                ButtonSound({check_sound})
                Node { position_type: PositionType::Absolute, left: cu(cbx), top: cu(cby), width: cu(cbw), height: cu(cbh) }
                Children [ (label_sized(check_text, check_font, panel_button_px) TextColor(Color::WHITE)) ]
                on(on_check_name_activate)
            ),
            (
                label_sized(sex_text, sex_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(slx), top: cu(sly), width: cu(slw), height: cu(slh) }
            ),
            (
                image_button(male_style, mw, mh)
                GenderOption(Gender::Male)
                ButtonSound({male_sound})
                Node { position_type: PositionType::Absolute, left: cu(mx), top: cu(my), width: cu(mw), height: cu(mh) }
                Children [ (
                    label_sized(male_text, male_font, panel_button_px) TextColor(Color::WHITE)
                    Node { margin: UiRect { left: cu(male_margin.0), right: cu(male_margin.1), top: px(0.0), bottom: px(0.0) } }
                ) ]
                on(|_a: On<Activate>, mut sel: ResMut<CharCreateSelection>| {
                    if sel.gender != Gender::Male {
                        sel.gender = Gender::Male;
                        sel.figure = 0;
                    }
                })
            ),
            (
                image_button(female_style, fw, fh)
                GenderOption(Gender::Female)
                ButtonSound({female_sound})
                Node { position_type: PositionType::Absolute, left: cu(fx), top: cu(fy), width: cu(fw), height: cu(fh) }
                Children [ (
                    label_sized(female_text, female_font, panel_button_px) TextColor(Color::WHITE)
                    Node { margin: UiRect { left: cu(female_margin.0), right: cu(female_margin.1), top: px(0.0), bottom: px(0.0) } }
                ) ]
                on(|_a: On<Activate>, mut sel: ResMut<CharCreateSelection>| {
                    if sel.gender != Gender::Female {
                        sel.gender = Gender::Female;
                        sel.figure = 0;
                    }
                })
            ),
            // The five rows, all on the one `Section = Slider` template.
            // Row 1 — `GDR_SLI_FIGURE` (`:352-370`).
            (
                label_sized(figure_text, figure_caption_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(LABEL_X), top: cu(LABEL_FIGURE_TOP), width: cu(LABEL_W), height: cu(LABEL_H) }
            ),
            ( slider_row(assets, SliderRow::Figure, ROW_FIGURE_TOP) ),
            // Rows 2 and 3 — `GDR_SLI_HEIGHT` / `_VOLUME`, on the same
            // template as the other three. Armed since the nibble assignment
            // is known, not guessed: from the `0x22` default, Height +1 sends
            // `0x23` and Volume +1 `0x32`, so low = Height, high = Volume.
            // Before that they were deliberately a dimmed readout, because a
            // swapped guess mints permanently mis-shaped characters.
            (
                label_sized(height_text, height_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(LABEL_X), top: cu(LABEL_HEIGHT_TOP), width: cu(LABEL_W), height: cu(LABEL_H) }
            ),
            ( slider_row(assets, SliderRow::Height, ROW_HEIGHT_TOP) ),
            (
                label_sized(volume_text, volume_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(LABEL_X), top: cu(LABEL_VOLUME_TOP), width: cu(LABEL_W), height: cu(LABEL_H) }
            ),
            ( slider_row(assets, SliderRow::Volume, ROW_VOLUME_TOP) ),
            // The two gear rows: `protector_row`/`weapon_row` carry the race's
            // authored order, so CH reads Protector-then-Weapon and EU reads
            // Weapon-then-Protector without a second layout.
            (
                label_sized(protector_text, garment_caption_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(LABEL_X), top: cu(protector_label), width: cu(LABEL_W), height: cu(LABEL_H) }
            ),
            ( slider_row(assets, SliderRow::Protector, protector_row) ),
            (
                label_sized(weapon_text, weapon_caption_font, caption_px) TextColor(Color::WHITE)
                TextLayout::justify(Justify::Left)
                Node { position_type: PositionType::Absolute, left: cu(LABEL_X), top: cu(weapon_label), width: cu(LABEL_W), height: cu(LABEL_H) }
            ),
            ( slider_row(assets, SliderRow::Weapon, weapon_row) ),
            // Name-check feedback. `GDR_TEXT_MESSAGE` (`:196-214`) is the
            // original's message sink and its rect is `0,0,0,0` — placed by
            // client code, so unknown. **openroad choice:** the line goes in
            // the panel's free strip below the last row, next to the field it
            // is about, rather than into an invented floating box.
            (
                label_sized("", feedback_font, code_drawn_px) TextColor(Color::WHITE) NameFeedback
                Node { position_type: PositionType::Absolute, left: cu(LABEL_X), top: cu(FEEDBACK_TOP), width: cu(FEEDBACK_W), height: cu(LABEL_H) }
            ),
        ]
    }
}

/// One `CIFSliderCtrl` instance at `top`, built from the `Section = Slider`
/// template. Every child rect is the authored one; only which row it drives is
/// ours to say.
fn slider_row(assets: &IntroV2Assets, row: SliderRow, top: f32) -> impl Scene {
    let prev_style = ImageButtonStyle {
        normal: assets.slider_arrow_left.clone(),
        hover: assets.slider_arrow_left_focus.clone(),
        press: assets.slider_arrow_left_press.clone(),
        ..Default::default()
    };
    let next_style = ImageButtonStyle {
        normal: assets.slider_arrow_right.clone(),
        hover: assets.slider_arrow_right_focus.clone(),
        press: assets.slider_arrow_right_press.clone(),
        ..Default::default()
    };
    let thumb_style = ImageButtonStyle {
        normal: assets.slider_thumb.clone(),
        hover: assets.slider_thumb_focus.clone(),
        press: assets.slider_thumb_press.clone(),
        ..Default::default()
    };
    let prev_sound = assets.sound_button_sound_a.clone();
    let next_sound = assets.sound_button_sound_a.clone();
    let (px_, py, pw, ph) = SLIDER_PREV_RECT;
    let (tx, ty, tw, th) = SLIDER_THUMB_RECT;
    let (nx, ny, nw, nh) = SLIDER_NEXT_RECT;

    bsn! {
        Name("Character Create Slider Row")
        Node {
            position_type: PositionType::Absolute,
            left: cu(ROW_X),
            top: cu(top),
            width: cu(SLIDER_TEMPLATE_W),
            height: cu(ROW_H),
        }
        Children [
            (
                image_button(prev_style, pw, ph)
                SliderStep(row, {-1})
                ButtonSound({prev_sound})
                Node { position_type: PositionType::Absolute, left: cu(px_), top: cu(py), width: cu(pw), height: cu(ph) }
                on(on_slider_step)
            ),
            (
                // The thumb is a `CIFButton` in the data, but dragging it is
                // not something the tree describes; it reads the row's index
                // and is moved by `update_slider_thumbs`.
                ImageNode { image: {thumb_style.normal.clone()}, image_mode: NodeImageMode::Stretch }
                SliderThumb(row)
                Node { position_type: PositionType::Absolute, left: cu(tx), top: cu(ty), width: cu(tw), height: cu(th) }
                Pickable::IGNORE
            ),
            (
                image_button(next_style, nw, nh)
                SliderStep(row, 1)
                ButtonSound({next_sound})
                Node { position_type: PositionType::Absolute, left: cu(nx), top: cu(ny), width: cu(nw), height: cu(nh) }
                on(on_slider_step)
            ),
        ]
    }
}

/// How many steps a row has and where it currently stands.
fn slider_row_state(
    row: SliderRow,
    selection: &CharCreateSelection,
    char_data: &ClientCharacterData,
    item_index: &ClientItemIndex,
) -> (usize, usize) {
    let (index, count) = match row {
        SliderRow::Figure => (
            selection.figure,
            figure_variants(char_data, selection.race, selection.gender).len(),
        ),
        SliderRow::Protector => (
            Garment::ALL
                .iter()
                .position(|g| *g == selection.garment)
                .unwrap_or(0),
            Garment::ALL.len(),
        ),
        SliderRow::Weapon => (
            selection.weapon,
            weapon_choices(selection.race, item_index).len(),
        ),
        // Not data-driven: the five steps are the wire encoding's own.
        SliderRow::Height => (selection.height as usize, SCALE_STEPS as usize),
        SliderRow::Volume => (selection.volume as usize, SCALE_STEPS as usize),
    };
    (index.min(count.saturating_sub(1)), count)
}

/// `Activate` observer shared by every slider arrow: steps its row and focuses
/// it, so the Explain box follows the control the user just touched.
///
/// Touching the gear rows also *commits* them: the original writes the picked
/// ref into `this+0x12c` / `this+0x138` at that moment, and the two pre-send
/// gates read exactly those slots ([`gate_selection`]). A row with a single
/// option still commits — the click is the commit, not the value change.
pub fn on_slider_step(
    activate: On<Activate>,
    steps: Query<&SliderStep>,
    char_data: Res<ClientCharacterData>,
    item_index: Res<ClientItemIndex>,
    mut focus: ResMut<CharCreateFocus>,
    mut selection: ResMut<CharCreateSelection>,
) {
    let Ok(SliderStep(row, step)) = steps.get(activate.entity) else {
        return;
    };
    match row {
        SliderRow::Weapon => selection.weapon_chosen = true,
        SliderRow::Protector => selection.garment_chosen = true,
        SliderRow::Figure | SliderRow::Height | SliderRow::Volume => {}
    }
    // The two scale rows are a *range*, not a cycler: the original's five
    // steps run thinnest..tallest, so the ends stop instead of wrapping around
    // to the opposite extreme. Whether the original's own arrows wrap is not
    // known; clamping is the choice that cannot
    // surprise the user with a jump from tallest to thinnest.
    if matches!(row, SliderRow::Height | SliderRow::Volume) {
        let last = SCALE_STEPS - 1;
        let clamped = |current: u8| (current as i32 + *step).clamp(0, last as i32) as u8;
        match row {
            SliderRow::Height => selection.height = clamped(selection.height),
            SliderRow::Volume => selection.volume = clamped(selection.volume),
            _ => unreachable!(),
        }
        if focus.0 != *row {
            focus.0 = *row;
        } else {
            focus.set_changed();
        }
        return;
    }
    let (_, count) = slider_row_state(*row, &selection, &char_data, &item_index);
    if count > 1 {
        let advance = |current: usize| {
            (current as isize + *step as isize).rem_euclid(count as isize) as usize
        };
        match row {
            SliderRow::Figure => selection.figure = advance(selection.figure),
            SliderRow::Weapon => selection.weapon = advance(selection.weapon),
            SliderRow::Protector => {
                selection.garment = selection.garment.cycle(*step as isize);
            }
            // handled above, before the cycling rows
            SliderRow::Height | SliderRow::Volume => {}
        }
    }
    if focus.0 != *row {
        focus.0 = *row;
    } else {
        // The row did not change, but its VALUE did — nudge the resource so the
        // Explain box re-reads it.
        focus.set_changed();
    }
}

/// Moves each thumb to its row's current index.
pub fn update_slider_thumbs(
    selection: Res<CharCreateSelection>,
    char_data: Res<ClientCharacterData>,
    item_index: Res<ClientItemIndex>,
    mut thumbs: Query<(&SliderThumb, &mut Node)>,
) {
    for (thumb, mut node) in thumbs.iter_mut() {
        let (index, count) = slider_row_state(thumb.0, &selection, &char_data, &item_index);
        node.left = cu(slider_thumb_left(index, count));
    }
}

/// `GDR_BTN_MALE` / `GDR_BTN_FEMALE`: the original expresses the *selected*
/// state as a texture swap — `man_on.ddj` when male is picked, `man_off.ddj`
/// when it is not — and gives both captions the identical `255,249,212`, so
/// there is no colour cue to read.
fn gender_button_style(assets: &IntroV2Assets, gender: Gender, selected: bool) -> ImageButtonStyle {
    match (gender, selected) {
        (Gender::Male, true) => ImageButtonStyle {
            normal: assets.man_on.clone(),
            hover: assets.man_on_focus.clone(),
            press: assets.man_on_press.clone(),
            ..Default::default()
        },
        (Gender::Male, false) => ImageButtonStyle {
            normal: assets.man_off.clone(),
            hover: assets.man_off_focus.clone(),
            press: assets.man_off_press.clone(),
            ..Default::default()
        },
        (Gender::Female, true) => ImageButtonStyle {
            normal: assets.woman_on.clone(),
            hover: assets.woman_on_focus.clone(),
            press: assets.woman_on_press.clone(),
            ..Default::default()
        },
        (Gender::Female, false) => ImageButtonStyle {
            normal: assets.woman_off.clone(),
            hover: assets.woman_off_focus.clone(),
            press: assets.woman_off_press.clone(),
            ..Default::default()
        },
    }
}

/// The original's preview controls: `GDR_STA_ROTATE`'s 152x56 window with
/// `Section = Rotate`'s three buttons at their authored child rects.
///
/// Idea: the window is a fixed-size sprite, so its children are absolutely
/// placed inside it with the resinfo rects used verbatim — no flex, no
/// padding, nothing to tune. The screen POSITION of the window is not in the
/// data either (`0,0,152,56` is size only); it is anchored to the bottom-right
/// corner at the inset the original uses — see [`ROTATE_WINDOW_RIGHT_INSET`]
/// for the numbers, the cross-check and why this is an edge inset rather than
/// a scaled position.
fn create_rotate_window(assets: &IntroV2Assets) -> impl Scene {
    let window = assets.rotate_window.clone();
    let left_style = ImageButtonStyle {
        normal: assets.rotate_left.clone(),
        hover: assets.rotate_left_focus.clone(),
        press: assets.rotate_left_press.clone(),
        ..Default::default()
    };
    let right_style = ImageButtonStyle {
        normal: assets.rotate_right.clone(),
        hover: assets.rotate_right_focus.clone(),
        press: assets.rotate_right_press.clone(),
        ..Default::default()
    };
    let zoom_style = zoom_button_style(assets, false);
    let left_sound = assets.sound_button_sound_a.clone();
    let right_sound = assets.sound_button_sound_a.clone();
    let zoom_sound = assets.sound_button_sound_a.clone();
    let (lx, ly, lw, lh) = ROTATE_LEFT_RECT;
    let (zx, zy, zw, zh) = ROTATE_ZOOM_RECT;
    let (rx, ry, rw, rh) = ROTATE_RIGHT_RECT;

    bsn! {
        CharCreateRoot
        Name("Character Create Rotate Window")
        Node {
            position_type: PositionType::Absolute,
            bottom: cu(ROTATE_WINDOW_BOTTOM_INSET),
            right: cu(ROTATE_WINDOW_RIGHT_INSET),
            width: cu(ROTATE_WINDOW_W),
            height: cu(ROTATE_WINDOW_H),
        }
        ImageNode { image: {window} }
        Children [
            (
                image_button(left_style, lw, lh)
                PreviewRotateButton({-PREVIEW_ROTATE_STEP})
                ButtonSound({left_sound})
                Node { position_type: PositionType::Absolute, left: cu(lx), top: cu(ly), width: cu(lw), height: cu(lh) }
                on(|activate: On<Activate>,
                    buttons: Query<&PreviewRotateButton>,
                    mut view: ResMut<CharCreatePreviewView>| {
                    if let Ok(button) = buttons.get(activate.entity) {
                        view.yaw += button.0;
                    }
                })
            ),
            (
                image_button(zoom_style, zw, zh)
                PreviewZoomButton
                ButtonSound({zoom_sound})
                Node { position_type: PositionType::Absolute, left: cu(zx), top: cu(zy), width: cu(zw), height: cu(zh) }
                on(|_a: On<Activate>, mut view: ResMut<CharCreatePreviewView>| {
                    view.zoomed_in = !view.zoomed_in;
                })
            ),
            (
                image_button(right_style, rw, rh)
                PreviewRotateButton(PREVIEW_ROTATE_STEP)
                ButtonSound({right_sound})
                Node { position_type: PositionType::Absolute, left: cu(rx), top: cu(ry), width: cu(rw), height: cu(rh) }
                on(|activate: On<Activate>,
                    buttons: Query<&PreviewRotateButton>,
                    mut view: ResMut<CharCreatePreviewView>| {
                    if let Ok(button) = buttons.get(activate.entity) {
                        view.yaw += button.0;
                    }
                })
            ),
        ]
    }
}

/// `GDR_BTN_ZOOM` is authored with `zoomin.ddj`; `zoomout.ddj` (and both
/// their focus/press variants) ship alongside it, so the zoomed-in state
/// shows the way back out.
fn zoom_button_style(assets: &IntroV2Assets, zoomed_in: bool) -> ImageButtonStyle {
    if zoomed_in {
        ImageButtonStyle {
            normal: assets.zoomout.clone(),
            hover: assets.zoomout_focus.clone(),
            press: assets.zoomout_press.clone(),
            ..Default::default()
        }
    } else {
        ImageButtonStyle {
            normal: assets.zoomin.clone(),
            hover: assets.zoomin_focus.clone(),
            press: assets.zoomin_press.clone(),
            ..Default::default()
        }
    }
}

// --- The bottom-right Confirm / Cancel row -----------------------------------
//
// Idea: like the other position-less widgets of this screen, `GDR_BTN_OK` and
// `GDR_BTN_BACK` carry a size but no position (`pscharactercreate{china,
// _europe}.txt:72,91` -> `Rect=RECT,"0,0,92,41"`); the original places them
// from code. So the three numbers below come from where the original puts the
// row, not from a data citation. In an 800x600 client area its opaque pixel
// extents are:
//
//   Confirm x 591..682, Cancel x 695..786, both y 542..582
//   -> right inset 800-787 = 13, bottom inset 600-583 = 17, pitch 104
//
// Anchoring is by inset from the bottom-right corner rather than by a
// fraction of the window: the original's numbers are an inset, and the same
// corner is what `GDR_STA_ROTATE` is anchored to.
//
// The inset is in the screen's own unit ([`cu`]), i.e. the same 1:1 pixels as
// the art it separates — which is what the geometry tests below check.

/// Distance from the window's bottom edge to the row's bottom edge.
/// 17 px (see above). The 13 px right inset is shared with `GDR_STA_ROTATE`,
/// whose frame ends at the same x=786 — that agreement between two
/// independent widgets is what makes 13 credible.
const CONTROL_ROW_BOTTOM_INSET: f32 = 17.0;
/// Distance from the window's right edge to Cancel's right edge, 13 px.
const CONTROL_ROW_RIGHT_INSET: f32 = 13.0;
/// Gap between the two buttons. Derived, not chosen: the original's left edges
/// are 591 and 695, i.e. a pitch of 104 px, and our button art is 91 px wide
/// (`interface/outer/button.ddj` is 91x40; the authored rect rounds it up to
/// 92x41), so 104 - 91 = 13 puts both outer edges within 1 px of the original.
const CONTROL_ROW_GAP: f32 = MAIN_BUTTON_PITCH - MAIN_BUTTON_W;
/// Left-edge distance between Confirm and Cancel in the original: 695 - 591.
const MAIN_BUTTON_PITCH: f32 = 104.0;
/// Drawn width/height of the shared `button.ddj` widget, as every other
/// intro-v2 screen spawns it.
const MAIN_BUTTON_W: f32 = 91.0;
const MAIN_BUTTON_H: f32 = 41.0;

/// Where the Confirm/Cancel row lands in a window of `w` x `h`, as
/// `(confirm_left, cancel_right, top, bottom)`. Exists so the anchor can be
/// checked against the original's numbers without a running app — and for
/// nothing else, hence the gate.
#[cfg(test)]
fn control_row_box(w: f32, h: f32) -> (f32, f32, f32, f32) {
    let cancel_right = w - cu_at(CONTROL_ROW_RIGHT_INSET);
    let confirm_left =
        cancel_right - cu_at(MAIN_BUTTON_W) - cu_at(CONTROL_ROW_GAP) - cu_at(MAIN_BUTTON_W);
    let bottom = h - cu_at(CONTROL_ROW_BOTTOM_INSET);
    (
        confirm_left,
        cancel_right,
        bottom - cu_at(MAIN_BUTTON_H),
        bottom,
    )
}

/// Bottom-right Confirm / Cancel row
/// (original: `GDR_BTN_OK` / `GDR_BTN_BACK`).
fn create_controls(
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let create_font = fonts.nine.clone();
    let back_font = fonts.nine.clone();
    let button_px = cu_font(intro_font_px(MAIN_BUTTON_FONT_INDEX));
    let create_sound = assets.sound_button_sound_a.clone();
    let back_sound = assets.sound_button_sound_a.clone();
    // `GDR_BTN_OK` (pscharactercreate{china,_europe}.txt:82) carries
    // `UIO_NEWCHAR_CTL_CONFIRM` -> "Confirm". `UIO_COMMON_CTL_CREATE`
    // ("Create") is the *modal's* button, not this one.
    let create_text = ui_strings.get_or("UIO_NEWCHAR_CTL_CONFIRM", "Confirm");
    let back_text = ui_strings.get_or("UIO_COMMON_CTL_CANCEL", "Cancel");

    bsn! {
        CharCreateRoot
        Name("Character Create Controls V2")
        Node {
            position_type: PositionType::Absolute,
            flex_direction: FlexDirection::Row,
            bottom: cu(CONTROL_ROW_BOTTOM_INSET),
            right: cu(CONTROL_ROW_RIGHT_INSET),
            column_gap: cu(CONTROL_ROW_GAP),
        }
        Children [
            (
                image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                // `image_button` sizes itself in logical pixels; on this screen
                // the size is authored in 800x600 units, so the Node is patched
                // to the same unit as everything else here (see `cu`).
                Node { width: cu(MAIN_BUTTON_W), height: cu(MAIN_BUTTON_H) }
                CreateButton
                ButtonSound({create_sound})
                Children [ (label_sized(create_text, create_font, button_px) TextColor(Color::WHITE)) ]
                on(on_create_activate)
            ),
            (
                image_button(main_button_style(assets), MAIN_BUTTON_W, MAIN_BUTTON_H)
                Node { width: cu(MAIN_BUTTON_W), height: cu(MAIN_BUTTON_H) }
                ButtonSound({back_sound})
                Children [ (label_sized(back_text, back_font, button_px) TextColor(Color::WHITE)) ]
                on(|_a: On<Activate>, mut next_state: ResMut<NextState<IntroV2State>>| {
                    next_state.set(IntroV2State::CharacterList);
                })
            ),
        ]
    }
}

/// `OnEnter(CharacterCreate)`: clears the keep-alive marker and spawns the
/// creation UI; the preview is spawned by [`update_preview`] on the first
/// change of the freshly-inserted [`CharCreateSelection`].
pub fn enter_character_create(
    mut commands: Commands,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    asset_server: Res<AssetServer>,
    selection: Option<Res<CharCreateSelection>>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    commands.remove_resource::<EnteringCharacterCreate>();
    // The race is picked on the region-select board, which inserts the
    // resource before the state switch; `init_resource` keeps that choice and
    // only fills in a default when the screen is entered some other way.
    let race = selection.map(|s| s.race).unwrap_or_default();
    commands.init_resource::<CharCreateSelection>();
    commands.init_resource::<CharCreatePreviewView>();
    commands.init_resource::<CharCreateFocus>();

    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };
    commands
        .spawn_scene(create_panel(&assets, &fonts, &ui_strings, race))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(create_controls(&assets, &fonts, &ui_strings))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(create_rotate_window(&assets))
        .insert((UiTargetCamera(camera), IntroV2Ui));

    // `GDR_STA_TITLE` (:215-233): a baked 428x36 image at 47,111 in the
    // 1600x1200 design space — the same shape region-select already uses for
    // `text-region.ddj`, so the position is expressed as the design-space
    // fraction and the art keeps its native size.
    commands.spawn((
        CharCreateRoot,
        IntroV2Ui,
        UiTargetCamera(camera),
        Name::from("Character Create Title"),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(47.0 / 16.0),
            top: Val::Percent(111.0 / 12.0),
            width: cu(TITLE_ART_W),
            height: cu(TITLE_ART_H),
            ..default()
        },
        ImageNode::new(asset_server.load(TITLE_CUSTOM_DDJ)),
        Pickable::IGNORE,
    ));

    // `GDR_STA_EXPLAIN` + `Section = Explain`: the figure's vanilla name and
    // backstory on the screen's own panel art, replacing the hand-built box
    // that borrowed the char-select chrome. The European tree swaps both the
    // art and the size (see EXPLAIN_EU_* for the one rect/art mismatch).
    let (explain_art, (explain_w, explain_h)) = if race == Race::EUROPEAN {
        (assets.explain_window_02.clone(), EXPLAIN_EU_ART)
    } else {
        (assets.explain_window.clone(), EXPLAIN_CH_SIZE)
    };
    commands
        .spawn((
            CharCreateRoot,
            IntroV2Ui,
            UiTargetCamera(camera),
            Name::from("Character Create Story V2"),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Percent(3.0),
                top: Val::Percent(22.0),
                width: cu(explain_w),
                height: cu(explain_h),
                ..default()
            },
            ImageNode::new(explain_art),
            Pickable::IGNORE,
        ))
        .with_children(|panel| {
            panel.spawn((
                FigureStoryTitle,
                Text::new(""),
                TextFont {
                    font: fonts.nine.clone().into(),
                    font_size: cu_font(intro_font_px(EXPLAIN_FONT_INDEX)),
                    ..default()
                },
                TextColor(EXPLAIN_NAME_COLOR),
                explain_child_node(EXPLAIN_NAME_RECT),
                Pickable::IGNORE,
            ));
            panel.spawn((
                FigureStoryText,
                Text::new(""),
                TextFont {
                    font: fonts.nine.clone().into(),
                    font_size: cu_font(intro_font_px(EXPLAIN_FONT_INDEX)),
                    ..default()
                },
                TextColor(Color::WHITE),
                explain_child_node(EXPLAIN_BODY_RECT),
                Pickable::IGNORE,
            ));
        });
}

/// A `Section = Explain` child at its authored rect, in the panel's own space.
fn explain_child_node(rect: (f32, f32, f32, f32)) -> Node {
    let (left, top, width, height) = rect;
    Node {
        position_type: PositionType::Absolute,
        left: cu(left),
        top: cu(top),
        width: cu(width),
        height: cu(height),
        ..default()
    }
}

/// The race whose creation stage we are standing on. Inserted by the region
/// board before the state switch (`region_select.rs`); a screen entered any
/// other way (dev jump) gets the default.
pub fn staged_race(selection: Option<&CharCreateSelection>) -> Race {
    selection.map(|s| s.race).unwrap_or_default()
}

/// `OnEnter(CharacterCreate)`: moves the floating world origin to the creation
/// stage's own region.
///
/// The idea: the original does not walk the camera from the selection stage to
/// the creation stage — each creation screen *is* a different place in the
/// world and says so itself, by writing a region id plus a local position into
/// the stage anchor. We already have that
/// mechanism: terrain, map objects, compounds, foliage and environment all
/// stream around [`WorldOrigin`], so moving the origin *is* moving the stage —
/// no second loader, no per-screen scene graph.
///
/// This is also why the region-board's loading screen finally has
/// something to cover: China's `168/98` is half a map away from the selection
/// stage's `81/105`, and the streamer unloads the old ring as soon as the
/// camera is out of its range (`plugins/map/terrain/mod.rs`).
///
/// The way back needs no counterpart: leaving creation always lands in
/// `CharacterList`, whose `OnEnter` chain starts with `set_origin_to_char_select`.
pub fn set_origin_to_create_stage(
    scene: Res<ActiveCharSelectSceneV2>,
    selection: Option<Res<CharCreateSelection>>,
    mut origin: ResMut<WorldOrigin>,
    mut terrain: Query<&mut Transform, With<Terrain>>,
) {
    let race = staged_race(selection.as_deref());
    let anchor = scene.0.create_stage_anchor(race);
    set_world_origin(anchor, &mut origin, &mut terrain);
}

/// `OnEnter(CharacterCreate)`: snaps the stage camera to the creation stage's
/// authored pose. One key per race, so this is a pose-set and never a tween —
/// the original does not animate here either.
pub fn set_create_camera_pose(
    mut cam_query: Query<&mut Transform, With<CinematicCamera2>>,
    scene: Res<ActiveCharSelectSceneV2>,
    selection: Option<Res<CharCreateSelection>>,
    origin: Res<WorldOrigin>,
    mut metrics: ResMut<CreateFigureMetrics>,
) {
    // The body of the *previous* visit must not frame this one: the metrics are
    // re-measured as soon as its meshes are on stage, and until then the
    // authored pose is what is shown.
    *metrics = CreateFigureMetrics::default();
    let Ok(mut transform) = cam_query.single_mut() else {
        return;
    };
    let (translation, rotation) =
        create_camera_pose(&scene, staged_race(selection.as_deref()), &origin);
    transform.translation = translation;
    transform.rotation = rotation;
}

/// The held creation pose of `race`.
fn create_camera_pose(
    scene: &ActiveCharSelectSceneV2,
    race: Race,
    origin: &WorldOrigin,
) -> (Vec3, Quat) {
    scene.0.create_camera_pose(race, origin.0)
}

// --- Framing the figure: zoom, sole line and screen axis ---------------------
//
// Idea: the authored stage pose says from *which side* the body is seen; it does
// not say how large it is on screen or where it stands in the frame. The
// original client's own screen answers all three, so those three numbers are
// taken from it and applied to the authored pose as a two-step transform,
// never as a second hand-written pose:
//
//   1. move the camera along its own view axis until the body — measured on
//      stage, not assumed — covers [`FIGURE_HEIGHT_FRACTION`] of the viewport,
//   2. aim the camera so that the point the body stands on lands at
//      [`FIGURE_SOLE_Y_FRACTION`] / [`FIGURE_AXIS_X_FRACTION`] of the viewport.
//
// Keeping the authored pose as the anchor is what makes this a *framing* and not
// a new source of truth: change the stage data and the frame follows.

/// How tall the body must be on screen, as a fraction of the client height.
///
/// Not chosen: in the original the body runs from crown y=62 to sole y=553 of
/// 600 — **82.0 %** of the client height. Ours was 43.4 %, the same at two
/// resolutions (43.6 % / 43.2 %), i.e. a 1.89x gap; that ratio is *derived*
/// from this fraction, so the fraction is what the code carries. Error band:
/// 1.83..1.94 on the ratio, i.e. 79.4..84.2 % here.
const FIGURE_HEIGHT_FRACTION: f32 = 0.82;

/// Where the body's sole sits, as a fraction of the client height.
///
/// In the original the sole sits at y=553 of 600 = 92.17 %. That is
/// deliberately *inside* the bottom band (which starts at y=515): the feet
/// stand 38 px into the band, 5 px above its middle — the "standing on the red
/// bar" the screen is supposed to show.
const FIGURE_SOLE_Y_FRACTION: f32 = 0.9217;

/// Where the body's axis sits, as a fraction of the client width.
///
/// In the original the head axis sits at x=465.5 and the foot group at x=460.5
/// of 800 → 57.9 %, i.e. 7.9 % right of centre, in the gap between the
/// customise window (ends at x=335) and the info box (starts at x=575). Ours
/// stood dead centre.
const FIGURE_AXIS_X_FRACTION: f32 = 0.579;

/// Solver passes of the framing. **Choice**, not a given: both loops
/// converge geometrically (the aim re-levels the horizon, which moves the target
/// again by a second-order amount; the distance correction is a ratio that
/// squares its error), and four passes put both residuals below a hundredth of a
/// pixel for the angles this screen uses. The loop is cheap: it is arithmetic on
/// one point, on one camera.
const FRAMING_PASSES: usize = 4;

/// The authored pose of the stage, framed on a body of `height` whose soles
/// stand at `sole`.
///
/// Pure on purpose: the whole framing is arithmetic on (pose, sole, height, fov,
/// aspect), so the placement can be asserted in a unit test instead of being
/// judged by eye.
///
/// Two steps, in this order:
///
/// 1. **distance.** A body of world height `h` seen from depth `d` covers
///    `h / (2 d tan(fov/2))` of the viewport, so the depth that makes it cover
///    [`FIGURE_HEIGHT_FRACTION`] is `h / (2 f tan(fov/2))`. The camera keeps the
///    authored *direction* onto the body and only moves along it — the stage
///    data still says from which side the screen looks.
/// 2. **aim.** Rotate so the sole lands on its target screen lines. This step
///    is iterative rather than closed-form because the camera must stay
///    **roll-free**: a single minimal-arc rotation would place the sole exactly
///    but tilt the horizon by a degree or two. Each pass rotates the target onto
///    its mark and then rebuilds the rotation from the resulting view direction
///    with world-up, which is roll-free by construction.
///
/// Measuring the body instead of scaling the pose by the 1.89x ratio is what
/// makes this resolution- and body-independent: dollying by the ratio comes out
/// at 93.0 % of the height instead of 82.0 %,
/// because apparent size is only ~1/distance for a *point* — a body half as far
/// away grows by more than its distance ratio.
fn framed_create_camera_pose(
    pose: (Vec3, Quat),
    sole: Vec3,
    height: f32,
    fov: f32,
    aspect: f32,
) -> (Vec3, Quat) {
    let centre = sole + Vec3::Y * (height * 0.5);
    let to_camera = pose.0 - centre;
    let tan_v = (fov * 0.5).tan();
    // Degenerate input (camera standing in the body, or no body measured yet)
    // has nothing to frame; hand the authored pose back rather than divide by
    // zero and put NaNs on screen.
    if !to_camera.is_finite() || to_camera.length() < 1e-3 || !(height > 0.0) || tan_v <= 0.0 {
        return pose;
    }
    let axis = to_camera.normalize();
    let crown = sole + Vec3::Y * height;
    let mut distance = height / (2.0 * FIGURE_HEIGHT_FRACTION * tan_v);
    let mut framed = pose;
    for _ in 0..FRAMING_PASSES {
        framed = aimed_pose(centre + axis * distance, sole, framed.1, fov, aspect);
        // What the body actually covers from here. It is not exactly
        // `h / (2 d tan)`: that closed form holds for a body on the view axis,
        // and this one deliberately stands off-axis (57.9 % across, sole at
        // 92.17 % down), where the perspective divide compresses it. So the
        // distance is corrected by what the projection reports, which converges
        // in two passes and is exact at the end of the third.
        let covered =
            viewport_y(framed, sole, fov, aspect) - viewport_y(framed, crown, fov, aspect);
        if !(covered > 1e-4) {
            break;
        }
        distance *= covered / FIGURE_HEIGHT_FRACTION;
    }
    framed
}

/// `translation`, rotated so `target` lands on the measured screen lines.
///
/// Iterative rather than closed-form because the camera must stay **roll-free**:
/// a single minimal-arc rotation would place the target exactly but tilt the
/// horizon by a degree or two. Each pass rotates the target onto its mark and
/// then rebuilds the rotation from the resulting view direction with world-up,
/// which is roll-free by construction.
fn aimed_pose(
    translation: Vec3,
    target: Vec3,
    fallback: Quat,
    fov: f32,
    aspect: f32,
) -> (Vec3, Quat) {
    let tan_v = (fov * 0.5).tan();
    // Where the target must land, as a direction in camera space: the near-plane
    // point of the target NDC, which is what the perspective divide inverts.
    let ndc_x = 2.0 * FIGURE_AXIS_X_FRACTION - 1.0;
    let ndc_y = 1.0 - 2.0 * FIGURE_SOLE_Y_FRACTION;
    let mark = Vec3::new(ndc_x * tan_v * aspect, ndc_y * tan_v, -1.0).normalize();

    let to_target = (target - translation).normalize();
    let mut rotation = level(look_rotation(to_target), fallback);
    for _ in 0..FRAMING_PASSES {
        let seen = (rotation.inverse() * to_target).normalize();
        rotation = rotation * Quat::from_rotation_arc(mark, seen);
        rotation = level(rotation, rotation);
    }
    (translation, rotation)
}

/// Where `point` lands vertically, as a fraction of the viewport (0 = top).
fn viewport_y(pose: (Vec3, Quat), point: Vec3, fov: f32, aspect: f32) -> f32 {
    let _ = aspect;
    let view = pose.1.inverse() * (point - pose.0);
    if view.z >= -1e-4 {
        return f32::NAN;
    }
    let ndc_y = (view.y / -view.z) / (fov * 0.5).tan();
    (1.0 - ndc_y) * 0.5
}

/// What the framing needs to know about the body actually on stage: where its
/// soles are and how tall it is, in world units.
///
/// Measured from the preview's own meshes rather than assumed, the same way the
/// paper doll frames its clone (`plugins::hud::inventory::paperdoll`): the
/// starter bodies differ per race and gender, and the figure the screen shows is
/// the one the frame has to fit.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct CreateFigureMetrics {
    pub sole: Vec3,
    pub height: f32,
    /// Mesh-set fingerprint, so the frame is re-solved when the body or its
    /// equipment changes and *not* every frame — a skinned body sways in its
    /// stand animation, and re-framing on that would make the camera breathe.
    signature: u64,
}

/// Measure the preview body whenever its mesh set changes.
pub fn measure_create_figure(
    previews: Query<Entity, With<CharCreatePreview>>,
    children: Query<&Children>,
    meshes: Query<
        (
            &GlobalTransform,
            &bevy::camera::primitives::Aabb,
            &Visibility,
        ),
        With<Mesh3d>,
    >,
    mut metrics: ResMut<CreateFigureMetrics>,
) {
    let Ok(root) = previews.single() else {
        return;
    };
    let mut signature = 0u64;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for entity in children.iter_descendants(root) {
        let Ok((global, aabb, visibility)) = meshes.get(entity) else {
            continue;
        };
        if *visibility == Visibility::Hidden {
            continue;
        }
        signature ^= entity.to_bits();
        // The eight corners, because a rotated body's world box is not its local
        // box transformed corner-to-corner.
        for corner in 0..8 {
            let sign = Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
            let local = Vec3::from(aabb.center) + Vec3::from(aabb.half_extents) * sign;
            let world = global.transform_point(local);
            min = min.min(world);
            max = max.max(world);
        }
    }
    if signature == 0 || signature == metrics.signature || !min.is_finite() || !max.is_finite() {
        return;
    }
    metrics.signature = signature;
    metrics.height = max.y - min.y;
    metrics.sole = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
}

/// Hold the create camera on the measured frame.
///
/// Runs every frame rather than on a change: the metrics, the window size and
/// the zoom toggle all feed the same pose, and one owner of the camera transform
/// is what keeps them from fighting over it.
pub fn frame_create_camera(
    metrics: Res<CreateFigureMetrics>,
    // Optional so a run without the screen's own resources (the headless
    // netcheck harness registers none) skips this system instead of failing the
    // whole schedule's parameter validation.
    view: Option<Res<CharCreatePreviewView>>,
    scene: Res<ActiveCharSelectSceneV2>,
    selection: Option<Res<CharCreateSelection>>,
    origin: Res<WorldOrigin>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut camera: Query<(&mut Transform, &Projection), With<CinematicCamera2>>,
) {
    if metrics.height <= 0.0 {
        return;
    }
    let Ok((mut transform, projection)) = camera.single_mut() else {
        return;
    };
    let race = staged_race(selection.as_deref());
    let aspect = window.iter().next().map(viewport_aspect).unwrap_or(1.0);
    let (translation, rotation) = framed_create_camera_pose(
        create_camera_pose(&scene, race, &origin),
        metrics.sole,
        metrics.height,
        camera_fov(projection),
        aspect,
    );
    // The zoom toggle dollies from the held frame, so it stays an offset from
    // the screen's own framing rather than a second framing of its own.
    let dolly = if view.is_some_and(|view| view.zoomed_in) {
        PREVIEW_ZOOM_DOLLY
    } else {
        0.0
    };
    transform.translation = translation + rotation * (Vec3::NEG_Z * dolly);
    transform.rotation = rotation;
}

// --- The figure overlay pass -------------------------------------------------
//
// Idea: in the original the body stands **in front of** the screen's ornamental
// bands and its title — the shoes are painted over the bottom band's gold trim,
// the hair over the top band's edge, and the headline's "E" is cut off by the
// silhouette. Ours is a 3d
// character while the bands are `bevy_ui`, and between a 3d pass and the UI pass
// it is the camera `order` that decides, not `ZIndex` (which only ever orders UI
// nodes against each other). So the figure gets its own render layer and a
// second camera ordered *after* the UI camera.
//
// This is not a new construction: the paper doll already draws a character on
// its own layer with its own camera (`plugins::hud::inventory::paperdoll`). The
// difference is only the target — the doll renders into an image, this one
// renders straight onto the window on top of the UI, so no render-to-texture
// layer (with its size/resize/resolution questions) is needed.
//
// What is deliberately NOT done: the figure is not put in front of *everything*.
// the original draws a modal dialog over the figure, so the overlay switches
// off while the create-confirm modal is up
// ([`hide_figure_overlay_behind_modal`]). Panels never overlap the figure at all
// (57 px clear on the left, 24 px on the right).

/// The camera that redraws the preview figure over the UI chrome.
#[derive(Component)]
pub struct FigureOverlayCamera;

/// Camera order of the overlay pass: after the world (0) and after the UI (1).
const FIGURE_OVERLAY_ORDER: isize = 2;

/// `OnEnter(CharacterCreate)`: the overlay camera. Pose and projection are
/// copied from the create camera every frame ([`sync_figure_overlay_camera`]),
/// so there is exactly one framing and this pass cannot drift away from it.
pub fn spawn_figure_overlay_camera(
    mut commands: Commands,
    existing: Query<Entity, With<FigureOverlayCamera>>,
    config: Res<crate::plugins::config::ClientConfig>,
    mut images: ResMut<Assets<Image>>,
) {
    if !existing.is_empty() {
        return;
    }
    let camera = commands
        .spawn((
            FigureOverlayCamera,
            Name::from("Create Figure Overlay Camera"),
            Camera3d::default(),
            Camera {
                order: FIGURE_OVERLAY_ORDER,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            RenderLayers::layer(CameraLayers::CreateFigure.into()),
            Transform::default(),
            // metal on the starter gear reflects the sky probe, exactly as it
            // does on the create camera this pass replaces
            sky_reflection_env_light(&mut images),
        ))
        .id();
    // The figure is alone on the `CreateFigure` layer, and a `DirectionalLight`
    // only lights the layers it is on — the scene's sun is on the main layer,
    // so without this the body was lit by the sky probe alone and read as a
    // flat silhouette. Same construction and the same illuminance as the paper
    // doll's headlight (`plugins::hud::inventory::paperdoll`): a light parented
    // to the camera with an identity transform shines wherever the camera
    // looks, so the figure keeps its lighting through the camera flight.
    commands.spawn((
        DirectionalLight {
            illuminance: 3_000.0,
            ..default()
        },
        RenderLayers::layer(CameraLayers::CreateFigure.into()),
        ChildOf(camera),
    ));
    // Not for bloom's sake: bevy keys the shared main texture on
    // `(target, usage, format, msaa)`, so a `clear_color: None` camera whose
    // format disagrees with the others composites into an empty texture and
    // blits *that* over the window — erasing the view. Same reason the UI
    // camera carries `Hdr` (see `plugins::camera::setup_ui_camera`).
    if config.graphics.bloom.enabled {
        commands.entity(camera).insert(bevy::camera::Hdr);
    }
}

/// `OnExit(CharacterCreate)`: the overlay belongs to this screen only.
pub fn despawn_figure_overlay_camera(
    mut commands: Commands,
    cameras: Query<Entity, With<FigureOverlayCamera>>,
) {
    for entity in cameras.iter() {
        commands.entity(entity).despawn();
    }
}

/// The overlay pass sees exactly what the create camera sees.
pub fn sync_figure_overlay_camera(
    create_camera: Query<
        (&Transform, &Projection),
        (With<CinematicCamera2>, Without<FigureOverlayCamera>),
    >,
    mut overlay: Query<(&mut Transform, &mut Projection), With<FigureOverlayCamera>>,
) {
    let Ok((source, projection)) = create_camera.single() else {
        return;
    };
    for (mut transform, mut overlay_projection) in overlay.iter_mut() {
        *transform = *source;
        *overlay_projection = projection.clone();
    }
}

/// Move the preview's meshes onto the overlay layer as they stream in, which
/// also takes them out of the world pass — the body is drawn once, just later.
pub fn tag_figure_overlay_meshes(
    previews: Query<Entity, With<CharCreatePreview>>,
    children: Query<&Children>,
    meshes: Query<Option<&RenderLayers>, With<Mesh3d>>,
    mut commands: Commands,
) {
    let layer = RenderLayers::layer(CameraLayers::CreateFigure.into());
    for root in previews.iter() {
        for entity in children.iter_descendants(root) {
            match meshes.get(entity) {
                Ok(Some(existing)) if *existing == layer => {}
                Ok(_) => {
                    commands.entity(entity).insert(layer.clone());
                }
                Err(_) => {}
            }
        }
    }
}

/// The one rule that makes the overlay admissible: a modal dialog is in front
/// of the figure, not behind it.
///
/// In the original's delete-confirm modal the figure's lower body disappears
/// behind the dialog box. That is the selection screen, so it says how this
/// engine layers dialogs over the figure in general — and it says the opposite
/// of "the figure is in front of everything".
pub fn hide_figure_overlay_behind_modal(
    modal: Query<(), With<CreateConfirmModal>>,
    mut overlay: Query<&mut Camera, With<FigureOverlayCamera>>,
) {
    let modal_open = !modal.is_empty();
    for mut camera in overlay.iter_mut() {
        if camera.is_active == modal_open {
            camera.is_active = !modal_open;
        }
    }
}

/// A roll-free rotation looking along `dir`.
fn look_rotation(dir: Vec3) -> Quat {
    Transform::default().looking_to(dir, Vec3::Y).rotation
}

/// `rotation` with its roll removed, falling back to `fallback` when the view
/// axis is parallel to world up (where "no roll" is undefined).
fn level(rotation: Quat, fallback: Quat) -> Quat {
    let forward = rotation * Vec3::NEG_Z;
    if forward.cross(Vec3::Y).length() < 1e-4 {
        return fallback;
    }
    look_rotation(forward)
}

/// Viewport aspect ratio the framing is computed against.
fn viewport_aspect(window: &Window) -> f32 {
    let (w, h) = (window.resolution.width(), window.resolution.height());
    if h > 0.0 {
        w / h
    } else {
        1.0
    }
}

/// Vertical fov of the create camera, from its own projection.
fn camera_fov(projection: &Projection) -> f32 {
    match projection {
        Projection::Perspective(p) => p.fov,
        _ => std::f32::consts::FRAC_PI_4,
    }
}

/// Applies `Section = Rotate`'s state to the preview and to the button art: the
/// accumulated yaw turns the body on the podium, and the zoom button's own art
/// swaps `zoomin` <-> `zoomout` with the state. The dolly the toggle asks for is
/// applied by [`frame_create_camera`], the one owner of the camera transform.
pub fn apply_preview_view(
    view: Res<CharCreatePreviewView>,
    assets: Res<IntroV2Assets>,
    scene: Res<ActiveCharSelectSceneV2>,
    selection: Option<Res<CharCreateSelection>>,
    mut preview: Query<&mut Transform, With<CharCreatePreview>>,
    mut zoom_button: Query<(&mut ImageNode, &mut ImageButtonStyle), With<PreviewZoomButton>>,
) {
    let race = staged_race(selection.as_deref());
    let facing = scene.0.create_stage_facing(race);
    for mut transform in preview.iter_mut() {
        transform.rotation = preview_rotation(facing, view.yaw);
    }
    // The camera is not touched here: [`frame_create_camera`] owns it and reads
    // the same `zoomed_in` flag, so the toggle and the measured frame cannot
    // fight over the transform.
    for (mut image, mut style) in zoom_button.iter_mut() {
        *style = zoom_button_style(&assets, view.zoomed_in);
        image.image = style.normal.clone();
    }
}

/// The podium pose plus the user's accumulated yaw.
///
/// The body faces its stage camera, so the pose is derived from the camera's
/// own yaw instead of the constant `PI` that stood here: that constant was
/// only ever right because the selection stage's camera happens to look down
/// −Z. On the Jangan stage, whose camera looks the other way, it turned the
/// figure's back to the player. The negation is the
/// preview's x-mirror — a mirrored transform shows a yaw of θ as −θ.
fn preview_rotation(stage_facing: f32, yaw: f32) -> Quat {
    Quat::from_rotation_y(-(stage_facing + PI) + yaw)
}

/// Re-spawns the 3d preview whenever the selection changes (and once on enter,
/// via the resource-added change). Centers a single body like a one-entry
/// char-select lineup, reusing the same mirroring + equipment path.
pub fn update_preview(
    selection: Res<CharCreateSelection>,
    view: Res<CharCreatePreviewView>,
    scene: Res<ActiveCharSelectSceneV2>,
    char_data: Res<ClientCharacterData>,
    item_data: Res<ClientItemData>,
    item_index: Res<ClientItemIndex>,
    asset_server: Res<AssetServer>,
    origin: Res<WorldOrigin>,
    existing: Query<Entity, With<CharCreatePreview>>,
    mut feedback: Query<&mut Text, With<NameFeedback>>,
    mut commands: Commands,
) {
    // Resolve before despawning: an unresolvable selection (e.g. European in
    // data without EU rows) keeps the previous preview on stage and
    // reports the gap in-UI instead of silently emptying the podium.
    let Some(refs) = resolve_starter(&selection, &char_data, &item_index) else {
        warn!(
            "starter data unavailable for {:?}/{:?}",
            selection.race, selection.gender
        );
        if let Ok(mut text) = feedback.single_mut() {
            text.0 = "Starter data unavailable for this selection.".to_string();
        }
        return;
    };

    for entity in existing.iter() {
        commands.entity(entity).despawn();
    }
    let Some(path) = char_data
        .get(&(refs.body as i32))
        .and_then(|row| char_data.model_path(row))
    else {
        return;
    };

    // The body stands on the CREATION stage, not on the line-up's quay: the
    // two are different world regions (`create_stage_anchor`), so the
    // selection stage's `char_*_offset` would put it half a map away.
    let pos = scene.0.create_character_position(selection.race, origin.0);
    // The preview is drawn at the model's own size. The `1.0 + DEFAULT_SCALE /
    // 255.0` that stood here read the wire byte as a linear 0..255 scale, which
    // it is not: it is two independent nibbles of five steps each (see
    // [`DEFAULT_SCALE`]/`pack_scale`), so 0x22/255 was an arithmetic on a
    // packed field. Which nibble is which is known (low = Height, high =
    // Volume), but the per-step *visual* delta is still not in any data we
    // hold, so the honest preview stays the unscaled body; the mirror stays
    // (the podium lineup is mirrored on x).
    let scale = 1.0_f32;
    // The re-spawn keeps whatever the rotate buttons have set, so changing
    // gender or gear does not snap the body back to the podium pose.
    let transform = Transform::from_translation(pos)
        .with_scale(Vec3::new(-scale, scale, scale))
        .with_rotation(preview_rotation(
            scene.0.create_stage_facing(selection.race),
            view.yaw,
        ));

    let mut root = commands.spawn((
        transform,
        Visibility::default(),
        CharCreatePreview,
        UnloadedResource(asset_server.load(path)),
        Name::from("Character Create Preview"),
    ));
    if needs_winding_reversal(&transform.to_matrix()) {
        root.insert(MirroredResource);
    }
    let char_entity = root.id();
    character_select::attach_equipment(
        &mut commands,
        &asset_server,
        &item_data,
        char_entity,
        [refs.chest, refs.pants, refs.boots, refs.weapon]
            .into_iter()
            .map(|id| (id, None)),
    );
}

/// Swaps the gender toggles' textures so the picked one reads `*_on` — the
/// original's own selection cue (both captions carry the same `255,249,212`,
/// so a colour change would be our invention). Race is fixed by the
/// region-select board and has no toggle here.
pub fn highlight_selection_buttons(
    selection: Res<CharCreateSelection>,
    assets: Res<IntroV2Assets>,
    mut genders: Query<(&GenderOption, &mut ImageNode, &mut ImageButtonStyle)>,
) {
    for (option, mut image, mut style) in genders.iter_mut() {
        *style = gender_button_style(&assets, option.0, option.0 == selection.gender);
        image.image = style.normal.clone();
    }
}

/// Fills the `Section = Explain` panel from the focused row.
///
/// Idea: the original's rows carry no value text — `GDR_TEXT_EXPLAIN` is the
/// readout, and it is context-sensitive per focused slider, which is why 24
/// `UIO_NEWCHAR_EXPLANATION_*` keys ship for the armour types, the weapon types
/// and Height/Volume. The key scheme is mechanical: a row's caption key
/// `UIO_NEWCHAR_STT_<X>` has the description `UIO_NEWCHAR_EXPLANATION_<X>`
/// (textuisystem:322-341, 394-417); only the figure rows use the older
/// `<key>_EXPLANATION` suffix form.
pub fn update_explain_panel(
    selection: Res<CharCreateSelection>,
    focus: Res<CharCreateFocus>,
    char_data: Res<ClientCharacterData>,
    item_index: Res<ClientItemIndex>,
    ui_strings: Res<ClientUiStrings>,
    mut texts: ParamSet<(
        Query<&mut Text, With<FigureStoryTitle>>,
        Query<&mut Text, With<FigureStoryText>>,
    )>,
) {
    // Two inputs, so the change check is in the body rather than a run
    // condition: either a new value or a new focused row re-reads the box.
    if !selection.is_changed() && !focus.is_changed() {
        return;
    }
    let (name, body) = match focus.0 {
        SliderRow::Figure => {
            let figures = figure_variants(&char_data, selection.race, selection.gender);
            let index = selection.figure.min(figures.len().saturating_sub(1));
            match figures.get(index) {
                Some((_, code_name)) => {
                    let key = figure_label_key(selection.race, selection.gender, code_name);
                    let name = ui_strings
                        .get_or(&key, &humanize_codename(code_name))
                        .to_string();
                    let body = ui_strings
                        .get_or(&format!("{key}_EXPLANATION"), "")
                        .to_string();
                    (name, body)
                }
                None => ("—".to_string(), String::new()),
            }
        }
        SliderRow::Protector => {
            let (key, fallback) = selection.garment.label(selection.race);
            (
                ui_strings.get_or(key, fallback).to_string(),
                explanation_body(key, &ui_strings),
            )
        }
        SliderRow::Weapon => {
            let weapons = weapon_choices(selection.race, &item_index);
            match weapons.get(selection.weapon.min(weapons.len().saturating_sub(1))) {
                Some((_, key, fallback)) => (
                    ui_strings.get_or(key, fallback).to_string(),
                    explanation_body(key, &ui_strings),
                ),
                None => ("—".to_string(), String::new()),
            }
        }
        // The Explain box IS the readout for these two as well; the step index
        // itself has no shipped string, so the caption plus the vanilla
        // "one of 5 levels/types" description is the whole text the original
        // shows here.
        SliderRow::Height => explain_pair("UIO_NEWCHAR_STT_HEIGHT", "Height", &ui_strings),
        SliderRow::Volume => explain_pair("UIO_NEWCHAR_STT_VOLUME", "Volume", &ui_strings),
    };
    if let Ok(mut text) = texts.p0().single_mut() {
        text.0 = name;
    }
    if let Ok(mut text) = texts.p1().single_mut() {
        text.0 = unescape_newlines(&body);
    }
}

/// Caption + description of a row whose value carries no string of its own.
fn explain_pair(key: &str, fallback: &str, ui_strings: &ClientUiStrings) -> (String, String) {
    (
        ui_strings.get_or(key, fallback).to_string(),
        explanation_body(key, ui_strings),
    )
}

/// The description paragraph for a caption key — **asked of the table**, not
/// constructed from a name rule.
///
/// Idea, and why this is not just `explanation_key`: the shipped keys do not
/// follow one rule. European *armour* keeps the `EU_` segment
/// (`UIO_NEWCHAR_EXPLANATION_EU_ROBE` exists), European *weapons* drop it (only
/// `UIO_NEWCHAR_EXPLANATION_TWOHANDSTAFF` exists, never `..._EU_TWOHANDSTAFF`).
/// Applying the mechanical swap alone therefore resolved to nothing for nine of
/// the twelve European weapons and drew an **empty description box** for a
/// paragraph that is fully present in the data
/// (`Media/server_dep/silkroad/textdata/textuisystem.txt`, 5364 keyed rows).
///
/// So: try the mechanical key, then the same key without the `EU_` segment, and
/// only then give up with an empty box. Order matters — the armour rows must
/// keep their `EU_` paragraph, which is the more specific one.
fn explanation_body(caption_key: &str, ui_strings: &ClientUiStrings) -> String {
    let primary = explanation_key(caption_key);
    if let Some(body) = ui_strings.get(&primary) {
        return body.to_string();
    }
    let without_eu = primary.replace("_EXPLANATION_EU_", "_EXPLANATION_");
    ui_strings.get_or(&without_eu, "").to_string()
}

/// `UIO_NEWCHAR_STT_<X>` -> `UIO_NEWCHAR_EXPLANATION_<X>`; a key that does not
/// follow the scheme is returned unchanged and simply resolves to nothing.
/// Only the first candidate — see [`explanation_body`] for why there is a
/// second one.
fn explanation_key(caption_key: &str) -> String {
    caption_key.replace("_STT_", "_EXPLANATION_")
}

/// `CHAR_CH_MAN_ADVENTURER` -> "Adventurer", for corpora without the
/// textuisystem rows.
fn humanize_codename(code_name: &str) -> String {
    code_name
        .rsplit('_')
        .next()
        .map(|s| {
            let mut chars = s.chars();
            chars
                .next()
                .map(|f| f.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

/// `Activate` observer of the Check button: runs the original's gates in the
/// original's order and only then sends a `CheckName` probe;
/// [`on_check_name_response`] shows the outcome.
///
/// Order matters and is the original's: the EU screen tests protector, then
/// weapon, then the name, the CN screen tests weapon, then the name.
/// Every rejection is a message plus `snd_error` and **no frame**.
pub fn on_check_name_activate(
    _activate: On<Activate>,
    name_query: Query<&EditableText, With<NameInput>>,
    conn_query: Query<&SilkroadConnection, With<AgentConnection>>,
    selection: Res<CharCreateSelection>,
    ui_strings: Res<ClientUiStrings>,
    assets: Res<IntroV2Assets>,
    options: Res<GameOptions>,
    mut feedback: Query<&mut Text, With<NameFeedback>>,
    mut commands: Commands,
) {
    let name = current_name(&name_query);
    let Ok(mut text) = feedback.single_mut() else {
        return;
    };
    if let Err((key, fallback)) = gate_selection(&selection) {
        text.0 = ui_strings.get_plain_or(key, fallback);
        play_error_sound(&mut commands, &assets, &options);
        return;
    }
    if let Err(msg) = validate_name(&name, &ui_strings) {
        text.0 = msg;
        play_error_sound(&mut commands, &assets, &options);
        return;
    }
    let Ok(conn) = conn_query.single() else {
        return;
    };
    send_action(
        conn,
        CharacterSelectionActionRequest {
            action: CharacterSelectionAction::CheckName,
            name: Some(name),
            create: None,
        },
    );
    // The original writes nothing while the probe is in flight: the next text
    // in that sink is the answer ("Valid ID" / the mapped error). The
    // "Checking name..." line that stood here had no original counterpart.
    text.0 = String::new();
}

/// Shows the `CheckName` availability result the way the create screen's own
/// handler does: `result == 1` prints `UIO_MSG_ERROR_ADMISSON` ("Valid ID"),
/// anything else the `0x0410` row "This ID already exists." (the only code this
/// action is known to use).
pub fn on_check_name_response(
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    ui_strings: Res<ClientUiStrings>,
    assets: Res<IntroV2Assets>,
    options: Res<GameOptions>,
    mut feedback: Query<&mut Text, With<NameFeedback>>,
    mut commands: Commands,
) {
    for res in reader.read() {
        if res.action != CharacterSelectionAction::CheckName {
            continue;
        }
        let Ok(mut text) = feedback.single_mut() else {
            continue;
        };
        if res.result == 1 {
            text.0 = ui_strings.get_plain_or("UIO_MSG_ERROR_ADMISSON", "Valid ID");
            continue;
        }
        // The refusal goes through the one dispatcher the original uses for
        // every lobby error ([`super::lobby_error_line`]) — the create screen
        // is one of its four callers, and a private copy of a single row would
        // put the wrong sentence on screen for every code but `0x0410`. A
        // refusal without a code (or the silent `0x0401`) keeps the row this
        // action actually uses.
        text.0 = super::lobby_error_line_or(
            res.error_code,
            &ui_strings,
            "UIO_MSG_ERROR_ID",
            "This ID already exists.",
        );
        play_error_sound(&mut commands, &assets, &options);
    }
}

/// The `Section = WCreate` confirmation modal: the name about to be created,
/// the original's own confirmation question, and the two 76x32 buttons.
///
/// Idea: this is char-select's delete-confirm shape, reused rather than
/// re-derived — a full-screen scrim that eats the clicks behind it, carrying
/// the warning frame at its native size. The frame's own rect is `0,0,248,128`,
/// i.e. code-placed in the original and unknown, so centring it is our choice and
/// matches the sibling modal already on screen one step earlier.
fn create_confirm_modal(
    character_name: &str,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let window = assets.warning_create_window.clone();
    let name_font = fonts.nine.clone();
    let msg_font = fonts.nine.clone();
    let ok_font = fonts.nine.clone();
    let cancel_font = fonts.nine.clone();
    let name_px = cu_font(intro_font_px(CONFIRM_NAME_FONT_INDEX));
    let body_px = cu_font(intro_font_px(CONFIRM_BODY_FONT_INDEX));
    let ok_sound = assets.sound_button_sound_a.clone();
    let cancel_sound = assets.sound_button_sound_a.clone();
    let name_text = character_name.to_string();
    let message = unescape_newlines(ui_strings.get_or(
        "UIO_NEWCHAR_MSG_CREATE",
        "Do you want to create a new character?",
    ));
    let ok_text = ui_strings.get_or("UIO_COMMON_CTL_CREATE", "Create");
    let cancel_text = ui_strings.get_or("UIO_COMMON_CTL_CANCEL", "Cancel");
    let (nx, ny, nw, nh) = WCREATE_NAME_RECT;
    let (mx, my, mw, mh) = WCREATE_MSG_RECT;
    let (ox, oy, ow, oh) = WCREATE_OK_RECT;
    let (cx, cy, cw, chh) = WCREATE_CANCEL_RECT;

    // The scrim and the centred plate come from the shared modal shell
    // (`hud::modal_dialog`), which is what char-select's two warning modals use.
    // This file used to spell both out by hand — the same `srgba(0,0,0,0.75)`
    // literal as `MODAL_SCRIM` and the same `margin: auto` `ImageNode` as
    // `modal_plate` — so the scrim decision (an openroad convention, not the
    // original's) lived in two places.
    bsn! {
        modal_scrim()
        CreateConfirmModal
        CharCreateRoot
        Name("Character Create Confirm Modal")
        Children [
            (
                modal_plate(window, WCREATE_W, WCREATE_H)
                // Same unit as the children below (`cu`): `modal_plate` is the
                // shared HUD shell and sizes in logical pixels, so the plate is
                // re-stated in the creation screen's 800x600 units — otherwise
                // its children would outgrow it.
                Node { width: cu(WCREATE_W), height: cu(WCREATE_H) }
                Children [
                    (
                        // label() starts transparent for the intro fade
                        // systems, which never run on this modal.
                        label_sized(&name_text, name_font, name_px) TextColor(Color::WHITE)
                        Node { position_type: PositionType::Absolute, left: cu(nx), top: cu(ny), width: cu(nw), height: cu(nh) }
                    ),
                    (
                        label_sized(&message, msg_font, body_px) TextColor(Color::WHITE)
                        Node { position_type: PositionType::Absolute, left: cu(mx), top: cu(my), width: cu(mw), height: cu(mh) }
                    ),
                    (
                        image_button(character_select::warning_button_style(assets), ow, oh)
                        ButtonSound({ok_sound})
                        Node { position_type: PositionType::Absolute, left: cu(ox), top: cu(oy), width: cu(ow), height: cu(oh) }
                        Children [ (label_sized(ok_text, ok_font, body_px) TextColor(Color::WHITE)) ]
                        on(on_create_confirm_activate)
                    ),
                    (
                        image_button(character_select::warning_button_style(assets), cw, chh)
                        ButtonSound({cancel_sound})
                        Node { position_type: PositionType::Absolute, left: cu(cx), top: cu(cy), width: cu(cw), height: cu(chh) }
                        Children [ (label_sized(cancel_text, cancel_font, body_px) TextColor(Color::WHITE)) ]
                        on(|_a: On<Activate>,
                            modal_query: Query<Entity, With<CreateConfirmModal>>,
                            mut commands: Commands| {
                            for modal in modal_query.iter() {
                                commands.entity(modal).despawn();
                            }
                        })
                    ),
                ]
            ),
        ]
    }
}

/// `Activate` observer of the screen's Confirm button. It no longer sends
/// anything: the original asks `UIO_NEWCHAR_MSG_CREATE` first, so this runs the
/// gates, validates the name, proves the starter set resolves, and opens the
/// modal. The request itself leaves from [`on_create_confirm_activate`].
pub fn on_create_activate(
    _activate: On<Activate>,
    selection: Res<CharCreateSelection>,
    name_query: Query<&EditableText, With<NameInput>>,
    char_data: Res<ClientCharacterData>,
    item_index: Res<ClientItemIndex>,
    existing_modal: Query<(), With<CreateConfirmModal>>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    options: Res<GameOptions>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut commands: Commands,
) {
    if !existing_modal.is_empty() {
        return;
    }
    // Same gates as the Check button, same order, same sound: the original
    // runs one validation path and both buttons enter it.
    if let Err((key, fallback)) = gate_selection(&selection) {
        info_text_writer.write(InfoTextV2Update(ui_strings.get_plain_or(key, fallback)));
        play_error_sound(&mut commands, &assets, &options);
        return;
    }
    let name = current_name(&name_query);
    if let Err(msg) = validate_name(&name, &ui_strings) {
        info_text_writer.write(InfoTextV2Update(msg));
        play_error_sound(&mut commands, &assets, &options);
        return;
    }
    if resolve_starter(&selection, &char_data, &item_index).is_none() {
        info_text_writer.write(InfoTextV2Update(
            "Starter data unavailable for this selection.".to_string(),
        ));
        return;
    }
    let Some(camera) = cam_query.iter().next() else {
        return;
    };
    commands
        .spawn_scene(create_confirm_modal(&name, &assets, &fonts, &ui_strings))
        .insert((UiTargetCamera(camera), IntroV2Ui));
}

/// `Activate` observer of the modal's Create button: the only place the
/// `Create` request leaves the client. [`on_character_create_response`] handles
/// the outcome and closes the modal.
pub fn on_create_confirm_activate(
    activate: On<Activate>,
    selection: Res<CharCreateSelection>,
    name_query: Query<&EditableText, With<NameInput>>,
    char_data: Res<ClientCharacterData>,
    item_index: Res<ClientItemIndex>,
    conn_query: Query<&SilkroadConnection, With<AgentConnection>>,
    mut commands: Commands,
) {
    let name = current_name(&name_query);
    let Some(refs) = resolve_starter(&selection, &char_data, &item_index) else {
        return;
    };
    let Ok(conn) = conn_query.single() else {
        return;
    };

    // double-activation guard (same pattern as the Start button); the response
    // re-enables it on rejection
    commands
        .entity(activate.entity)
        .insert(InteractionDisabled)
        .remove::<Pressed>();

    let sent = send_action(
        conn,
        CharacterSelectionActionRequest {
            action: CharacterSelectionAction::Create,
            name: None,
            create: Some(CharacterCreate {
                name,
                ref_obj_id: refs.body,
                // Volume in the high nibble, Height in the low one
                scale: selection_scale(&selection),
                chest: refs.chest,
                pants: refs.pants,
                boots: refs.boots,
                weapon: refs.weapon,
            }),
        },
    );
    if !sent {
        commands
            .entity(activate.entity)
            .remove::<InteractionDisabled>();
    }
}

/// Handles the create response: success returns to a refreshed `CharacterList`;
/// a server rejection (e.g. the same server gap seen on item-use) shows the
/// info text + error sound and keeps the screen so no panic/disconnect occurs.
pub fn on_character_create_response(
    mut reader: MessageReader<CharacterSelectionActionResponse>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut next_state: ResMut<NextState<IntroV2State>>,
    create_buttons: Query<Entity, (With<CreateButton>, With<InteractionDisabled>)>,
    modal_query: Query<Entity, With<CreateConfirmModal>>,
    assets: Res<IntroV2Assets>,
    ui_strings: Res<ClientUiStrings>,
    options: Res<GameOptions>,
    mut commands: Commands,
) {
    for res in reader.read() {
        if res.action != CharacterSelectionAction::Create {
            continue;
        }
        if res.result == 1 {
            next_state.set(IntroV2State::CharacterList);
        } else {
            // The per-code catalogue is in this PR, so use it: `0x0404`
            // "Select a Weapon.", `0x0405` "A maximum of %d characters ..."
            // and `0x0410` "This ID already exists." are all rows the original
            // shows here, and the generic create failure is only what the
            // table itself falls back to. Same renderer as delete/restore and
            // world join ([`super::lobby_error_line`]).
            let code = res.error_code.unwrap_or_default();
            warn!("character create rejected (error {:#06x})", code);
            info_text_writer.write(InfoTextV2Update(super::lobby_error_line_or(
                res.error_code,
                &ui_strings,
                "UIO_SMERR_FAILED_TO_CREATE_CHARACTER",
                "Failed to create a character. Please try to connect again.",
            )));

            for entity in create_buttons.iter() {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
            // The request left from the modal, so the modal is what has to go:
            // leaving a disabled Create button behind a scrim would strand the
            // screen. Closing it puts the user back on the panel to fix the
            // name or the selection and ask again.
            for modal in modal_query.iter() {
                commands.entity(modal).despawn();
            }
            play_error_sound(&mut commands, &assets, &options);
        }
    }
}

/// `OnExit(CharacterCreate)`: tears the UI + preview down and drops the
/// selection (the camera is despawned by the shared `despawn_cinematic_camera`).
pub fn despawn_character_create(
    // The confirm modal is a *sibling* root on the camera rather than a child
    // of the panel (so its scrim covers the panel too), but it carries
    // `CharCreateRoot` itself (`create_confirm_modal`, the `bsn!` root), so this
    // one query reaches both and nothing is left behind.
    ui: Query<Entity, With<CharCreateRoot>>,
    preview: Query<Entity, With<CharCreatePreview>>,
    mut commands: Commands,
) {
    for entity in ui.iter().chain(preview.iter()) {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<CharCreateSelection>();
    commands.remove_resource::<CharCreatePreviewView>();
    commands.remove_resource::<CharCreateFocus>();
}

fn current_name(name_query: &Query<&EditableText, With<NameInput>>) -> String {
    name_query
        .single()
        .map(|input| input.value().to_string().trim().to_string())
        .unwrap_or_default()
}

/// Whether the code point passes the original's name charset (`abusefilter`
/// column 4, see [`NAME_ALLOWED_RANGES`]).
fn name_charset_allows(c: char) -> bool {
    NAME_ALLOWED_RANGES
        .iter()
        .any(|(lo, hi)| c >= *lo && c <= *hi)
}

/// Pre-send validation mirroring the original client's two stages: the length
/// window `MIN_NAME_LEN ..= MAX_NAME_LEN` and the `abusefilter` allow table.
/// Returns the original's own message key text on violation; a violation also
/// means **no packet leaves the client** — an invalid name produces no frame at
/// all.
///
/// The profanity word lists the original checks as a third stage
/// are not modelled: the
/// lists live in the same `abusefilter.txt` we have, but the server rejects
/// them anyway (`UIO_SMERR_NOT_ALLOWED_CHARNAME`), so this is a gap, not a
/// deviation with a rationale.
fn validate_name(name: &str, ui_strings: &ClientUiStrings) -> Result<(), String> {
    if name.is_empty() {
        return Err(ui_strings.get_plain_or(
            "UIO_MSG_ERROR_CHARACTER_NAME",
            "Enter the name of character.",
        ));
    }
    // Only the LOWER bound is a message: typing 15 characters leaves 12 in the
    // field with no message at all, so the upper bound is the edit
    // cap on the input (`max_characters`, see the `NameInput` spawn) and the
    // original's `(unsigned)(len - 2) > 10` test can only ever fire on the
    // short side of a field that cannot hold more than 12.
    let len = name.chars().count();
    if len < MIN_NAME_LEN {
        return Err(ui_strings.get_plain_or(
            "UIO_MSG_ERROR_CHARACTER_NAME_STRING",
            "Exceeded the letter limit. \nOnly 12 English letters are available.[Min., Max.]",
        ));
    }
    // Shape of this case in the original: an invalid character produces **no
    // packet**, and the screen shows *two* status lines at once, "Invalid
    // character name." and "Invalid letter used." — which is exactly this one
    // shipped string with its embedded `\n`, so a single message here is the
    // original's two lines, not a shortcut.
    if !name.chars().all(name_charset_allows) {
        return Err(ui_strings.get_plain_or(
            "UIO_MSG_ERROR_CHARACTER_WRONGSTRING",
            "Invalid character name. \nInvalid letter used.",
        ));
    }
    Ok(())
}

/// The original's two hard pre-send gates, in the original's order.
///
/// Idea: the create screen keeps the picked refs in `this+0x12c` (chest) and
/// `this+0x138` (weapon) and both start at 0, so "nothing chosen" and "no
/// packet" are the same state. The European screen checks the
/// protector first and then the weapon; the Chinese screen is
/// the same shape but checks **only** the weapon, so
/// `UIO_MSG_ERROR_CHARACTER_SELECTARMOR` is never used there. Both messages are
/// accompanied by `snd_error`.
///
/// In the original, "Select a Weapon." appears as the **status line at the
/// bottom left**, not as a message box, and **no packet leaves** — every
/// mandatory-field check is local, in front of the wire.
///
/// Returns the key + fallback of the message to show, or `Ok(())` when the
/// request may leave.
fn gate_selection(selection: &CharCreateSelection) -> Result<(), (&'static str, &'static str)> {
    if selection.race == Race::EUROPEAN && !selection.garment_chosen {
        return Err(GATE_PROTECTOR);
    }
    if !selection.weapon_chosen {
        return Err(GATE_WEAPON);
    }
    Ok(())
}

/// The two gate lines as one pair each, so [`gate_selection`] and
/// [`clear_satisfied_gate_line`] cannot drift apart: one of them writes the
/// line, the other one has to recognise it again.
const GATE_PROTECTOR: (&str, &str) = ("UIO_MSG_ERROR_CHARACTER_SELECTARMOR", "Select a Protector.");
const GATE_WEAPON: (&str, &str) = ("UIO_MSG_ERROR_CHARACTER_SELECTWEAPON", "Select a Weapon.");

/// Takes the gate line down once the player has done what it asked.
///
/// The defect: "Select a Weapon." is written by [`gate_selection`] and by
/// nothing else, and the notice line only ever changes when somebody writes a
/// new one — so after picking a weapon the demand stood on the screen while the
/// screen was ready to send. A line that outlives its reason reads as a second,
/// unexplained refusal.
///
/// Only the two lines this screen's gates authored are cleared, and only once
/// the gates pass: a server refusal or a name error keeps standing, because
/// picking a weapon has not answered those.
pub fn clear_satisfied_gate_line(
    selection: Res<CharCreateSelection>,
    ui_strings: Res<ClientUiStrings>,
    notice: Query<&Text, With<InfoTextV2>>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
) {
    if gate_selection(&selection).is_err() {
        return;
    }
    let Ok(shown) = notice.single() else {
        return;
    };
    if [GATE_PROTECTOR, GATE_WEAPON]
        .iter()
        .any(|(key, fallback)| ui_strings.get_plain_or(key, fallback) == shown.0)
    {
        info_text_writer.write(InfoTextV2Update(String::new()));
    }
}

/// Sends a lobby action frame; returns whether it was queued.
fn send_action(conn: &SilkroadConnection, request: CharacterSelectionActionRequest) -> bool {
    let frame = Packet::from(request).into();
    if let Err(e) = conn.get_sender().send(frame) {
        error!("failed to send frame: {}", e.0);
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Project a world point through a camera pose, as viewport fractions
    /// (0,0 = top-left). This is the perspective divide, written out, so the
    /// framing assertions below measure the same thing the screen shows.
    fn viewport_fraction(pose: (Vec3, Quat), point: Vec3, fov: f32, aspect: f32) -> (f32, f32) {
        let (translation, rotation) = pose;
        let view = rotation.inverse() * (point - translation);
        let tan_v = (fov * 0.5).tan();
        let ndc_x = (view.x / -view.z) / (tan_v * aspect);
        let ndc_y = (view.y / -view.z) / tan_v;
        ((ndc_x + 1.0) * 0.5, (1.0 - ndc_y) * 0.5)
    }

    /// A stand-in body height, in world units: the framing must work off the
    /// measured body, so the tests hand it one.
    const FIGURE_H: f32 = 5.0;

    /// The authored stage pose, as an arbitrary but off-axis stand-in: the
    /// framing must not depend on the camera looking down a world axis.
    fn authored_pose(sole: Vec3) -> (Vec3, Quat) {
        let translation = sole + Vec3::new(12.0, 9.0, 40.0);
        (translation, look_rotation((sole - translation).normalize()))
    }

    /// The framing puts the sole exactly on the target lines: 92.17 % of the
    /// height (38 px into the bottom band on the 800x600 original) and 57.9 %
    /// of the width (7.9 % right of centre).
    #[test]
    fn the_framing_puts_the_sole_on_the_screen_lines() {
        let sole = Vec3::new(3.0, 0.0, -7.0);
        let fov = std::f32::consts::FRAC_PI_4;
        for aspect in [4.0 / 3.0, 16.0 / 9.0, 1.0] {
            let framed =
                framed_create_camera_pose(authored_pose(sole), sole, FIGURE_H, fov, aspect);
            let (x, y) = viewport_fraction(framed, sole, fov, aspect);
            // The two numbers are written out, not read back from the
            // constants: a test that compares the code against itself passes
            // for any value (flipping the constant to 0.5 leaves it green).
            // 463/800 = 0.579 and 553/600 = 0.9217, the two numbers the
            // original's own screen gives.
            assert!(
                (x - 0.579).abs() < 1e-3,
                "aspect {aspect}: axis at {x}, expected 0.579 (x=463 of 800)"
            );
            assert!(
                (y - 0.9217).abs() < 1e-3,
                "aspect {aspect}: sole at {y}, expected 0.9217 (y=553 of 600)"
            );
        }
    }

    /// The measured body fills 82.0 % of the viewport height, whatever its size
    /// and whatever the window: the frame is solved from the body's own height,
    /// not from a fixed dolly. 82.0 % is the original's own figure
    /// (crown y=62, sole y=553 of 600), written out rather than read back.
    #[test]
    fn the_framed_body_fills_its_share_of_the_height() {
        let fov = std::f32::consts::FRAC_PI_4;
        for height in [FIGURE_H, 1.0, 12.0] {
            for aspect in [4.0 / 3.0, 16.0 / 9.0] {
                let sole = Vec3::new(3.0, 0.0, -7.0);
                let framed =
                    framed_create_camera_pose(authored_pose(sole), sole, height, fov, aspect);
                let (_, sole_y) = viewport_fraction(framed, sole, fov, aspect);
                let (_, crown_y) = viewport_fraction(framed, sole + Vec3::Y * height, fov, aspect);
                let share = sole_y - crown_y;
                // A tenth of a percentage point, not one: the closed form
                // `h / (2 d tan)` alone lands 0.44 pp off for this off-axis
                // body (measured), so this bound is what pins the distance
                // correction loop rather than merely the closed form.
                assert!(
                    (share - 0.82).abs() < 1e-3,
                    "height {height}, aspect {aspect}: body covers {share} of the height, expected 0.82"
                );
            }
        }
    }

    /// The camera keeps the stage's authored *direction* onto the body and only
    /// moves along it — the scene data still says from which side the screen
    /// looks at the creation stage.
    #[test]
    fn the_framing_only_moves_along_the_authored_view_axis() {
        let sole = Vec3::new(3.0, 0.0, -7.0);
        let pose = authored_pose(sole);
        let (translation, _) =
            framed_create_camera_pose(pose, sole, FIGURE_H, std::f32::consts::FRAC_PI_4, 4.0 / 3.0);
        let centre = sole + Vec3::Y * (FIGURE_H * 0.5);
        let before = (pose.0 - centre).normalize();
        let after = (translation - centre).normalize();
        assert!(
            before.dot(after) > 0.9999,
            "the framing left the authored view axis: {before} -> {after}"
        );
        // and it moved *closer*, as the 43.4 % -> 82.0 % change requires
        assert!((translation - centre).length() < (pose.0 - centre).length());
    }

    /// The framed camera keeps the horizon level. Aiming with a single
    /// minimal-arc rotation would place the sole correctly *and* roll the view
    /// by ~2 deg, which is why the aim is iterative — this is the assertion
    /// that pins that.
    #[test]
    fn the_framed_camera_has_no_roll() {
        let sole = Vec3::new(3.0, 0.0, -7.0);
        let (_, rotation) = framed_create_camera_pose(
            authored_pose(sole),
            sole,
            FIGURE_H,
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
        );
        let right = rotation * Vec3::X;
        assert!(
            right.y.abs() < 1e-4,
            "camera right vector tilts out of the horizon: y = {}",
            right.y
        );
    }

    /// The overlay is what puts the figure in front of the bands — and the
    /// confirm modal is what must still be in front of the *figure* (in the
    /// original a dialog covers the body). Without the rule the figure would
    /// stand over its own confirmation dialog, which the original never does.
    #[test]
    fn the_confirm_modal_switches_the_figure_overlay_off() {
        let mut app = App::new();
        app.add_systems(Update, hide_figure_overlay_behind_modal);
        let camera = app
            .world_mut()
            .spawn((FigureOverlayCamera, Camera::default()))
            .id();
        let active = |app: &App| app.world().get::<Camera>(camera).unwrap().is_active;

        app.update();
        assert!(active(&app), "no modal: the figure draws over the chrome");

        let modal = app.world_mut().spawn(CreateConfirmModal).id();
        app.update();
        assert!(
            !active(&app),
            "the create-confirm modal must cover the figure, not stand behind it"
        );

        app.world_mut().entity_mut(modal).despawn();
        app.update();
        assert!(active(&app), "closing the modal brings the figure back");
    }

    /// The overlay pass draws *after* the UI pass — that ordering is the whole
    /// mechanism, and a `ZIndex` cannot express it: `ZIndex` orders UI nodes
    /// against each other, while a 3d pass and the UI pass are ordered by
    /// camera `order` (world 0, UI 1, figure 2).
    #[test]
    fn the_figure_overlay_is_ordered_after_the_ui_pass() {
        const UI_CAMERA_ORDER: isize = 1;
        assert!(
            FIGURE_OVERLAY_ORDER > UI_CAMERA_ORDER,
            "the overlay pass must run after the UI camera (order {UI_CAMERA_ORDER})"
        );
    }

    /// Degenerate input has no frame to solve — a camera standing in the body,
    /// and a body that has not been measured yet (the first frames of the
    /// screen, before its meshes have streamed in). Both hand the authored pose
    /// back untouched instead of putting NaNs on screen.
    #[test]
    fn degenerate_input_leaves_the_authored_pose_alone() {
        let sole = Vec3::new(1.0, 2.0, 3.0);
        let centre = sole + Vec3::Y * (FIGURE_H * 0.5);
        let pose = (centre, Quat::IDENTITY);
        let framed =
            framed_create_camera_pose(pose, sole, FIGURE_H, std::f32::consts::FRAC_PI_4, 1.0);
        assert_eq!(framed.0, pose.0);
        assert_eq!(framed.1, pose.1);

        let unframed = framed_create_camera_pose(
            authored_pose(sole),
            sole,
            0.0,
            std::f32::consts::FRAC_PI_4,
            1.0,
        );
        assert_eq!(unframed.0, authored_pose(sole).0);
        assert_eq!(unframed.1, authored_pose(sole).1);
    }

    /// The Confirm/Cancel row is anchored by constant pixel inset, and the
    /// insets are the original's own: in an 800x600 client area its opaque
    /// extents are Confirm x 591..682, Cancel x 695..786, both y 542..582.
    ///
    /// Dropped onto an 800x600 client area our box must reproduce those edges
    /// (within the 1 px our 91-wide art is narrower than the authored 92).
    #[test]
    fn the_control_row_sits_at_the_bottom_right_inset() {
        let (confirm_left, cancel_right, top, bottom) = control_row_box(800.0, 600.0);
        // right and bottom edge: exactly the original's
        assert_eq!(cancel_right, 787.0, "last opaque column 786");
        assert_eq!(bottom, 583.0, "last opaque row 582");
        assert_eq!(top, 542.0, "first opaque row 542");
        // left edge: 592 against the original's 591 — one pixel, and it is the
        // art's, not the anchor's.
        assert!(
            (confirm_left - 591.0).abs() <= 1.0,
            "confirm left {confirm_left} is more than 1 px off 591"
        );
        // Confirm's right edge, the one the pitch is there to preserve.
        assert_eq!(confirm_left + MAIN_BUTTON_W, 683.0);
    }

    /// Pinned against a too-large scale: the row is the same 1:1 art at every
    /// window size, like the login form and the region plates. The Vh unit this
    /// replaced drew the row 1.5x at 1600x900 (buttons 137x62 instead of 91x41).
    #[test]
    fn the_control_row_is_drawn_1_to_1_at_every_window_size() {
        let (left_600, right_600, top_600, bottom_600) = control_row_box(800.0, 600.0);
        let (left_900, right_900, top_900, bottom_900) = control_row_box(1600.0, 900.0);
        // the 800x600 case is the original's own layout
        assert_eq!(600.0 - bottom_600, 17.0);
        assert_eq!(800.0 - right_600, 13.0);
        // and at 1600x900 the same insets and the same button art
        assert_eq!(900.0 - bottom_900, 17.0);
        assert_eq!(1600.0 - right_900, 13.0);
        assert_eq!(bottom_900 - top_900, MAIN_BUTTON_H);
        assert_eq!(bottom_900 - top_900, bottom_600 - top_600);
        assert_eq!(right_900 - left_900, 2.0 * MAIN_BUTTON_W + CONTROL_ROW_GAP);
        assert_eq!(right_900 - left_900, right_600 - left_600);
    }

    /// The unit itself: one `cu` is one authored pixel, at every window size,
    /// so the customize panel is the original's 264x316 at 1600x900 too (the
    /// Vh unit drew it 396x474 there).
    #[test]
    fn one_creation_unit_is_one_original_pixel() {
        assert_eq!(cu(1.0), Val::Px(1.0));
        assert_eq!(cu_font(12.0), FontSize::Px(12.0));
        assert_eq!(cu_at(CUSTOM_W), 264.0);
        assert_eq!(cu_at(CUSTOM_H), 316.0);
        assert_eq!(cu(CUSTOM_W), Val::Px(264.0));
        assert_eq!(cu_at(EXPLAIN_CH_SIZE.0), 212.0);
        assert_eq!(cu_at(ROTATE_WINDOW_W), 152.0);
        assert_eq!(cu_at(MAIN_BUTTON_W), 91.0);
    }

    /// `Section = Slider` is ONE authored template, transcribed verbatim, and
    /// its drawn width is the next arrow's right edge — 140, not the row
    /// `Rect`'s 120. The panel is what makes that credible: the rows start at
    /// x=88 and 88+140 still fits inside the 264-wide window.
    #[test]
    fn the_slider_template_is_the_authored_one_and_fits_the_panel() {
        assert_eq!(SLIDER_PREV_RECT, (0.0, 2.0, 20.0, 20.0));
        assert_eq!(SLIDER_THUMB_RECT, (20.0, 0.0, 16.0, 24.0));
        assert_eq!(SLIDER_NEXT_RECT, (120.0, 2.0, 20.0, 20.0));
        assert_eq!(SLIDER_TEMPLATE_W, 140.0);
        assert!(ROW_X + SLIDER_TEMPLATE_W <= CUSTOM_W);
        // the template is taller than the arrows: the thumb defines the height
        assert_eq!(SLIDER_THUMB_RECT.3, ROW_H);
    }

    /// The thumb's travel is derived from the authored rects, not chosen: it
    /// starts at the thumb's own x, ends where the next arrow begins, and the
    /// steps divide that evenly.
    #[test]
    fn the_thumb_spans_the_authored_track_and_never_leaves_it() {
        let start = SLIDER_THUMB_RECT.0;
        let track_end = SLIDER_NEXT_RECT.0;
        for count in [2usize, 3, 5, 26] {
            assert_eq!(slider_thumb_left(0, count), start, "count {count}");
            assert_eq!(
                slider_thumb_left(count - 1, count),
                track_end - SLIDER_THUMB_RECT.2,
                "count {count}"
            );
            let mut previous = f32::MIN;
            for index in 0..count {
                let left = slider_thumb_left(index, count);
                assert!(left > previous, "thumb must advance at {index}/{count}");
                assert!(left >= start && left + SLIDER_THUMB_RECT.2 <= track_end);
                previous = left;
            }
        }
        // a row with nothing (or one thing) to choose parks at the start
        assert_eq!(slider_thumb_left(0, 0), start);
        assert_eq!(slider_thumb_left(0, 1), start);
        // an out-of-range index is clamped, never drawn past the track
        assert_eq!(slider_thumb_left(99, 3), slider_thumb_left(2, 3));
    }

    /// The Explain box is the rows' readout, and its key scheme is mechanical:
    /// the caption key's `_STT_` becomes `_EXPLANATION_` (textuisystem:322-341,
    /// 394-417). Pinned against the real keys of both race trees.
    #[test]
    fn the_explanation_key_is_the_caption_key_with_one_segment_swapped() {
        assert_eq!(
            explanation_key("UIO_NEWCHAR_STT_LIGHT_ARMOR"),
            "UIO_NEWCHAR_EXPLANATION_LIGHT_ARMOR"
        );
        assert_eq!(
            explanation_key("UIO_NEWCHAR_STT_EU_ROBE"),
            "UIO_NEWCHAR_EXPLANATION_EU_ROBE"
        );
        assert_eq!(
            explanation_key("UIO_NEWCHAR_STT_TWOHANDSTAFF"),
            "UIO_NEWCHAR_EXPLANATION_TWOHANDSTAFF"
        );
        // every garment caption of both races maps onto a shipped key name
        for race in [Race::CHINESE, Race::EUROPEAN] {
            for set in Garment::ALL {
                let (key, _) = set.label(race);
                assert!(explanation_key(key).starts_with("UIO_NEWCHAR_EXPLANATION_"));
            }
        }
        // a key outside the scheme is left alone rather than mangled
        assert_eq!(
            explanation_key("UIO_COMMON_CTL_CANCEL"),
            "UIO_COMMON_CTL_CANCEL"
        );
    }

    /// The shipped keys do **not** follow one rule, so the box asks the table
    /// instead of building a name: European armour keeps its `EU_` paragraph,
    /// European weapons only have the un-prefixed one. The rows below are
    /// transcribed from
    /// `Media/server_dep/silkroad/textdata/textuisystem.txt` (5364 keyed
    /// rows) — deliberately including the *absence* of
    /// `UIO_NEWCHAR_EXPLANATION_EU_TWOHANDSTAFF`, which is the whole defect:
    /// nine of the twelve European weapons drew an empty description box.
    #[test]
    fn the_explain_box_falls_back_to_the_key_the_data_actually_ships() {
        let strings = ClientUiStrings::from_rows(&[
            // European armour: the EU_ paragraph exists and must win
            ("UIO_NEWCHAR_EXPLANATION_EU_ROBE", "robe paragraph"),
            ("UIO_NEWCHAR_EXPLANATION_ROBE", "chinese robe paragraph"),
            // European weapon: only the un-prefixed paragraph ships
            ("UIO_NEWCHAR_EXPLANATION_TWOHANDSTAFF", "staff paragraph"),
            ("UIO_NEWCHAR_EXPLANATION_HARP", "harp paragraph"),
        ]);

        assert_eq!(
            explanation_body("UIO_NEWCHAR_STT_EU_ROBE", &strings),
            "robe paragraph",
            "the more specific EU_ paragraph must not be skipped"
        );
        assert_eq!(
            explanation_body("UIO_NEWCHAR_STT_EU_TWOHANDSTAFF", &strings),
            "staff paragraph",
            "this row was empty before the fix"
        );
        assert_eq!(
            explanation_body("UIO_NEWCHAR_STT_EU_HARP", &strings),
            "harp paragraph"
        );
        // a caption with no paragraph at all stays empty — the row labels
        // (NAME/SEX/FIGURE/PROTECTOR/WEAPON) ship none and show none
        assert_eq!(explanation_body("UIO_NEWCHAR_STT_NAME", &strings), "");
    }

    /// `Section = WCreate` is a complete authored modal and is transcribed
    /// verbatim: the two 76x32 buttons, the name line and the message line.
    /// The arithmetic closes inside the 248x128 frame, so nothing here is a
    /// magic number: 4+239 = 243 < 248, and 130+76 = 206 < 248.
    #[test]
    fn the_create_confirm_modal_uses_the_authored_wcreate_rects() {
        assert_eq!((WCREATE_W, WCREATE_H), (248.0, 128.0));
        assert_eq!(WCREATE_NAME_RECT, (4.0, 23.0, 239.0, 15.0));
        assert_eq!(WCREATE_MSG_RECT, (4.0, 47.0, 239.0, 13.0));
        assert_eq!(WCREATE_OK_RECT, (42.0, 75.0, 76.0, 32.0));
        assert_eq!(WCREATE_CANCEL_RECT, (130.0, 75.0, 76.0, 32.0));
        // every child stays inside the frame
        for (x, y, w, h) in [
            WCREATE_NAME_RECT,
            WCREATE_MSG_RECT,
            WCREATE_OK_RECT,
            WCREATE_CANCEL_RECT,
        ] {
            assert!(x + w <= WCREATE_W, "{x}+{w} escapes the frame");
            assert!(y + h <= WCREATE_H, "{y}+{h} escapes the frame");
        }
        // the two buttons share a row and do not overlap
        assert_eq!(WCREATE_OK_RECT.1, WCREATE_CANCEL_RECT.1);
        assert!(WCREATE_OK_RECT.0 + WCREATE_OK_RECT.2 <= WCREATE_CANCEL_RECT.0);
    }

    /// The caption sits on the plate, not on the control: the gem is part of
    /// the button art, so half of it is exactly how far the centred caption
    /// has to move — and the two buttons move in OPPOSITE directions. A fix
    /// that shifts both the same way is at the wrong place, which is what this
    /// pins.
    #[test]
    fn the_gender_caption_is_centred_on_the_plate_not_on_the_control() {
        let control_centre = MALE_RECT.2 / 2.0;
        for gender in [Gender::Male, Gender::Female] {
            let (l, r) = gender_plate_span(gender);
            assert_eq!(r - l, MALE_RECT.2 - GENDER_GEM_W, "{gender:?} plate width");
            let (ml, mr) = gender_caption_margin(gender);
            // A margin on one side moves a centred child by half of it.
            let caption_centre = control_centre + (ml - mr) / 2.0;
            assert_eq!(
                caption_centre,
                (l + r) / 2.0,
                "{gender:?} caption is not on the plate's centre"
            );
        }
        // Opposite directions, same size — 10 px each way.
        assert_eq!(gender_caption_margin(Gender::Male), (GENDER_GEM_W, 0.0));
        assert_eq!(gender_caption_margin(Gender::Female), (0.0, GENDER_GEM_W));
    }

    /// The gem lives in the art, so the plate is narrower than the control by
    /// exactly the gem — and it sits on the left for male, on the right for
    /// female (alpha of the 72x28 textures: man 20..68, woman 1..49).
    #[test]
    fn the_gem_is_on_opposite_sides_of_the_two_gender_buttons() {
        assert_eq!(gender_plate_span(Gender::Male), (20.0, 70.0));
        assert_eq!(gender_plate_span(Gender::Female), (0.0, 50.0));
        assert_eq!(GENDER_GEM_W, 20.0);
    }

    /// The customize panel and every `Section = Custom` child are transcribed
    /// from the resinfo tree, not laid out by flex/padding. If any of these
    /// drift, the panel art and its slots stop lining up.
    #[test]
    fn the_customize_panel_children_are_the_authored_resinfo_rects() {
        assert_eq!((CUSTOM_W, CUSTOM_H), (264.0, 316.0));
        // The caption column is deliberately *not* the authored x/w any more
        // (see `LABEL_X`); its tops are still the resinfo's.
        assert_eq!(NAME_LABEL_RECT.1, 35.0);
        assert_eq!(NAME_EDIT_RECT, (92.0, 32.0, 135.0, 20.0));
        assert_eq!(CHECK_RECT, (156.0, 57.0, 75.0, 25.0));
        assert_eq!(SEX_LABEL_RECT.1, 100.0);
        assert_eq!(MALE_RECT, (87.0, 95.0, 70.0, 25.0));
        assert_eq!(FEMALE_RECT, (166.0, 95.0, 70.0, 25.0));
        // Five rows, one pitch: 131/162/193/224/255, step 31.
        let rows = [
            ROW_FIGURE_TOP,
            ROW_HEIGHT_TOP,
            ROW_VOLUME_TOP,
            ROW_GEAR_TOPS.0,
            ROW_GEAR_TOPS.1,
        ];
        assert_eq!(rows, [131.0, 162.0, 193.0, 224.0, 255.0]);
        for pair in rows.windows(2) {
            assert_eq!(pair[1] - pair[0], 31.0);
        }
        assert_eq!((ROW_X, ROW_W, ROW_H), (88.0, 120.0, 24.0));
        assert_eq!(
            (LABEL_AUTHORED_X, LABEL_AUTHORED_W, LABEL_H),
            (32.0, 45.0, 15.0)
        );
        assert_eq!(
            [
                LABEL_FIGURE_TOP,
                LABEL_HEIGHT_TOP,
                LABEL_VOLUME_TOP,
                LABEL_GEAR_TOPS.0,
                LABEL_GEAR_TOPS.1
            ],
            [137.0, 167.0, 198.0, 229.0, 260.0]
        );
    }

    /// Advance widths of the seven captions at Arial/Arimo 16 px (FontIndex 2),
    /// in the panel's own pixels.
    ///
    /// Not estimated: the `hmtx` advances of
    /// `assets/fonts/Arimo-Variable.ttf` summed per string. Arimo is metrically
    /// Arial by construction, equal to two decimals over 13 strings. The strings
    /// are the `textuisystem.txt` values for `UIO_NEWCHAR_STT_*`,
    /// and both `resinfo` trees use the same seven keys, so this table covers
    /// BOTH races.
    const CAPTION_ADVANCES_16PX: [(&str, f32); 7] = [
        ("Name", 42.68),
        ("Sex", 27.57),
        ("Figure", 45.35),
        ("Height", 46.25),
        ("Volume", 54.25),
        ("Cloth", 37.35),
        ("Weapon", 59.59),
    ];

    /// The whole point of the deviation: every caption fits its box now, and
    /// none of them fitted the authored one.
    #[test]
    fn every_caption_fits_the_widened_caption_box() {
        for (text, advance) in CAPTION_ADVANCES_16PX {
            assert!(
                advance <= LABEL_W,
                "{text} needs {advance} px but LABEL_W is {LABEL_W}"
            );
        }
        // Positive control on the same table: the authored 45 px box holds
        // neither the widest caption nor three of the others, which is the
        // original's own clipping and the reason this column moved at all.
        let over = CAPTION_ADVANCES_16PX
            .iter()
            .filter(|(_, advance)| *advance > LABEL_AUTHORED_W)
            .count();
        assert_eq!(over, 4, "the authored box should still fail this table");
    }

    /// Growing the column leftwards must not walk into the panel art's
    /// ornamental frame on one side, nor into the controls on the other — the
    /// slider arrows, sliders and gender buttons do not move for this change.
    #[test]
    fn the_caption_column_stays_clear_of_the_frame_and_the_controls() {
        // `customize_window[_europe].ddj`: ornament in columns 4..12, flat
        // interior from 13 on.
        const FRAME_INNER_EDGE: f32 = 13.0;
        assert!(
            LABEL_X >= FRAME_INNER_EDGE + 3.0,
            "caption column would touch the frame ornament"
        );
        // The controls to the right keep their authored rects.
        assert_eq!(MALE_RECT.0, 87.0);
        assert_eq!(ROW_X, 88.0);
        let nearest = MALE_RECT.0.min(ROW_X);
        assert!(
            LABEL_RIGHT <= nearest - 5.0,
            "caption box {LABEL_RIGHT} runs into the control at {nearest}"
        );
        // And no caption's *ink* gets near them either.
        let widest = CAPTION_ADVANCES_16PX
            .iter()
            .fold(0.0f32, |acc, (_, advance)| acc.max(*advance));
        assert!(
            LABEL_X + widest <= nearest - 10.0,
            "widest caption ends at {} with the control at {nearest}",
            LABEL_X + widest
        );
    }

    /// The flush left column survives the move: all
    /// seven captions are placed from the one `LABEL_X`, so they cannot fray.
    #[test]
    fn all_seven_captions_share_one_left_edge() {
        assert_eq!(NAME_LABEL_RECT.0, LABEL_X);
        assert_eq!(SEX_LABEL_RECT.0, LABEL_X);
        assert_eq!(NAME_LABEL_RECT.2, LABEL_W);
        assert_eq!(SEX_LABEL_RECT.2, LABEL_W);
        // The five row captions are placed with `LABEL_X`/`LABEL_W` literally
        // (one constant, five call sites), so their tops are what varies.
        assert_eq!(
            [
                LABEL_FIGURE_TOP,
                LABEL_HEIGHT_TOP,
                LABEL_VOLUME_TOP,
                LABEL_GEAR_TOPS.0,
                LABEL_GEAR_TOPS.1
            ],
            [137.0, 167.0, 198.0, 229.0, 260.0]
        );
    }

    /// The CH/EU gear-row swap is behavioural, not decoration: Chinese reads
    /// Protector-then-Weapon (`china :295-313` / `:276-294`), European reads
    /// Weapon-then-Protector. Two layouts, never an average of them.
    #[test]
    fn the_gear_rows_swap_between_the_race_trees() {
        assert_eq!(gear_row_tops(Race::CHINESE, ROW_GEAR_TOPS), (224.0, 255.0));
        assert_eq!(gear_row_tops(Race::EUROPEAN, ROW_GEAR_TOPS), (255.0, 224.0));
        assert_eq!(
            gear_row_tops(Race::CHINESE, LABEL_GEAR_TOPS),
            (229.0, 260.0)
        );
        assert_eq!(
            gear_row_tops(Race::EUROPEAN, LABEL_GEAR_TOPS),
            (260.0, 229.0)
        );
        // and the swap is a swap, not a shift: the pair of occupied rows is
        // the same set for both races.
        let (cp, cw) = gear_row_tops(Race::CHINESE, ROW_GEAR_TOPS);
        let (ep, ew) = gear_row_tops(Race::EUROPEAN, ROW_GEAR_TOPS);
        assert_eq!((cp.min(cw), cp.max(cw)), (ep.min(ew), ep.max(ew)));
    }

    /// The Explain panel's children are authored; its European art is the
    /// tree's one rect/art mismatch and is kept visible rather than lost.
    #[test]
    fn the_explain_panel_records_the_eu_rect_art_mismatch() {
        assert_eq!(EXPLAIN_NAME_RECT, (15.0, 18.0, 183.0, 14.0));
        assert_eq!(EXPLAIN_BODY_RECT, (15.0, 42.0, 183.0, 122.0));
        assert_eq!(EXPLAIN_CH_SIZE, (212.0, 180.0));
        // The EU tree declares 212x250 but ships a 220x236 sprite; we draw the
        // art at its own size, and this pins that the two genuinely differ.
        assert_ne!(EXPLAIN_EU_RECT, EXPLAIN_EU_ART);
        assert_eq!(EXPLAIN_EU_ART, (220.0, 236.0));
    }

    /// The protector row is one cycling control (the original is one slider),
    /// so stepping through it wraps and returns to where it started.
    #[test]
    fn the_protector_row_cycles_through_all_three_sets() {
        let mut set = Garment::Clothes;
        for _ in 0..Garment::ALL.len() {
            set = set.cycle(1);
        }
        assert_eq!(set, Garment::Clothes);
        assert_eq!(Garment::Clothes.cycle(-1), Garment::Heavy);
        assert_eq!(Garment::Clothes.cycle(1), Garment::Light);
    }

    /// `GDR_STA_TITLE` (`pscharactercreate{china,_europe}.txt:215`) is a baked
    /// 428x36 image at 47,111 — `text-custom.ddj`, shared by both race trees.
    /// We drew a hardcoded "Create Character" string with no textuisystem key.
    #[test]
    fn the_title_is_baked_art_at_its_resinfo_size() {
        assert_eq!((TITLE_ART_W, TITLE_ART_H), (428.0, 36.0));
        assert!(TITLE_CUSTOM_DDJ.ends_with("outer/text-custom.ddj"));
    }

    /// Padded art is drawn at its *authored* rect, not at its file size.
    ///
    /// Idea: several `interface/outer` sprites ship a texture that is larger
    /// than the picture inside it; the surplus texels are fully transparent
    /// (alpha bit 0 in the ARGB1555 payload) and always sit on the right and
    /// bottom edges, because the content is anchored at (0,0). The obvious
    /// reading is that the original blits the file 1:1 and lets the padding
    /// hang; then drawing the file into the smaller resinfo rect (what we do)
    /// would squash the picture by up to 2.8 %.
    ///
    /// That reading is wrong. The original draws each of these files into its
    /// authored rect:
    ///
    /// | art | file | drawn |
    /// |---|---|---|
    /// | `rotate_left.ddj` | 44x36 | **42x35** |
    /// | `rotate_right.ddj` | 44x36 | **42x35** |
    /// | `zoomin.ddj` | 40x36 | **39x35** |
    /// | `zoomout.ddj` | 40x36 | **39x35** |
    /// | `man_on.ddj` | 72x28 | **70x25** |
    /// | `man_off.ddj` | 72x28 | **70x25** |
    ///
    /// And the target size is not merely *close*, it is determined: for
    /// `man_on` no other size in 66..74 x 23..29 fits the original's pixels
    /// nearly as well as 70x25.
    ///
    /// The rule is "file -> authored rect", in both directions: `button.ddj` is
    /// 91x40 against an authored rect of 92x41, and there the original draws
    /// the *upscaled* 92x41.
    ///
    /// So changing these rects to the file size would be a **deviation from**
    /// the original (ADR-0009), not a fix.
    #[test]
    fn padded_art_is_drawn_at_the_authored_rect_not_the_file_size() {
        // (art, file size, drawn size) — file sizes are the DDS headers in
        // `Media.pk2`, drawn sizes are the resinfo rects above.
        const CASES: [(&str, (f32, f32), (f32, f32)); 4] = [
            (
                "man_*/woman_*.ddj",
                (72.0, 28.0),
                (MALE_RECT.2, MALE_RECT.3),
            ),
            (
                "rotate_left.ddj",
                (44.0, 36.0),
                (ROTATE_LEFT_RECT.2, ROTATE_LEFT_RECT.3),
            ),
            (
                "rotate_right.ddj",
                (44.0, 36.0),
                (ROTATE_RIGHT_RECT.2, ROTATE_RIGHT_RECT.3),
            ),
            (
                "zoomin.ddj/zoomout.ddj",
                (40.0, 36.0),
                (ROTATE_ZOOM_RECT.2, ROTATE_ZOOM_RECT.3),
            ),
        ];
        const DRAWN: [(f32, f32); 4] = [(70.0, 25.0), (42.0, 35.0), (42.0, 35.0), (39.0, 35.0)];

        for (case, drawn) in CASES.iter().zip(DRAWN) {
            assert_eq!(
                case.2, drawn,
                "{} is drawn at the size the original uses",
                case.0
            );
            assert_ne!(
                case.2, case.1,
                "{} would be a deviation from the original if drawn at its file size",
                case.0
            );
            // the padding is on the right/bottom only, so the drawn rect is
            // never larger than the file for these four
            assert!(case.2 .0 < case.1 .0 && case.2 .1 < case.1 .1);
        }

        // Female shares the male art and therefore the male rect.
        assert_eq!((FEMALE_RECT.2, FEMALE_RECT.3), (MALE_RECT.2, MALE_RECT.3));
    }

    /// `Section = Rotate` is a complete authored template and is transcribed
    /// verbatim: the 152x56 window (`GDR_STA_ROTATE`, :120-138) and the three
    /// child rects (`:626-682`). Nothing here is padding or taste.
    #[test]
    fn the_rotate_window_and_its_buttons_use_the_authored_resinfo_rects() {
        assert_eq!((ROTATE_WINDOW_W, ROTATE_WINDOW_H), (152.0, 56.0));
        assert_eq!(ROTATE_LEFT_RECT, (12.0, 11.0, 42.0, 35.0));
        assert_eq!(ROTATE_ZOOM_RECT, (57.0, 11.0, 39.0, 35.0));
        assert_eq!(ROTATE_RIGHT_RECT, (99.0, 11.0, 42.0, 35.0));
        // the two rotate buttons are the same size and sit symmetrically
        // about the zoom button's centre seam
        assert_eq!(ROTATE_LEFT_RECT.2, ROTATE_RIGHT_RECT.2);
        assert_eq!(ROTATE_LEFT_RECT.1, ROTATE_ZOOM_RECT.1);
    }

    /// The window's position comes from the original, not from authored data,
    /// so the test re-runs that cross-check: put the window at its inset in the
    /// original's 800x600 client area and the three authored child rects must
    /// land on the original's four black edges (client coords 647 / 692 / 734,
    /// y 457 — window coords 650/695/737, y 483 minus the (3,26) client
    /// offset).
    /// A centred window (the old `left: 50%`) fails this by 711 px.
    #[test]
    fn the_rotate_window_sits_at_the_bottom_right_inset() {
        const CLIENT_W: f32 = 800.0;
        const CLIENT_H: f32 = 600.0;
        let left = CLIENT_W - ROTATE_WINDOW_RIGHT_INSET - ROTATE_WINDOW_W;
        let top = CLIENT_H - ROTATE_WINDOW_BOTTOM_INSET - ROTATE_WINDOW_H;
        assert_eq!((left, top), (635.0, 446.0), "window origin");

        // cross-check: children on the original's edges, 0 px error
        assert_eq!(left + ROTATE_LEFT_RECT.0, 647.0);
        assert_eq!(left + ROTATE_ZOOM_RECT.0, 692.0);
        assert_eq!(left + ROTATE_RIGHT_RECT.0, 734.0);
        assert_eq!(top + ROTATE_LEFT_RECT.1, 457.0);

        // and the inset is an inset: a centred window would sit here instead
        let centred = CLIENT_W / 2.0 - ROTATE_WINDOW_W / 2.0;
        assert!(
            (left - centred).abs() > 200.0,
            "bottom-right anchored, not centred"
        );
    }

    /// Rotating is a symmetric step around the podium pose, and the podium
    /// pose itself (facing the camera) is what a fresh screen shows.
    #[test]
    fn rotating_right_then_left_returns_to_the_podium_pose() {
        // A camera looking down -Z (yaw 0) is the case the old constant `PI`
        // covered, so the podium pose is unchanged for it.
        let podium = preview_rotation(0.0, 0.0);
        assert!(podium.angle_between(Quat::from_rotation_y(PI)) < 1e-5);
        // ...and a stage whose camera looks the other way turns the body
        // around with it, instead of showing its back (the Jangan defect).
        let jangan = preview_rotation(PI, 0.0);
        assert!(
            jangan.angle_between(Quat::from_rotation_y(0.0)) < 1e-5,
            "a camera at yaw PI must be met by a body at yaw 0"
        );

        let view = CharCreatePreviewView::default();
        assert_eq!(view.yaw, 0.0);
        assert!(!view.zoomed_in);

        let turned = preview_rotation(0.0, PREVIEW_ROTATE_STEP);
        assert!(
            turned.angle_between(podium) > 1e-3,
            "a click turns the body"
        );
        let back = preview_rotation(0.0, PREVIEW_ROTATE_STEP - PREVIEW_ROTATE_STEP);
        assert!(back.angle_between(podium) < 1e-5);
        // 24 clicks are a full turn (the stated openroad step choice)
        assert!(((PI / PREVIEW_ROTATE_STEP) - 12.0).abs() < 1e-5);
    }

    /// The CH garment captions are `Protector` and `Armor` in the data, not the
    /// "Light Armor"/"Heavy Armor" we invented. The EU keys genuinely DO read
    /// "Light Armor"/"Heavy Armor", so only the CH pair was wrong (#304).
    #[test]
    fn chinese_garment_fallbacks_match_the_textdata() {
        let (ch_light_key, ch_light) = Garment::Light.label(Race::CHINESE);
        assert_eq!(ch_light_key, "UIO_NEWCHAR_STT_LIGHT_ARMOR");
        assert_eq!(ch_light, "Protector");

        let (ch_heavy_key, ch_heavy) = Garment::Heavy.label(Race::CHINESE);
        assert_eq!(ch_heavy_key, "UIO_NEWCHAR_STT_HEAVY_ARMOR");
        assert_eq!(ch_heavy, "Armor");

        // unchanged on purpose — these two are correct against the data
        assert_eq!(Garment::Light.label(Race::EUROPEAN).1, "Light Armor");
        assert_eq!(Garment::Heavy.label(Race::EUROPEAN).1, "Heavy Armor");
    }

    /// The scale byte is two nibbles of five steps, not a 0..255 scale: the
    /// original's own values (0x22, 0x11, 0x00 and the decisive 0x20) move the
    /// nibbles independently, and both axes ship "one of 5" texts. So the
    /// untouched default has to be the middle step in BOTH nibbles.
    #[test]
    fn the_scale_byte_is_two_nibbles_and_the_default_is_the_middle_step() {
        assert_eq!(SCALE_STEPS, 5);
        assert_eq!(SCALE_MIDDLE_STEP, 2);
        assert_eq!(DEFAULT_SCALE, 0x22);
        // the packing is per nibble: 0x20 is only expressible if
        // the two axes are independent
        assert_eq!(pack_scale(2, 0), 0x20);
        assert_eq!(pack_scale(1, 1), 0x11);
        assert_eq!(pack_scale(0, 0), 0x00);
        // every step of both axes stays inside its own nibble
        for high in 0..SCALE_STEPS {
            for low in 0..SCALE_STEPS {
                let byte = pack_scale(high, low);
                assert_eq!(byte >> 4, high);
                assert_eq!(byte & 0x0f, low);
            }
        }
        // and the old defect stays dead: 1.0 + 0x22/255 is not a scale factor
        assert!(
            DEFAULT_SCALE < 0x55,
            "a nibble pair never fills a 0..255 range"
        );
    }

    /// The nibble assignment, as the original sends it: from the
    /// untouched default, Height +1 puts `0x23` on the wire and Volume +1
    /// `0x32`. A swap of these two lines is exactly the defect that kept the
    /// rows read-only, so it gets a test rather than only a comment.
    #[test]
    fn height_is_the_low_nibble_and_volume_the_high_one() {
        let default = CharCreateSelection::default();
        assert_eq!(selection_scale(&default), 0x22);

        let taller = CharCreateSelection {
            height: SCALE_MIDDLE_STEP + 1,
            ..Default::default()
        };
        assert_eq!(selection_scale(&taller), 0x23);

        let wider = CharCreateSelection {
            volume: SCALE_MIDDLE_STEP + 1,
            ..Default::default()
        };
        assert_eq!(selection_scale(&wider), 0x32);
    }

    /// The two scale rows are a range: five steps, clamped at both ends, and
    /// the row state a thumb reads is the step index itself.
    #[test]
    fn the_scale_rows_report_five_clamped_steps() {
        let selection = CharCreateSelection::default();
        assert_eq!(selection.height, SCALE_MIDDLE_STEP);
        assert_eq!(selection.volume, SCALE_MIDDLE_STEP);

        // what `on_slider_step` computes, without a world
        let last = SCALE_STEPS - 1;
        let clamped = |current: u8, step: i32| (current as i32 + step).clamp(0, last as i32) as u8;
        assert_eq!(clamped(0, -1), 0);
        assert_eq!(clamped(last, 1), last);
        assert_eq!(clamped(SCALE_MIDDLE_STEP, 1), 3);
        // a clamped end never leaves its nibble
        assert_eq!(pack_scale(clamped(last, 1), clamped(0, -1)), 0x40);
    }

    /// The original rejects with `(unsigned)(len - 2) > 10`, so the accepted
    /// window is exactly 2..=12 — the "minimum is UNKNOWN" note that stood at
    /// this constant is settled. The upper end of that window is unreachable:
    /// 15 typed characters become 12 in the
    /// field and no message appears, so the cap belongs on the input and only
    /// the lower end is a message. This test pins both halves.
    #[test]
    fn the_name_length_window_matches_the_original() {
        assert_eq!((MIN_NAME_LEN, MAX_NAME_LEN), (2, 12));
        for len in 0..16usize {
            let rejected = (len.wrapping_sub(2)) > 10;
            assert_eq!(
                rejected,
                !(MIN_NAME_LEN..=MAX_NAME_LEN).contains(&len),
                "len {len}"
            );
            // what our own screen can produce: the edit cap truncates first,
            // so a message is only possible below the minimum.
            let capped = len.min(MAX_NAME_LEN);
            assert_eq!(capped < MIN_NAME_LEN, len < MIN_NAME_LEN, "len {len}");
        }
    }

    /// The charset is the `abusefilter.txt` allow table, column 4: digits,
    /// both cases and `_` — nothing else. A name built only from those four
    /// ranges is accepted; any other character is refused.
    #[test]
    fn the_name_charset_is_the_abusefilter_allow_table() {
        assert_eq!(
            NAME_ALLOWED_RANGES,
            [('0', '9'), ('A', 'Z'), ('_', '_'), ('a', 'z')]
        );
        for name in ["Player001", "Tester77", "abcdefghijkl", "Tester_77"] {
            assert!(name.chars().all(name_charset_allows), "{name} was accepted");
        }
        for bad in [' ', '-', '.', '\u{e9}', '@', '!'] {
            assert!(!name_charset_allows(bad), "{bad:?} is not in the table");
        }
        // column 5 is the other N1 candidate and differs ONLY in Latin-1
        // accents; if that ever wins, this is the whole delta.
        assert!(NAME_ALLOWED_RANGES_COLUMN5_EXTRA
            .iter()
            .all(|(lo, _)| !name_charset_allows(*lo)));
    }

    /// The two hard gates, in the original's order and with the original's
    /// race asymmetry: CN checks the weapon only, EU checks the protector
    /// first. A fresh screen refuses even though a value is displayed, because
    /// the refs start at 0.
    #[test]
    fn a_fresh_screen_is_gated_on_the_weapon_and_eu_also_on_the_protector() {
        let mut cn = CharCreateSelection::default();
        assert_eq!(cn.race, Race::CHINESE);
        assert_eq!(
            gate_selection(&cn),
            Err(("UIO_MSG_ERROR_CHARACTER_SELECTWEAPON", "Select a Weapon."))
        );
        // a protector pick alone does not open the Chinese gate
        cn.garment_chosen = true;
        assert!(gate_selection(&cn).is_err());
        cn.weapon_chosen = true;
        assert_eq!(gate_selection(&cn), Ok(()));
        // ... and the Chinese screen never asks for a protector
        let cn_weapon_only = CharCreateSelection {
            weapon_chosen: true,
            ..Default::default()
        };
        assert_eq!(gate_selection(&cn_weapon_only), Ok(()));

        let mut eu = CharCreateSelection {
            race: Race::EUROPEAN,
            weapon_chosen: true,
            ..Default::default()
        };
        assert_eq!(
            gate_selection(&eu),
            Err(("UIO_MSG_ERROR_CHARACTER_SELECTARMOR", "Select a Protector."))
        );
        eu.garment_chosen = true;
        assert_eq!(gate_selection(&eu), Ok(()));
    }

    /// The stale demand: "Select a Weapon." is written once, and the notice line
    /// only changes when somebody writes a new one — so after the weapon was
    /// picked the demand stood on a screen that was ready to send. Asserted in
    /// both directions, plus the line that must **not** be cleared.
    #[test]
    fn the_gate_line_goes_when_the_gate_is_satisfied() {
        use crate::plugins::textdata::ClientUiStrings;
        use crate::scenes::intro_v2::chrome::InfoTextV2;

        fn app_showing(line: &str, selection: CharCreateSelection) -> App {
            let mut app = App::new();
            app.add_message::<InfoTextV2Update>()
                .init_resource::<ClientUiStrings>()
                .insert_resource(selection)
                .add_systems(Update, super::clear_satisfied_gate_line);
            app.world_mut().spawn((InfoTextV2, Text(line.to_string())));
            app
        }

        fn cleared(app: &App) -> bool {
            let messages = app.world().resource::<Messages<InfoTextV2Update>>();
            let mut cursor = messages.get_cursor();
            cursor.read(messages).any(|line| line.0.is_empty())
        }

        // The demand still stands: a fresh screen has chosen no weapon.
        let mut app = app_showing("Select a Weapon.", CharCreateSelection::default());
        app.update();
        assert!(
            !cleared(&app),
            "the demand may not be taken down while it is unmet"
        );

        // The weapon is picked: the line goes.
        let mut app = app_showing(
            "Select a Weapon.",
            CharCreateSelection {
                weapon_chosen: true,
                ..Default::default()
            },
        );
        app.update();
        assert!(cleared(&app), "the met demand stayed on the screen");

        // Not our line: a server refusal survives a weapon pick, because
        // picking a weapon has not answered it.
        let mut app = app_showing(
            "This ID already exists.",
            CharCreateSelection {
                weapon_chosen: true,
                ..Default::default()
            },
        );
        app.update();
        assert!(!cleared(&app), "an unrelated line was wiped");
    }

    /// The overlay pass has to bring its own light.
    ///
    /// [`spawn_figure_overlay_camera`] moves the preview onto the
    /// `CreateFigure` render layer ([`tag_figure_overlay_meshes`]), and a
    /// `DirectionalLight` only lights the layers it is on — the scene's sun is
    /// on the main layer, so the figure was lit by the sky probe alone and read
    /// as a flat silhouette. The light is a child of the camera with an
    /// identity transform (the paper doll's headlight construction), so it
    /// follows the camera flight.
    #[test]
    fn the_overlay_camera_carries_a_headlight_on_the_figure_layer() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>();
        let example = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../config.example.yaml")
            .with_extension("")
            .to_str()
            .expect("the example path is utf-8")
            .to_string();
        let config = crate::plugins::config::ClientConfig::from_file(&example)
            .expect("config.example.yaml loads");
        app.insert_resource(config);
        app.world_mut()
            .run_system_once(spawn_figure_overlay_camera)
            .expect("the overlay camera spawns");

        let figure_layer = RenderLayers::layer(CameraLayers::CreateFigure.into());
        let mut cameras = app
            .world_mut()
            .query_filtered::<Entity, With<FigureOverlayCamera>>();
        let camera = cameras
            .iter(app.world())
            .next()
            .expect("the overlay camera exists");

        let mut lights = app
            .world_mut()
            .query::<(&DirectionalLight, &RenderLayers, &ChildOf)>();
        let on_the_layer: Vec<_> = lights
            .iter(app.world())
            .filter(|(_, layers, _)| layers.intersects(&figure_layer))
            .collect();
        assert_eq!(
            on_the_layer.len(),
            1,
            "exactly one light on the figure's own layer"
        );
        assert_eq!(
            on_the_layer[0].2.parent(),
            camera,
            "a headlight is parented to the camera, or it stops following the flight"
        );
    }
}

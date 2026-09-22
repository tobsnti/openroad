//! Options -> Setting pane (`OptionsTab::Game`): the gameplay/HUD toggle rows.
//!
//! Idea: vanilla's Game pane declares **no rows of its own**. `ifoption_game.txt`
//! is four blocks — two section headers (`UIIT_STT_NAMEVIEW` at `13,19`,
//! `UIIT_STT_GAMESET` at `13,162`) over two `CIFScrollManager` viewports
//! (`14,46,336,101` and `14,189,336,101`) that the original fills at runtime. The
//! row shape comes from `ifgameoptionslot.txt`: label `14,8,70,16`, a second
//! static `93,8,31,16`, and one `CIFCheckBox` `130,6,16,16` on
//! `com_checkbutton_off.ddj`. So the rows below are a table, not a layout.
//!
//! That is the **classic** tree, and it is the one we build — but say so out
//! loud, because the corpus ships a second one. `Media/res_ui/optionwnd.2dt`
//! holds a 4th-generation Options window (20 `CNIFGameOptionSlot` instances
//! under four sub-tabs `UIIT_STT_DISPLAY` / `_COMMUNITY` / `_NAMEVIEW` /
//! `_SILKMALL_ETC`), and `APPLY_UI_4TH` is uncommented in `Media/config/define.txt`
//! (line 15; the file is CP949, so grep it accordingly). Which generation
//! v1.188 actually renders is still UNKNOWN.
//!
//! What tips it here is that the two files above carry **no `#ifdef` at all** —
//! unlike `ifoption.txt`, `ifoption_video.txt` and `ifvideooptionslot.txt`, which
//! do gate blocks on `APPLY_UI_4TH`. The Game tab's own resinfo is unconditional,
//! so this layout is what its data describes either way. If the generation
//! question later resolves toward the 2DT tree, this pane's row set is scoped
//! wrong by construction and the `Setting` ids with no home here (2001-2004,
//! 2008-2009, 2016-2018, 2025-2028) gain one.
//!
//! Each row carries its `SROptionSet` id (`docs/formats/sroptionset.md:106-133`)
//! and its textuisystem key, and writes straight into
//! `GameOptions.gameplay.toggles`, which is already id-keyed and already
//! persisted. The ids with a runtime consumer today are the five name
//! indicators (`hud/nameplates.rs:88-92`) and the two Community switches
//! 2002/2003 (`hud/petition.rs::auto_refusal`) — those are `Backing::Live`;
//! every other row renders but is inert and dimmed rather than silently doing
//! nothing. 2024 `MonsterConditionCheckbox` has no reader in the tree either,
//! so it is inert like its three siblings.
//!
//! Why 2021/2022/2023/2024 are inert: the four ids 2021-2024 are **one** family,
//! not four features. Their keys are `UIIT_STT_QUICKSTATE_OWNER` / `_COS` /
//! `_PARTY` / `_MONSTER` (textuisystem :866-869) — the same `QUICKSTATE` widget,
//! `GDR_QUICK_STATE*` (`ginterface.txt:6,25`), gated per **entity category**.
//! openroad has no quick-state widget at all: it went with the over-head HP bar.
//! So none of the four has a display to gate — the COS
//! stack (`hud/cos_status.rs`, `ifcosstatus.txt`) and the party window
//! (`hud/party/`, opened by keymap 3005 only) are *different* resinfo trees
//! with their own gauges, and hanging these ids on them would wire an option to
//! the wrong widget. Giving them a reader therefore means extending the
//! quick-state widget to three more categories first, which is a HUD change and
//! not an options one; documented rather than faked here.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::ScrollPosition;
use bevy::ui_widgets::{Activate, Button};

use crate::plugins::settings::options::{default_toggle, GameOptions};
use crate::plugins::settings::persistence::{sroptionset_import_path, ImportOriginalSettings};
use crate::plugins::settings::tooltip::{attach_tooltip, spawn_tooltip_line};
use crate::plugins::textdata::ClientUiStrings;

/// Section header positions, pane-local (`ifoption_game.txt:31,50`).
const NAME_VIEW_HEADER: (f32, f32) = (13.0, 19.0);
const GAME_SET_HEADER: (f32, f32) = (13.0, 162.0);
/// The two scroll viewports (`ifoption_game.txt:69,88`).
const NAME_VIEW_VIEWPORT: (f32, f32, f32, f32) = (14.0, 46.0, 336.0, 101.0);
const GAME_SET_VIEWPORT: (f32, f32, f32, f32) = (14.0, 189.0, 336.0, 101.0);

/// Row metrics from the `ifgameoptionslot.txt` prototype.
const ROW_H: f32 = 24.0;
const ROW_LABEL: (f32, f32, f32, f32) = (14.0, 8.0, 70.0, 16.0);
const ROW_MARK: (f32, f32, f32, f32) = (93.0, 8.0, 31.0, 16.0);
const ROW_CHECKBOX: (f32, f32, f32, f32) = (130.0, 6.0, 16.0, 16.0);

/// What `CIFScrollManager` ships with, and it is not a bare clip: the class's
/// own prototype `ifscrollmanager.txt` is a single `GDR_VSCROLL:CIFVerticalScroll`
/// **16 px wide** (rect `312,15,16,153`), and that class's parts are
/// `ifverticalscroll.txt` — a 16x16 thumb on `com_scroll_button.ddj` plus
/// `chat_arrow_up.ddj` / `chat_arrow_down.ddj`, with **no track art** (the
/// manager's own 9-slice shows through).
/// So the original's Setting list is reachable by dragging a visible bar, not
/// only by a wheel. Same three assets and the same "no track" rule as
/// `hud/chat/ui.rs:78-82`, which is the tree's transcription of that file and
/// the pattern this pane docks onto.
const SCROLLBAR_W: f32 = 16.0;
const SCROLL_ARROW: f32 = 16.0;
const SCROLL_THUMB_ART: &str = "media://interface/ifcommon/com_scroll_button.ddj";
const SCROLL_UP_ART: &str = "media://interface/chattingwnd/chat_arrow_up.ddj";
const SCROLL_DOWN_ART: &str = "media://interface/chattingwnd/chat_arrow_down.ddj";
/// One wheel notch and one arrow click move exactly one row. `ROW_H` is the
/// list's only natural unit, and a row-quantised step is what keeps a 4.2-row
/// viewport from ending on a half-drawn label.
const WHEEL_STEP: f32 = ROW_H;

/// The prototype's own right edge (checkbox `130 + 16`) versus the width the
/// manager it is instantiated into actually offers (`336` minus the 16 px
/// `CIFVerticalScroll`): the slot art was authored 174 px narrower than its
/// host, and vanilla's Korean labels fit the 70 px label column where the
/// English ones do not (`Party Member Status` is 105 px wide at our 11 px
/// FiraSans-Bold, `Exchange Request` 91, `Monster Condition` 90).
///
/// Stated deviation (ADR 0009): the label keeps its authored origin `14,8` and
/// its authored **single** line — the slot's static is 16 px high and vanilla's
/// wrapping class is `CIFPML`, which this prototype does not use — and the two
/// right-hand columns are anchored to the host's usable right edge instead of
/// to the prototype's left origin, keeping every authored gap (state column 31
/// wide, 6 px to the checkbox, checkbox 16). Without this the label wrapped to
/// two lines inside a 24 px row and overprinted the row below it.
const ROW_RIGHT_SHIFT: f32 =
    (GAME_SET_VIEWPORT.2 - SCROLLBAR_W) - (ROW_CHECKBOX.0 + ROW_CHECKBOX.2);

/// The three columns as this pane draws them: authored y, authored heights,
/// authored 9 px gap between label and state column ([`ROW_LABEL`] ends at 84,
/// [`ROW_MARK`] starts at 93), with the label taking every pixel the shift
/// frees. The authored constants above stay untouched because they are the
/// citation; these are the drawing.
const ROW_LABEL_W: f32 = (ROW_MARK.0 + ROW_RIGHT_SHIFT) - 9.0 - ROW_LABEL.0;
const ROW_LABEL_DRAWN: (f32, f32, f32, f32) = (ROW_LABEL.0, ROW_LABEL.1, ROW_LABEL_W, ROW_LABEL.3);
const ROW_MARK_DRAWN: (f32, f32, f32, f32) = (
    ROW_MARK.0 + ROW_RIGHT_SHIFT,
    ROW_MARK.1,
    ROW_MARK.2,
    ROW_MARK.3,
);
const ROW_CHECKBOX_DRAWN: (f32, f32, f32, f32) = (
    ROW_CHECKBOX.0 + ROW_RIGHT_SHIFT,
    ROW_CHECKBOX.1,
    ROW_CHECKBOX.2,
    ROW_CHECKBOX.3,
);
/// The widest label this pane can draw, in the pane's own font
/// (`FiraSans-Bold.ttf` at the 11 px this file passes to `TextFont`):
/// `Party Member Status` = 105 px, `Exchange Request` = 91, `Monster
/// Condition` = 90, `Beginner's Mark` = 80. The authored 70 px column breaks
/// six of the fifteen labels onto a second line; [`ROW_LABEL_W`] must stay
/// above this or the overlap is back.
#[cfg(test)]
const WIDEST_LABEL_PX: f32 = 105.0;

const CHECKBOX_OFF: &str = "media://interface/ifcommon/com_checkbutton_off.ddj";
const CHECKBOX_ON: &str = "media://interface/ifcommon/com_checkbutton_on.ddj";
const CHECKBOX_DISABLED: &str = "media://interface/ifcommon/com_checkbutton_on_disable.ddj";

/// `FontColor="255,255,255,255"` on the slot prototype (`ifgameoptionslot.txt:11`).
const LABEL_COLOR: Color = Color::srgb_u8(255, 255, 255);
/// The two section headers are gold, not white: `FontColor="255,239,218,164"`
/// (AARRGGBB) on both `CIFStatic`s in `ifoption_game.txt`. They also carry
/// `DDJ="interface\option\opt_video_quality_tab.ddj"` with `ClientRect="28,12,0,0"`,
/// i.e. vanilla draws each header on a tab graphic — not transcribed here yet.
const HEADER_COLOR: Color = Color::srgb_u8(239, 218, 164);
/// openroad-only: rows whose toggle nothing reads yet are dimmed and inert, so
/// they read as present-but-unimplemented instead of as silent no-ops.
const LABEL_INERT: Color = Color::srgb_u8(128, 128, 128);

/// The row prototype's **second** static column, `GDR_GAME_OPTION_SLOT_STA2`
/// (`ifgameoptionslot.txt:25`, `CIFStatic` id 2, rect `93,8,31,16`) — the one
/// between the label and the checkbox. The prototype declares **no `Text=`** on
/// it, so vanilla fills it at runtime and the *content* is unknown.
///
/// openroad's decision (ADR 0009: a stated deviation is legitimate, an
/// unsourced number is not): the column shows the row's **state text**, `On` /
/// `Off`, taken from the two keys the original itself ships for exactly that
/// purpose — `UIIT_STT_ON` / `UIIT_STT_OFF` (textuisystem :959/:960, the
/// same `UIIT_STT_OFF` the item mall's toggle button uses). Reasons, in order:
/// 31 px at font 12 fits a two- or three-letter word and little else; the state
/// is the only datum
/// that varies per row and is not already in the label; and a checkbox glyph
/// alone is the weakest possible state indicator for a colour-blind or
/// low-vision player, so a redundant textual state is an accessibility gain
/// even if vanilla turns out to have left the column empty.
///
/// If the column turns out to be empty in the original, or to show something
/// else, the two constants and [`spawn_state_column`] are the whole change.
const STATE_ON_KEY: &str = "UIIT_STT_ON";
const STATE_OFF_KEY: &str = "UIIT_STT_OFF";
const STATE_ON_FALLBACK: &str = "On";
const STATE_OFF_FALLBACK: &str = "Off";
/// What an *inert* row (no consumer yet) puts in that column instead: a dash,
/// so "present but not wired" stays visible rather than implied.
const STATE_INERT_MARK: &str = "-";

/// The original's hover help for this pane: `UIIT_STT_GAMESET_TTDESC_01..05`
/// (textuisystem :985-989). **Five strings for seven rows**, and not in row
/// order — the mapping below is by what each English string names, which is the
/// only thing that ties a tooltip to a row here:
///
/// | key | string names | row |
/// |---|---|---|
/// | `_01` | "status of COS such as Pet, Vehicles" | 2022 |
/// | `_02` | "party member status while in a party" | 2023 |
/// | `_03` | "warning when the HP drops" | 2019 |
/// | `_04` | "warning when the MP drops" | 2020 |
/// | `_05` | "Warning tone is sounded" | Warning Sound (no id) |
///
/// So **Self Condition (2021) and Monster Condition (2024) have no tooltip in
/// the original's data** — that is the answer to the open count in the
/// `GAME_SET_ROWS` comment above, and the reason this table has five rows.
///
/// The `Indicate Name Settings` section is deliberately left without help:
/// `UIIT_STT_GAME_NAMEVIEW_TTDESC_01..06` (:1027-1032) exists, but its order
/// disagrees with the id order this pane renders (see the `NAME_VIEW_ROWS`
/// comment), so every assignment would be a guess.
const ROW_TOOLTIPS: [(&str, &str, &str); 5] = [
    (
        "UIIT_STT_QUICKSTATE_COS",
        "UIIT_STT_GAMESET_TTDESC_01",
        "Displays the status of COS such as Pet, Vehicles, etc.",
    ),
    (
        "UIIT_STT_QUICKSTATE_PARTY",
        "UIIT_STT_GAMESET_TTDESC_02",
        "Displays party member status while in a party.",
    ),
    (
        "UIIT_STT_CAUTION_HP",
        "UIIT_STT_GAMESET_TTDESC_03",
        "Displays warning when the HP drops below a certain level.",
    ),
    (
        "UIIT_STT_CAUTION_MP",
        "UIIT_STT_GAMESET_TTDESC_04",
        "Displays warning when the MP drops below a certain level.",
    ),
    (
        "UIIT_STT_CAUTION_SOUND",
        "UIIT_STT_GAMESET_TTDESC_05",
        "Warning tone is sounded when the HP / MP drops below a certain level.",
    ),
];

/// The `Community` rows' help, from a **different** block: the two switches are
/// named outright by `UIIT_STT_GAME_OPTION_TTDESC_06` and `_07` (textuisystem
/// :1038/:1039), so the mapping is the string's own wording, not an order
/// guess. (`_08` names the personal-message switch 2004, which this pane does
/// not render — see [`COMMUNITY_ROWS`].)
const COMMUNITY_TOOLTIPS: [(&str, &str, &str); 2] = [
    (
        "UIIT_STT_INVITE_PARTY_SIGN",
        "UIIT_STT_GAME_OPTION_TTDESC_06",
        "Party invitation is allowed. If the check box is not clicked, party invitation will not activate so please be aware of it.",
    ),
    (
        "UIIT_STT_REQUEST_EXCHANGE_SIGN",
        "UIIT_STT_GAME_OPTION_TTDESC_07",
        "Exchange request is allowed. If the check box is not clicked, exchange request will not activate so please be aware of it.",
    ),
];

/// The resolved help text for a row's label key, or `None` when the original
/// ships none for it (see [`ROW_TOOLTIPS`], [`COMMUNITY_TOOLTIPS`]).
fn row_tooltip(ui_strings: &ClientUiStrings, label_key: &str) -> Option<String> {
    ROW_TOOLTIPS
        .iter()
        .chain(COMMUNITY_TOOLTIPS.iter())
        .find(|(row_key, _, _)| *row_key == label_key)
        .map(|(_, key, english)| ui_strings.get_or(key, english).to_string())
}

/// Whether flipping a row changes anything in this build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Backing {
    /// A system reads this id today.
    Live,
    /// Stored and persisted, but nothing consumes it yet.
    Inert,
}

/// One checkbox row.
struct ToggleRow {
    /// `SROptionSet` id, or `None` for a vanilla row whose id is not known.
    id: Option<u16>,
    key: &'static str,
    english: &'static str,
    backing: Backing,
}

/// `Indicate Name Settings`. Ordered by ascending id, which matches the label
/// block's own order (`UIIT_STT_*_SIGN`, textuisystem :854-859). The tooltip
/// block `UIIT_STT_GAME_NAMEVIEW_TTDESC_01..06` lists a *different* order, so
/// the on-screen row order stays UNKNOWN; ids win because they are verifiable.
const NAME_VIEW_ROWS: [ToggleRow; 6] = [
    ToggleRow {
        id: Some(2010),
        key: "UIIT_STT_ONESELF_SIGN",
        english: "Own Name",
        backing: Backing::Live,
    },
    ToggleRow {
        id: Some(2011),
        key: "UIIT_STT_OTHER_CHAR_SIGN",
        english: "Other Name",
        backing: Backing::Live,
    },
    ToggleRow {
        id: Some(2012),
        key: "UIIT_STT_MONSTER_SIGN",
        english: "Monster Name",
        backing: Backing::Live,
    },
    ToggleRow {
        id: Some(2013),
        key: "UIIT_STT_NPC_SIGN",
        english: "NPC Name",
        backing: Backing::Live,
    },
    ToggleRow {
        id: Some(2014),
        key: "UIIT_STT_GUILDVIEW_SIGN",
        english: "Guild Name",
        backing: Backing::Live,
    },
    // The NAMEVIEW section has six rows but only five known ids — the beginner's
    // mark has a label and a tooltip (TTDESC_06) and no id we can place.
    ToggleRow {
        id: None,
        key: "UIIT_STT_FIRSTSTEP_MARK_SIGN",
        english: "Beginner\u{2019}s Mark",
        backing: Backing::Inert,
    },
];

/// `Game Settings`. The seven labels the vanilla block offers (textuisystem
/// :866-872); five of them have tooltips (`UIIT_STT_GAMESET_TTDESC_01..05`) and
/// two do not, which is why the tooltip count is not the row count.
const GAME_SET_ROWS: [ToggleRow; 7] = [
    ToggleRow {
        id: Some(2021),
        key: "UIIT_STT_QUICKSTATE_OWNER",
        english: "Self Condition",
        backing: Backing::Inert,
    },
    ToggleRow {
        id: Some(2022),
        key: "UIIT_STT_QUICKSTATE_COS",
        english: "COS Condition",
        backing: Backing::Inert,
    },
    ToggleRow {
        id: Some(2023),
        key: "UIIT_STT_QUICKSTATE_PARTY",
        english: "Party Member Status",
        backing: Backing::Inert,
    },
    // Inert like its three siblings: the reader this row was marked `Live` for
    // went with the over-head vitality bars. Nothing in the tree reads 2024, so
    // a clickable switch here would be the dead wire this table exists to avoid.
    ToggleRow {
        id: Some(2024),
        key: "UIIT_STT_QUICKSTATE_MONSTER",
        english: "Monster Condition",
        backing: Backing::Inert,
    },
    ToggleRow {
        id: Some(2019),
        key: "UIIT_STT_CAUTION_HP",
        english: "HP Warning",
        backing: Backing::Inert,
    },
    ToggleRow {
        id: Some(2020),
        key: "UIIT_STT_CAUTION_MP",
        english: "MP Warning",
        backing: Backing::Inert,
    },
    // The warning *tone* has a label and a tooltip but no id in the documented
    // 2001..=2028 range.
    ToggleRow {
        id: None,
        key: "UIIT_STT_CAUTION_SOUND",
        english: "Warning Sound",
        backing: Backing::Inert,
    },
];

/// The `Community` group (`docs/formats/sroptionset.md:110-113`): the two
/// "refuse" switches this pane never offered, and which nothing read.
///
/// Both ids have a real consumer in the original: its 0x3080 petition handler
/// checks 2002 for petition types 2 and 3 and 2003 for type 1, and with the
/// box unchecked opens **no dialog** — it sends a C->S `0x3080` decline right
/// away. So the answer to "does the original hide the request or refuse it?"
/// is **refuse it, immediately**; the asker is never left waiting. Both checks
/// sit directly behind the type dispatch, ahead of every state test, so the
/// switch applies unconditionally (also mid-exchange). The reader is
/// `hud/petition.rs::auto_refusal`.
///
/// **2004 `PersonalMsgCheckbox` is deliberately not a row here.** Its slot is
/// dead *in the original*: the Setting group's writer and reader skip it, and
/// nothing reads it. `UIIT_STT_GET_WHISPER_SIGN` and
/// `UIIT_STT_GAME_OPTION_TTDESC_08` survive as strings for a row v1.188 does
/// not render. Adding it would invent a feature, not clone one.
const COMMUNITY_ROWS: [ToggleRow; 2] = [
    ToggleRow {
        id: Some(2002),
        key: "UIIT_STT_INVITE_PARTY_SIGN",
        english: "Party Invitation",
        backing: Backing::Live,
    },
    ToggleRow {
        id: Some(2003),
        key: "UIIT_STT_REQUEST_EXCHANGE_SIGN",
        english: "Exchange Request",
        backing: Backing::Live,
    },
];

/// The import control's pane-local rect. openroad-only, so there is no
/// `ifoption_game.txt` line to cite: it fills the empty band to the right of
/// the first section header (`NAME_VIEW_HEADER` at x 13, viewport starts at
/// y 46), which is the only free space in the pane no other control claims.
const IMPORT_BUTTON: (f32, f32, f32, f32) = (206.0, 17.0, 144.0, 16.0);

/// Marks the import control so its observer can tell it from a toggle row.
#[derive(Component)]
struct ImportOriginalSettingsButton;

/// A checkbox bound to an `SROptionSet` id.
#[derive(Component, Clone, Copy)]
pub(crate) struct GameToggle {
    id: u16,
    live: bool,
}

/// A row's checkbox state. A missing entry (hand-edited `user_settings.yaml`,
/// or a partial `SROptionSet.dat` import) falls back to the *shipped* value of
/// that id rather than to a blanket "on" — the original ships 2018-2024, 2026
/// and 2028 off (`settings::options::SHIPPED_TOGGLES`).
pub(crate) fn toggle_on(options: &GameOptions, id: u16) -> bool {
    options
        .gameplay
        .toggles
        .get(&id)
        .copied()
        .unwrap_or_else(|| default_toggle(id))
}

/// The three checkbox states, resolved once so the row spawner needs no
/// `AssetServer` inside the child-spawner closures.
struct CheckboxArt {
    off: Handle<Image>,
    on: Handle<Image>,
    disabled: Handle<Image>,
}

/// Build the Setting pane's two sections into an already-positioned pane node.
pub(crate) fn spawn_game_pane(
    pane: &mut RelatedSpawnerCommands<ChildOf>,
    asset_server: &AssetServer,
    font: &Handle<Font>,
    ui_strings: &ClientUiStrings,
    options: &GameOptions,
) {
    let art = CheckboxArt {
        off: asset_server.load(CHECKBOX_OFF),
        on: asset_server.load(CHECKBOX_ON),
        disabled: asset_server.load(CHECKBOX_DISABLED),
    };

    // The `Community` rows share the `Game Settings` viewport. Stated
    // deviation (ADR 0009): vanilla files them under a `Community` group that
    // only the 4th-generation window has a surface for, and the classic
    // `ifoption_game.txt` declares exactly two headers and two viewports — a
    // third section here would mean inventing a rect and a header position.
    // The second viewport is a `CIFScrollManager` the original fills at
    // runtime and our stack already overflows it, so appending two rows is
    // inside what the data describes; making them unreachable is not.
    for (section, (header, pos, viewport, rows)) in [
        (
            "UIIT_STT_NAMEVIEW",
            NAME_VIEW_HEADER,
            NAME_VIEW_VIEWPORT,
            NAME_VIEW_ROWS.iter().collect::<Vec<&ToggleRow>>(),
        ),
        (
            "UIIT_STT_GAMESET",
            GAME_SET_HEADER,
            GAME_SET_VIEWPORT,
            GAME_SET_ROWS
                .iter()
                .chain(COMMUNITY_ROWS.iter())
                .collect::<Vec<&ToggleRow>>(),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let section = section as u8;
        pane.spawn((
            Text::new(
                ui_strings
                    .get_or(header, header_fallback(header))
                    .to_string(),
            ),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(12.0),
                ..default()
            },
            TextColor(HEADER_COLOR),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(pos.0),
                top: Val::Px(pos.1),
                ..default()
            },
            Pickable::IGNORE,
        ));

        // The vanilla control is a CIFScrollManager; the row stack is taller
        // than the viewport (9 rows of 24 in 101 px, i.e. 4.2 rows visible), so
        // it clips *and* scrolls. It used to only clip: with no wheel observer
        // and no scrollbar, the last five rows — including both Community
        // switches — were out of a player's reach entirely. The rows are laid
        // out in flow rather than absolutely on purpose: bevy derives the
        // scrollable content size from the in-flow children, so an absolutely
        // positioned row stack reports a content height of zero and layout
        // clamps every `ScrollPosition` back to 0. Same top-to-bottom 24 px pitch either way.
        pane.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(viewport.0),
                top: Val::Px(viewport.1),
                width: Val::Px(viewport.2 - SCROLLBAR_W),
                height: Val::Px(viewport.3),
                flex_direction: FlexDirection::Column,
                overflow: Overflow::scroll_y(),
                ..default()
            },
            RowList {
                section,
                rows: rows.len(),
                viewport_h: viewport.3,
            },
            ScrollPosition(Vec2::ZERO),
            Pickable::default(),
        ))
        .with_children(|list| {
            for row in rows.iter() {
                spawn_toggle_row(list, font, ui_strings, row, options, &art);
            }
        })
        .observe(
            move |mut scroll: On<Pointer<Scroll>>,
                  mut lists: Query<(&RowList, &mut ScrollPosition)>,
                  mut thumbs: Query<(&ScrollThumb, &mut Node)>| {
                scroll_section(
                    section,
                    -scroll.event.y * WHEEL_STEP,
                    &mut lists,
                    &mut thumbs,
                );
                scroll.propagate(false);
            },
        );

        spawn_scrollbar(pane, asset_server, section, viewport, rows.len());
    }

    spawn_import_button(pane, font);

    // Hover-help footer, spanning the two scroll viewports' width.
    spawn_tooltip_line(pane, font, GAME_SET_VIEWPORT.0, GAME_SET_VIEWPORT.2);
}

/// openroad-only control, stated as a deviation per ADR 0009: the original has
/// no "import" button because it *is* the client that wrote the file. We have a
/// parser for `SROptionSet.dat` and, until this, only an environment variable
/// read once at boot to reach it — a migration, but not something a player can
/// try and take back.
///
/// It sits in the Setting pane rather than in the footer because
/// `ifoption.txt:6,25,44,63` declares exactly four footer buttons and their
/// meanings are data; a fifth one there would overwrite the original's layout
/// with an invention. Here it costs one row of empty pane and stays reversible: the
/// click only writes the live options, so Cancel restores the baseline
/// (`settings::edit_session`).
fn spawn_import_button(pane: &mut RelatedSpawnerCommands<ChildOf>, font: &Handle<Font>) {
    let mut entity = pane.spawn((
        ImportOriginalSettingsButton,
        Name::from("Options Setting Import Button"),
        Button,
        Hovered::default(),
        Text::new("Import SROptionSet.dat"),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(11.0),
            ..default()
        },
        // Not `LABEL_INERT`: in this pane that grey means
        // "rendered but nothing reads it" (see the constant), and this button
        // works.
        TextColor(LABEL_COLOR),
        abs(IMPORT_BUTTON),
        Pickable::default(),
    ));
    // No textuisystem key exists for this control (it has no original), so the
    // help text is openroad's own and names the file it reads.
    attach_tooltip(
        &mut entity,
        format!(
            "openroad only: read the original client's settings from {} \
             (Cancel undoes it).",
            sroptionset_import_path().display()
        ),
    );
    entity.observe(on_import_activate);
}

/// The click: a message, not a direct write, so the import lands in the same
/// live `GameOptions` every pane edits and `settings::persistence` stays the one
/// place that knows how to read a `.dat`.
fn on_import_activate(
    activate: On<Activate>,
    buttons: Query<&ImportOriginalSettingsButton>,
    mut requests: MessageWriter<ImportOriginalSettings>,
) {
    if buttons.get(activate.entity).is_ok() {
        requests.write(ImportOriginalSettings { path: None });
    }
}

fn header_fallback(key: &str) -> &'static str {
    if key == "UIIT_STT_NAMEVIEW" {
        "Indicate Name Settings"
    } else {
        "Game Settings"
    }
}

/// One of the pane's two `CIFScrollManager` lists, carrying what its scroll
/// maths needs: which section it is (0 = Indicate Name, 1 = Game Settings), how
/// many rows it holds, and the authored viewport height. Kept on the node
/// instead of read from `ComputedNode` so the arrow and thumb observers work on
/// the same numbers the tests do.
#[derive(Component, Clone, Copy)]
struct RowList {
    section: u8,
    rows: usize,
    viewport_h: f32,
}

/// The `ifverticalscroll.txt` thumb of one section's bar.
#[derive(Component, Clone, Copy)]
struct ScrollThumb {
    section: u8,
}

/// How far a section can scroll: the row stack minus what the viewport shows.
fn max_scroll(rows: usize, viewport_h: f32) -> f32 {
    (rows as f32 * ROW_H - viewport_h).max(0.0)
}

fn clamp_scroll(offset: f32, rows: usize, viewport_h: f32) -> f32 {
    offset.clamp(0.0, max_scroll(rows, viewport_h))
}

/// Is row `index` **fully** inside the viewport at scroll `offset`? Fully, not
/// partly: a row whose checkbox is half cut off is not a reachable control, and
/// that is exactly the state the Community pair was in. Test-only — bevy does
/// the clamping at runtime, but this pane's reachability claim is arithmetic
/// over `ROW_H` and the authored viewport, so it is checkable without a layout
/// pass.
#[cfg(test)]
fn row_is_visible(index: usize, offset: f32, viewport_h: f32) -> bool {
    let top = index as f32 * ROW_H;
    top >= offset - f32::EPSILON && top + ROW_H <= offset + viewport_h + f32::EPSILON
}

/// The smallest scroll offset that brings row `index` fully into view — the
/// answer to "can a player get to this row at all", in one number. Test-only,
/// same reason as [`row_is_visible`].
#[cfg(test)]
fn offset_revealing_row(index: usize, rows: usize, viewport_h: f32) -> f32 {
    let bottom = (index + 1) as f32 * ROW_H;
    clamp_scroll(bottom - viewport_h, rows, viewport_h)
}

/// Thumb height and top, pane-local to the bar column: the track is the
/// viewport minus the two 16 px arrows, the thumb covers the visible fraction
/// of the content (never smaller than one arrow, so it stays grabbable), and it
/// sits at the same fraction of the track as the list is through its scroll.
fn thumb_geometry(rows: usize, viewport_h: f32, offset: f32) -> (f32, f32) {
    let content = (rows as f32 * ROW_H).max(viewport_h);
    let track = (viewport_h - 2.0 * SCROLL_ARROW).max(SCROLL_ARROW);
    let height = (track * viewport_h / content).clamp(SCROLL_ARROW, track);
    let max = max_scroll(rows, viewport_h);
    let fraction = if max > 0.0 {
        (offset / max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (height, SCROLL_ARROW + fraction * (track - height))
}

/// Move one section's list and keep its thumb in step. Both inputs (wheel,
/// arrow) and the drag path go through here, so there is one clamp.
fn scroll_section(
    section: u8,
    delta: f32,
    lists: &mut Query<(&RowList, &mut ScrollPosition)>,
    thumbs: &mut Query<(&ScrollThumb, &mut Node)>,
) {
    for (list, mut pos) in lists.iter_mut() {
        if list.section != section {
            continue;
        }
        let next = clamp_scroll(pos.y + delta, list.rows, list.viewport_h);
        pos.y = next;
        let (height, top) = thumb_geometry(list.rows, list.viewport_h, next);
        for (thumb, mut node) in thumbs.iter_mut() {
            if thumb.section == section {
                node.height = Val::Px(height);
                node.top = Val::Px(top);
            }
        }
    }
}

/// The section's `CIFVerticalScroll`: up arrow, thumb, down arrow in a 16 px
/// column at the manager's right edge. Deviation worth naming: the prototype
/// puts its arrows *outside* the instance rect (`ifverticalscroll.txt` has the
/// up button at y `-16`), which here would push the up arrow onto the section
/// header and the down arrow onto the pane's next block; both arrows therefore
/// sit inside the 101 px column, and the list is 16 px narrower to make room
/// (the row prototype only claims 146 of the 336, so no authored rect moves
/// into the bar).
fn spawn_scrollbar(
    pane: &mut RelatedSpawnerCommands<ChildOf>,
    asset_server: &AssetServer,
    section: u8,
    viewport: (f32, f32, f32, f32),
    rows: usize,
) {
    let (thumb_h, thumb_top) = thumb_geometry(rows, viewport.3, 0.0);
    let left = viewport.0 + viewport.2 - SCROLLBAR_W;
    pane.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left),
            top: Val::Px(viewport.1),
            width: Val::Px(SCROLLBAR_W),
            height: Val::Px(viewport.3),
            ..default()
        },
        Pickable::default(),
    ))
    .with_children(|bar| {
        for (art, top, step) in [
            (SCROLL_UP_ART, 0.0, -WHEEL_STEP),
            (SCROLL_DOWN_ART, viewport.3 - SCROLL_ARROW, WHEEL_STEP),
        ] {
            bar.spawn((
                ImageNode {
                    image: asset_server.load(art),
                    ..default()
                },
                abs((0.0, top, SCROLLBAR_W, SCROLL_ARROW)),
                Button,
                Hovered::default(),
                Pickable::default(),
            ))
            .observe(
                move |_activate: On<Activate>,
                      mut lists: Query<(&RowList, &mut ScrollPosition)>,
                      mut thumbs: Query<(&ScrollThumb, &mut Node)>| {
                    scroll_section(section, step, &mut lists, &mut thumbs);
                },
            );
        }

        // The thumb: dragging it maps track px onto scroll px linearly, the
        // same conversion `hud/chat/ui.rs:574` uses for its identical bar.
        bar.spawn((
            ImageNode {
                image: asset_server.load(SCROLL_THUMB_ART),
                ..default()
            },
            abs((0.0, thumb_top, SCROLLBAR_W, thumb_h)),
            ScrollThumb { section },
            Pickable::default(),
        ))
        .observe(
            move |drag: On<Pointer<Drag>>,
                  mut lists: Query<(&RowList, &mut ScrollPosition)>,
                  mut thumbs: Query<(&ScrollThumb, &mut Node)>| {
                let track = (viewport.3 - 2.0 * SCROLL_ARROW).max(SCROLL_ARROW);
                let (thumb_h, _) = thumb_geometry(rows, viewport.3, 0.0);
                let travel = (track - thumb_h).max(1.0);
                let delta = drag.event.delta.y / travel * max_scroll(rows, viewport.3);
                scroll_section(section, delta, &mut lists, &mut thumbs);
            },
        );
    });
}

fn spawn_toggle_row(
    list: &mut RelatedSpawnerCommands<ChildOf>,
    font: &Handle<Font>,
    ui_strings: &ClientUiStrings,
    row: &ToggleRow,
    options: &GameOptions,
    art: &CheckboxArt,
) {
    let live_id = match (row.backing, row.id) {
        (Backing::Live, Some(id)) => Some(id),
        _ => None,
    };
    let live = live_id.is_some();
    let color = if live { LABEL_COLOR } else { LABEL_INERT };

    // The hit target is the whole row, not the 16x16 box: vanilla's checkbox is
    // under the WCAG 2.2 AA minimum, and widening only the click area changes no
    // vanilla geometry. `options_video.rs`
    // rows work the same way. The row carries its own `GameToggle` so the
    // observer can read the id off the entity it fired on; the box keeps a copy
    // because `refresh_game_toggles` repaints by `(&GameToggle, &mut ImageNode)`,
    // which only ever matches the box.
    // In flow, not absolute: see the scroll comment in `spawn_game_pane` —
    // bevy sizes scrollable content from the in-flow children only. The 24 px
    // pitch is unchanged, `flex_shrink: 0.0` keeps it from being squeezed when
    // the stack outgrows the viewport (which it always does).
    let mut entity = list.spawn(Node {
        width: Val::Percent(100.0),
        height: Val::Px(ROW_H),
        min_height: Val::Px(ROW_H),
        flex_shrink: 0.0,
        ..default()
    });
    if let Some(id) = live_id {
        entity.insert((
            GameToggle { id, live: true },
            Button,
            Hovered::default(),
            Pickable::default(),
        ));
        entity.observe(on_toggle_activate);
    } else {
        entity.insert(Pickable::IGNORE);
    }
    // `UIIT_STT_GAMESET_TTDESC_*` (see `ROW_TOOLTIPS`). Inert rows get it too:
    // hovering is not clicking, and the help text is what a row without a
    // consumer can still say truthfully.
    if let Some(help) = row_tooltip(ui_strings, row.key) {
        attach_tooltip(&mut entity, help);
    }

    entity.with_children(|slot| {
        slot.spawn((
            Text::new(ui_strings.get_or(row.key, row.english).to_string()),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(11.0),
                ..default()
            },
            TextColor(color),
            abs(ROW_LABEL_DRAWN),
            Pickable::IGNORE,
        ));

        // The prototype's second static column (`ROW_MARK`).
        spawn_state_column(slot, font, ui_strings, live_id, options);

        let Some(id) = row.id else {
            // No id means nothing to store; draw the disabled box and stop.
            slot.spawn((
                ImageNode {
                    image: art.disabled.clone(),
                    ..default()
                },
                abs(ROW_CHECKBOX_DRAWN),
                Pickable::IGNORE,
            ));
            return;
        };

        let image = if !live {
            art.disabled.clone()
        } else if toggle_on(options, id) {
            art.on.clone()
        } else {
            art.off.clone()
        };
        // Always `IGNORE`: the click belongs to the row, and the box must not
        // swallow it.
        slot.spawn((
            GameToggle { id, live },
            ImageNode { image, ..default() },
            abs(ROW_CHECKBOX_DRAWN),
            Pickable::IGNORE,
        ));
    });
}

/// The second static column of a row whose toggle a system actually reads.
/// Carries the id so [`refresh_game_toggles`] can repaint it next to the box.
#[derive(Component)]
pub(crate) struct StateColumn {
    // `pub(crate)` because it appears in the signature of the `pub(crate)`
    // system `refresh_game_toggles`, and a private type there makes the whole
    // system tuple in `options_window.rs` fail to compile in the test profile.
    id: u16,
}

/// `On` / `Off` for one toggle, from the original's own two keys.
fn state_text(ui_strings: &ClientUiStrings, on: bool) -> String {
    if on {
        ui_strings
            .get_or(STATE_ON_KEY, STATE_ON_FALLBACK)
            .to_string()
    } else {
        ui_strings
            .get_or(STATE_OFF_KEY, STATE_OFF_FALLBACK)
            .to_string()
    }
}

/// Fill `GDR_GAME_OPTION_SLOT_STA2` (`ROW_MARK`): the state word for a row with
/// a live consumer, a dash for one without. See [`STATE_ON_KEY`] for why this
/// column carries the state at all.
fn spawn_state_column(
    slot: &mut RelatedSpawnerCommands<ChildOf>,
    font: &Handle<Font>,
    ui_strings: &ClientUiStrings,
    live_id: Option<u16>,
    options: &GameOptions,
) {
    let (text, color) = match live_id {
        Some(id) => (state_text(ui_strings, toggle_on(options, id)), LABEL_COLOR),
        None => (STATE_INERT_MARK.to_string(), LABEL_INERT),
    };
    let mut entity = slot.spawn((
        Text::new(text),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(color),
        abs(ROW_MARK_DRAWN),
        Pickable::IGNORE,
    ));
    if let Some(id) = live_id {
        entity.insert(StateColumn { id });
    }
}

fn abs(rect: (f32, f32, f32, f32)) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(rect.0),
        top: Val::Px(rect.1),
        width: Val::Px(rect.2),
        height: Val::Px(rect.3),
        ..default()
    }
}

/// Flip the row's id in `GameOptions`; persistence picks the change up on its
/// own (`settings::persistence::save_on_change`).
fn on_toggle_activate(
    activate: On<Activate>,
    toggles: Query<&GameToggle>,
    mut options: ResMut<GameOptions>,
) {
    let Ok(toggle) = toggles.get(activate.entity) else {
        return;
    };
    if !toggle.live {
        return;
    }
    let next = !toggle_on(&options, toggle.id);
    options.gameplay.toggles.insert(toggle.id, next);
}

/// Repaint the boxes when the options change from anywhere (this pane, a load
/// of `user_settings.yaml`, or a future `SROptionSet.dat` import).
pub(crate) fn refresh_game_toggles(
    options: Res<GameOptions>,
    asset_server: Res<AssetServer>,
    ui_strings: Res<ClientUiStrings>,
    mut boxes: Query<(&GameToggle, &mut ImageNode)>,
    mut states: Query<(&StateColumn, &mut Text)>,
) {
    if !options.is_changed() {
        return;
    }
    for (toggle, mut image) in &mut boxes {
        if !toggle.live {
            continue;
        }
        let art = if toggle_on(&options, toggle.id) {
            CHECKBOX_ON
        } else {
            CHECKBOX_OFF
        };
        image.image = asset_server.load(art);
    }
    // The second column says the same thing in words (`STATE_ON_KEY`), so it
    // has to move with the box — including when the change came from a load or
    // an import rather than from a click here.
    for (column, mut text) in &mut states {
        let next = state_text(&ui_strings, toggle_on(&options, column.id));
        if text.0 != next {
            text.0 = next;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The five `UIIT_STT_GAMESET_TTDESC_*` strings (:985-989) belong to
    /// five *specific* rows, and the block is not in row order. Every wired key
    /// must name a row this pane renders, no key twice, no row twice — and the
    /// two rows the original leaves without help must stay without it, because
    /// filling them in would mean showing another row's text.
    #[test]
    fn the_wired_tooltips_are_the_five_the_original_ships() {
        let strings = ClientUiStrings::default();
        for (row_key, key, _) in ROW_TOOLTIPS {
            let number = key
                .strip_prefix("UIIT_STT_GAMESET_TTDESC_")
                .unwrap_or_else(|| panic!("{key} is not from the GAMESET_TTDESC block"))
                .parse::<u8>()
                .expect("the suffix is a two-digit number");
            assert!((1..=5).contains(&number), "{key} is outside :985-989");
            assert!(
                NAME_VIEW_ROWS
                    .iter()
                    .chain(GAME_SET_ROWS.iter())
                    .any(|row| row.key == row_key),
                "{row_key} is not a row of this pane"
            );
        }

        // The Community pair is helped from the *other* block, `_06`/`_07`,
        // whose strings name the switch they belong to outright.
        for (row_key, key, _) in COMMUNITY_TOOLTIPS {
            assert!(
                key.starts_with("UIIT_STT_GAME_OPTION_TTDESC_"),
                "{key} is not from the GAME_OPTION_TTDESC block"
            );
            assert!(
                COMMUNITY_ROWS.iter().any(|row| row.key == row_key),
                "{row_key} is not a Community row"
            );
        }
        // `_08` names 2004, which this pane deliberately does not render.
        assert!(!COMMUNITY_TOOLTIPS
            .iter()
            .any(|(_, key, _)| *key == "UIIT_STT_GAME_OPTION_TTDESC_08"));

        let with_help: Vec<&str> = GAME_SET_ROWS
            .iter()
            .filter(|row| row_tooltip(&strings, row.key).is_some())
            .map(|row| row.key)
            .collect();
        assert_eq!(with_help.len(), 5, "{with_help:?}");
        // Self Condition and Monster Condition have no tooltip in the data.
        assert!(row_tooltip(&strings, "UIIT_STT_QUICKSTATE_OWNER").is_none());
        assert!(row_tooltip(&strings, "UIIT_STT_QUICKSTATE_MONSTER").is_none());
        // ...and the NAMEVIEW section is deliberately unwired (order unknown).
        for row in NAME_VIEW_ROWS.iter() {
            assert!(
                row_tooltip(&strings, row.key).is_none(),
                "{} got help from a block whose order is unproven",
                row.key
            );
        }
    }

    /// Every row that claims a backing must be one an actual system reads, and
    /// every row an actual system reads must claim it. The two readers in the
    /// tree are `hud/nameplates.rs:88-92` (2010..=2014) and
    /// `hud/petition.rs::auto_refusal` (2002/2003); nothing else looks at
    /// `gameplay.toggles`. 2024 is deliberately absent — its reader is gone.
    /// Moving a row in or out of this list has to be a deliberate edit here, in
    /// both directions.
    #[test]
    fn the_live_rows_are_exactly_the_ids_a_system_reads() {
        let live: Vec<Option<u16>> = NAME_VIEW_ROWS
            .iter()
            .chain(GAME_SET_ROWS.iter())
            .chain(COMMUNITY_ROWS.iter())
            .filter(|r| r.backing == Backing::Live)
            .map(|r| r.id)
            .collect();
        assert_eq!(
            live,
            vec![
                Some(2010),
                Some(2011),
                Some(2012),
                Some(2013),
                Some(2014),
                Some(2002),
                Some(2003),
            ]
        );
    }

    /// Regel 6 in table form: the two Community ids exist here **because**
    /// `hud/petition.rs` reads them. If that reader ever loses an id, this
    /// fails rather than leaving a drawn switch that does nothing.
    #[test]
    fn the_community_rows_are_the_ids_the_petition_path_reads() {
        use crate::plugins::hud::petition::refusal_option;
        use packets::agent::ingame::{
            PETITION_EXCHANGE, PETITION_PARTY_CREATION, PETITION_PARTY_INVITATION,
        };

        let drawn: Vec<u16> = COMMUNITY_ROWS.iter().filter_map(|r| r.id).collect();
        assert_eq!(drawn, vec![2002, 2003]);
        assert_eq!(refusal_option(PETITION_PARTY_CREATION), Some(2002));
        assert_eq!(refusal_option(PETITION_PARTY_INVITATION), Some(2002));
        assert_eq!(refusal_option(PETITION_EXCHANGE), Some(2003));
        // 2004 is dead in the original itself, so no arm may claim it.
        for kind in 0u8..=10 {
            assert_ne!(refusal_option(kind), Some(2004));
        }
    }

    /// The ids are the documented `Setting`-tab ones
    /// (`docs/formats/sroptionset.md:106-133`): all in 2001..=2028, never 2015
    /// (which is Video/Window Mode), and never repeated.
    #[test]
    fn ids_come_from_the_documented_setting_range() {
        let mut seen = Vec::new();
        for row in NAME_VIEW_ROWS
            .iter()
            .chain(GAME_SET_ROWS.iter())
            .chain(COMMUNITY_ROWS.iter())
        {
            if let Some(id) = row.id {
                assert!(
                    (2001..=2028).contains(&id),
                    "{id} outside the Setting range"
                );
                assert_ne!(id, 2015, "2015 is the Video tab's window-mode id");
                assert!(!seen.contains(&id), "duplicate id {id}");
                seen.push(id);
            }
        }
        assert_eq!(seen.len(), 13);
    }

    /// Vanilla's classic pane is two sections of six and seven rows — the row
    /// count is the label block's, not the tooltip block's (five tooltips
    /// against seven labels).
    #[test]
    fn section_row_counts_match_the_label_blocks() {
        assert_eq!(NAME_VIEW_ROWS.len(), 6);
        assert_eq!(GAME_SET_ROWS.len(), 7);
        // The Community pair is ours to place, not a vanilla block of this
        // pane — kept separate so the two counts above stay checkable.
        assert_eq!(COMMUNITY_ROWS.len(), 2);
    }

    /// The openroad-only import control must not sit on top of anything the
    /// data placed: `ifoption_game.txt` puts the headers at x 13 and both
    /// scroll viewports at y 46..147 / 189..290, so the band above the first
    /// viewport, right of the header, is the only free space in the pane.
    #[test]
    fn the_import_button_stays_out_of_every_rect() {
        let (x, y, w, h) = IMPORT_BUTTON;
        assert!(
            y + h <= NAME_VIEW_VIEWPORT.1,
            "the button must end above the first scroll viewport"
        );
        assert!(
            x > NAME_VIEW_HEADER.0 + 100.0,
            "and start right of the section header"
        );
        // Inside the pane (`ifoption.txt:82` GDR_OPTION_WND_GAME 11,62,364,313).
        assert!(x + w <= 364.0);
    }

    /// The second static column (`ifgameoptionslot.txt:25`, rect
    /// `93,8,31,16`) carries the state word, and the two strings must be the
    /// original's own keys rather than invented English — if the table ever
    /// loses them, the fallback is still `On`/`Off` and nothing silently
    /// becomes blank.
    #[test]
    fn the_state_column_uses_the_originals_on_off_keys() {
        let strings = ClientUiStrings::default();
        assert_eq!(state_text(&strings, true), STATE_ON_FALLBACK);
        assert_eq!(state_text(&strings, false), STATE_OFF_FALLBACK);
        assert_ne!(state_text(&strings, true), state_text(&strings, false));
        assert!(STATE_ON_KEY.starts_with("UIIT_STT_"));
        assert!(STATE_OFF_KEY.starts_with("UIIT_STT_"));
        // The column must stay inside the slot's own 31px and clear of the
        // checkbox at x 130 (both from the prototype).
        assert!(ROW_MARK.0 + ROW_MARK.2 <= ROW_CHECKBOX.0);
        assert!(ROW_LABEL.0 + ROW_LABEL.2 <= ROW_MARK.0);
    }

    /// A row stack taller than its 101px viewport is why vanilla uses a
    /// scroll manager; keep that true so the clip/scroll stays justified.
    #[test]
    fn game_set_rows_overflow_the_vanilla_viewport() {
        assert!(GAME_SET_ROWS.len() as f32 * ROW_H > GAME_SET_VIEWPORT.3);
    }

    /// **The reachability proof.** This pane used to sort nine rows into a
    /// 101 px viewport with no scroll input of any kind, so rows 5..9 — both
    /// Community switches among them — were drawn and then clipped away. A test
    /// that only found 2002 and 2003 in the row table passed the whole time,
    /// which is why this one is about *geometry after scrolling* instead.
    #[test]
    fn the_community_switches_are_reachable_by_scrolling() {
        let rows = GAME_SET_ROWS.len() + COMMUNITY_ROWS.len();
        let viewport_h = GAME_SET_VIEWPORT.3;
        // Premise: they are out of view at rest. If this ever stops being true
        // the pane got shorter or taller and the rest of the test is moot.
        let visible_at_rest = (0..rows)
            .filter(|i| row_is_visible(*i, 0.0, viewport_h))
            .count();
        assert_eq!(visible_at_rest, 4, "101 px / 24 px = 4 whole rows");
        for (offset, row) in COMMUNITY_ROWS.iter().enumerate() {
            let index = GAME_SET_ROWS.len() + offset;
            assert!(
                !row_is_visible(index, 0.0, viewport_h),
                "{} is below the fold at rest, that is the defect",
                row.key
            );
            // ...and the scroll range actually reaches it.
            let needed = offset_revealing_row(index, rows, viewport_h);
            assert!(
                needed <= max_scroll(rows, viewport_h),
                "{} lies past the end of the scroll range",
                row.key
            );
            assert!(
                row_is_visible(index, needed, viewport_h),
                "{} is still not fully visible at its own offset",
                row.key
            );
            // The last row must be reachable by scrolling to the very end,
            // which is where an arrow-click or wheel run lands.
            assert!(row_is_visible(
                index,
                clamp_scroll(f32::MAX, rows, viewport_h),
                viewport_h
            ));
        }
        // Every row of both sections is reachable, not just the new pair.
        for (section_rows, count) in [(NAME_VIEW_ROWS.len(), 6), (rows, 9)] {
            assert_eq!(section_rows, count);
            for index in 0..section_rows {
                let offset = offset_revealing_row(index, section_rows, viewport_h);
                assert!(row_is_visible(index, offset, viewport_h), "row {index}");
            }
        }

        // The *upper* box has the identical defect and nobody had noticed: six
        // rows in the same 101 px, so `Guild Name` (2014, a live nameplate id)
        // and `Beginner's Mark` were unreachable since this pane was written.
        let name_rows = NAME_VIEW_ROWS.len();
        for key in ["UIIT_STT_GUILDVIEW_SIGN", "UIIT_STT_FIRSTSTEP_MARK_SIGN"] {
            let index = NAME_VIEW_ROWS
                .iter()
                .position(|row| row.key == key)
                .unwrap_or_else(|| panic!("{key} is a row of the upper box"));
            assert!(
                !row_is_visible(index, 0.0, viewport_h),
                "{key} is below the fold at rest"
            );
            assert!(row_is_visible(
                index,
                offset_revealing_row(index, name_rows, viewport_h),
                viewport_h
            ));
            assert!(row_is_visible(
                index,
                clamp_scroll(f32::MAX, name_rows, viewport_h),
                viewport_h
            ));
        }
    }

    /// Wheel and arrow both move exactly one row and both clamp; a scroll that
    /// ran past the ends would either hide the first row or leave blank space
    /// under the last one.
    #[test]
    fn one_notch_is_one_row_and_the_range_is_closed() {
        let rows = GAME_SET_ROWS.len() + COMMUNITY_ROWS.len();
        let h = GAME_SET_VIEWPORT.3;
        assert_eq!(WHEEL_STEP, ROW_H);
        assert_eq!(clamp_scroll(-WHEEL_STEP, rows, h), 0.0);
        assert_eq!(clamp_scroll(WHEEL_STEP, rows, h), ROW_H);
        assert_eq!(max_scroll(rows, h), rows as f32 * ROW_H - h);
        // Five notches take the list from the top to the bottom of a 9-row
        // stack (216 - 101 = 115 px, i.e. 4.8 rows).
        let mut offset = 0.0;
        for _ in 0..5 {
            offset = clamp_scroll(offset + WHEEL_STEP, rows, h);
        }
        assert_eq!(offset, max_scroll(rows, h));
        // A section that fits needs no scrolling and offers none.
        assert_eq!(max_scroll(4, h), 0.0);
    }

    /// The bar the player grabs: thumb inside the track, never smaller than an
    /// arrow, at the top when the list is at the top and flush with the track's
    /// end when it is at the bottom.
    #[test]
    fn the_thumb_tracks_the_scroll_without_leaving_the_track() {
        let rows = GAME_SET_ROWS.len() + COMMUNITY_ROWS.len();
        let h = GAME_SET_VIEWPORT.3;
        let track = h - 2.0 * SCROLL_ARROW;
        let (height, top) = thumb_geometry(rows, h, 0.0);
        assert!(height >= SCROLL_ARROW && height <= track);
        assert_eq!(top, SCROLL_ARROW, "at rest the thumb sits under the arrow");
        let (bottom_h, bottom_top) = thumb_geometry(rows, h, max_scroll(rows, h));
        assert_eq!(
            bottom_h, height,
            "the thumb does not resize while scrolling"
        );
        assert!((bottom_top + bottom_h - (SCROLL_ARROW + track)).abs() < 0.01);
        // Half the range is half the travel.
        let (_, mid) = thumb_geometry(rows, h, max_scroll(rows, h) / 2.0);
        assert!((mid - (SCROLL_ARROW + (track - height) / 2.0)).abs() < 0.01);
        // A list that cannot scroll shows a full-track thumb at the top.
        let (full_h, full_top) = thumb_geometry(4, h, 0.0);
        assert_eq!(full_h, track);
        assert_eq!(full_top, SCROLL_ARROW);
    }

    /// The scroll input exists as *input*, not only as a clipped node: the list
    /// carries a wheel observer and the bar carries two arrow buttons and a
    /// draggable thumb. Textual assertion over the spawner for the same reason
    /// `hud/job/ranking.rs`'s gate test is one — nothing else in a unit test
    /// can see an observer.
    #[test]
    fn the_scroll_manager_has_all_three_inputs_the_original_has() {
        // Split on the test module, not on `#[cfg(test)]`: the file carries
        // test-only helpers above it (`WIDEST_LABEL_PX`).
        let source = include_str!("options_game.rs")
            .split("mod test {")
            .next()
            .expect("the file has a non-test part");
        assert!(
            source.contains("On<Pointer<Scroll>>"),
            "the row list has no wheel observer"
        );
        assert!(
            source.contains("On<Pointer<Drag>>"),
            "the thumb is decoration unless it can be dragged"
        );
        assert!(
            source.matches("scroll_section(").count() >= 4,
            "wheel, both arrows and the drag all go through the one clamp"
        );
        assert!(
            source.contains("Overflow::scroll_y()"),
            "the viewport still has to clip"
        );
        // The rows must stay in flow: an absolutely positioned stack reports a
        // zero content height and layout clamps every scroll back to 0.
        let row_node = source
            .split("let mut entity = list.spawn(Node {")
            .nth(1)
            .expect("the row spawner is still here")
            .split("});")
            .next()
            .expect("its node literal is still a block");
        assert!(
            !row_node.contains("PositionType::Absolute"),
            "the row stack went back to absolute positioning; scrolling is dead"
        );
    }

    /// The labels must not wrap: the slot's static is 16 px high inside a 24 px
    /// row, so a second line lands on the row below — which is what happened to
    /// `Monster Name`, `Self Condition`, `COS Condition`, `Party Member Status`
    /// and `Monster Condition`.
    #[test]
    fn the_label_column_is_wide_enough_that_no_label_wraps() {
        assert!(
            ROW_LABEL_W >= WIDEST_LABEL_PX,
            "{ROW_LABEL_W} px cannot hold a {WIDEST_LABEL_PX} px label on one line"
        );
        assert!(
            ROW_LABEL.2 < WIDEST_LABEL_PX,
            "the authored 70 px is the defect"
        );
        // The columns keep the authored gaps and stay inside the manager's
        // usable width (viewport minus the 16 px CIFVerticalScroll).
        assert_eq!(ROW_LABEL_DRAWN.0, ROW_LABEL.0);
        assert_eq!(ROW_LABEL_DRAWN.3, ROW_LABEL.3);
        assert_eq!(ROW_MARK_DRAWN.0 - ROW_MARK.0, ROW_RIGHT_SHIFT);
        assert_eq!(ROW_CHECKBOX_DRAWN.0 - ROW_CHECKBOX.0, ROW_RIGHT_SHIFT);
        assert_eq!(
            ROW_MARK_DRAWN.0 - (ROW_LABEL_DRAWN.0 + ROW_LABEL_DRAWN.2),
            9.0
        );
        assert_eq!(
            ROW_CHECKBOX_DRAWN.0 - (ROW_MARK_DRAWN.0 + ROW_MARK_DRAWN.2),
            ROW_CHECKBOX.0 - (ROW_MARK.0 + ROW_MARK.2)
        );
        assert_eq!(
            ROW_CHECKBOX_DRAWN.0 + ROW_CHECKBOX_DRAWN.2,
            GAME_SET_VIEWPORT.2 - SCROLLBAR_W
        );
    }
}

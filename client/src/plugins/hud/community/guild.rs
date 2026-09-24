//! The Guild page — page 10 of the Community window (`GDR_COMMUNITY_GUILD`).
//!
//! Idea: the whole page is transcribed from the user's own
//! `resinfo/ifguild.txt`; this module builds its first two sections — §Create
//! (the page chrome, 3 controls) and §GuildInfo (the header block, 15
//! controls, `ifguild.txt:65-353`). Every rect const carries the
//! `ifguild.txt:NNN` line it came from, so no number here was chosen.
//! The roster list (§MemberView/§SortBtn), the notice strip (§NotifySubBox)
//! and the command column (§Command) are separate children of #25 and are
//! deliberately absent.
//!
//! Rects are page-local: the six community pages all share
//! `ifcommunity.txt`'s `13,61,451,320`, and `community/ui.rs` already spawns
//! that container — exactly the frame `letter.rs` draws into.
//!
//! Two things the original leaves us to decide, both stated rather than
//! silently done:
//!
//! * **The two art overflows.** `gil_windo01.ddj` is 588x108 at x=6 (right
//!   edge 594) and `gil_bar01.ddj` is 376x28 at x=120 (right edge 496), both
//!   past the 451-wide page. Both were re-authored for the 588-px 4th-gen
//!   pane and the shipped classic tree still points at them. Whether the
//!   original clips or overdraws is `[U]` (§9-U4 — it needs a decompile or a
//!   screenshot), so we **clip at the page's right edge**: the art is drawn at
//!   its native size inside a clipping node, which keeps the bitmap 1:1 (no
//!   squash) and keeps the page from painting over the Community frame.
//! * **The GP gauge fill.** The record carries `guild_points`, but nothing in
//!   the client's data carries the GP a level *requires* — that threshold is
//!   server-authoritative and arrives only with the `ifguildlevelup` flow. So
//!   the gauge track is drawn and the fill stays at zero, and the percent
//!   readout shows the empty marker, rather than us inventing a maximum.
//!   [`GuildGpFill`] is the one node a later ticket writes to.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::cursor::interactions::entity_select::SelectedEntity;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::gauge::{gauge_art_node, gauge_crop_node, gauge_fill_width};
use crate::plugins::hud::inventory::ui::format_thousands;
use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::net::entities::NetworkId;
use crate::plugins::net::guild::{GuildAction, GuildRoster, GuildUnion};
use crate::plugins::player::Player;
use crate::plugins::textdata::ClientUiStrings;
use packets::agent::guild_union::UnionRoster;

// --- §Create — page chrome, `ifguild.txt:4-63` (3 controls) -----------------

/// `GDR_GUILD_INFO_WND:CIFStatic` (`ifguild.txt:6`, Rect `6,4,0,0`) — art-sized
/// from `gil_windo01.ddj`, measured **588x108**. `6 + 588 = 594 > 451`, so it
/// overflows the page by 143 px; see the module doc for the clip decision.
const INFO_WND_POS: (f32, f32) = (6.0, 4.0);
const INFO_WND_ART: (f32, f32) = (588.0, 108.0);
const INFO_WND_DDJ: &str = "media://interface/guild/gil_windo01.ddj";
/// `GDR_GUILD_FRAME:CIFFrame` (`ifguild.txt:25`, Rect `6,103,440,211`).
const FRAME_RECT: (f32, f32, f32, f32) = (6.0, 103.0, 440.0, 211.0);
const FRAME_PIECE: f32 = 16.0;
const FRAME_DIR: &str = "media://interface/frame/frameg01_wnd_";
/// `GDR_GUILD_BG:CIFStatic` (`ifguild.txt:44`, Rect `27,105,403,58`) — the
/// tile behind the notice strip.
const BG_RECT: (f32, f32, f32, f32) = (27.0, 105.0, 403.0, 58.0);
const BG_TILE_DDJ: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// The page rect every community page shares (`ifcommunity.txt`), used as the
/// clip boundary for the two overflowing statics.
const PAGE_SIZE: (f32, f32) = (451.0, 320.0);

// --- §GuildInfo — header block, `ifguild.txt:65-353` (15 controls) ----------

/// `GDR_GUILD_INFO_GUILD_MARK` (`:219`, Rect `144,16,0,0`) — art-sized
/// `gil_mark01.ddj`, measured 16x16. The emblem is server art in vanilla; we
/// draw the default plate until an emblem source exists.
const MARK_POS: (f32, f32) = (144.0, 16.0);
const MARK_SIZE: (f32, f32) = (16.0, 16.0);
const MARK_DDJ: &str = "media://interface/guild/gil_mark01.ddj";
/// `GDR_GUILD_INFO_GUILD_NAME` (`:200`, Rect `166,18,80,14`, HAlign 0).
const NAME_RECT: (f32, f32, f32, f32) = (166.0, 18.0, 80.0, 14.0);
/// `GDR_GUILD_INFO_STA_GUILD_LEVEL` (`:295`, Rect `381,18,34,15`, HAlign 0,
/// FontColor `255,255,217,83`, Text `UIO_CHARINFO_STT_LEVEL`).
const LEVEL_LABEL_RECT: (f32, f32, f32, f32) = (381.0, 18.0, 34.0, 15.0);
/// `GDR_GUILD_INFO_GUILD_LEVEL` (`:181`, Rect `419,18,17,15`, HAlign 0).
const LEVEL_RECT: (f32, f32, f32, f32) = (419.0, 18.0, 17.0, 15.0);
/// `GDR_GUILD_INFO_STA_GUILD_LEADER` (`:276`, Rect `28,51,84,14`, HAlign 0,
/// FontColor `255,239,218,164`, Text `UIIT_STT_GUILD_LEADER`).
const LEADER_LABEL_RECT: (f32, f32, f32, f32) = (28.0, 51.0, 84.0, 14.0);
/// `GDR_GUILD_INFO_GUILD_LEADER_RACE` (`:162`, Rect `90,49,16,16`) — the file
/// names `com_kindred_china16.ddj`, which is the *placeholder*: the original
/// swaps the art for the master's race at runtime. We draw the same
/// placeholder, because the record's `model_id` -> race mapping is a separate
/// piece of work (the roster row ticket needs it too).
const LEADER_RACE_RECT: (f32, f32, f32, f32) = (90.0, 49.0, 16.0, 16.0);
const LEADER_RACE_DDJ: &str = "media://interface/ifcommon/com_kindred_china16.ddj";
/// `GDR_GUILD_INFO_GUILD_LEADER` (`:143`, Rect `112,51,110,14`, HAlign 0).
const LEADER_RECT: (f32, f32, f32, f32) = (112.0, 51.0, 110.0, 14.0);
/// `GDR_GUILD_INFO_STA_GUILDSMAN_NUM` (`:257`, Rect `261,51,67,14`, **HAlign
/// 2**, FontColor `255,239,218,164`, Text `UIIT_STT_GUILDSMAN_NUM`).
const MEMBER_LABEL_RECT: (f32, f32, f32, f32) = (261.0, 51.0, 67.0, 14.0);
/// `GDR_GUILD_INFO_GUILD_MEMBER_NUM` (`:124`, Rect `339,51,67,14`, **HAlign 2**).
const MEMBER_NUM_RECT: (f32, f32, f32, f32) = (339.0, 51.0, 67.0, 14.0);
/// `GDR_GUILD_INFO_BAR_BOARD` (`:333`, Rect `120,67,0,0`) — art-sized
/// `gil_bar01.ddj`, measured **376x28**. `120 + 376 = 496 > 451`: overflows by
/// 45 px, clipped like `INFO_WND` above (module doc).
const BAR_BOARD_POS: (f32, f32) = (120.0, 67.0);
const BAR_BOARD_ART: (f32, f32) = (376.0, 28.0);
const BAR_BOARD_DDJ: &str = "media://interface/guild/gil_bar01.ddj";
/// `GDR_GUILD_INFO_STA_GUILD_POINT_GP` (`:238`, Rect `28,75,89,14`, HAlign 0,
/// FontColor `255,239,218,164`, Text `UIIT_STT_GUILD_POINT`).
const GP_LABEL_RECT: (f32, f32, f32, f32) = (28.0, 75.0, 89.0, 14.0);
/// `GDR_GUILD_INFO_GP_GAUGE:CIFGauge` (`:314`, Rect `128,76,0,0`) — art-sized
/// `gil_point.ddj`, measured 144x8 (R5G6B5, no alpha channel).
const GP_GAUGE_POS: (f32, f32) = (128.0, 76.0);
const GP_GAUGE_ART: (f32, f32) = (144.0, 8.0);
const GP_GAUGE_DDJ: &str = "media://interface/guild/gil_point.ddj";
/// `GDR_GUILD_INFO_GUILD_POINT_PERCENT` (`:105`, Rect `168,75,64,14`, HAlign 1).
const GP_PERCENT_RECT: (f32, f32, f32, f32) = (168.0, 75.0, 64.0, 14.0);
/// `GDR_GUILD_INFO_GUILD_POINT_GP` (`:86`, Rect `276,75,129,14`, HAlign 1).
const GP_VALUE_RECT: (f32, f32, f32, f32) = (276.0, 75.0, 129.0, 14.0);
/// `GDR_GUILD_INFO_GUILD_POINT_BTN:CIFButton` (`:67`, Rect `410,72,0,0`) —
/// art-sized `com_donation_button.ddj`, measured 16x16. Drawn as art only: the
/// GP donation flow is `ifguildpointup.txt`, a separate child of #25, and a
/// button that opens nothing would be a dead wire.
const GP_BUTTON_POS: (f32, f32) = (410.0, 72.0);
const GP_BUTTON_SIZE: (f32, f32) = (16.0, 16.0);
const GP_BUTTON_DDJ: &str = "media://interface/ifcommon/com_donation_button.ddj";

/// `FontColor=255,255,255,255` — the seven runtime statics.
const VALUE_COLOR: Color = Color::srgb(1.0, 1.0, 1.0);
/// `FontColor=255,239,218,164` — leader / member-count / GP labels.
const LABEL_COLOR: Color = Color::srgb(239.0 / 255.0, 218.0 / 255.0, 164.0 / 255.0);
/// `FontColor=255,255,217,83` — the "Level" label alone (`:300`).
const LEVEL_LABEL_COLOR: Color = Color::srgb(1.0, 217.0 / 255.0, 83.0 / 255.0);

/// What a runtime static shows while the player is guildless. The page is
/// *shown*, not hidden — the original has no `Visible` key and page identity
/// is the id alone, so an empty guild page is the guildless state.
const EMPTY: &str = "-";

/// The seven `Text=""` statics of §GuildInfo, i.e. the ones the record fills.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuildInfoField {
    Name,
    Level,
    Leader,
    MemberCount,
    GpPercent,
    GpValue,
}

/// The GP gauge's crop node — the only node the fill width is written to
/// (`hud/gauge.rs`).
#[derive(Component)]
pub struct GuildGpFill;

/// The text a field shows for the given record (`None` = guildless).
fn field_text(field: GuildInfoField, data: Option<&packets::agent::guild::GuildData>) -> String {
    let Some(data) = data else {
        return EMPTY.to_string();
    };
    match field {
        GuildInfoField::Name => data.name.clone(),
        GuildInfoField::Level => data.level.to_string(),
        // The record carries no "this is the master" field other than the
        // per-member flag, so the master is the member that claims it.
        GuildInfoField::Leader => data
            .members
            .iter()
            .find(|m| m.is_master)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| EMPTY.to_string()),
        // Count only — the per-level member *cap* is server-side (it never
        // reaches the client), so no "n/max" is rendered.
        GuildInfoField::MemberCount => data.member_count.to_string(),
        // See the module doc: no GP threshold exists client-side.
        GuildInfoField::GpPercent => EMPTY.to_string(),
        GuildInfoField::GpValue => format_thousands(u64::from(data.guild_points)),
    }
}

// --- §MemberView — the roster pane, `ifguild.txt:415-476` (3) ---------------

/// `GDR_GUILD_MEMBER_VIEW_BLACKSQUARE:CIFStretchWnd` (`:445`, Rect
/// `14,138,333,166`) — the list plate; `com_blacksquare_` is six 4 px trim
/// pieces around a flat black interior, the same family `letter.rs` uses.
const MEMBER_PLATE_RECT: (f32, f32, f32, f32) = (14.0, 138.0, 333.0, 166.0);
const BLACKSQUARE_PIECE: f32 = 4.0;
pub(super) const BLACKSQUARE_DIR: &str = "media://interface/ifcommon/com_blacksquare_";
/// `GDR_GUILD_USERVIEW_SCROLLMGR:CIFScrollManager` (`:426`, Rect
/// `17,163,328,139`) — the row area. The manager itself (its pooled rows and
/// its `CIFVerticalScroll`) is the shared widget of #56 and is **not** built
/// here; this row draws the fixed visible rows the manager's own height
/// implies.
const MEMBER_LIST_RECT: (f32, f32, f32, f32) = (17.0, 163.0, 328.0, 139.0);
/// `GDR_GUILD_MEMBER_VIEW_BG:CIFStatic` (`:465`, Rect `328,163,102,135`) — the
/// right gutter tile behind the command column.
const MEMBER_VIEW_BG_RECT: (f32, f32, f32, f32) = (328.0, 163.0, 102.0, 135.0);

/// Row pitch and height. **Both are sourced, not chosen**
/// (`hud-guild-window.md` §3.5): the pitch-23/height-24 law is proven inside
/// this very file set (`ifguildmasterelection.txt` / `ifguildmasterleave.txt`
/// each lay out 7 rows at y 64/87/110/… inside a 164-tall manager) and
/// `guild.2dt`'s 6 `CNIFGuildUserSlot` entries repeat it for this window.
const ROW_PITCH: f32 = 23.0;
const ROW_HEIGHT: f32 = 24.0;
/// `139 = 6·23 + 1` ⇒ six visible rows. `[S]` in the doc, and the 4th-gen
/// tree's explicit six slots agree.
const VISIBLE_ROWS: usize = 6;

// --- §SortBtn — the column header strip, `ifguild.txt:595-731` (7) ----------

/// `GDR_GUILD_SORT_STATIC1` (`:720`, Rect `17,141,0,0`) — art-sized
/// `gil_shape01.ddj`, measured 24x24, the strip's left cap.
const SORT_CAP_LEFT: (f32, f32, f32, f32) = (17.0, 141.0, 24.0, 24.0);
const SORT_CAP_LEFT_DDJ: &str = "media://interface/guild/gil_shape01.ddj";
/// `GDR_GUILD_CONDITION_BUTTON` (`:606`, Rect `18,142,0,0`) — art-sized
/// `stl_condition.ddj`, 20x20, the online/offline filter. It sits **inside**
/// the left cap (x 18 vs 17, y 142 vs 141), which is why it is drawn after it.
/// Presentational: sorting/filtering the roster is not on the wire and not in
/// this row's scope.
const SORT_CONDITION: (f32, f32, f32, f32) = (18.0, 142.0, 20.0, 20.0);
const SORT_CONDITION_DDJ: &str = "media://interface/stall/stl_condition.ddj";
/// `GDR_GUILD_SORT_STATIC2` (`:625`, Rect `329,141,0,0`) — art-sized
/// `gil_shape.ddj` 16x24, the right cap.
const SORT_CAP_RIGHT: (f32, f32, f32, f32) = (329.0, 141.0, 16.0, 24.0);
const SORT_CAP_RIGHT_DDJ: &str = "media://interface/guild/gil_shape.ddj";

/// The four sort buttons, in file order 1..4 (`:701,:682,:663,:644`). The art
/// widths are the measured DDJ extents (`w=h=0` ⇒ art-sized), and the x-run
/// **overlaps its neighbours on purpose**: 39+132 = 171 vs 170 (−1),
/// 170+44 = 214 vs 211 (−3), 211+52 = 263 vs 261 (−2). Those seams are how
/// vanilla butts the header plates together, so they are reproduced verbatim
/// rather than "corrected" to a clean tiling.
const SORT_BUTTONS: [(f32, f32, &str, &str, &str); 4] = [
    (
        39.0,
        132.0,
        "media://interface/guild/gil_subj_button02.ddj",
        "UIIT_STT_GUILDSMAN",
        "Member",
    ),
    (
        170.0,
        44.0,
        "media://interface/guild/gil_subj_button03.ddj",
        "UIIT_STT_LEVEL",
        "Level",
    ),
    (
        211.0,
        52.0,
        "media://interface/guild/gil_subj_button04.ddj",
        "UIIT_STT_GRADE",
        "Grade",
    ),
    (
        261.0,
        68.0,
        "media://interface/guild/gil_subj_button05.ddj",
        "UIIT_STT_GP_SUBSCRIPION",
        "Donate GP",
    ),
];
const SORT_BUTTON_Y: f32 = 141.0;
const SORT_BUTTON_H: f32 = 24.0;

// --- The row prototype, `ifguildmemberslot.txt` (6), slot-local ------------

/// `GDR_GMS_ONOFF` (`:110`, Rect `5,6,0,0`) — art-sized `gil_contact_off.ddj`,
/// 12x16; `_on` is the same size. This is the one cell whose **art** carries
/// state.
const SLOT_ONOFF: (f32, f32, f32, f32) = (5.0, 6.0, 12.0, 16.0);
const SLOT_ONOFF_OFF_DDJ: &str = "media://interface/guild/gil_contact_off.ddj";
const SLOT_ONOFF_ON_DDJ: &str = "media://interface/guild/gil_contact_on.ddj";
/// `GDR_GMS_RACE_MARK` (`:91`, Rect `36,5,16,16`) — the same
/// `com_kindred_china16.ddj` placeholder the header's leader mark uses.
const SLOT_RACE_MARK: (f32, f32, f32, f32) = (36.0, 5.0, 16.0, 16.0);
/// `GDR_GMS_NAME` (`:72`, `62,7,84,14`, HAlign 1).
const SLOT_NAME: (f32, f32, f32, f32) = (62.0, 7.0, 84.0, 14.0);
/// `GDR_GMS_LEVEL` (`:53`, `160,7,26,14`, HAlign 1).
const SLOT_LEVEL: (f32, f32, f32, f32) = (160.0, 7.0, 26.0, 14.0);
/// `GDR_GMS_GRADE` (`:34`, `199,7,40,14`, HAlign 1).
const SLOT_GRADE: (f32, f32, f32, f32) = (199.0, 7.0, 40.0, 14.0);
/// `GDR_GMS_DONATEDGP` (`:15`, `254,7,48,14`, HAlign 1).
const SLOT_DONATED_GP: (f32, f32, f32, f32) = (254.0, 7.0, 48.0, 14.0);

/// One roster cell, addressed by row index and column.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct GuildRosterCell {
    pub row: usize,
    pub column: RosterColumn,
}

/// The four text columns of a member row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RosterColumn {
    Name,
    Level,
    /// `UIIT_STT_GRADE`. **Deliberately blank**: the `0x3101` roster record
    /// (`packets::agent::guild::GuildMember`) carries no grade/rank field —
    /// it has the character level, the guild points, the permission bits and
    /// the grant-name `nickname`, none of which *is* a grade. Filling this
    /// from `nickname` or from the permission bits would be inventing the
    /// column's meaning, so it renders empty until a capture names it.
    Grade,
    DonatedGp,
}

/// The online/offline dot of a row — the only cell whose art changes.
#[derive(Component, Clone, Copy)]
pub struct GuildRosterOnline(pub usize);

/// Which roster row the player last clicked, or `None`.
///
/// The window needs one because 0x70F4 (expel) is the family's only
/// **name-addressed** operation (`packets/src/agent/guild.rs`, builder
/// writes a `strA`), so the command column has to be able to
/// name a member — and the only member names the client holds are the record's.
/// `friend.rs` set the pattern for this page group: one selection resource, the
/// command column acts on it.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct GuildRosterSelection(pub Option<usize>);

/// A clickable roster row, by index into [`GuildRoster`]'s member list.
///
/// `ifguildmemberslot.txt` declares six statics and **no** hit area (unlike the
/// notice strip, which has an explicit `CIFSelectableArea`) — selection lives
/// in the slot's own class `CNIFGuildUserSlot`, which the resinfo tree cannot
/// show. So the hit area is ours: one transparent node over the whole row.
#[derive(Component, Clone, Copy)]
pub struct GuildRosterRow(pub usize);

/// One of the three `com_bar01select_` pieces that mark the selected row. Only
/// the row is carried: which piece it is decides the art at spawn time and
/// never changes afterwards, so keeping it here would be a field nothing reads.
#[derive(Component, Clone, Copy)]
pub struct GuildRosterRowBar {
    pub row: usize,
}

/// The selected row's plate: `com_bar01select_` (4/24/4 pieces), the
/// tree's own selected-row art — `friend.rs:114` draws the same pair for the
/// friends list.
///
/// **Deliberate deviation, stated (ADR-0009)**: the classic data gives this
/// list no selection art at all, because the drawing happens inside
/// `CNIFGuildUserSlot`. Rather than invent a tint, the highlight reuses the art
/// the same window family already uses for the same meaning, and it is drawn
/// **only** under the selected row — an unselected row keeps today's look
/// exactly, so nothing the data states changes.
const ROW_BAR_SELECTED_DIR: &str = "media://interface/ifcommon/com_bar01select_";
const ROW_BAR_CAP: f32 = 4.0;

/// The 3 pieces of a `com_bar0*_` plate over a rect: two caps and the middle.
fn bar(rect: (f32, f32, f32, f32), cap: f32) -> [((f32, f32, f32, f32), &'static str); 3] {
    let (x, y, w, h) = rect;
    [
        ((x, y, cap, h), "left"),
        ((x + cap, y, w - 2.0 * cap, h), "mid"),
        ((x + w - cap, y, cap, h), "right"),
    ]
}

/// The per-row cells for a member, or the empty state past the roster's end.
fn cell_text(column: RosterColumn, member: Option<&packets::agent::guild::GuildMember>) -> String {
    let Some(member) = member else {
        return String::new();
    };
    match column {
        RosterColumn::Name => member.name.clone(),
        RosterColumn::Level => member.level.to_string(),
        RosterColumn::Grade => String::new(),
        RosterColumn::DonatedGp => format_thousands(u64::from(member.guild_points)),
    }
}

// --- §NotifySubBox — the notice strip, `ifguild.txt:354-414` (3) ------------

/// `GDR_GUILD_NOTIFY_SUBJECT_STATIC` (`:365`, `24,117,53,14`, FontColor
/// `255,239,218,164`, Text `UIIT_STT_GUILD_COMMON_KNOW`).
const NOTICE_LABEL_RECT: (f32, f32, f32, f32) = (24.0, 117.0, 53.0, 14.0);
/// `GDR_GUILD_NOTIFY_SUBJECT:CIFSelectableArea` (`:403`, `15,110,428,28`) —
/// the click target that opens the read pane. `CIFSelectableArea` occurs
/// **twice** in the whole 247-file resinfo corpus (here and in
/// `ifapprenticeship.txt`), so it is a real class, not a typo: an invisible
/// hit area over the strip, which is why it draws no art.
const NOTICE_SUBJECT_RECT: (f32, f32, f32, f32) = (15.0, 110.0, 428.0, 28.0);
/// Where the subject text itself is drawn inside that area. **Ours**: the
/// `CIFSelectableArea` carries no text rect of its own, so the subject is laid
/// out just right of the label, on the label's own baseline.
const NOTICE_SUBJECT_TEXT_RECT: (f32, f32, f32, f32) = (82.0, 117.0, 330.0, 14.0);
/// `GDR_GUILD_NOTIFY_EDIT_BTN:CIFButton` (`:384`, `417,111,0,0`) — art-sized
/// `stl_edit_button.ddj`, 24x24. It opens the notice **write** modal,
/// which is a separate tree (`ifguildnotifywrite.txt` under its own
/// `ginterface.txt` host) and lives in [`super::notice_write`]; that modal is
/// what sends `0x70F9`.
const NOTICE_EDIT_BTN: (f32, f32, f32, f32) = (417.0, 111.0, 24.0, 24.0);
const NOTICE_EDIT_BTN_DDJ: &str = "media://interface/stall/stl_edit_button.ddj";

// --- §NotifyContents — the read pane, `ifguild.txt:732-753` + its child -----

/// `GDR_GUILD_NOTIFY_CONTENTS:CIFGuildNotifyContents` (`:743`, `6,138,440,177`)
/// — the host. It is **co-anchored at y=138** with `§MemberView`'s plate
/// (`14,138,333,166`) and `§GrantPower`'s (`14,138,423,143`): the shared rect
/// *is* the client's "if", so exactly one of the three may be visible. This
/// host is spawned hidden and shown by clicking the subject strip; it is wider
/// and taller than the roster plate it covers (asserted in the tests).
const NOTICE_PANE_RECT: (f32, f32, f32, f32) = (6.0, 138.0, 440.0, 177.0);

/// `ifguildnotifycontents.txt`, host-relative (6 controls).
/// `GDR_GUILD_NOTIFY_CON_BLACKSQUARE:CIFStretchWnd` (`:110`, `15,15,410,125`).
const NOTICE_PLATE_RECT: (f32, f32, f32, f32) = (15.0, 15.0, 410.0, 125.0);
/// `_BG_02:CIFStatic` (`:72`, `19,19,402,117`) — `com_bg_tile_e.ddj`.
const NOTICE_BG02_RECT: (f32, f32, f32, f32) = (19.0, 19.0, 402.0, 117.0);
const NOTICE_BG02_DDJ: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_e.ddj";
/// `_TEXT:CIFTextBox` (`:15`, `27,27,371,101`) — the notice body.
const NOTICE_TEXT_RECT: (f32, f32, f32, f32) = (27.0, 27.0, 371.0, 101.0);
/// `_SCROLL:CIFVerticalScroll` (`:34`, `406,32,16,75`) — declared with an
/// empty `DDJ=`, i.e. the engine's own scrollbar art. The shared scroll widget
/// is #56's, so the rect is transcribed and reserved; nothing is drawn into it
/// rather than inventing a bar.
const NOTICE_SCROLL_RECT: (f32, f32, f32, f32) = (406.0, 32.0, 16.0, 75.0);
/// `_BG_01:CIFNormalTile` (`:91`, `16,140,408,21`) — `com_bg_tile_b.ddj`.
const NOTICE_BG01_RECT: (f32, f32, f32, f32) = (16.0, 140.0, 408.0, 21.0);
/// `_BUTTON:CIFButton` (`:53`, `182,145,0,0`) — art-sized `com_button.ddj`,
/// measured 76x24, FontColor `255,255,245,218`, Text `UIIS_CTL_CONFIRM`. It
/// closes the pane, which is the only thing a read pane's OK can do.
const NOTICE_OK_RECT: (f32, f32, f32, f32) = (182.0, 145.0, 76.0, 24.0);
const NOTICE_OK_DDJ: &str = "media://interface/ifcommon/com_button.ddj";
/// `FontColor=255,255,245,218` — the read pane's Confirm caption.
pub(super) const NOTICE_OK_COLOR: Color = Color::srgb(1.0, 1.0, 218.0 / 255.0);

// --- §Command — the action column, `ifguild.txt:477-594` (6 in 5 slots) -----

/// Every command button is `com_mid_button.ddj` (measured **88x24**) at
/// `353,y,0,0`, HAlign 1 / VAlign 1, FontColor `255,255,245,218`.
/// `353 + 88 = 441 <= 451`, so the column fits the page as authored.
const COMMAND_X: f32 = 353.0;
const COMMAND_BTN_SIZE: (f32, f32) = (88.0, 24.0);
const COMMAND_BTN_DDJ: &str = "media://interface/ifcommon/com_mid_button.ddj";
/// `FontColor=255,255,245,218` (`ifguild.txt:579` and its five siblings).
const COMMAND_TEXT_COLOR: Color = Color::srgb(1.0, 1.0, 218.0 / 255.0);

/// The five slot positions, top to bottom, on a **27** pitch
/// (`:583,:564,:545,:526,:507`).
const COMMAND_YS: [f32; 5] = [142.0, 169.0, 196.0, 223.0, 250.0];

/// Slot index of "Join" — the first command with a wire path.
const COMMAND_SLOT_INVITE: usize = 0;
/// Slot index of "Withdraw" (`UIIT_STT_GUILD_EXPULSION`, `ifguild.txt:548`) —
/// the expel command, 0x70F4. Third of the five slots, i.e. `COMMAND_KEYS[2]`.
const COMMAND_SLOT_EXPEL: usize = 2;

/// The four unconditional commands, in id order 101..104.
///
/// **Generation check**: the 4th-gen shell `res_ui/guild.2dt`, whose
/// command row is five wide and reads `Join · Check authority · Withdraw ·
/// Grant name · Position allocating` — its slot 2 is id 38, key
/// `UIIT_CTL_GUILD_UNION_CHAT_CONFIRM` = "Check authority", and it has **no**
/// `Leave` at all. We clone the classic tree, and there slot 2 is
/// `ifguild.txt:567` `UIIT_CTL_AUTHORITY_GRANT` = "Grant authority" and
/// `:529` `UIIT_STT_GUILD_EXIT` = "Leave" is a real, declared sixth button.
/// Both captions resolve out of the same `textuisystem.txt`, so this is two
/// generations disagreeing, not a wrong string — the classic keys stay.
const COMMAND_KEYS: [(&str, &str); 4] = [
    ("UIIT_STT_GUILD_JOIN", "Join"),
    ("UIIT_CTL_AUTHORITY_GRANT", "Grant authority"),
    ("UIIT_STT_GUILD_EXPULSION", "Withdraw"),
    ("UIIT_STT_GUILD_EXIT", "Leave"),
];

/// The fifth slot's two candidates. Ids **105 and 106 share
/// `353,250,0,0` byte-for-byte** (`:507` / `:488`), so exactly one is ever
/// drawn — the data expresses the choice as a geometric collision and states
/// no rule. Id 106 additionally carries `Style=64` where every sibling carries
/// `0`, and what that bit means is `[U]` (`hud-guild-window.md` §9-U5). Hence
/// the config flag rather than a guess: default id 105, the era-specific
/// id 106 behind `guild.position_grant`.
const COMMAND_SLOT5_DEFAULT: (&str, &str) = ("UIIT_STT_GUILD_NAME_GRANT", "Grant name");
const COMMAND_SLOT5_POSITION_GRANT: (&str, &str) =
    ("UIIT_CTL_GUILD_POSITION_GRANT", "Position allocating");

/// The five captions actually drawn, for the given flag.
fn command_keys(position_grant: bool) -> [(&'static str, &'static str); 5] {
    let slot5 = if position_grant {
        COMMAND_SLOT5_POSITION_GRANT
    } else {
        COMMAND_SLOT5_DEFAULT
    };
    [
        COMMAND_KEYS[0],
        COMMAND_KEYS[1],
        COMMAND_KEYS[2],
        COMMAND_KEYS[3],
        slot5,
    ]
}

/// The first of the two commands with a fully sourced request *and* an
/// addressable target: invite (0x70F3). It sends the click-selected entity's
/// spawn id, which is how the original addresses it too — the guild window has
/// no target picker of its own, the world selection is the target.
///
/// The remaining three stay presentational: leave (0x70F2), disband (0x70F1)
/// and promote (0x70FA) each carry one `u32` the original does not name, and
/// all three candidate readings answer the same generic `0x0003` — with a
/// single-member guild there is no target to promote, so what decides it is a
/// session with a **second** guild member. Until then sending one would mean
/// inventing its value.
fn on_guild_invite(
    _: On<Activate>,
    selected: Res<SelectedEntity>,
    ids: Query<&NetworkId>,
    mut actions: MessageWriter<GuildAction>,
) {
    let Some(entity) = selected.0 else {
        info!("guild: invite clicked with nothing selected");
        return;
    };
    match ids.get(entity) {
        Ok(id) => {
            actions.write(GuildAction::Invite(id.0));
        }
        // A selected world entity always carries a NetworkId; sending a zero
        // uid instead would be a silent, server-visible mistake.
        Err(_) => warn!("guild: invite on {entity:?}, which has no NetworkId"),
    }
}

/// A roster row was clicked: select it, if a member occupies it.
///
/// An empty row is not selectable — "row 5 of a one-member guild" is not a
/// person, and the command column acts on a person (`friend.rs`'s
/// `on_friend_row_press` makes the same distinction for the same reason).
fn on_guild_roster_row_press(
    press: On<Pointer<Press>>,
    rows: Query<&GuildRosterRow>,
    roster: Res<GuildRoster>,
    mut selection: ResMut<GuildRosterSelection>,
) {
    let Ok(row) = rows.get(press.entity) else {
        return;
    };
    let members = roster.data.as_ref().map(|d| d.members.len()).unwrap_or(0);
    selection.0 = row_selection(row.0, members);
}

/// The rule, apart from the query plumbing so it can be pinned by a test.
fn row_selection(row: usize, member_count: usize) -> Option<usize> {
    (row < member_count).then_some(row)
}

/// Slot 3, "Withdraw": expel the selected roster member — 0x70F4.
///
/// Why this one can send while leave/disband/promote cannot: 0x70F4 is the
/// family's only **name-addressed** membership operation (`strA`, see the builder),
/// so its whole payload is a value the client already
/// holds — the selected row's `GuildMember::name`. Nothing is invented here.
///
/// Three refusals, in the order the original checks them:
///
/// * no selection — a client-side answer in our own words, because the
///   original's `UIIT_MSG_GUILDERR_*` strings all speak with the *server's*
///   voice and reusing one here would put a false server refusal on screen;
/// * no `KICK` bit — `UIIT_MSG_GUILDERR_PERMISSION_DENIED`, the same key and
///   the same client-side pre-check `notice_write.rs` makes for the notice.
///   Advisory only: the server decides, and it answers 0xB0F4, which
///   `net::guild` already turns into a chat line;
/// * expelling **yourself** — the record's own master flag makes this
///   checkable, and the original has a separate command ("Leave") for it.
fn on_guild_expel(
    _: On<Activate>,
    roster: Res<GuildRoster>,
    selection: Res<GuildRosterSelection>,
    players: Query<&CharacterInfo, With<Player>>,
    ui_strings: Res<ClientUiStrings>,
    mut history: ResMut<ChatHistory>,
    mut actions: MessageWriter<GuildAction>,
) {
    let Some(target) = selection
        .0
        .and_then(|row| roster.data.as_ref()?.members.get(row))
        .cloned()
    else {
        history.push(ChatLine::system(EXPEL_NO_SELECTION));
        return;
    };
    let own_name = players.single().ok().and_then(|info| info.name.clone());
    if own_name.as_deref() == Some(target.name.as_str()) {
        history.push(ChatLine::system(EXPEL_SELF));
        return;
    }
    let permitted = own_name
        .as_deref()
        .and_then(|n| roster.permissions_for(n))
        .is_some_and(|p| p.can_kick());
    if !permitted {
        history.push(ChatLine::system(ui_strings.get_or(
            "UIIT_MSG_GUILDERR_PERMISSION_DENIED",
            "You are not authorized.",
        )));
        return;
    }
    actions.write(GuildAction::Kick(target.name));
}

/// Ours, not the original's: see [`on_guild_expel`] for why a
/// `UIIT_MSG_GUILDERR_*` key would be the wrong voice for a client-side stop.
const EXPEL_NO_SELECTION: &str = "Select a member in the guild list first.";
const EXPEL_SELF: &str = "Use Leave to leave the guild yourself.";

/// Is the notice read pane open? The three y=138 panes are mutually exclusive
/// by construction (they share the rect), so this is a boolean today and
/// becomes a three-way when §GrantPower lands.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct GuildNoticeOpen(pub bool);

/// The read pane's host node.
#[derive(Component)]
pub struct GuildNoticePane;

/// The strip's subject text (the notice **title**) and the pane's body text.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GuildNoticeField {
    /// `GuildData::notice` — the title half of `GuildNoticeEditRequest`.
    Subject,
    /// `GuildData::message` — the body half.
    Body,
}

fn notice_text(field: GuildNoticeField, data: Option<&packets::agent::guild::GuildData>) -> String {
    let Some(data) = data else {
        return String::new();
    };
    match field {
        GuildNoticeField::Subject => data.notice.clone(),
        GuildNoticeField::Body => data.message.clone(),
    }
}

/// Clicking the subject strip opens the read pane.
fn on_notice_subject(_: On<Activate>, mut open: ResMut<GuildNoticeOpen>) {
    open.0 = true;
}

/// The pane's Confirm closes it — a read pane has nothing else to confirm.
fn on_notice_confirm(_: On<Activate>, mut open: ResMut<GuildNoticeOpen>) {
    open.0 = false;
}

/// Mirror [`GuildNoticeOpen`] onto the pane, and push the record's notice
/// strings onto the two texts.
pub fn update_guild_notice(
    roster: Res<GuildRoster>,
    open: Res<GuildNoticeOpen>,
    mut panes: Query<&mut Node, With<GuildNoticePane>>,
    mut fields: Query<(&GuildNoticeField, &mut Text)>,
) {
    if open.is_changed() {
        for mut node in panes.iter_mut() {
            node.display = if open.0 { Display::Flex } else { Display::None };
        }
    }
    if !roster.is_changed() {
        return;
    }
    for (field, mut text) in fields.iter_mut() {
        let next = notice_text(*field, roster.data.as_ref());
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// Fill the Guild page container with §Create + §GuildInfo + the roster.
pub fn spawn_guild_page(
    page: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    position_grant: bool,
    s: f32,
) {
    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };
    let image = |rect: (f32, f32, f32, f32), path: String| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        )
    };

    // --- §Create -----------------------------------------------------------

    // GDR_GUILD_INFO_WND — clipped at the page's right edge (module doc).
    let (ix, iy) = INFO_WND_POS;
    page.spawn((
        Node {
            overflow: Overflow::clip(),
            ..abs_node(
                (ix, iy, clipped_width(ix, INFO_WND_ART.0), INFO_WND_ART.1),
                s,
            )
        },
        Pickable::IGNORE,
    ))
    .with_children(|clip| {
        clip.spawn(image(
            (0.0, 0.0, INFO_WND_ART.0, INFO_WND_ART.1),
            INFO_WND_DDJ.to_string(),
        ));
    });

    // GDR_GUILD_BG — the tiled plate under the notice strip.
    page.spawn((
        abs_node(BG_RECT, s),
        ImageNode {
            image: asset_server.load(BG_TILE_DDJ),
            image_mode: NodeImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: s,
            },
            ..default()
        },
        Pickable::IGNORE,
    ));

    // GDR_GUILD_FRAME — the frameg01_wnd_ ring around the body.
    let (fx, fy, fw, fh) = FRAME_RECT;
    for ((x, y, w, h), piece) in ring(fw, fh, FRAME_PIECE) {
        page.spawn(image(
            (fx + x, fy + y, w, h),
            format!("{FRAME_DIR}{piece}.ddj"),
        ));
    }

    // --- §GuildInfo --------------------------------------------------------

    // GDR_GUILD_INFO_BAR_BOARD — clipped exactly like INFO_WND.
    let (bx, by) = BAR_BOARD_POS;
    page.spawn((
        Node {
            overflow: Overflow::clip(),
            ..abs_node(
                (bx, by, clipped_width(bx, BAR_BOARD_ART.0), BAR_BOARD_ART.1),
                s,
            )
        },
        Pickable::IGNORE,
    ))
    .with_children(|clip| {
        clip.spawn(image(
            (0.0, 0.0, BAR_BOARD_ART.0, BAR_BOARD_ART.1),
            BAR_BOARD_DDJ.to_string(),
        ));
    });

    // The three plain art statics.
    page.spawn(image(
        (MARK_POS.0, MARK_POS.1, MARK_SIZE.0, MARK_SIZE.1),
        MARK_DDJ.to_string(),
    ));
    page.spawn(image(LEADER_RACE_RECT, LEADER_RACE_DDJ.to_string()));
    page.spawn(image(
        (
            GP_BUTTON_POS.0,
            GP_BUTTON_POS.1,
            GP_BUTTON_SIZE.0,
            GP_BUTTON_SIZE.1,
        ),
        GP_BUTTON_DDJ.to_string(),
    ));

    // GDR_GUILD_INFO_GP_GAUGE — the shared three-node gauge recipe
    // (`hud/gauge.rs`): track (authored rect) / crop (the fill) / art (native).
    let (gx, gy) = GP_GAUGE_POS;
    page.spawn((
        abs_node((gx, gy, GP_GAUGE_ART.0, GP_GAUGE_ART.1), s),
        Pickable::IGNORE,
    ))
    .with_children(|track| {
        track
            .spawn((
                GuildGpFill,
                gauge_crop_node(
                    gauge_fill_width(0.0, GP_GAUGE_ART.0 * s),
                    GP_GAUGE_ART.1 * s,
                ),
                Pickable::IGNORE,
            ))
            .with_children(|crop| {
                crop.spawn((
                    gauge_art_node(GP_GAUGE_ART.0 * s, GP_GAUGE_ART.1 * s),
                    ImageNode {
                        image: asset_server.load(GP_GAUGE_DDJ),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            });
    });

    // The four fixed labels.
    for (rect, key, fallback, color, justify) in [
        (
            LEVEL_LABEL_RECT,
            "UIO_CHARINFO_STT_LEVEL",
            "Level",
            LEVEL_LABEL_COLOR,
            Justify::Left,
        ),
        (
            LEADER_LABEL_RECT,
            "UIIT_STT_GUILD_LEADER",
            "Guild master",
            LABEL_COLOR,
            Justify::Left,
        ),
        (
            MEMBER_LABEL_RECT,
            "UIIT_STT_GUILDSMAN_NUM",
            "Number",
            LABEL_COLOR,
            Justify::Right,
        ),
        (
            GP_LABEL_RECT,
            "UIIT_STT_GUILD_POINT",
            "Guild point (GP)",
            LABEL_COLOR,
            Justify::Left,
        ),
    ] {
        page.spawn((
            Text::new(ui_strings.get_or(key, fallback).to_string()),
            text_font(7.5),
            TextColor(color),
            TextLayout::justify(justify),
            abs_node(rect, s),
            Pickable::IGNORE,
        ));
    }

    // The six runtime statics, all starting in the guildless empty state.
    for (field, rect, justify) in [
        (GuildInfoField::Name, NAME_RECT, Justify::Left),
        (GuildInfoField::Level, LEVEL_RECT, Justify::Left),
        (GuildInfoField::Leader, LEADER_RECT, Justify::Left),
        (GuildInfoField::MemberCount, MEMBER_NUM_RECT, Justify::Right),
        (GuildInfoField::GpPercent, GP_PERCENT_RECT, Justify::Center),
        (GuildInfoField::GpValue, GP_VALUE_RECT, Justify::Center),
    ] {
        page.spawn((
            field,
            Text::new(EMPTY.to_string()),
            text_font(7.5),
            TextColor(VALUE_COLOR),
            TextLayout::justify(justify),
            abs_node(rect, s),
            Pickable::IGNORE,
        ));
    }

    // --- §MemberView + §SortBtn --------------------------------------------

    // GDR_GUILD_MEMBER_VIEW_BG: the right gutter tile.
    page.spawn((
        abs_node(MEMBER_VIEW_BG_RECT, s),
        ImageNode {
            image: asset_server.load(BG_TILE_DDJ),
            image_mode: NodeImageMode::Tiled {
                tile_x: true,
                tile_y: true,
                stretch_value: s,
            },
            ..default()
        },
        Pickable::IGNORE,
    ));

    // GDR_GUILD_MEMBER_VIEW_BLACKSQUARE: the list plate + its 4px trim.
    let (mx, my, mw, mh) = MEMBER_PLATE_RECT;
    page.spawn((
        abs_node(MEMBER_PLATE_RECT, s),
        BackgroundColor(Color::BLACK),
        Pickable::IGNORE,
    ));
    for ((x, y, w, h), piece) in blacksquare_ring(mw, mh) {
        page.spawn(image(
            (mx + x, my + y, w, h),
            format!("{BLACKSQUARE_DIR}{piece}.ddj"),
        ));
    }

    // The header strip: left cap, the four sort plates, right cap — then the
    // condition button *on top of* the left cap (see SORT_CONDITION).
    page.spawn(image(SORT_CAP_LEFT, SORT_CAP_LEFT_DDJ.to_string()));
    for (x, w, ddj, key, fallback) in SORT_BUTTONS {
        page.spawn(image((x, SORT_BUTTON_Y, w, SORT_BUTTON_H), ddj.to_string()))
            .with_children(|header| {
                header.spawn((
                    Text::new(ui_strings.get_or(key, fallback).to_string()),
                    text_font(7.5),
                    TextColor(VALUE_COLOR),
                    TextLayout::justify(Justify::Center),
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(6.0 * s),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            });
    }
    page.spawn(image(SORT_CAP_RIGHT, SORT_CAP_RIGHT_DDJ.to_string()));
    page.spawn(image(SORT_CONDITION, SORT_CONDITION_DDJ.to_string()));

    // The six visible member rows, each a slot-local transcription of
    // `ifguildmemberslot.txt` translated to the manager's origin.
    let (lx, ly, _, _) = MEMBER_LIST_RECT;
    for row in 0..VISIBLE_ROWS {
        let top = ly + row as f32 * ROW_PITCH;
        // The selection plate first, so every cell of the row draws on top of
        // it. Hidden until the row is both occupied and selected.
        for (rect, piece) in bar((lx, top, MEMBER_LIST_RECT.2, ROW_HEIGHT), ROW_BAR_CAP) {
            page.spawn((
                GuildRosterRowBar { row },
                {
                    let mut node = abs_node(rect, s);
                    node.display = Display::None;
                    node
                },
                ImageNode {
                    image: asset_server.load(format!("{ROW_BAR_SELECTED_DIR}{piece}.ddj")),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        // The row's own hit area: transparent, full width, above the plate and
        // below nothing — the cells are `Pickable::IGNORE`, so a click on a
        // name still lands here.
        page.spawn((
            GuildRosterRow(row),
            Button,
            Hovered::default(),
            Pickable::default(),
            abs_node((lx, top, MEMBER_LIST_RECT.2, ROW_HEIGHT), s),
        ))
        .observe(on_guild_roster_row_press);
        page.spawn((
            GuildRosterOnline(row),
            {
                let mut node = abs_node(
                    (
                        lx + SLOT_ONOFF.0,
                        top + SLOT_ONOFF.1,
                        SLOT_ONOFF.2,
                        SLOT_ONOFF.3,
                    ),
                    s,
                );
                node.display = Display::None;
                node
            },
            ImageNode {
                image: asset_server.load(SLOT_ONOFF_OFF_DDJ),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        for (column, rect) in [
            (RosterColumn::Name, SLOT_NAME),
            (RosterColumn::Level, SLOT_LEVEL),
            (RosterColumn::Grade, SLOT_GRADE),
            (RosterColumn::DonatedGp, SLOT_DONATED_GP),
        ] {
            page.spawn((
                GuildRosterCell { row, column },
                Text::new(String::new()),
                text_font(7.5),
                TextColor(VALUE_COLOR),
                TextLayout::justify(Justify::Center),
                abs_node((lx + rect.0, top + rect.1, rect.2, rect.3), s),
                Pickable::IGNORE,
            ));
        }
    }

    // --- §NotifySubBox ------------------------------------------------------

    page.spawn(label_static(
        NOTICE_LABEL_RECT,
        ui_strings
            .get_or("UIIT_STT_GUILD_COMMON_KNOW", "Notice")
            .to_string(),
        LABEL_COLOR,
        Justify::Left,
        &text_font(7.5),
        s,
    ));
    page.spawn((
        GuildNoticeField::Subject,
        Text::new(String::new()),
        text_font(7.5),
        TextColor(VALUE_COLOR),
        TextLayout::justify(Justify::Left),
        abs_node(NOTICE_SUBJECT_TEXT_RECT, s),
        Pickable::IGNORE,
    ));
    // the invisible CIFSelectableArea over the strip
    page.spawn((
        Button,
        Hovered::default(),
        abs_node(NOTICE_SUBJECT_RECT, s),
        Pickable::default(),
    ))
    .observe(on_notice_subject);
    // The edit button leads somewhere now: the write modal is its own tree
    // (`ifguildnotifywrite.txt`, `super::notice_write`), which is why this one
    // control was presentational for as long as that tree was unbuilt.
    // NOTE: spawned in two steps on purpose. As one
    // tuple this **panicked the client at startup** — `image()` already carries
    // `Pickable::IGNORE`, so adding `Pickable::default()` beside it makes a
    // bundle with a duplicate component, which Bevy rejects when the command
    // buffer is applied ("has duplicate components: [Pickable]"). `cargo build`
    // and `cargo test` do not see it; `SCENE=ui_testing` dies on it. `insert`
    // replaces instead of duplicating, which is why the two live command
    // buttons below use the same shape.
    let mut edit_button = page.spawn(image(NOTICE_EDIT_BTN, NOTICE_EDIT_BTN_DDJ.to_string()));
    edit_button.insert((Button, Hovered::default(), Pickable::default()));
    edit_button.observe(super::notice_write::on_notice_edit);

    // --- §NotifyContents — the read pane, hidden until the strip is clicked --

    let mut pane_node = abs_node(NOTICE_PANE_RECT, s);
    pane_node.display = Display::None;
    page.spawn((GuildNoticePane, pane_node, Pickable::IGNORE))
        .with_children(|pane| {
            // the plate: flat black + its 4px trim
            pane.spawn((
                abs_node(NOTICE_PLATE_RECT, s),
                BackgroundColor(Color::BLACK),
                Pickable::IGNORE,
            ));
            let (px, py, pw, ph) = NOTICE_PLATE_RECT;
            for ((x, y, w, h), piece) in blacksquare_ring(pw, ph) {
                pane.spawn((
                    abs_node((px + x, py + y, w, h), s),
                    ImageNode {
                        image: asset_server.load(format!("{BLACKSQUARE_DIR}{piece}.ddj")),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            for (rect, ddj) in [
                (NOTICE_BG02_RECT, NOTICE_BG02_DDJ),
                (NOTICE_BG01_RECT, BG_TILE_DDJ),
            ] {
                pane.spawn((
                    abs_node(rect, s),
                    ImageNode {
                        image: asset_server.load(ddj),
                        image_mode: NodeImageMode::Tiled {
                            tile_x: true,
                            tile_y: true,
                            stretch_value: s,
                        },
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            pane.spawn((
                GuildNoticeField::Body,
                Text::new(String::new()),
                text_font(7.5),
                TextColor(VALUE_COLOR),
                TextLayout::justify(Justify::Left),
                abs_node(NOTICE_TEXT_RECT, s),
                Pickable::IGNORE,
            ));
            pane.spawn((
                Button,
                Hovered::default(),
                abs_node(NOTICE_OK_RECT, s),
                ImageNode {
                    image: asset_server.load(NOTICE_OK_DDJ),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
            ))
            .observe(on_notice_confirm)
            .with_children(|button| {
                button.spawn((
                    Text::new(ui_strings.get_or("UIIS_CTL_CONFIRM", "OK").to_string()),
                    text_font(7.5),
                    TextColor(NOTICE_OK_COLOR),
                    TextLayout::justify(Justify::Center),
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(6.0 * s),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            });
        });

    // --- §Command — the action column ---------------------------------------
    //
    // Two slots are live: 1 ("Join") invites the click-selected player via
    // 0x70F3, and 3 ("Withdraw") expels the selected roster row via 0x70F4 —
    // the family's only name-addressed operation, so the row selection this
    // page now has is the whole payload. The other three stay presentational,
    // and not for lack of an opcode: leave/disband/promote each carry one
    // `u32` the original does not name and whose three candidate readings all
    // failed identically against a live server.
    // `letter.rs` set the precedent: draw the plate and its caption rather
    // than attach an observer that would send an invented value.
    for (slot, (y, (key, fallback))) in COMMAND_YS
        .iter()
        .zip(command_keys(position_grant))
        .enumerate()
    {
        let mut button = page.spawn(image(
            (COMMAND_X, *y, COMMAND_BTN_SIZE.0, COMMAND_BTN_SIZE.1),
            COMMAND_BTN_DDJ.to_string(),
        ));
        if slot == COMMAND_SLOT_INVITE {
            button.insert((Button, Hovered::default(), Pickable::default()));
            button.observe(on_guild_invite);
        }
        if slot == COMMAND_SLOT_EXPEL {
            button.insert((Button, Hovered::default(), Pickable::default()));
            button.observe(on_guild_expel);
        }
        button.with_children(|button| {
            button.spawn((
                Text::new(ui_strings.get_or(key, fallback).to_string()),
                text_font(7.5),
                TextColor(COMMAND_TEXT_COLOR),
                TextLayout::justify(Justify::Center),
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(6.0 * s),
                    width: Val::Percent(100.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
    }
}

/// A plain `CIFStatic` label bundle — the roster/notice sections spawn enough
/// of them that the tuple is worth a name.
fn label_static(
    rect: (f32, f32, f32, f32),
    text: String,
    color: Color,
    justify: Justify,
    font: &TextFont,
    s: f32,
) -> impl Bundle {
    (
        Text::new(text),
        font.clone(),
        TextColor(color),
        TextLayout::justify(justify),
        abs_node(rect, s),
        Pickable::IGNORE,
    )
}

/// The six 4px trim pieces of a `com_blacksquare_` plate over a `w x h` box.
pub(super) fn blacksquare_ring(w: f32, h: f32) -> [((f32, f32, f32, f32), &'static str); 6] {
    let p = BLACKSQUARE_PIECE;
    [
        ((0.0, 0.0, p, p), "left_up"),
        ((w - p, 0.0, p, p), "right_up"),
        ((0.0, p, p, h - 2.0 * p), "left_side"),
        ((w - p, p, p, h - 2.0 * p), "right_side"),
        ((0.0, h - p, p, p), "left_down"),
        ((w - p, h - p, p, p), "right_down"),
    ]
}

/// Push `GuildRoster`'s members onto the six visible rows, and draw the
/// selection.
///
/// No scrolling: the `CIFScrollManager` row pool is the shared widget of #56
/// and is not built here, so a guild with more than six members shows its
/// first six. Stated rather than silently truncated: this list uses no
/// `Overflow::scroll_y`, so nothing here is clipped out of reach — the six rows
/// are absolutely placed siblings.
///
/// Runs on a change of either the record or the selection, because the plate
/// under a row *is* the selection's rendering.
pub fn update_guild_roster(
    roster: Res<GuildRoster>,
    selection: Res<GuildRosterSelection>,
    asset_server: Res<AssetServer>,
    mut cells: Query<(&GuildRosterCell, &mut Text)>,
    // Both `&mut Node` queries have to exclude each other's marker: Bevy's
    // access check needs only one pair of queries that *could* match the same
    // entity, and a partial set compiles and then panics with B0001 at
    // startup (`friend.rs` paid for that lesson).
    mut dots: Query<(&GuildRosterOnline, &mut Node, &mut ImageNode), Without<GuildRosterRowBar>>,
    mut bars: Query<(&GuildRosterRowBar, &mut Node), Without<GuildRosterOnline>>,
) {
    if !roster.is_changed() && !selection.is_changed() {
        return;
    }
    let members = roster
        .data
        .as_ref()
        .map(|data| data.members.as_slice())
        .unwrap_or(&[]);
    for (cell, mut text) in cells.iter_mut() {
        let next = cell_text(cell.column, members.get(cell.row));
        if text.0 != next {
            text.0 = next;
        }
    }
    for (dot, mut node, mut image) in dots.iter_mut() {
        match members.get(dot.0) {
            Some(member) => {
                node.display = Display::Flex;
                image.image = asset_server.load(if member.is_offline {
                    SLOT_ONOFF_OFF_DDJ
                } else {
                    SLOT_ONOFF_ON_DDJ
                });
            }
            None => node.display = Display::None,
        }
    }
    // The selection plate: visible only where a member sits *and* is selected,
    // so an out-of-range selection (a shorter record after a kick) draws
    // nothing rather than a bar under an empty row.
    for (piece, mut node) in bars.iter_mut() {
        node.display = if selection.0 == Some(piece.row) && piece.row < members.len() {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Width of an art-sized static after clipping at the page's right edge.
/// The `[U]` in `hud-guild-window.md` §9-U4 is *how* the original handles the
/// two overflows; clipping is our stated decision, and this is the one place
/// it happens.
fn clipped_width(x: f32, art_w: f32) -> f32 {
    (PAGE_SIZE.0 - x).min(art_w).max(0.0)
}

/// Push `GuildRoster` onto the six runtime statics whenever the record moves.
pub fn update_guild_info(
    roster: Res<GuildRoster>,
    mut fields: Query<(&GuildInfoField, &mut Text)>,
) {
    if !roster.is_changed() {
        return;
    }
    for (field, mut text) in fields.iter_mut() {
        let next = field_text(*field, roster.data.as_ref());
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// The 8 pieces of a `*_wnd_` frame ring over a `w x h` box, at `p` px pieces.
fn ring(w: f32, h: f32, p: f32) -> [((f32, f32, f32, f32), &'static str); 8] {
    [
        ((0.0, 0.0, p, p), "left_up"),
        ((p - 1.0, 0.0, w - 2.0 * p + 2.0, p), "mid_up"),
        ((w - p, 0.0, p, p), "right_up"),
        ((0.0, p - 1.0, p, h - 2.0 * p + 2.0), "left_side"),
        ((w - p, p - 1.0, p, h - 2.0 * p + 2.0), "right_side"),
        ((0.0, h - p, p, p), "left_down"),
        ((p - 1.0, h - p, w - 2.0 * p + 2.0, p), "mid_down"),
        ((w - p, h - p, p, p), "right_down"),
    ]
}

#[cfg(test)]
mod test {
    use super::*;
    use packets::agent::guild::{GuildData, GuildMember, GuildPermissions};

    fn member(name: &str, is_master: bool) -> GuildMember {
        GuildMember {
            member_id: 1,
            name: name.to_string(),
            unk_u8_01: 0,
            level: 40,
            guild_points: 0,
            permissions: 0,
            unk_u32_01: 0,
            unk_u32_02: 0,
            unk_u32_03: 0,
            nickname: String::new(),
            model_id: 0,
            is_master,
            // Present in a live guild record; its meaning is open.
            unk_u8_02: 0,
            is_offline: false,
        }
    }

    fn record() -> GuildData {
        GuildData {
            guild_id: 7,
            name: "Roadmen".to_string(),
            level: 3,
            guild_points: 12_345,
            notice: String::new(),
            message: String::new(),
            unk_u32_00: 0,
            unk_u8_00: 0,
            member_count: 2,
            members: vec![member("Grunt", false), member("Master", true)],
        }
    }

    /// The header strip is transcribed with its **overlapping** seams: the
    /// plates butt into each other by −1/−3/−2 and the last one ends exactly
    /// on the right cap. Reproducing that is the point — a "corrected" clean
    /// tiling would not be vanilla's strip.
    #[test]
    fn sort_header_seams_overlap_exactly_as_ifguild_declares() {
        let x: Vec<f32> = SORT_BUTTONS.iter().map(|b| b.0).collect();
        let w: Vec<f32> = SORT_BUTTONS.iter().map(|b| b.1).collect();
        assert_eq!(x, vec![39.0, 170.0, 211.0, 261.0]);
        assert_eq!(w, vec![132.0, 44.0, 52.0, 68.0]);
        assert_eq!(x[0] + w[0] - x[1], 1.0); // −1
        assert_eq!(x[1] + w[1] - x[2], 3.0); // −3
        assert_eq!(x[2] + w[2] - x[3], 2.0); // −2
        assert_eq!(x[3] + w[3], SORT_CAP_RIGHT.0); // 261+68 = 329, no gap
                                                   // the strip spans exactly the scroll manager's 17..345
        assert_eq!(SORT_CAP_LEFT.0, MEMBER_LIST_RECT.0);
        assert_eq!(
            SORT_CAP_RIGHT.0 + SORT_CAP_RIGHT.2,
            MEMBER_LIST_RECT.0 + MEMBER_LIST_RECT.2
        );
    }

    /// Six rows at pitch 23 fit the manager's 139 height exactly
    /// (`139 = 6·23 + 1`), and every slot-local cell fits the row's 24 px.
    #[test]
    fn six_member_rows_fit_the_scroll_manager() {
        assert_eq!(VISIBLE_ROWS as f32 * ROW_PITCH + 1.0, MEMBER_LIST_RECT.3);
        let last_top = (VISIBLE_ROWS - 1) as f32 * ROW_PITCH;
        assert!(last_top + ROW_HEIGHT <= MEMBER_LIST_RECT.3 + ROW_PITCH - ROW_HEIGHT + 1.0);
        for (x, y, w, h) in [
            SLOT_ONOFF,
            SLOT_RACE_MARK,
            SLOT_NAME,
            SLOT_LEVEL,
            SLOT_GRADE,
            SLOT_DONATED_GP,
        ] {
            assert!(
                y + h <= ROW_HEIGHT,
                "cell {x},{y},{w},{h} overflows the row"
            );
            assert!(
                x + w <= MEMBER_LIST_RECT.2,
                "cell {x},{y},{w},{h} overflows the manager width"
            );
        }
        // the doc's alignment check: every cell centre falls in its column
        assert!(SLOT_NAME.0 + SLOT_NAME.2 / 2.0 > SORT_BUTTONS[0].0 - MEMBER_LIST_RECT.0);
        assert_eq!(SLOT_DONATED_GP.0 + SLOT_DONATED_GP.2, 302.0);
    }

    /// The roster cells bind the record, and the Grade column stays blank:
    /// `GuildMember` carries no grade field, so filling it would be inventing
    /// the column's meaning.
    #[test]
    fn roster_cells_bind_the_member_record_and_leave_grade_blank() {
        let members = record().members;
        let grunt = members.first();
        assert_eq!(cell_text(RosterColumn::Name, grunt), "Grunt");
        assert_eq!(cell_text(RosterColumn::Level, grunt), "40");
        assert_eq!(cell_text(RosterColumn::DonatedGp, grunt), "0");
        assert_eq!(cell_text(RosterColumn::Grade, grunt), "");
        // past the roster's end every cell is empty, not a stale row
        for column in [
            RosterColumn::Name,
            RosterColumn::Level,
            RosterColumn::Grade,
            RosterColumn::DonatedGp,
        ] {
            assert_eq!(cell_text(column, None), "");
        }
    }

    /// The roster pane's own three rects stay inside the page, and the plate
    /// contains the row area it hosts.
    #[test]
    fn member_view_rects_fit_the_page_and_contain_the_row_area() {
        let (pw, ph) = PAGE_SIZE;
        for (x, y, w, h) in [MEMBER_PLATE_RECT, MEMBER_LIST_RECT, MEMBER_VIEW_BG_RECT] {
            assert!(x + w <= pw, "rect {x},{y},{w},{h} overflows the page width");
            assert!(
                y + h <= ph,
                "rect {x},{y},{w},{h} overflows the page height"
            );
        }
        assert!(MEMBER_PLATE_RECT.0 <= MEMBER_LIST_RECT.0);
        assert!(
            MEMBER_PLATE_RECT.1 + MEMBER_PLATE_RECT.3 >= MEMBER_LIST_RECT.1 + MEMBER_LIST_RECT.3
        );
    }

    /// The three y=138 panes share their anchor — that shared rect *is* the
    /// client's "if" — so the read pane must fully cover the roster plate it
    /// replaces, and only one may be visible.
    #[test]
    fn the_notice_pane_covers_the_roster_plate_it_replaces() {
        assert_eq!(NOTICE_PANE_RECT.1, MEMBER_PLATE_RECT.1); // both anchored at y=138
        assert!(NOTICE_PANE_RECT.0 <= MEMBER_PLATE_RECT.0);
        assert!(
            NOTICE_PANE_RECT.0 + NOTICE_PANE_RECT.2 >= MEMBER_PLATE_RECT.0 + MEMBER_PLATE_RECT.2
        );
        assert!(
            NOTICE_PANE_RECT.1 + NOTICE_PANE_RECT.3 >= MEMBER_PLATE_RECT.1 + MEMBER_PLATE_RECT.3
        );
        // and the pane still fits the page
        assert!(NOTICE_PANE_RECT.0 + NOTICE_PANE_RECT.2 <= PAGE_SIZE.0);
        assert!(NOTICE_PANE_RECT.1 + NOTICE_PANE_RECT.3 <= PAGE_SIZE.1);
    }

    /// Every `ifguildnotifycontents.txt` child is host-relative and fits the
    /// `6,138,440,177` host — the reading that makes the file consistent.
    #[test]
    fn notice_pane_children_are_host_relative_and_fit() {
        for (x, y, w, h) in [
            NOTICE_PLATE_RECT,
            NOTICE_BG02_RECT,
            NOTICE_TEXT_RECT,
            NOTICE_SCROLL_RECT,
            NOTICE_BG01_RECT,
            NOTICE_OK_RECT,
        ] {
            assert!(
                x + w <= NOTICE_PANE_RECT.2,
                "child {x},{y},{w},{h} overflows the host width"
            );
            assert!(
                y + h <= NOTICE_PANE_RECT.3,
                "child {x},{y},{w},{h} overflows the host height"
            );
        }
        // the body text sits inside its bg tile, which sits inside the plate
        assert!(NOTICE_TEXT_RECT.0 >= NOTICE_BG02_RECT.0);
        assert!(NOTICE_BG02_RECT.0 >= NOTICE_PLATE_RECT.0);
    }

    /// The strip shows the notice **title** and the pane its **body** — the
    /// same split `GuildNoticeEditRequest { title, message }` writes back.
    #[test]
    fn notice_strip_shows_the_title_and_the_pane_the_body() {
        let mut data = record();
        data.notice = "Server maintenance".to_string();
        data.message = "We move at 20:00.".to_string();
        assert_eq!(
            notice_text(GuildNoticeField::Subject, Some(&data)),
            "Server maintenance"
        );
        assert_eq!(
            notice_text(GuildNoticeField::Body, Some(&data)),
            "We move at 20:00."
        );
        // guildless: empty, not a stale notice
        assert_eq!(notice_text(GuildNoticeField::Subject, None), "");
        assert_eq!(notice_text(GuildNoticeField::Body, None), "");
    }

    /// The §NotifySubBox strip's own three rects sit above the pane anchor and
    /// inside the page.
    #[test]
    fn notify_subbox_strip_fits_above_the_pane_anchor() {
        for (x, y, w, h) in [
            NOTICE_LABEL_RECT,
            NOTICE_SUBJECT_RECT,
            NOTICE_SUBJECT_TEXT_RECT,
            NOTICE_EDIT_BTN,
        ] {
            assert!(x + w <= PAGE_SIZE.0, "rect {x},{y},{w},{h} overflows width");
            assert!(y < NOTICE_PANE_RECT.1 + NOTICE_PANE_RECT.3);
        }
        // the subject text starts right of the label and ends before the button
        assert!(NOTICE_SUBJECT_TEXT_RECT.0 >= NOTICE_LABEL_RECT.0 + NOTICE_LABEL_RECT.2);
        assert!(NOTICE_SUBJECT_TEXT_RECT.0 + NOTICE_SUBJECT_TEXT_RECT.2 <= NOTICE_EDIT_BTN.0);
    }

    /// The five slots are the vanilla 27-pitch column and the whole column
    /// The wire path is attached to the caption the data names "Join", not to a
    /// slot index that a later reorder could silently move.
    #[test]
    fn the_live_command_slot_is_the_join_button() {
        assert_eq!(
            command_keys(false)[COMMAND_SLOT_INVITE].0,
            "UIIT_STT_GUILD_JOIN"
        );
        assert_eq!(
            command_keys(true)[COMMAND_SLOT_INVITE].0,
            "UIIT_STT_GUILD_JOIN"
        );
    }

    /// fits the page (`353 + 88 = 441 <= 451`).
    #[test]
    fn command_column_keeps_the_vanilla_pitch_and_fits() {
        assert_eq!(COMMAND_YS, [142.0, 169.0, 196.0, 223.0, 250.0]);
        for pair in COMMAND_YS.windows(2) {
            assert_eq!(pair[1] - pair[0], 27.0);
        }
        assert_eq!(COMMAND_X + COMMAND_BTN_SIZE.0, 441.0);
        assert!(COMMAND_X + COMMAND_BTN_SIZE.0 <= PAGE_SIZE.0);
        assert!(COMMAND_YS[4] + COMMAND_BTN_SIZE.1 <= PAGE_SIZE.1);
    }

    /// Six buttons, five slots: ids 105 and 106 share `353,250` byte-for-byte,
    /// so exactly one caption is ever drawn in the last slot and the flag is
    /// the only thing that picks it.
    #[test]
    fn slot_five_draws_exactly_one_of_the_two_colliding_buttons() {
        let default = command_keys(false);
        let flagged = command_keys(true);
        assert_eq!(default.len(), COMMAND_YS.len());
        assert_eq!(flagged.len(), COMMAND_YS.len());
        // the first four are identical in both configurations
        assert_eq!(default[..4], flagged[..4]);
        // ...and only the fifth differs
        assert_eq!(default[4], COMMAND_SLOT5_DEFAULT);
        assert_eq!(flagged[4], COMMAND_SLOT5_POSITION_GRANT);
        assert_ne!(default[4], flagged[4]);
        // the position-grant surface is off unless asked for
        assert!(!crate::plugins::config::guild::GuildSettings::default().position_grant);
    }

    /// The captions are the six string keys `ifguild.txt` binds, and nothing
    /// else — no invented sixth slot, no renamed command.
    #[test]
    fn command_captions_are_the_ifguild_string_keys() {
        let keys: Vec<&str> = command_keys(false)
            .iter()
            .chain(std::iter::once(&COMMAND_SLOT5_POSITION_GRANT))
            .map(|(key, _)| *key)
            .collect();
        assert_eq!(
            keys,
            vec![
                "UIIT_STT_GUILD_JOIN",           // id 101, :586
                "UIIT_CTL_AUTHORITY_GRANT",      // id 102, :567
                "UIIT_STT_GUILD_EXPULSION",      // id 103, :548
                "UIIT_STT_GUILD_EXIT",           // id 104, :529
                "UIIT_STT_GUILD_NAME_GRANT",     // id 105, :510
                "UIIT_CTL_GUILD_POSITION_GRANT", // id 106, :491
            ]
        );
    }

    /// Transcription pin: every §GuildInfo rect is the one `ifguild.txt`
    /// declares, so a later refactor cannot quietly nudge the layout.
    #[test]
    fn guild_info_rects_are_the_ifguild_txt_rects() {
        assert_eq!(INFO_WND_POS, (6.0, 4.0)); // :15
        assert_eq!(FRAME_RECT, (6.0, 103.0, 440.0, 211.0)); // :34
        assert_eq!(BG_RECT, (27.0, 105.0, 403.0, 58.0)); // :53
        assert_eq!(MARK_POS, (144.0, 16.0)); // :228
        assert_eq!(NAME_RECT, (166.0, 18.0, 80.0, 14.0)); // :209
        assert_eq!(LEVEL_LABEL_RECT, (381.0, 18.0, 34.0, 15.0)); // :304
        assert_eq!(LEVEL_RECT, (419.0, 18.0, 17.0, 15.0)); // :190
        assert_eq!(LEADER_LABEL_RECT, (28.0, 51.0, 84.0, 14.0)); // :285
        assert_eq!(LEADER_RACE_RECT, (90.0, 49.0, 16.0, 16.0)); // :171
        assert_eq!(LEADER_RECT, (112.0, 51.0, 110.0, 14.0)); // :152
        assert_eq!(MEMBER_LABEL_RECT, (261.0, 51.0, 67.0, 14.0)); // :266
        assert_eq!(MEMBER_NUM_RECT, (339.0, 51.0, 67.0, 14.0)); // :133
        assert_eq!(BAR_BOARD_POS, (120.0, 67.0)); // :342
        assert_eq!(GP_LABEL_RECT, (28.0, 75.0, 89.0, 14.0)); // :247
        assert_eq!(GP_GAUGE_POS, (128.0, 76.0)); // :323
        assert_eq!(GP_PERCENT_RECT, (168.0, 75.0, 64.0, 14.0)); // :114
        assert_eq!(GP_VALUE_RECT, (276.0, 75.0, 129.0, 14.0)); // :95
        assert_eq!(GP_BUTTON_POS, (410.0, 72.0)); // :76
    }

    /// Exactly two art-sized statics run past the 451x320 page, and both are
    /// the ones §3.10 measured — everything else must fit as authored.
    #[test]
    fn only_the_two_documented_guild_statics_overflow_the_page() {
        let (pw, ph) = PAGE_SIZE;
        for (x, y, w, h) in [
            FRAME_RECT,
            BG_RECT,
            NAME_RECT,
            LEVEL_LABEL_RECT,
            LEVEL_RECT,
            LEADER_LABEL_RECT,
            LEADER_RACE_RECT,
            LEADER_RECT,
            MEMBER_LABEL_RECT,
            MEMBER_NUM_RECT,
            GP_LABEL_RECT,
            GP_PERCENT_RECT,
            GP_VALUE_RECT,
            (MARK_POS.0, MARK_POS.1, MARK_SIZE.0, MARK_SIZE.1),
            (
                GP_GAUGE_POS.0,
                GP_GAUGE_POS.1,
                GP_GAUGE_ART.0,
                GP_GAUGE_ART.1,
            ),
            (
                GP_BUTTON_POS.0,
                GP_BUTTON_POS.1,
                GP_BUTTON_SIZE.0,
                GP_BUTTON_SIZE.1,
            ),
        ] {
            assert!(x + w <= pw, "rect {x},{y},{w},{h} overflows the page width");
            assert!(
                y + h <= ph,
                "rect {x},{y},{w},{h} overflows the page height"
            );
        }
        // The two that do overflow (+143 and +45) are clipped, not clamped
        // away and not squashed: the art keeps its native width inside.
        assert_eq!(INFO_WND_POS.0 + INFO_WND_ART.0, 594.0);
        assert_eq!(BAR_BOARD_POS.0 + BAR_BOARD_ART.0, 496.0);
        assert_eq!(clipped_width(INFO_WND_POS.0, INFO_WND_ART.0), 445.0);
        assert_eq!(clipped_width(BAR_BOARD_POS.0, BAR_BOARD_ART.0), 331.0);
        // A static that fits is never shrunk by the clip helper.
        assert_eq!(clipped_width(MARK_POS.0, MARK_SIZE.0), MARK_SIZE.0);
    }

    /// Guildless renders the same layout with a defined empty state — the
    /// page is not hidden and no field is left blank.
    #[test]
    fn guildless_guild_page_shows_the_empty_state() {
        for field in [
            GuildInfoField::Name,
            GuildInfoField::Level,
            GuildInfoField::Leader,
            GuildInfoField::MemberCount,
            GuildInfoField::GpPercent,
            GuildInfoField::GpValue,
        ] {
            assert_eq!(field_text(field, None), EMPTY, "{field:?}");
        }
    }

    /// The header binds `GuildRoster`: name, level, the master's name and the
    /// member count all come from the record, GP is thousands-grouped.
    #[test]
    fn guild_header_binds_the_guild_record() {
        let data = record();
        assert_eq!(field_text(GuildInfoField::Name, Some(&data)), "Roadmen");
        assert_eq!(field_text(GuildInfoField::Level, Some(&data)), "3");
        assert_eq!(field_text(GuildInfoField::Leader, Some(&data)), "Master");
        assert_eq!(field_text(GuildInfoField::MemberCount, Some(&data)), "2");
        assert_eq!(field_text(GuildInfoField::GpValue, Some(&data)), "12,345");
    }

    /// A record with no master flag must not invent one.
    #[test]
    fn guild_leader_falls_back_when_no_member_claims_the_master_flag() {
        let mut data = record();
        for m in data.members.iter_mut() {
            m.is_master = false;
        }
        assert_eq!(field_text(GuildInfoField::Leader, Some(&data)), EMPTY);
    }

    /// No GP threshold exists client-side, so the percent readout stays empty
    /// and the gauge fill stays at zero even for a guild with points — the
    /// alternative is inventing a maximum (module doc).
    #[test]
    fn gp_gauge_stays_empty_until_a_threshold_exists() {
        let data = record();
        assert_eq!(field_text(GuildInfoField::GpPercent, Some(&data)), EMPTY);
        assert_eq!(gauge_fill_width(0.0, GP_GAUGE_ART.0), Val::Px(0.0));
    }

    /// The row-press rule: a row selects itself only while a member occupies
    /// it — "row 5 of a two-member guild" is nobody, and the command column
    /// acts on somebody.
    #[test]
    fn only_an_occupied_roster_row_can_be_selected() {
        assert_eq!(row_selection(1, 2), Some(1));
        assert_eq!(row_selection(4, 2), None, "past the record's end");
        assert_eq!(row_selection(0, 0), None, "an empty record selects nothing");
        assert_eq!(row_selection(VISIBLE_ROWS - 1, VISIBLE_ROWS), Some(5));
    }

    /// The expel chain end to end: a master with the selected non-self row
    /// sends 0x70F4 **by name**, which is the request's whole payload.
    #[test]
    fn expel_sends_the_selected_members_name() {
        let (mut app, sent) = expel_case(Some(0), "Master", GuildPermissions::MASTER);
        assert_eq!(sent, vec![GuildAction::Kick("Grunt".into())]);
        assert!(
            app.world_mut().resource_mut::<ChatHistory>().iter().count() == 0,
            "a permitted expel says nothing locally — the ack 0xB0F4 speaks"
        );
    }

    /// The three client-side stops, each with its own line and **no** packet:
    /// nothing selected, no KICK bit, and yourself.
    #[test]
    fn expel_refuses_without_a_target_a_permission_or_a_stranger() {
        for (selection, name, permissions, why) in [
            (None, "Master", GuildPermissions::MASTER, "no selection"),
            (Some(0), "Grunt", 0u32, "no KICK bit"),
            (Some(0), "Grunt", GuildPermissions::MASTER, "self"),
        ] {
            let (mut app, sent) = expel_case(selection, name, permissions);
            assert!(sent.is_empty(), "{why} still sent a packet");
            assert_eq!(
                app.world_mut().resource_mut::<ChatHistory>().iter().count(),
                1,
                "{why} produced no line"
            );
        }
    }

    /// One expel run: build the app, click Withdraw, collect what went out.
    fn expel_case(
        selection: Option<usize>,
        own_name: &str,
        own_permissions: u32,
    ) -> (App, Vec<GuildAction>) {
        let mut app = App::new();
        app.add_message::<GuildAction>()
            .init_resource::<ChatHistory>()
            .init_resource::<ClientUiStrings>()
            .insert_resource(GuildRosterSelection(selection));
        let mut data = record();
        // Row 0 is "Grunt", row 1 is "Master": give the acting character the
        // permissions under test on whichever row carries their name.
        for member in data.members.iter_mut() {
            if member.name == own_name {
                member.permissions = own_permissions;
            }
        }
        app.insert_resource(GuildRoster { data: Some(data) });
        app.world_mut().spawn((
            Player,
            CharacterInfo {
                name: Some(own_name.to_string()),
                ..default()
            },
        ));
        let button = app.world_mut().spawn_empty().observe(on_guild_expel).id();
        // No `app.update()` between the trigger and the read: the observer
        // runs at trigger time, and an update would rotate the message buffer
        // out from under `iter_current_update_messages` (which is how this
        // test first "passed" while sending nothing).
        app.world_mut().trigger(Activate { entity: button });
        let sent: Vec<GuildAction> = app
            .world()
            .resource::<Messages<GuildAction>>()
            .iter_current_update_messages()
            .cloned()
            .collect();
        (app, sent)
    }

    /// Slot 3 is the expel slot, and its caption is the data's own — a shifted
    /// index would attach the observer to "Grant authority".
    #[test]
    fn the_expel_observer_sits_on_the_withdraw_slot() {
        assert_eq!(
            command_keys(false)[COMMAND_SLOT_EXPEL].0,
            "UIIT_STT_GUILD_EXPULSION"
        );
        assert_eq!(
            command_keys(true)[COMMAND_SLOT_EXPEL].0,
            "UIIT_STT_GUILD_EXPULSION",
            "the fifth slot's flag must not move the third"
        );
    }

    /// The selection plate covers exactly the row it marks: the manager's full
    /// width, the row's own height, on the row pitch.
    #[test]
    fn the_selection_plate_covers_its_row_exactly() {
        let pieces = bar(
            (
                MEMBER_LIST_RECT.0,
                MEMBER_LIST_RECT.1,
                MEMBER_LIST_RECT.2,
                ROW_HEIGHT,
            ),
            ROW_BAR_CAP,
        );
        assert_eq!(pieces[0].0 .2, ROW_BAR_CAP);
        assert_eq!(pieces[2].0 .2, ROW_BAR_CAP);
        assert_eq!(
            pieces[0].0 .2 + pieces[1].0 .2 + pieces[2].0 .2,
            MEMBER_LIST_RECT.2,
            "the three com_bar01select_ pieces tile the list width"
        );
        assert_eq!(
            pieces[2].0 .0 + pieces[2].0 .2,
            MEMBER_LIST_RECT.0 + MEMBER_LIST_RECT.2
        );
        for piece in pieces {
            assert_eq!(piece.0 .3, ROW_HEIGHT);
        }
    }
}

// ===========================================================================
// The Guild-relations page (`CommunityPage::GuildRelations`, `ifcommunity.txt`
// id 11).
// ===========================================================================

/// Idea: page 11 is a *host for two mutually exclusive sub-pages*, not a layout
/// of its own. `resinfo/ifguildrelations.txt` declares exactly three controls —
/// a 22px tile strip (`:44`, `12,12,427,22`) and `CIFAllianceGuild` (`:25`,
/// id 9) and `CIFHostileGuild` (`:6`, id 10) **on one shared rect**
/// `6,29,440,285`. One shared rect with no `Visible` key is the same
/// exclusive-by-construction idiom the six community pages use, so the strip is
/// where the switch belongs.
///
/// The classic generation declares no tab control anywhere (as with the outer
/// community strip, `community/ui.rs`), so the two sub-tabs are transcribed from
/// the 4th-gen twin `res_ui/guild_r.2dt`: `Type=16` ids 87 (`동맹탭`, alliance,
/// `ContentId=86`) and 88 (`적대탭`, hostile, `ContentId=2000`) at
/// `265|343,132,76,28` — 76x28 on a pitch of 78, alliance on the left. Rebased
/// onto the classic pane the same way the 2dt authors them (tab x = pane x + 7,
/// tab y = pane y − 24, i.e. the tab foot tucks 4px under the pane's top edge),
/// which lands them on the declared tile strip.
///
/// Both sub-pages are **data-blocked, not unbuilt**: no wired packet carries a
/// hostile- or alliance-guild list (0x3109 `GuildWarInfo` and 0x3102 are both
/// unwired, `packets/src/lib.rs:506`/`:481`). So each pane is built as far as
/// the data reaches — chrome, captions, list header, buttons — and the two
/// places that would need a record stay empty rather than invented:
///
/// * The detail box shows only its `<Details>` title and the
///   `UIIT_STT_NOT_EXIST_GUILD` line. That is not a shortcut: the "no guild"
///   static (`ifhostileguild.txt:67`, `10,73,215,15`) sits on the **same y** as
///   the `Guild master` caption (`:200`, `31,73,52,15`), so the caption/value
///   block and that line are mutually exclusive in the data itself. With no
///   record to select, the line is the arm that is true.
/// * The list has its two sort headers and its scroll shaft, and no rows.
const RELATIONS_STRIP_RECT: (f32, f32, f32, f32) = (12.0, 12.0, 427.0, 22.0);
const RELATIONS_STRIP_DDJ: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_d.ddj";
/// The rect `CIFAllianceGuild` and `CIFHostileGuild` share.
const RELATIONS_PANE_RECT: (f32, f32, f32, f32) = (6.0, 29.0, 440.0, 285.0);

/// `guild_r.2dt` ids 87/88: 76x28, pitch 78, tab origin = pane origin + (7,−24).
const RELATIONS_TAB_SIZE: (f32, f32) = (76.0, 28.0);
const RELATIONS_TAB_PITCH: f32 = 78.0;
const RELATIONS_TAB_ORIGIN: (f32, f32) =
    (RELATIONS_PANE_RECT.0 + 7.0, RELATIONS_PANE_RECT.1 - 24.0);

/// Which sub-page of the relations page shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RelationsTab {
    /// `CIFAllianceGuild` id 9 → `resinfo/ifallianceguild.txt`.
    #[default]
    Alliance,
    /// `CIFHostileGuild` id 10 → `resinfo/ifhostileguild.txt`.
    Hostility,
}

impl RelationsTab {
    /// Both tabs in the original's left-to-right order (`guild_r.2dt` x
    /// 265 < 343), with the shipped art stem of each.
    pub const ALL: [(RelationsTab, &'static str); 2] = [
        (RelationsTab::Alliance, "gil_union_sub_tab"),
        (RelationsTab::Hostility, "gil_hostility_sub_tab"),
    ];
}

/// Selection state of the relations page's two sub-tabs.
#[derive(Resource, Default)]
pub struct GuildRelationsState(pub RelationsTab);

/// One sub-tab button, carrying what it selects and its art stem.
#[derive(Component)]
pub struct GuildRelationsSubTab(pub RelationsTab, pub &'static str);

/// One of the two sub-panes.
#[derive(Component)]
pub struct GuildRelationsPane(pub RelationsTab);

fn relations_tab_art(stem: &str, on: bool) -> String {
    format!(
        "media://interface/guild/{stem}_{}.ddj",
        if on { "on" } else { "off" }
    )
}

// --- The shared sub-page furniture -----------------------------------------

/// `com_mid_button.ddj` is 88x24 (`COMMAND_BTN_SIZE`). The classic grammar
/// writes buttons `w=h=0` and sizes them from the art, and the data confirms
/// that size twice: the two war buttons are authored at
/// x 131 and 228 (`ifhostileguild.txt:438`/`:419`) and the three alliance
/// buttons at 79/176/273 (`ifallianceguild.txt:689`/`:670`/`:651`) — both a
/// uniform step of **97 = 88 + 9**.
const RELATIONS_BTN_Y: f32 = 253.0;
const RELATIONS_BTN_PITCH: f32 = 97.0;

/// The list block, byte-identical in both sub-pages
/// (`ifhostileguild.txt:395/376/357/338/319/300` = `ifallianceguild.txt`
/// `:627/:608/:589/:570/:551/:532`).
const RELATIONS_LIST_BLACKSQUARE: (f32, f32, f32, f32) = (234.0, 10.0, 198.0, 236.0);
const RELATIONS_LIST_SHAFT: (f32, f32, f32, f32) = (413.0, 35.0, 16.0, 207.0);
const RELATIONS_LIST_CAP: (f32, f32) = (415.0, 13.0);
/// `gil_shape.ddj`, 16x24 — the same cap `SORT_CAP_RIGHT` uses.
const RELATIONS_LIST_CAP_SIZE: (f32, f32) = (16.0, 24.0);
const RELATIONS_LIST_CAP_DDJ: &str = "media://interface/guild/gil_shape.ddj";
/// `CIFScrollManager` (`:338`) — the row pool, transcribed and left empty
/// (no wired packet carries either list; see the page doc).
const RELATIONS_LIST_SCROLL: (f32, f32, f32, f32) = (236.0, 35.0, 194.0, 207.0);
/// The two sort headers: x from the data, widths from the authored x-deltas
/// (236 → 367 → 415), height 24 from the 2dt twin (`guild_r.2dt` ids 56/60).
const RELATIONS_SORT_Y: f32 = 13.0;
const RELATIONS_SORT_H: f32 = 24.0;
const RELATIONS_SORT_BUTTONS: [(f32, f32, &str, &str, &str); 2] = [
    (
        236.0,
        131.0,
        "media://interface/guild/gil_subj_button02.ddj",
        "UIIT_CTL_GUILD_NAME",
        "Name",
    ),
    (
        367.0,
        48.0,
        "media://interface/guild/gil_subj_button08.ddj",
        "UIIT_STT_LEVEL_LV",
        "Lv",
    ),
];

/// The detail box, in the hostile pane's coordinates
/// (`ifhostileguild.txt:276/257/238/67`).
const HOSTILE_DETAIL_BLACKSQUARE: (f32, f32, f32, f32) = (13.0, 10.0, 215.0, 236.0);
const HOSTILE_DETAIL_TILE: (f32, f32, f32, f32) = (17.0, 14.0, 207.0, 228.0);
const HOSTILE_DETAIL_TITLE: (f32, f32, f32, f32) = (55.0, 25.0, 130.0, 15.0);
const HOSTILE_DETAIL_NONE: (f32, f32, f32, f32) = (10.0, 73.0, 215.0, 15.0);
/// The caption the "no guild" line overlaps (`ifhostileguild.txt:200`) — kept
/// as the proof of exclusivity, not spawned.
const HOSTILE_DETAIL_LEADER_CAPTION_Y: f32 = 73.0;

/// The two war buttons (`ifhostileguild.txt:438`/`:419`).
///
/// **Visible and inactive, deliberately.** A send path exists on the wire
/// (`packets/src/lib.rs:513` `0x7110 GuildWarStartRequest`, `:514` `0x7112
/// GuildWarEndRequest`), but `packets/src/agent/guild_war.rs:71` marks four of
/// the start request's five fields unknown, and nothing wired carries the
/// hostile list, so there is no addressable target either. Declaring a war with
/// four invented scalars is the most expensive mistake this window can make, so
/// no `Button` and no observer goes on these two until the fields are named.
///
/// **This greying is our deviation, and the original does otherwise**: with no
/// guild selected the original draws both buttons in their *normal* state, and
/// clicking one produces nothing at all — no error box, no chat line, no
/// change to `<Details>`. So "visible and inactive" is not what v1.188 does; it
/// is an ADR-0009 deviation chosen on purpose, because a button that greys out
/// tells the player why nothing happens, whereas one that silently swallows the
/// click teaches them the window is broken. Recorded here so that nobody later
/// reads this state as the original's — it is a decision, and the original
/// does the opposite.
const HOSTILE_COMMANDS: [(&str, &str); 2] = [
    ("UIIT_CTL_GUILDWAR_DECLAREWAR", "Request to start the war"),
    ("UIIT_CTL_GUILDWAR_WARCONCLUSION", "Request to end the war"),
];

/// The alliance pane's own blocks (`ifallianceguild.txt`).
const ALLIANCE_INFO_FRAME: (f32, f32, f32, f32) = (10.0, 10.0, 215.0, 120.0);
/// `frameg_wnd_`'s pieces measure 24x16 at the corners (`cos/info.rs:55`).
const ALLIANCE_INFO_FRAME_SIDE: f32 = 24.0;
const ALLIANCE_INFO_FRAME_TOP: f32 = 16.0;
const ALLIANCE_INFO_TITLE: (f32, f32, f32, f32) = (50.0, 29.0, 132.0, 15.0);
const ALLIANCE_INFO_NONE: (f32, f32, f32, f32) = (10.0, 72.0, 215.0, 15.0);
const ALLIANCE_DETAIL_BLACKSQUARE: (f32, f32, f32, f32) = (10.0, 139.0, 215.0, 107.0);
const ALLIANCE_DETAIL_TILE: (f32, f32, f32, f32) = (14.0, 142.0, 207.0, 100.0);
const ALLIANCE_DETAIL_TITLE: (f32, f32, f32, f32) = (52.0, 150.0, 131.0, 15.0);
const ALLIANCE_DETAIL_NONE: (f32, f32, f32, f32) = (10.0, 195.0, 215.0, 15.0);

/// The three alliance commands (`ifallianceguild.txt:689`/`:670`/`:651`).
///
/// Inactive for the same reason as the war pair, one step milder: the three
/// union opcodes *are* wired (`0x70FB`/`0x70FC`/`0x70FD`), but each addresses a
/// guild that only the unwired union roster (0x3102) could name, so there is
/// nothing to send them about yet.
/// Slot index of Secede — the one alliance command that can be sent today.
///
/// 0x70FC has **no body at all**, and that is read rather than assumed: its
/// builder writes no field, and a server accepts the empty frame and answers
/// it. So there is nothing to invent here — unlike Apply (0x70FB, which
/// needs the invited master's *spawned entity id*, i.e. a world selection this
/// list cannot give) and Expel (0x70FD, whose single `u32` the builder shows the
/// width of and not the meaning).
const ALLIANCE_COMMAND_SECEDE: usize = 1;

const ALLIANCE_COMMANDS: [(&str, &str); 3] = [
    ("UIIT_STT_GUILD_RESPECT_ALLY_JOIN", "Apply"),
    ("UIIT_STT_GUILD_RESPECT_ALLY_EXIT", "Secede"),
    ("UIIT_STT_GUILD_RESPECT_ALLY_EXPULSION", "Expel"),
];

/// Both panes' `§Create` block: `frameg01_wnd_` ring on `0,0,440,285` plus the
/// two `com_bg_tile_b` fills (`ifhostileguild.txt:44/:25/:6`, byte-identical in
/// `ifallianceguild.txt:44/:25/:6`).
const RELATIONS_PANE_FRAME: (f32, f32, f32, f32) = (0.0, 0.0, 440.0, 285.0);
const RELATIONS_PANE_BG_01: (f32, f32, f32, f32) = (16.0, 16.0, 218.0, 230.0);
const RELATIONS_PANE_BG_02: (f32, f32, f32, f32) = (16.0, 246.0, 408.0, 23.0);

/// Fill the Guild-relations page container: the tile strip, the two sub-tabs
/// and the two panes that share `6,29,440,285`.
pub fn spawn_guild_relations_page(
    page: &mut ChildSpawnerCommands,
    asset_server: &AssetServer,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
    state: &GuildRelationsState,
    s: f32,
) {
    let text_font = |size: f32| TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(size * s),
        ..default()
    };
    let image = |rect: (f32, f32, f32, f32), path: String| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        )
    };
    let tile = |rect: (f32, f32, f32, f32), path: &str| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path.to_string()),
                image_mode: NodeImageMode::Tiled {
                    tile_x: true,
                    tile_y: true,
                    stretch_value: s,
                },
                ..default()
            },
            Pickable::IGNORE,
        )
    };

    // GDR_GUILD_RELATIONS_BG — the strip the sub-tabs sit on.
    page.spawn(tile(RELATIONS_STRIP_RECT, RELATIONS_STRIP_DDJ));

    for (index, (tab, stem)) in RelationsTab::ALL.into_iter().enumerate() {
        page.spawn((
            GuildRelationsSubTab(tab, stem),
            Button,
            Hovered::default(),
            abs_node(
                (
                    RELATIONS_TAB_ORIGIN.0 + index as f32 * RELATIONS_TAB_PITCH,
                    RELATIONS_TAB_ORIGIN.1,
                    RELATIONS_TAB_SIZE.0,
                    RELATIONS_TAB_SIZE.1,
                ),
                s,
            ),
            ImageNode {
                image: asset_server.load(relations_tab_art(stem, tab == state.0)),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
        ))
        .observe(on_relations_sub_tab);
    }

    for (tab, _) in RelationsTab::ALL {
        let mut node = abs_node(RELATIONS_PANE_RECT, s);
        node.display = if tab == state.0 {
            Display::Flex
        } else {
            Display::None
        };
        page.spawn((GuildRelationsPane(tab), node, Pickable::IGNORE))
            .with_children(|pane| {
                // §Create, identical in both files.
                let (_, _, fw, fh) = RELATIONS_PANE_FRAME;
                for ((x, y, w, h), piece) in ring(fw, fh, FRAME_PIECE) {
                    pane.spawn(image((x, y, w, h), format!("{FRAME_DIR}{piece}.ddj")));
                }
                pane.spawn(tile(RELATIONS_PANE_BG_01, BG_TILE_DDJ));
                pane.spawn(tile(RELATIONS_PANE_BG_02, BG_TILE_DDJ));

                // The list block, byte-identical in both files.
                for ((x, y, w, h), piece) in
                    blacksquare_ring(RELATIONS_LIST_BLACKSQUARE.2, RELATIONS_LIST_BLACKSQUARE.3)
                {
                    pane.spawn(image(
                        (
                            RELATIONS_LIST_BLACKSQUARE.0 + x,
                            RELATIONS_LIST_BLACKSQUARE.1 + y,
                            w,
                            h,
                        ),
                        format!("{BLACKSQUARE_DIR}{piece}.ddj"),
                    ));
                }
                pane.spawn(tile(RELATIONS_LIST_SHAFT, NOTICE_BG02_DDJ));
                pane.spawn(image(
                    (
                        RELATIONS_LIST_CAP.0,
                        RELATIONS_LIST_CAP.1,
                        RELATIONS_LIST_CAP_SIZE.0,
                        RELATIONS_LIST_CAP_SIZE.1,
                    ),
                    RELATIONS_LIST_CAP_DDJ.to_string(),
                ));
                for (x, w, art, key, fallback) in RELATIONS_SORT_BUTTONS {
                    pane.spawn(image(
                        (x, RELATIONS_SORT_Y, w, RELATIONS_SORT_H),
                        art.to_string(),
                    ))
                    .with_children(|button| {
                        button.spawn(label_static(
                            (0.0, (RELATIONS_SORT_H - 14.0) / 2.0, w, 14.0),
                            ui_strings.get_or(key, fallback).to_string(),
                            VALUE_COLOR,
                            Justify::Center,
                            &text_font(7.5),
                            s,
                        ));
                    });
                }

                match tab {
                    RelationsTab::Hostility => {
                        for ((x, y, w, h), piece) in blacksquare_ring(
                            HOSTILE_DETAIL_BLACKSQUARE.2,
                            HOSTILE_DETAIL_BLACKSQUARE.3,
                        ) {
                            pane.spawn(image(
                                (
                                    HOSTILE_DETAIL_BLACKSQUARE.0 + x,
                                    HOSTILE_DETAIL_BLACKSQUARE.1 + y,
                                    w,
                                    h,
                                ),
                                format!("{BLACKSQUARE_DIR}{piece}.ddj"),
                            ));
                        }
                        pane.spawn(tile(HOSTILE_DETAIL_TILE, NOTICE_BG02_DDJ));
                        pane.spawn(label_static(
                            HOSTILE_DETAIL_TITLE,
                            ui_strings
                                .get_or("UIIT_CTL_GUILD_INFORMATION", "<Details>")
                                .to_string(),
                            LABEL_COLOR,
                            Justify::Center,
                            &text_font(7.5),
                            s,
                        ));
                        pane.spawn(label_static(
                            HOSTILE_DETAIL_NONE,
                            ui_strings
                                .get_or("UIIT_STT_NOT_EXIST_GUILD", "No guild is selected.")
                                .to_string(),
                            VALUE_COLOR,
                            Justify::Center,
                            &text_font(7.5),
                            s,
                        ));
                        spawn_relations_commands(
                            pane,
                            &HOSTILE_COMMANDS,
                            131.0,
                            // Neither war button may fire: four of 0x7110's five
                            // fields are unknown and 0x3109 carries no target.
                            None,
                            asset_server,
                            ui_strings,
                            &text_font(7.5),
                            s,
                        );
                    }
                    RelationsTab::Alliance => {
                        let (afx, afy, afw, afh) = ALLIANCE_INFO_FRAME;
                        for ((x, y, w, h), piece) in
                            frameg_ring(afw, afh, ALLIANCE_INFO_FRAME_SIDE, ALLIANCE_INFO_FRAME_TOP)
                        {
                            pane.spawn(image(
                                (afx + x, afy + y, w, h),
                                format!("media://interface/frame/frameg_wnd_{piece}.ddj"),
                            ));
                        }
                        pane.spawn(label_static(
                            ALLIANCE_INFO_TITLE,
                            ui_strings
                                .get_or(
                                    "UIIT_STT_GUILD_RESPECT_ALLY_INFO",
                                    "<Union guild information>",
                                )
                                .to_string(),
                            LABEL_COLOR,
                            Justify::Center,
                            &text_font(7.5),
                            s,
                        ));
                        pane.spawn((
                            AllianceEmptyLine,
                            label_static(
                                ALLIANCE_INFO_NONE,
                                ui_strings
                                    .get_or(
                                        "UIIT_STT_NOT_EXIST_GUILD_RESPECT_ALLY",
                                        "Not in the alliance",
                                    )
                                    .to_string(),
                                VALUE_COLOR,
                                Justify::Center,
                                &text_font(7.5),
                                s,
                            ),
                        ));
                        // The three header values 0x3102 fills, hidden until it
                        // arrives (`update_alliance_pane`).
                        for field in [
                            AllianceHeaderField::LeaderGuild,
                            AllianceHeaderField::LeaderMaster,
                            AllianceHeaderField::Count,
                        ] {
                            pane.spawn((
                                field,
                                Text::new(String::new()),
                                text_font(7.5),
                                TextColor(VALUE_COLOR),
                                TextLayout::justify(Justify::Center),
                                {
                                    let mut node = abs_node(alliance_header_rect(field), s);
                                    node.display = Display::None;
                                    node
                                },
                                Pickable::IGNORE,
                            ));
                        }
                        // The list rows: eight, the alliance ceiling from the
                        // original's own header format, on the family's pitch.
                        for row in 0..ALLIANCE_ROWS {
                            for column in [AllianceColumn::Name, AllianceColumn::Level] {
                                pane.spawn((
                                    AllianceRowCell { row, column },
                                    Text::new(String::new()),
                                    text_font(7.5),
                                    TextColor(VALUE_COLOR),
                                    TextLayout::justify(Justify::Left),
                                    {
                                        let mut node = abs_node(alliance_cell_rect(row, column), s);
                                        node.display = Display::None;
                                        node
                                    },
                                    Pickable::IGNORE,
                                ));
                            }
                        }
                        for ((x, y, w, h), piece) in blacksquare_ring(
                            ALLIANCE_DETAIL_BLACKSQUARE.2,
                            ALLIANCE_DETAIL_BLACKSQUARE.3,
                        ) {
                            pane.spawn(image(
                                (
                                    ALLIANCE_DETAIL_BLACKSQUARE.0 + x,
                                    ALLIANCE_DETAIL_BLACKSQUARE.1 + y,
                                    w,
                                    h,
                                ),
                                format!("{BLACKSQUARE_DIR}{piece}.ddj"),
                            ));
                        }
                        pane.spawn(tile(ALLIANCE_DETAIL_TILE, NOTICE_BG02_DDJ));
                        pane.spawn(label_static(
                            ALLIANCE_DETAIL_TITLE,
                            ui_strings
                                .get_or("UIIT_CTL_GUILD_INFORMATION", "<Details>")
                                .to_string(),
                            LABEL_COLOR,
                            Justify::Center,
                            &text_font(7.5),
                            s,
                        ));
                        pane.spawn(label_static(
                            ALLIANCE_DETAIL_NONE,
                            ui_strings
                                .get_or("UIIT_STT_NOT_EXIST_GUILD", "No guild is selected.")
                                .to_string(),
                            VALUE_COLOR,
                            Justify::Center,
                            &text_font(7.5),
                            s,
                        ));
                        spawn_relations_commands(
                            pane,
                            &ALLIANCE_COMMANDS,
                            79.0,
                            Some(ALLIANCE_COMMAND_SECEDE),
                            asset_server,
                            ui_strings,
                            &text_font(7.5),
                            s,
                        );
                    }
                }
            });
    }
}

// --- The alliance pane's live content (0x3102) -------------------------------
//
// Idea: `ifallianceguildslot.txt` is the one slot file of this window family
// whose three cells are authored `0,0,0,0` — the manager sizes them at runtime,
// so the row geometry cannot come from the slot. It comes from the list's own
// two sort headers instead (`RELATIONS_SORT_BUTTONS`: Name at 236 w 131, Lv at
// 367 w 48), which is the same derivation the header strip itself is built on.

/// The header line's three runtime values. Their **rects are ours**: the
/// original draws them from code onto the labels `+0x78C` / `+0x794` / `+0x798`
/// and `ifallianceguild.txt` authors no rect for any of them — only the title and
/// the "not in the alliance" line. So they are stacked inside the info frame on
/// the authored none-line's x/width, one row above and one below it.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllianceHeaderField {
    /// The leading guild's name (`Name1` of the entry `leader_guild_id` keys).
    LeaderGuild,
    /// That guild's master (`master_name`).
    LeaderMaster,
    /// `"%d/%d"` — members and the ceiling of 8 the original's own header
    /// prints.
    Count,
}

/// The "not in the alliance" line, which the data replaces when a push arrives.
#[derive(Component)]
pub struct AllianceEmptyLine;

/// One cell of an alliance list row.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct AllianceRowCell {
    pub row: usize,
    pub column: AllianceColumn,
}

/// The two columns the list's sort headers declare.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllianceColumn {
    /// `GDR_ALLIANCE_GUILD_NAME`, under the `UIIT_CTL_GUILD_NAME` header.
    Name,
    /// `GDR_ALLIANCE_GUILD_LEVEL`, under the `UIIT_STT_LEVEL_LV` header.
    ///
    /// **Drawn empty, and this is the interesting kind of hole**: the slot
    /// declares a level cell, and the wire has exactly one unnamed `u8` per
    /// entry in the plausible place ([`UnionGuild::unknown_a`], record `+0x20`)
    /// — but "plausible place" is the guess ADR-0009 bans, and the other
    /// candidate (member count) would print an equally convincing wrong number.
    /// What fills this column is named at `unknown_a`: the row renderer off the
    /// vector `wnd+0x7FC`, or a 0x3102 from a session with a real alliance.
    Level,
}

/// The rows the list area can hold: `207 / 23 = 9` on the same pitch the whole
/// window family uses. The alliance itself tops out at
/// [`UnionRoster::MAX_GUILDS`] = 8, so the ninth row can never fill — it is not
/// spawned, and that difference is the point: the ceiling is a code constant,
/// not a geometry accident.
const ALLIANCE_ROWS: usize = UnionRoster::MAX_GUILDS as usize;

/// A row's y inside the pane: the scroll manager's top plus the pitch.
fn alliance_row_top(row: usize) -> f32 {
    RELATIONS_LIST_SCROLL.1 + row as f32 * ROW_PITCH
}

/// A row cell's rect, taken from the column's sort header.
fn alliance_cell_rect(row: usize, column: AllianceColumn) -> (f32, f32, f32, f32) {
    let (x, w, ..) = match column {
        AllianceColumn::Name => RELATIONS_SORT_BUTTONS[0],
        AllianceColumn::Level => RELATIONS_SORT_BUTTONS[1],
    };
    (x, alliance_row_top(row) + 5.0, w, 14.0)
}

/// The header value rows, stacked around the authored none-line.
fn alliance_header_rect(field: AllianceHeaderField) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = ALLIANCE_INFO_NONE;
    match field {
        AllianceHeaderField::LeaderGuild => (x, y - 17.0, w, h),
        AllianceHeaderField::LeaderMaster => (x, y, w, h),
        AllianceHeaderField::Count => (x, y + 17.0, w, h),
    }
}

/// Push [`GuildUnion`] onto the alliance pane: the header line, the list rows
/// and the "not in the alliance" line they replace.
pub fn update_alliance_pane(
    union: Res<GuildUnion>,
    mut headers: Query<(&AllianceHeaderField, &mut Text, &mut Node), Without<AllianceRowCell>>,
    mut rows: Query<(&AllianceRowCell, &mut Text, &mut Node), Without<AllianceHeaderField>>,
    mut empty: Query<
        &mut Node,
        (
            With<AllianceEmptyLine>,
            Without<AllianceHeaderField>,
            Without<AllianceRowCell>,
        ),
    >,
) {
    if !union.is_changed() {
        return;
    }
    let leader = union.leader().cloned();
    let guilds = union
        .data
        .as_ref()
        .map(|d| d.guilds.clone())
        .unwrap_or_default();
    let in_alliance = union.data.is_some();

    for mut node in empty.iter_mut() {
        node.display = if in_alliance {
            Display::None
        } else {
            Display::Flex
        };
    }
    for (field, mut text, mut node) in headers.iter_mut() {
        // A push whose `leader_guild_id` names no entry leaves the header
        // blank rather than guessing at entry 0: the id is a map key, and a key
        // that misses is a server/parse disagreement we want to see, not to
        // paper over.
        let next = match (field, leader.as_ref()) {
            (AllianceHeaderField::LeaderGuild, Some(g)) => g.guild_name.clone(),
            (AllianceHeaderField::LeaderMaster, Some(g)) => g.master_name.clone(),
            (AllianceHeaderField::Count, _) if in_alliance => {
                format!("{}/{}", guilds.len(), UnionRoster::MAX_GUILDS)
            }
            _ => String::new(),
        };
        node.display = if next.is_empty() {
            Display::None
        } else {
            Display::Flex
        };
        if text.0 != next {
            text.0 = next;
        }
    }
    for (cell, mut text, mut node) in rows.iter_mut() {
        let next = match (guilds.get(cell.row), cell.column) {
            (Some(guild), AllianceColumn::Name) => guild.guild_name.clone(),
            // See `AllianceColumn::Level`: the cell exists, its source does not.
            (Some(_), AllianceColumn::Level) => String::new(),
            (None, _) => String::new(),
        };
        node.display = if cell.row < guilds.len() {
            Display::Flex
        } else {
            Display::None
        };
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// Secede from the alliance — 0x70FC, the empty-bodied request.
///
/// Gated on actually being in one: without a 0x3102 push there is no alliance
/// to leave, and the refusal would come back as `0x4C0D` ("no guild") or its
/// alliance sibling, which is a round trip for a state the client already
/// knows. Who *may* secede is the server's call (permission bit unproven for
/// this op), so nothing else is pre-checked — `net::guild` already turns
/// 0xB0FC's refusal into a chat line with the original's own text.
fn on_union_secede(
    _: On<Activate>,
    union: Res<GuildUnion>,
    mut history: ResMut<ChatHistory>,
    mut actions: MessageWriter<GuildAction>,
) {
    if union.data.is_none() {
        history.push(ChatLine::system(SECEDE_NO_ALLIANCE));
        return;
    }
    actions.write(GuildAction::UnionLeave);
}

/// Ours: the original's `UIIT_MSG_GUILDERR_*` strings are server refusals, and
/// this stop happens before any packet exists.
const SECEDE_NO_ALLIANCE: &str = "Your guild is not in an alliance.";

/// The `frameg_wnd_` 9-slice: rectangular 24x16 corners, unlike
/// `frameg01_wnd_`'s square ones (`cos/info.rs:55`, the art's own extent).
fn frameg_ring(w: f32, h: f32, side: f32, top: f32) -> [((f32, f32, f32, f32), &'static str); 8] {
    [
        ((0.0, 0.0, side, top), "left_up"),
        ((side, 0.0, w - 2.0 * side, top), "mid_up"),
        ((w - side, 0.0, side, top), "right_up"),
        ((0.0, top, side, h - 2.0 * top), "left_side"),
        ((w - side, top, side, h - 2.0 * top), "right_side"),
        ((0.0, h - top, side, top), "left_down"),
        ((side, h - top, w - 2.0 * side, top), "mid_down"),
        ((w - side, h - top, side, top), "right_down"),
    ]
}

/// Spawn a sub-page's command row: `com_mid_button.ddj` at the authored pitch,
/// captioned, **without** `Button` — see [`HOSTILE_COMMANDS`] /
/// [`ALLIANCE_COMMANDS`] for why none of them may fire yet.
fn spawn_relations_commands(
    pane: &mut ChildSpawnerCommands,
    commands: &[(&str, &str)],
    first_x: f32,
    // Index of the one command in this row that may actually send, if any. The
    // hostile pane passes `None`; the alliance pane passes its Secede slot.
    live: Option<usize>,
    asset_server: &AssetServer,
    ui_strings: &ClientUiStrings,
    font: &TextFont,
    s: f32,
) {
    for (index, (key, fallback)) in commands.iter().enumerate() {
        let x = first_x + index as f32 * RELATIONS_BTN_PITCH;
        let mut button = pane.spawn((
            abs_node(
                (x, RELATIONS_BTN_Y, COMMAND_BTN_SIZE.0, COMMAND_BTN_SIZE.1),
                s,
            ),
            ImageNode {
                image: asset_server.load(COMMAND_BTN_DDJ.to_string()),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));
        if live == Some(index) {
            // `insert`, not a second `Pickable` in the tuple — that spelling
            // panics the client when the command buffer is applied.
            button.insert((Button, Hovered::default(), Pickable::default()));
            button.observe(on_union_secede);
        }
        button.with_children(|button| {
            button.spawn(label_static(
                (
                    0.0,
                    (COMMAND_BTN_SIZE.1 - 14.0) / 2.0,
                    COMMAND_BTN_SIZE.0,
                    14.0,
                ),
                ui_strings.get_or(key, fallback).to_string(),
                COMMAND_TEXT_COLOR,
                Justify::Center,
                font,
                s,
            ));
        });
    }
}

/// A sub-tab press selects its pane and nothing else.
fn on_relations_sub_tab(
    activate: On<Activate>,
    tabs: Query<&GuildRelationsSubTab>,
    mut state: ResMut<GuildRelationsState>,
) {
    if let Ok(tab) = tabs.get(activate.entity) {
        state.0 = tab.0;
    }
}

/// Mirror [`GuildRelationsState`] onto the two panes and the two tab images.
pub fn apply_guild_relations_tab(
    state: Res<GuildRelationsState>,
    asset_server: Res<AssetServer>,
    mut panes: Query<(&GuildRelationsPane, &mut Node)>,
    mut tabs: Query<(&GuildRelationsSubTab, &mut ImageNode)>,
) {
    if !state.is_changed() {
        return;
    }
    for (pane, mut node) in panes.iter_mut() {
        node.display = if pane.0 == state.0 {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (tab, mut image) in tabs.iter_mut() {
        image.image = asset_server.load(relations_tab_art(tab.1, tab.0 == state.0));
    }
}

#[cfg(test)]
mod relations_test {
    use super::*;

    /// The page's own three controls, transcribed: one strip and two sub-pages
    /// that share a rect (`ifguildrelations.txt:44/:25/:6`).
    #[test]
    fn the_two_sub_pages_share_one_rect_inside_the_page() {
        assert_eq!(RELATIONS_STRIP_RECT, (12.0, 12.0, 427.0, 22.0));
        assert_eq!(RELATIONS_PANE_RECT, (6.0, 29.0, 440.0, 285.0));
        assert!(RELATIONS_PANE_RECT.0 + RELATIONS_PANE_RECT.2 <= PAGE_SIZE.0);
        assert!(RELATIONS_PANE_RECT.1 + RELATIONS_PANE_RECT.3 <= PAGE_SIZE.1);
        // the pane's own §Create frame is exactly the container it fills
        assert_eq!(RELATIONS_PANE_FRAME.2, RELATIONS_PANE_RECT.2);
        assert_eq!(RELATIONS_PANE_FRAME.3, RELATIONS_PANE_RECT.3);
    }

    /// The sub-tabs are `guild_r.2dt`'s (76x28, pitch 78, alliance left), placed
    /// on the pane the way the 2dt places them on its own: +7 across, −24 up.
    #[test]
    fn the_sub_tabs_are_the_twins_geometry_on_the_declared_strip() {
        assert_eq!(RelationsTab::ALL[0].0, RelationsTab::Alliance);
        assert_eq!(RelationsTab::ALL[0].1, "gil_union_sub_tab");
        assert_eq!(RelationsTab::ALL[1].1, "gil_hostility_sub_tab");
        // 2dt: 343 − 265 = 78, and 265 − 258 = 7, 132 − 156 = −24
        assert_eq!(RELATIONS_TAB_PITCH, 343.0 - 265.0);
        assert_eq!(RELATIONS_TAB_ORIGIN.0, RELATIONS_PANE_RECT.0 + 7.0);
        assert_eq!(RELATIONS_TAB_ORIGIN.1, RELATIONS_PANE_RECT.1 - 24.0);
        // both tabs land on the declared tile strip and inside the page
        let last = RELATIONS_TAB_ORIGIN.0 + RELATIONS_TAB_PITCH + RELATIONS_TAB_SIZE.0;
        assert!(last <= RELATIONS_STRIP_RECT.0 + RELATIONS_STRIP_RECT.2);
        assert!(RELATIONS_TAB_ORIGIN.0 >= RELATIONS_STRIP_RECT.0);
        assert_eq!(
            relations_tab_art("gil_hostility_sub_tab", true),
            "media://interface/guild/gil_hostility_sub_tab_on.ddj"
        );
    }

    /// The `w=h=0` command buttons: `com_mid_button.ddj` is 88x24, and both
    /// sub-pages author a step of 97 = 88 + 9. Two files, one number.
    #[test]
    fn the_command_pitch_confirms_the_button_width() {
        assert_eq!(RELATIONS_BTN_PITCH, COMMAND_BTN_SIZE.0 + 9.0);
        // the authored x values: hostile 131/228, alliance 79/176/273
        assert_eq!(131.0 + RELATIONS_BTN_PITCH, 228.0);
        assert_eq!(79.0 + RELATIONS_BTN_PITCH, 176.0);
        assert_eq!(79.0 + 2.0 * RELATIONS_BTN_PITCH, 273.0);
        // and the row fits its own tile plate (`16,246,408,23`)
        let right = 273.0 + COMMAND_BTN_SIZE.0;
        assert!(right <= RELATIONS_PANE_BG_02.0 + RELATIONS_PANE_BG_02.2);
        assert!(RELATIONS_BTN_Y + COMMAND_BTN_SIZE.1 <= RELATIONS_PANE_FRAME.3);
    }

    /// The war buttons carry the two keys the recording shows verbatim, and
    /// nothing here may send: the recorded captions and `textuisystem.txt`
    /// agree word for word.
    #[test]
    fn the_war_buttons_are_the_recorded_captions() {
        assert_eq!(
            HOSTILE_COMMANDS,
            [
                ("UIIT_CTL_GUILDWAR_DECLAREWAR", "Request to start the war"),
                ("UIIT_CTL_GUILDWAR_WARCONCLUSION", "Request to end the war"),
            ]
        );
        assert_eq!(ALLIANCE_COMMANDS.len(), 3);
    }

    /// The "no guild is selected" line and the `Guild master` caption share a
    /// y, so the data itself makes them exclusive — that is why the detail box
    /// shows the line and no caption block.
    #[test]
    fn the_no_guild_line_overlaps_the_caption_block() {
        assert_eq!(HOSTILE_DETAIL_NONE.1, HOSTILE_DETAIL_LEADER_CAPTION_Y);
        // and it sits inside its own tile
        assert!(HOSTILE_DETAIL_NONE.0 >= HOSTILE_DETAIL_BLACKSQUARE.0 - 3.0);
        assert!(
            HOSTILE_DETAIL_TILE.0 + HOSTILE_DETAIL_TILE.2
                <= HOSTILE_DETAIL_BLACKSQUARE.0 + HOSTILE_DETAIL_BLACKSQUARE.2
        );
    }

    /// The list block is byte-identical in both sub-pages, and its scroll
    /// manager sits inside its own black square.
    #[test]
    fn the_list_block_fits_its_frame() {
        assert!(RELATIONS_LIST_SCROLL.0 >= RELATIONS_LIST_BLACKSQUARE.0);
        assert!(
            RELATIONS_LIST_SCROLL.1 + RELATIONS_LIST_SCROLL.3
                <= RELATIONS_LIST_BLACKSQUARE.1 + RELATIONS_LIST_BLACKSQUARE.3
        );
        // the two sort headers span the manager: 236..367..415
        assert_eq!(
            RELATIONS_SORT_BUTTONS[0].0 + RELATIONS_SORT_BUTTONS[0].1,
            RELATIONS_SORT_BUTTONS[1].0
        );
        assert_eq!(
            RELATIONS_SORT_BUTTONS[1].0 + RELATIONS_SORT_BUTTONS[1].1,
            RELATIONS_LIST_CAP.0
        );
    }

    /// Secede sends the empty-bodied 0x70FC, and only while an alliance exists.
    /// The red case is the point: without a push there is a chat line and **no**
    /// packet, so the button cannot fabricate a secession from nothing.
    #[test]
    fn secede_sends_only_while_an_alliance_exists() {
        use packets::agent::guild_union::{UnionGuild, UnionRoster};

        for (alliance, expect_packet) in [(false, false), (true, true)] {
            let mut app = App::new();
            app.add_message::<GuildAction>()
                .init_resource::<ChatHistory>()
                .init_resource::<GuildUnion>();
            if alliance {
                app.world_mut().resource_mut::<GuildUnion>().data = Some(UnionRoster {
                    union_id: 1,
                    union_crest_rev: 0,
                    leader_guild_id: 1,
                    count: 1,
                    guilds: vec![UnionGuild {
                        guild_id: 1,
                        guild_name: "OpenRoad".into(),
                        unknown_a: 0,
                        master_name: "Mira".into(),
                        master_object_id: 0,
                        unknown_b: 0,
                    }],
                });
            }
            let button = app.world_mut().spawn_empty().observe(on_union_secede).id();
            // No `app.update()` before the read — it would rotate the buffer.
            app.world_mut().trigger(Activate { entity: button });
            let sent: Vec<GuildAction> = app
                .world()
                .resource::<Messages<GuildAction>>()
                .iter_current_update_messages()
                .cloned()
                .collect();
            if expect_packet {
                assert_eq!(sent, vec![GuildAction::UnionLeave]);
                assert_eq!(
                    app.world_mut().resource_mut::<ChatHistory>().iter().count(),
                    0
                );
            } else {
                assert!(sent.is_empty(), "seceded from no alliance");
                assert_eq!(
                    app.world_mut().resource_mut::<ChatHistory>().iter().count(),
                    1
                );
            }
        }
    }

    /// Only Secede is live in the alliance row, and it is the middle button —
    /// a shifted index would wire Apply (which needs a world selection) or
    /// Expel (whose `u32` has no established meaning).
    #[test]
    fn the_live_alliance_command_is_secede() {
        assert_eq!(
            ALLIANCE_COMMANDS[ALLIANCE_COMMAND_SECEDE].0,
            "UIIT_STT_GUILD_RESPECT_ALLY_EXIT"
        );
    }

    /// The alliance rows come out of the list's own sort headers, since the
    /// slot file authors `0,0,0,0` for all three of its cells: the name cell
    /// sits under the Name header, the level cell under `Lv`, and both stay
    /// inside the scroll manager.
    #[test]
    fn alliance_row_cells_sit_under_their_sort_headers() {
        for row in 0..ALLIANCE_ROWS {
            let name = alliance_cell_rect(row, AllianceColumn::Name);
            let level = alliance_cell_rect(row, AllianceColumn::Level);
            assert_eq!(name.0, RELATIONS_SORT_BUTTONS[0].0);
            assert_eq!(name.2, RELATIONS_SORT_BUTTONS[0].1);
            assert_eq!(level.0, RELATIONS_SORT_BUTTONS[1].0);
            assert_eq!(level.2, RELATIONS_SORT_BUTTONS[1].1);
            for rect in [name, level] {
                assert!(
                    rect.1 >= RELATIONS_LIST_SCROLL.1,
                    "row {row} above the list"
                );
                assert!(
                    rect.1 + rect.3 <= RELATIONS_LIST_SCROLL.1 + RELATIONS_LIST_SCROLL.3,
                    "row {row} below the list"
                );
                assert!(rect.0 + rect.2 <= RELATIONS_LIST_CAP.0);
            }
        }
    }

    /// Eight rows, because eight is the alliance ceiling the original's own
    /// header format states — the manager could show nine, and drawing nine
    /// would be geometry inventing a rule.
    #[test]
    fn the_alliance_draws_the_codes_eight_rows_not_the_geometrys_nine() {
        assert_eq!(ALLIANCE_ROWS, 8);
        let fits = (RELATIONS_LIST_SCROLL.3 / ROW_PITCH).floor() as usize;
        assert_eq!(fits, 9, "the manager's own height holds nine");
        assert!(ALLIANCE_ROWS < fits);
    }

    /// The three header values stay inside the authored info frame and do not
    /// collide with its title.
    #[test]
    fn the_alliance_header_values_stay_in_their_frame() {
        let (fx, fy, fw, fh) = ALLIANCE_INFO_FRAME;
        for field in [
            AllianceHeaderField::LeaderGuild,
            AllianceHeaderField::LeaderMaster,
            AllianceHeaderField::Count,
        ] {
            let (x, y, w, h) = alliance_header_rect(field);
            assert!(x >= fx && x + w <= fx + fw, "{field:?} leaves the frame");
            assert!(
                y > ALLIANCE_INFO_TITLE.1 + ALLIANCE_INFO_TITLE.3,
                "{field:?} overlaps the title"
            );
            assert!(y + h <= fy + fh, "{field:?} below the frame");
        }
        // and the middle one is exactly the authored none-line it replaces
        assert_eq!(
            alliance_header_rect(AllianceHeaderField::LeaderMaster),
            ALLIANCE_INFO_NONE
        );
    }

    /// The whole display chain in one app: a push fills the header from the
    /// **keyed** leader and the rows from the list, and it hides the
    /// "not in the alliance" line. The red case is included on purpose: without
    /// a push, the empty line shows and every value cell is hidden.
    #[test]
    fn the_alliance_pane_shows_the_push_and_hides_the_empty_line() {
        use packets::agent::guild_union::{UnionGuild, UnionRoster};

        let mut app = App::new();
        app.init_resource::<GuildUnion>();
        app.add_systems(Update, update_alliance_pane);
        let empty = app
            .world_mut()
            .spawn((AllianceEmptyLine, Node::default()))
            .id();
        let mut header_ids = Vec::new();
        for field in [
            AllianceHeaderField::LeaderGuild,
            AllianceHeaderField::LeaderMaster,
            AllianceHeaderField::Count,
        ] {
            header_ids.push((
                field,
                app.world_mut()
                    .spawn((field, Text::new(String::new()), Node::default()))
                    .id(),
            ));
        }
        let mut row_ids = Vec::new();
        for row in 0..ALLIANCE_ROWS {
            row_ids.push(
                app.world_mut()
                    .spawn((
                        AllianceRowCell {
                            row,
                            column: AllianceColumn::Name,
                        },
                        Text::new(String::new()),
                        Node::default(),
                    ))
                    .id(),
            );
        }

        // Red case first: no push at all.
        app.update();
        assert_eq!(
            app.world().get::<Node>(empty).unwrap().display,
            Display::Flex
        );
        for (_, id) in &header_ids {
            assert_eq!(app.world().get::<Node>(*id).unwrap().display, Display::None);
        }

        let guild = |id: u32, name: &str, master: &str| UnionGuild {
            guild_id: id,
            guild_name: name.to_string(),
            unknown_a: 0,
            master_name: master.to_string(),
            master_object_id: 0,
            unknown_b: 0,
        };
        app.world_mut().resource_mut::<GuildUnion>().data = Some(UnionRoster {
            union_id: 4,
            union_crest_rev: 0,
            // the leader is the SECOND entry: an index-based reading names
            // "OpenRoad" here and is wrong
            leader_guild_id: 78,
            count: 2,
            guilds: vec![guild(77, "OpenRoad", "Mira"), guild(78, "Roadmen", "Grunt")],
        });
        app.update();

        assert_eq!(
            app.world().get::<Node>(empty).unwrap().display,
            Display::None
        );
        for (field, id) in &header_ids {
            let text = app.world().get::<Text>(*id).unwrap().0.clone();
            let expected = match field {
                AllianceHeaderField::LeaderGuild => "Roadmen",
                AllianceHeaderField::LeaderMaster => "Grunt",
                AllianceHeaderField::Count => "2/8",
            };
            assert_eq!(text, expected, "{field:?}");
        }
        assert_eq!(app.world().get::<Text>(row_ids[0]).unwrap().0, "OpenRoad");
        assert_eq!(app.world().get::<Text>(row_ids[1]).unwrap().0, "Roadmen");
        assert_eq!(
            app.world().get::<Node>(row_ids[2]).unwrap().display,
            Display::None,
            "row 2 has no guild and must stay hidden"
        );
    }

    /// A sub-tab press selects its pane.
    #[test]
    fn a_sub_tab_press_selects_its_pane() {
        let mut app = App::new();
        app.init_resource::<GuildRelationsState>();
        for (tab, stem) in RelationsTab::ALL {
            let entity = app
                .world_mut()
                .spawn(GuildRelationsSubTab(tab, stem))
                .observe(on_relations_sub_tab)
                .id();
            app.world_mut().trigger(Activate { entity });
            app.update();
            assert_eq!(app.world().resource::<GuildRelationsState>().0, tab);
        }
    }
}

//! The job-suit predicate, typed.
//!
//! **The idea:** `net::entity_spawn::is_job_suit` already answers "is this worn
//! item a job suit?" and carries the trap documentation (`t4 == 5` is the
//! free-PvP cape, not a suit). Job-mode consumers need one thing more — *which*
//! job — so this module wraps that predicate instead of writing a second one.
//! The rank names and the `JobType` numbering live in
//! `assets/textdata/job.rs`, which is a closed surface and may not reach into
//! `net`; that is why the two halves are separate files.
//!
//! The rest of the module is the job wire consumers: `0x34D5` (safe trade),
//! `0x30E7` (the transport leash), `0xB0E6` (previous-job info), `0xB0E4` (the
//! two ranking pages) and `0x30E0` (the specialty price list). See
//! `docs/planning/JOB-etappe3.md`.

use crate::assets::textdata::job::JobType;
use crate::net::entity_spawn::{is_job_suit, ItemTypeIds};

/// Whether a worn item's TypeID tuple marks its wearer as job-suited, and for
/// which job.
///
/// This is the typed companion of [`is_job_suit`], which stays the single
/// predicate — including the `t4 == 5` free-PvP-cape trap documented there.
/// The `t4` → job mapping is the code-name census of
/// `docs/re/systems/job-trade-system.md` §3 `[V]`:
/// `1`/`6` → `*_TRADE_TRADER_*`, `2` → `*_TRADE_THIEF_*`,
/// `3`/`7` → `*_TRADE_HUNTER_*` (98 rows, no false positive).
pub fn job_suit_type(type_ids: ItemTypeIds) -> Option<JobType> {
    if !is_job_suit(type_ids) {
        return None;
    }
    match type_ids.3 {
        1 | 6 => Some(JobType::Trader),
        2 => Some(JobType::Thief),
        3 | 7 => Some(JobType::Hunter),
        // Unreachable while `is_job_suit` accepts exactly {1,2,3,6,7}; kept so
        // widening that predicate cannot silently mislabel a suit.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The wire consumers: 0x34D5 safe trade and 0x30E7 the transport leash
// ---------------------------------------------------------------------------
//
// Idea: both packets were wired in Etappe 2 (`packets/src/agent/job.rs`) and had
// no consumer at all, which is the defect this repo calls a dead wire. They are
// the two *smallest* job surfaces — one byte and two-to-four bytes — and neither
// needs a window, so they are the honest first half of Etappe 3
// (`docs/re/systems/job-trade-system.md` §8). The rank/prev-info windows need
// the `msgbox2_window_` shell, which this tree does not ship (#664).
//
// Words come from `textdata/textuisystem.txt` by key only, exactly as
// `net/siege.rs` does it; the two leash distances come from the *client's own
// push immediates* (100 / 30, §5.2 and §11.1), because the packet does not carry
// them.
//
// HUD-optional by construction: everything under `client/src/plugins/net/**`
// must run in the headless netcheck harness, so `ChatHistory` and
// `ClientUiStrings` are taken as `Option<Res<_>>` and the fallback is a log line
// (AGENTS.md; same note in `net/siege.rs` and `net/party.rs`).

use bevy::prelude::*;

use packets::agent::job::{
    describe_job_error, JobPrevInfoResponse, JobPriceUpdate, JobRankEntry, JobRankingResponse,
    JobSafeTradeUpdate, COS_LEASH_MONSTER, COS_LEASH_TRANSPORT, JOB_RANK_KIND_ACTIVITY,
    JOB_RANK_KIND_CONTRIBUTION, JOB_RESULT_SUCCESS, SAFE_TRADE_CODE_LIMIT,
};
use packets::agent::pet::{
    StuckDistanceWarning, STUCK_REASON_QUEST_MONSTER, STUCK_REASON_TRADE_CART,
};

use crate::assets::textdata::specialty::SpecialtyGoods;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::textdata::ClientItemData;
use crate::plugins::textdata::ClientUiStrings;

/// `(key, shipped English)` — the fallback is the string that key carries in the
/// user's own `textdata/textuisystem.txt`, quoted so headless runs and the tests
/// show the same words the data ships.
type UiText = (&'static str, &'static str);

/// The status label for "safe mode is on". A *label*, not an event message:
/// `UIIT_MSG_SAFETRADE_PROGRESS` is the one with the counters, this one is the
/// standing state.
const SAFE_TRADE_ON_TEXT: UiText = ("UIIT_STT_SAFETRADE_PROGRESS", "Safe trade is in progress.");
/// The counter message the original formats on the `'P'` arm (§5.4).
const SAFE_TRADE_PROGRESS_TEXT: UiText = (
    "UIIT_MSG_SAFETRADE_PROGRESS",
    "Safe trade will begin.[%d]/[%d]",
);
/// The `'A'` refusal (§5.4).
const SAFE_TRADE_LIMIT_TEXT: UiText = (
    "UIIT_MSG_SAFETRADE_NUMBER_ERR_LIMIT",
    "Unable to initiate Safe Trade. Maximum number of Safe Trade attempts has been reached.",
);
/// `reason == 1`, carrying the transport leash as its `%d`.
const LEASH_TRANSPORT_TEXT: UiText = (
    "UIIT_MSG_COSERR_TOO_FAR_FROM_TRADECART",
    "A character cannot move if he is %d m away from his transport.",
);
/// `reason == 2`, carrying the monster leash as its `%d`.
const LEASH_MONSTER_TEXT: UiText = (
    "UIIT_MSG_QUEST_ERR_TOO_FAR_FROM_MONSTER",
    "You cannot move if you are %d m away from captured monster.",
);

/// What the job wire currently says about trading, for anything that wants to
/// draw it later (the `iftw_jobplayer` plate, an underbar badge).
///
/// Kept in the net core rather than in the HUD because the HUD may be absent:
/// the headless harness builds `NetworkCorePlugin` alone, and a HUD-owned
/// resource would make these systems fail parameter validation.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct SafeTradeState {
    /// The `state` byte the original pushes into HUD state `0xB` (`§5.4`).
    pub active: bool,
    /// The last code byte. `0x40` is *not* provably `'@'`: the sender collapses
    /// `0x42..=0x4F` onto `state = 0, code = 0x40` (§11.6), so this is "one of
    /// sixteen" and is kept as the raw byte rather than as a meaning.
    pub code: u8,
    /// `(current, max)` from the four-byte arm, while it is being sent.
    pub attempts: Option<(u8, u8)>,
}

/// Push one line into the chat log when there is one, else log it — see the
/// module note on running without a HUD.
fn report(history: &mut Option<ResMut<ChatHistory>>, text: String) {
    match history {
        Some(history) => history.push(ChatLine::system(text)),
        None => info!("job (headless): {text}"),
    }
}

/// Fills a single `%d`, the way `inventory::tooltip` and `system_window` do.
fn fill(template: &str, value: impl std::fmt::Display) -> String {
    template.replacen("%d", &value.to_string(), 1)
}

/// The line a `0x34D5` should produce given the previous state, or `None` for
/// "say nothing".
///
/// **Why a transition and not a message per packet:** `0x34D5` rides the
/// periodic world-state bundle, not an event — 24 of the 31 captured frames
/// follow a `0x3206` immediately and the gaps run from 36 s to 26 h (§5.4). A
/// line per frame would therefore spam the log with the same `00 40`. Only a
/// change of state, an explicit refusal, or a counter update says something.
fn safe_trade_line(
    previous: &SafeTradeState,
    packet: &JobSafeTradeUpdate,
    strings: &ClientUiStrings,
) -> Option<String> {
    if packet.code == SAFE_TRADE_CODE_LIMIT {
        return Some(
            strings
                .get_or(SAFE_TRADE_LIMIT_TEXT.0, SAFE_TRADE_LIMIT_TEXT.1)
                .to_string(),
        );
    }
    if let (Some(current), Some(max)) = (packet.current, packet.max) {
        let template = strings.get_or(SAFE_TRADE_PROGRESS_TEXT.0, SAFE_TRADE_PROGRESS_TEXT.1);
        // Two `%d` in this one template: fill them left to right.
        return Some(fill(&fill(template, current), max));
    }
    let active = packet.state != 0;
    if active && !previous.active {
        return Some(
            strings
                .get_or(SAFE_TRADE_ON_TEXT.0, SAFE_TRADE_ON_TEXT.1)
                .to_string(),
        );
    }
    // Switching safe mode *off* has no string in the data — inventing "safe
    // trade ended" is exactly the unsourced text ADR-0009 forbids, so the state
    // resource changes and the log carries the raw bytes.
    None
}

/// 0x34D5 — keep [`SafeTradeState`] current and say the sourced line, if any.
pub fn on_safe_trade_update(
    mut reader: MessageReader<JobSafeTradeUpdate>,
    mut state: ResMut<SafeTradeState>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    for packet in reader.read() {
        let strings = strings.as_deref().unwrap_or(&default_strings);
        let line = safe_trade_line(&state, packet, strings);
        let next = SafeTradeState {
            active: packet.state != 0,
            code: packet.code,
            attempts: packet.current.zip(packet.max),
        };
        if next != *state {
            debug!(
                "job: 0x34D5 state {:#04X} code {:#04X} attempts {:?}",
                packet.state, packet.code, next.attempts
            );
            *state = next;
        }
        if let Some(line) = line {
            report(&mut history, line);
        }
    }
}

/// 0x30E7 — "you walked too far", with the distance the *client* owns.
///
/// Reads the chain's own [`StuckDistanceWarning`] rather than `job.rs`'s
/// `JobCosDistance`: `packets/src/lib.rs` binds 0x30E7 once, in the pet/COS
/// block, because the packet is not job-only (reason 2 is a quest monster).
///
/// The packet carries one reason byte and no distance; the two numbers are push
/// immediates next to the message keys in the original
/// (`883c04: 6a 64` = 100 for the transport, `883bac: 6a 1e` = 30 for the quest
/// monster — §11.1). An unknown reason has no key in the data, so it is logged
/// with its number instead of being described.
pub fn on_cos_distance(
    mut reader: MessageReader<StuckDistanceWarning>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    for packet in reader.read() {
        let strings = strings.as_deref().unwrap_or(&default_strings);
        let line = match packet.reason {
            STUCK_REASON_TRADE_CART => Some(fill(
                strings.get_or(LEASH_TRANSPORT_TEXT.0, LEASH_TRANSPORT_TEXT.1),
                COS_LEASH_TRANSPORT,
            )),
            STUCK_REASON_QUEST_MONSTER => Some(fill(
                strings.get_or(LEASH_MONSTER_TEXT.0, LEASH_MONSTER_TEXT.1),
                COS_LEASH_MONSTER,
            )),
            other => {
                info!("job: 0x30E7 reason {other:#04X} has no textdata key — not shown");
                None
            }
        };
        if let Some(line) = line {
            report(&mut history, line);
        }
    }
}

// --- 0xB0E6 previous-job info ----------------------------------------------
//
// Idea: the window this belongs in (`ifprevjobinfo.txt`, 364x164, three gauges)
// needs the `msgbox2_window_` shell, which this tree does not ship (#664), and
// the typed resinfo `Properties` view still panics on the corpus (#652). So
// Etappe 3 lands the *receive -> state -> visible line* half now and leaves the
// three-gauge window to the chrome ticket: three sourced lines beat a window
// built on guessed chrome.
//
// The three labels are the descriptor's own, read from
// `Media/resinfo/ifprevjobinfo.txt`: `GDR_PREV_JOB_INFO_{MERCHANT,HUNTER,THIEF}_GRADE_STA`
// carry `UIIT_STT_{MERCHANT,HUNTER,THIEF}_LEVEL`, and the exp column's label is
// `UIIT_STT_JOBEXP` ("Experience", `ifjobrank.txt:234`). The window title is
// `UIIT_STT_NPC_CHATTING_JOBINFO_OLD` (`ginterface.txt:1296`).
//
// **This is also a measurement.** The pair order in the body is `[S]`, not `[V]`
// (open point 2 / W3, `job-trade-system.md` §11.3): the reading is MERCHANT,
// THIEF, HUNTER. Printing all three labelled lines is what lets a human compare
// them against the original's window and settle it — see
// `docs/planning/JOB-etappe3.md`.

/// The archived job figures, as last answered. Cleared by nothing: the original
/// keeps the window's contents until the next query too.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct PrevJobInfo {
    /// `(level, exp)` per job, in the `[S]` wire order MERCHANT, THIEF, HUNTER.
    pub merchant: Option<(u8, u32)>,
    pub thief: Option<(u8, u32)>,
    pub hunter: Option<(u8, u32)>,
}

/// The window title, used as the heading of the reported block.
const PREV_JOB_TITLE_TEXT: UiText = (
    "UIIT_STT_NPC_CHATTING_JOBINFO_OLD",
    "Check previous job information",
);
/// `GDR_PREV_JOB_INFO_MERCHANT_GRADE_STA`'s key.
const MERCHANT_LEVEL_TEXT: UiText = ("UIIT_STT_MERCHANT_LEVEL", "Trader Grade");
/// `GDR_PREV_JOB_INFO_HUNTER_GRADE_STA`'s key.
const HUNTER_LEVEL_TEXT: UiText = ("UIIT_STT_HUNTER_LEVEL", "Hunter Grade");
/// `GDR_PREV_JOB_INFO_THIEF_GRADE_STA`'s key.
const THIEF_LEVEL_TEXT: UiText = ("UIIT_STT_THIEF_LEVEL", "Thief Grade");
/// The rank windows' EXP column header (`ifjobrank.txt` `_SUBJ_STA_EXP`).
const JOB_EXP_TEXT: UiText = ("UIIT_STT_JOBEXP", "Experience");

/// One `"<Grade label> 3, <Experience label> 1000"` row.
///
/// Deliberately **without** a rank name: `assets::textdata::job::rank_name` can
/// resolve one, but only after choosing the Chinese or the European key set, and
/// which set applies to a given character is not in this packet. Showing the
/// number the server sent is sourced; picking a set would be a guess.
fn prev_job_row(label: &str, exp_label: &str, level: u8, exp: u32) -> String {
    format!("{label} {level}, {exp_label} {exp}")
}

/// 0xB0E6 — keep [`PrevJobInfo`] current and report the three archived rows.
pub fn on_prev_job_info(
    mut reader: MessageReader<JobPrevInfoResponse>,
    mut state: ResMut<PrevJobInfo>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    for packet in reader.read() {
        let strings = strings.as_deref().unwrap_or(&default_strings);
        if packet.result != JOB_RESULT_SUCCESS {
            // The error text table is unread ([U], open point 1), so a code gets
            // a log line with our own description — never an invented chat line.
            let code = packet.error_code.unwrap_or_default();
            info!(
                "job: 0xB0E6 refused, code {code:#06X} ({})",
                describe_job_error(code)
            );
            continue;
        }
        *state = PrevJobInfo {
            merchant: packet.merchant_level.zip(packet.merchant_exp),
            thief: packet.thief_level.zip(packet.thief_exp),
            hunter: packet.hunter_level.zip(packet.hunter_exp),
        };
        let exp_label = strings.get_or(JOB_EXP_TEXT.0, JOB_EXP_TEXT.1);
        report(
            &mut history,
            strings
                .get_or(PREV_JOB_TITLE_TEXT.0, PREV_JOB_TITLE_TEXT.1)
                .to_string(),
        );
        // The descriptor's own top-to-bottom order is MERCHANT, HUNTER, THIEF
        // (`job-trade-system.md` §11.3's widget quartets), so the lines are
        // printed in *that* order while the bytes are read in the `[S]` wire
        // order MERCHANT, THIEF, HUNTER.
        for (text, value) in [
            (MERCHANT_LEVEL_TEXT, state.merchant),
            (HUNTER_LEVEL_TEXT, state.hunter),
            (THIEF_LEVEL_TEXT, state.thief),
        ] {
            if let Some((level, exp)) = value {
                let label = strings.get_or(text.0, text.1);
                report(&mut history, prev_job_row(label, exp_label, level, exp));
            }
        }
    }
}

// --- 0xB0E4 job ranking -----------------------------------------------------
//
// Idea: `0xB0E4` is a *page* — job_type, rank_kind and up to ten rows — and the
// two windows that draw it (`ifjobrank` 388x369, `ifjobcontributionrank`
// 388x500) live in the HUD, which may be absent. So the page is kept here as
// plain state and reported line by line; `hud::job::ranking` turns the same
// resource into the authored windows.
//
// **The §5.5 prediction "two tabs" does not survive the data.** The descriptors
// declare *two separate windows*, each with its own `ginterface.txt`
// registration (`GDR_JOB_RANK` id 64 `0,0,388,369` :1238, and
// `GDR_JOB_CONTRIBUTION_RANK` id 65 `0,0,388,500` :1261), its own tree, its own
// row prototype and its own title static — and neither tree declares a tab
// control of any kind (0 hits for a tab class in both files, against
// `ifcommunity.txt`'s four). `rank_kind` therefore selects a *window*, not a
// page inside one.

/// One answered ranking page.
#[derive(Debug, Clone, PartialEq)]
pub struct JobRankingPage {
    /// `[S]` — the job the page is about; the request echoes back (§5.5).
    pub job_type: u8,
    /// [`JOB_RANK_KIND_ACTIVITY`] or [`JOB_RANK_KIND_CONTRIBUTION`].
    pub rank_kind: u8,
    /// The rows as sent, in the server's own order. Ten is what the descriptors
    /// draw (`_STA_SLOT_01..10`); a longer page is kept whole here and the
    /// window says how many it shows.
    pub entries: Vec<JobRankEntry>,
}

/// The last ranking page the server answered, or `None` before the first one.
///
/// A page with *zero* rows is a legitimate answer (nobody ranked yet), which is
/// why this is an `Option` around the page rather than a bare `Vec`: "no answer
/// yet" and "an empty ranking" must not collapse into the same state.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct JobRanking(pub Option<JobRankingPage>);

/// `GDR_JOB_RANK_STA_TITLE`'s key (`ifjobrank.txt:310`).
const RANK_ACTIVITY_TITLE_TEXT: UiText = (
    "UIIT_STT_JOBGUILD_TITLE",
    "Last week's job activity ranking",
);
/// `GDR_JOB_CONTRIBUTION_RANK_STA_TITLE`'s key
/// (`ifjobcontributionrank.txt:310`).
const RANK_CONTRIBUTION_TITLE_TEXT: UiText = (
    "UIIT_STT_JOBGUILD_CONTRIBUTERANK",
    "Last week's donation ranking",
);
/// The `_SUBJ_STA_GRADE` column header, shared by both windows.
const RANK_GRADE_TEXT: UiText = ("UIIT_STT_GRADE", "Level");
/// The contribution window's value column (`UIIT_STT_DONATION`).
const RANK_DONATION_TEXT: UiText = ("UIIT_STT_DONATION", "Donation amount");

/// One `"1. Alias, Level 5, Experience 1234"` row.
///
/// No rank *name*: the `_GRADENAME` column exists in the activity descriptor,
/// but resolving it needs the Chinese-vs-European key set, and this packet does
/// not say which applies — the same blank, for the same reason, that
/// [`prev_job_row`] leaves.
fn job_rank_row(entry: &JobRankEntry, grade_label: &str, points_label: &str) -> String {
    format!(
        "{}. {}, {grade_label} {}, {points_label} {}",
        entry.rank, entry.name, entry.job_level, entry.points
    )
}

/// 0xB0E4 — keep [`JobRanking`] current and report the answered page.
pub fn on_job_ranking(
    mut reader: MessageReader<JobRankingResponse>,
    mut state: ResMut<JobRanking>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    for packet in reader.read() {
        let strings = strings.as_deref().unwrap_or(&default_strings);
        if !packet.is_success() {
            let code = packet.error_code.unwrap_or_default();
            info!(
                "job: 0xB0E4 refused, code {code:#06X} ({})",
                describe_job_error(code)
            );
            continue;
        }
        let rank_kind = packet.rank_kind.unwrap_or(JOB_RANK_KIND_ACTIVITY);
        let page = JobRankingPage {
            job_type: packet.job_type.unwrap_or_default(),
            rank_kind,
            entries: packet.entries.clone(),
        };
        let (title, points_label) = if rank_kind == JOB_RANK_KIND_CONTRIBUTION {
            (RANK_CONTRIBUTION_TITLE_TEXT, RANK_DONATION_TEXT)
        } else {
            (RANK_ACTIVITY_TITLE_TEXT, JOB_EXP_TEXT)
        };
        report(&mut history, strings.get_or(title.0, title.1).to_string());
        let grade_label = strings.get_or(RANK_GRADE_TEXT.0, RANK_GRADE_TEXT.1);
        let points_label = strings.get_or(points_label.0, points_label.1);
        for entry in &page.entries {
            report(&mut history, job_rank_row(entry, grade_label, points_label));
        }
        *state = JobRanking(Some(page));
    }
}

// --- 0x30E0 specialty price list --------------------------------------------
//
// Idea: the price list is the *first* half of the specialty trade — the goods
// table itself is already indexed (`assets::textdata::specialty`, 43 rows with
// TID `(3,3,8,1|2)` and a placeholder price of 383 on every one of them), so
// what the wire adds is the only real number: the market price.
//
// **This consumer is also the measurement W4** (`job-trade-system.md` §5.1):
// `JobPriceEntry` carries two `u32`s whose *names* are `[S]` — the sender walks
// a map and writes key then value, so "id, price" is the reading, but nothing
// has confirmed it. We do not need a capture to settle that: we know all 43
// specialty ref ids, so on the first page that arrives one column either
// resolves against that set or does not. [`measure_id_column`] answers it, the
// verdict is logged, and until a page arrives the state says `Unmeasured`
// rather than assuming the reading is right.
//
// Nothing is *shown* yet: the specialty-deal window (`ifspecialtydeal`) is its
// own change. The page is kept as state and reported to the log — HUD-optional
// like the rest of this module.

/// Which of `JobPriceEntry`'s two `u32`s carried the ref item id, as measured
/// on the pages that actually arrived.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriceIdColumn {
    /// No page seen yet — say so rather than assuming the `[S]` reading.
    #[default]
    Unmeasured,
    /// The first `u32` (`ref_item_id`) resolved against the specialty table and
    /// the second did not: the published reading holds.
    First,
    /// The other way round — the packet's field names would be swapped.
    Second,
    /// Neither column resolved: either the goods table was not loaded, or this
    /// page is not about specialty goods at all. Nothing is concluded.
    Inconclusive,
}

/// The last specialty price page, as sent.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct SpecialtyPrices {
    /// `(first u32, second u32)` per row, verbatim — deliberately *not* named
    /// `(id, price)` until [`SpecialtyPrices::id_column`] says which is which.
    pub entries: Vec<(u32, u32)>,
    pub id_column: PriceIdColumn,
}

impl SpecialtyPrices {
    /// `(ref_id, price)` per row once the columns are measured, else `None`.
    pub fn resolved(&self) -> Option<Vec<(u32, u32)>> {
        match self.id_column {
            PriceIdColumn::First => Some(self.entries.clone()),
            PriceIdColumn::Second => Some(self.entries.iter().map(|(a, b)| (*b, *a)).collect()),
            PriceIdColumn::Unmeasured | PriceIdColumn::Inconclusive => None,
        }
    }
}

/// Which column is the ref item id, given a predicate that knows the 43
/// specialty ref ids.
///
/// The rule is deliberately strict: a column wins only if **every** row
/// resolves through it and the other column resolves for none of them. A ref
/// id and a price cannot both be a good's id, and a partial match would be a
/// coincidence rather than a measurement.
pub fn measure_id_column(entries: &[(u32, u32)], is_good: impl Fn(u32) -> bool) -> PriceIdColumn {
    if entries.is_empty() {
        return PriceIdColumn::Inconclusive;
    }
    let first = entries.iter().all(|(a, _)| is_good(*a));
    let second = entries.iter().all(|(_, b)| is_good(*b));
    let any_first = entries.iter().any(|(a, _)| is_good(*a));
    let any_second = entries.iter().any(|(_, b)| is_good(*b));
    match (first && !any_second, second && !any_first) {
        (true, false) => PriceIdColumn::First,
        (false, true) => PriceIdColumn::Second,
        _ => PriceIdColumn::Inconclusive,
    }
}

/// 0x30E0 — keep [`SpecialtyPrices`] current and settle W4 while doing it.
pub fn on_specialty_prices(
    mut reader: MessageReader<JobPriceUpdate>,
    mut state: ResMut<SpecialtyPrices>,
    item_data: Option<Res<ClientItemData>>,
) {
    for packet in reader.read() {
        let entries: Vec<(u32, u32)> = packet
            .entries
            .iter()
            .map(|entry| (entry.ref_item_id, entry.price))
            .collect();
        // The goods index is rebuilt per page on purpose: pages are rare (an
        // NPC visit), the index is 43 rows out of already-parsed itemdata, and
        // caching it here would duplicate `ClientItemData`'s lifetime rules.
        let goods = item_data
            .as_ref()
            .and_then(|data| data.data())
            .map(SpecialtyGoods::from_item_data);
        let id_column = match goods.as_ref() {
            Some(goods) => measure_id_column(&entries, |value| {
                i32::try_from(value)
                    .ok()
                    .is_some_and(|ref_id| goods.get(ref_id).is_some())
            }),
            None => PriceIdColumn::Inconclusive,
        };
        if id_column != state.id_column {
            info!(
                "job: 0x30E0 id column measured as {id_column:?} over {} rows (W4, \
                 job-trade-system.md §5.1)",
                entries.len()
            );
        }
        match (id_column, goods.as_ref()) {
            (PriceIdColumn::First | PriceIdColumn::Second, Some(goods)) => {
                let resolved = if id_column == PriceIdColumn::First {
                    entries.clone()
                } else {
                    entries.iter().map(|(a, b)| (*b, *a)).collect()
                };
                for (ref_id, price) in resolved.iter().take(SPECIALTY_LOG_ROWS) {
                    let name = i32::try_from(*ref_id)
                        .ok()
                        .and_then(|id| goods.get(id))
                        .map(|good| good.code_name.clone())
                        .unwrap_or_else(|| format!("ref {ref_id}"));
                    debug!("job: specialty price {name} = {price}");
                }
            }
            _ => debug!(
                "job: 0x30E0 with {} rows, columns unresolved — raw {:?}",
                entries.len(),
                &entries[..entries.len().min(SPECIALTY_LOG_ROWS)]
            ),
        }
        *state = SpecialtyPrices { entries, id_column };
    }
}

/// How many rows of a page reach the log. A page is up to 255 rows and this is
/// a debug aid, not a report.
const SPECIALTY_LOG_ROWS: usize = 8;

/// Registers the job wire consumers. Part of the networking core so the lines
/// appear wherever job packets can arrive — including headless, where they
/// degrade to logs.
pub struct JobStatusPlugin;

impl Plugin for JobStatusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SafeTradeState>()
            .init_resource::<PrevJobInfo>()
            .init_resource::<JobRanking>()
            .init_resource::<SpecialtyPrices>()
            // The five messages the systems below read. Registering them here,
            // in the plugin that owns the readers, is what keeps the app from
            // panicking on the first frame: a `MessageReader<T>` for a `T` no
            // one registered does not idle, it fails parameter validation and
            // takes the whole schedule down. `make ci` cannot see that — it
            // never starts the app — and the tests below each build their own
            // `App` and call `add_message` themselves, so they could not see it
            // either. `NETCHECK=1 cargo run -p client` found it: the client
            // panicked in `on_cos_distance` right after the gateway connect.
            .add_message::<JobSafeTradeUpdate>()
            .add_message::<StuckDistanceWarning>()
            .add_message::<JobPrevInfoResponse>()
            .add_message::<JobRankingResponse>()
            .add_message::<JobPriceUpdate>()
            .add_systems(
                Update,
                (
                    on_safe_trade_update,
                    on_cos_distance,
                    on_prev_job_info,
                    on_job_ranking,
                    on_specialty_prices,
                ),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn job_suit_type_reuses_the_spawn_predicate() {
        assert_eq!(job_suit_type((3, 1, 7, 1)), Some(JobType::Trader));
        assert_eq!(job_suit_type((3, 1, 7, 6)), Some(JobType::Trader));
        assert_eq!(job_suit_type((3, 1, 7, 2)), Some(JobType::Thief));
        assert_eq!(job_suit_type((3, 1, 7, 3)), Some(JobType::Hunter));
        assert_eq!(job_suit_type((3, 1, 7, 7)), Some(JobType::Hunter));
        // The trap: t4 == 5 is the free-PvP cape, not a job suit.
        assert_eq!(job_suit_type((3, 1, 7, 5)), None);
        assert_eq!(job_suit_type((3, 1, 7, 4)), None);
        assert_eq!(job_suit_type((3, 1, 6, 1)), None, "weapons");
        assert_eq!(job_suit_type((3, 3, 8, 1)), None, "specialty goods");
        // Same answer as the boolean predicate, for every tuple it accepts.
        for t4 in 0..=10 {
            assert_eq!(
                is_job_suit((3, 1, 7, t4)),
                job_suit_type((3, 1, 7, t4)).is_some()
            );
        }
    }
    // --- the two wire consumers -------------------------------------------

    use bytes::Bytes;
    use packets::agent::job::SAFE_TRADE_CODE_NEUTRAL;

    fn app_with_chat() -> App {
        let mut app = App::new();
        app.init_resource::<ChatHistory>()
            .init_resource::<SafeTradeState>()
            .add_message::<JobSafeTradeUpdate>()
            .add_message::<StuckDistanceWarning>()
            .add_systems(Update, (on_safe_trade_update, on_cos_distance));
        app
    }

    fn lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| line.display())
            .collect()
    }

    /// The captured reality: `packet_dump/0x34d5.log` is 31 identical `00 40`
    /// frames riding the periodic world-state bundle. Feeding all 31 must leave
    /// the chat log empty — a line per frame would be spam, not information.
    #[test]
    fn the_captured_safe_trade_frames_say_nothing_and_leave_the_state_off() {
        let mut app = app_with_chat();
        for _ in 0..31 {
            let packet = JobSafeTradeUpdate::try_from(Bytes::from_static(&[0x00, 0x40])).unwrap();
            app.world_mut().write_message(packet);
        }
        app.update();
        assert!(lines(&app).is_empty(), "no line for a no-change refresh");
        let state = app.world().resource::<SafeTradeState>();
        assert!(!state.active);
        assert_eq!(state.code, SAFE_TRADE_CODE_NEUTRAL);
        assert_eq!(state.attempts, None);
    }

    /// Switching safe mode on says the client's own status label, once.
    #[test]
    fn safe_trade_turning_on_reports_the_shipped_status_label() {
        let mut app = app_with_chat();
        app.world_mut().write_message(JobSafeTradeUpdate {
            state: 1,
            code: SAFE_TRADE_CODE_NEUTRAL,
            current: None,
            max: None,
        });
        app.update();
        assert_eq!(lines(&app), vec![SAFE_TRADE_ON_TEXT.1.to_string()]);
        assert!(app.world().resource::<SafeTradeState>().active);

        // The same state again is a refresh, not an event.
        app.world_mut().write_message(JobSafeTradeUpdate {
            state: 1,
            code: SAFE_TRADE_CODE_NEUTRAL,
            current: None,
            max: None,
        });
        app.update();
        assert_eq!(lines(&app).len(), 1, "no second line for a repeat");
    }

    /// The four-byte `'P'` arm fills both `%d`s of the shipped template, left to
    /// right (§5.4).
    #[test]
    fn safe_trade_progress_fills_both_counters() {
        let mut app = app_with_chat();
        app.world_mut().write_message(
            JobSafeTradeUpdate::try_from(Bytes::from_static(&[0x01, 0x50, 0x02, 0x05])).unwrap(),
        );
        app.update();
        assert_eq!(
            lines(&app),
            vec!["Safe trade will begin.[2]/[5]".to_string()]
        );
        assert_eq!(
            app.world().resource::<SafeTradeState>().attempts,
            Some((2, 5))
        );
    }

    /// The `'A'` refusal is its own message and does not depend on the state
    /// byte.
    #[test]
    fn safe_trade_limit_code_reports_the_refusal() {
        let mut app = app_with_chat();
        app.world_mut().write_message(JobSafeTradeUpdate {
            state: 0,
            code: SAFE_TRADE_CODE_LIMIT,
            current: None,
            max: None,
        });
        app.update();
        assert_eq!(lines(&app), vec![SAFE_TRADE_LIMIT_TEXT.1.to_string()]);
    }

    /// The leash numbers are the client's push immediates, not packet fields:
    /// reason 1 → 100, reason 2 → 30 (§5.2 / §11.1).
    #[test]
    fn cos_distance_reports_the_leash_the_client_owns() {
        let mut app = app_with_chat();
        app.world_mut().write_message(StuckDistanceWarning {
            reason: STUCK_REASON_TRADE_CART,
        });
        app.world_mut().write_message(StuckDistanceWarning {
            reason: STUCK_REASON_QUEST_MONSTER,
        });
        app.update();
        assert_eq!(
            lines(&app),
            vec![
                "A character cannot move if he is 100 m away from his transport.".to_string(),
                "You cannot move if you are 30 m away from captured monster.".to_string(),
            ]
        );
    }

    /// A reason the data has no key for is logged, never invented.
    #[test]
    fn an_unknown_cos_distance_reason_produces_no_line() {
        let mut app = app_with_chat();
        app.world_mut()
            .write_message(StuckDistanceWarning { reason: 9 });
        app.update();
        assert!(lines(&app).is_empty());
    }

    /// AGENTS.md: every system under `plugins/net/**` must run without the HUD.
    /// Built exactly like the netcheck harness — no `ChatHistory`, no
    /// `ClientUiStrings` — the schedule must not panic on parameter validation.
    #[test]
    fn the_consumers_run_without_any_hud_resource() {
        let mut app = App::new();
        app.add_message::<JobSafeTradeUpdate>()
            .add_message::<StuckDistanceWarning>()
            .add_message::<JobPrevInfoResponse>()
            // Every message a system of this plugin reads must be registered,
            // or Bevy fails the `MessageReader` parameter validation and panics
            // the schedule — the exact failure class AGENTS.md describes for
            // missing resources. The real app registers all of them through
            // `packets!`.
            .add_message::<JobRankingResponse>()
            .add_message::<JobPriceUpdate>()
            .add_plugins(JobStatusPlugin);
        app.world_mut().write_message(JobSafeTradeUpdate {
            state: 1,
            code: SAFE_TRADE_CODE_NEUTRAL,
            current: None,
            max: None,
        });
        app.world_mut().write_message(StuckDistanceWarning {
            reason: STUCK_REASON_TRADE_CART,
        });
        app.world_mut().write_message(
            JobPrevInfoResponse::try_from(Bytes::from_static(&[
                0x01, 0x03, 0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00,
                0x00, 0x00,
            ]))
            .unwrap(),
        );
        app.update();
        assert!(app.world().resource::<SafeTradeState>().active);
        assert_eq!(
            app.world().resource::<PrevJobInfo>().merchant,
            Some((3, 1000))
        );
    }
    // --- 0x30E0 specialty prices ------------------------------------------

    /// W4 (`job-trade-system.md` §5.1) without a capture: the goods table knows
    /// all 43 ref ids, so a page decides which column is the id. Strict rule —
    /// every row through one column, none through the other.
    #[test]
    fn the_price_page_measures_which_column_is_the_item_id() {
        // Pretend 8 and 9 are specialty ref ids; prices are not.
        let is_good = |value: u32| value == 8 || value == 9;
        assert_eq!(
            measure_id_column(&[(8, 1000), (9, 2500)], is_good),
            PriceIdColumn::First,
            "the published reading: key then value"
        );
        assert_eq!(
            measure_id_column(&[(1000, 8), (2500, 9)], is_good),
            PriceIdColumn::Second,
            "the packet's field names would be swapped"
        );
        // A single row that resolves on both sides settles nothing.
        assert_eq!(
            measure_id_column(&[(8, 9)], is_good),
            PriceIdColumn::Inconclusive
        );
        // Half a column is a coincidence, not a measurement.
        assert_eq!(
            measure_id_column(&[(8, 1000), (77, 2500)], is_good),
            PriceIdColumn::Inconclusive
        );
        // No rows, no verdict.
        assert_eq!(measure_id_column(&[], is_good), PriceIdColumn::Inconclusive);
    }

    /// Until a page has been measured, nothing is named `(id, price)`.
    #[test]
    fn an_unmeasured_page_refuses_to_name_its_columns() {
        let mut state = SpecialtyPrices::default();
        assert_eq!(state.id_column, PriceIdColumn::Unmeasured);
        assert_eq!(state.resolved(), None);
        state.entries = vec![(8, 1000)];
        assert_eq!(state.resolved(), None, "still unmeasured");
        state.id_column = PriceIdColumn::Second;
        assert_eq!(
            state.resolved(),
            Some(vec![(1000, 8)]),
            "the swapped reading is applied, not assumed away"
        );
    }

    /// The consumer runs headless and without any item data — the case the
    /// netcheck harness builds — and says "inconclusive" instead of guessing.
    #[test]
    fn a_price_page_without_item_data_concludes_nothing() {
        let mut app = App::new();
        app.init_resource::<SpecialtyPrices>()
            .add_message::<JobPriceUpdate>()
            .add_systems(Update, on_specialty_prices);
        // count 2, then two (u32, u32) rows.
        let body = Bytes::from_static(&[
            0x02, //
            0x0A, 0x00, 0x00, 0x00, 0xE8, 0x03, 0x00, 0x00, //
            0x0B, 0x00, 0x00, 0x00, 0xD0, 0x07, 0x00, 0x00,
        ]);
        app.world_mut()
            .write_message(JobPriceUpdate::try_from(body).unwrap());
        app.update();
        let state = app.world().resource::<SpecialtyPrices>();
        assert_eq!(state.entries, vec![(10, 1000), (11, 2000)]);
        assert_eq!(state.id_column, PriceIdColumn::Inconclusive);
        assert_eq!(state.resolved(), None);
    }

    /// An activity page reaches the chat log as the descriptor's own words.
    ///
    /// The body is the `rank_kind == 0` branch, so each row carries the extra
    /// `[U]` byte — proof that the row width follows the kind byte and that the
    /// unknown byte is read but never shown.
    #[test]
    fn an_activity_ranking_reports_the_authored_title_and_one_row_per_entry() {
        let mut app = app_with_chat();
        app.init_resource::<JobRanking>()
            .add_message::<JobRankingResponse>()
            .add_systems(Update, on_job_ranking);
        // 01 | job_type 1 | rank_kind 0 | count 2
        // rank 1, "Ann", level 7, 300, trailing 0x09
        // rank 2, "Bob", level 5, 200, trailing 0x00
        let body = Bytes::from_static(&[
            0x01, 0x01, 0x00, 0x02, //
            0x01, 0x03, 0x00, b'A', b'n', b'n', 0x07, 0x2C, 0x01, 0x00, 0x00, 0x09, //
            0x02, 0x03, 0x00, b'B', b'o', b'b', 0x05, 0xC8, 0x00, 0x00, 0x00, 0x00,
        ]);
        app.world_mut()
            .write_message(JobRankingResponse::try_from(body).unwrap());
        app.update();

        assert_eq!(
            lines(&app),
            vec![
                "Last week's job activity ranking".to_string(),
                "1. Ann, Level 7, Experience 300".to_string(),
                "2. Bob, Level 5, Experience 200".to_string(),
            ]
        );
        let page = app
            .world()
            .resource::<JobRanking>()
            .0
            .clone()
            .expect("the page landed");
        assert_eq!(page.rank_kind, JOB_RANK_KIND_ACTIVITY);
        assert_eq!(page.entries[0].trailing, Some(9), "read, not shown");
    }

    /// A refusal leaves both the chat log and the page untouched: the error
    /// table is `[U]`, so a code gets a log line and never an invented text.
    #[test]
    fn a_refused_ranking_says_nothing_and_keeps_the_previous_page() {
        let mut app = app_with_chat();
        app.init_resource::<JobRanking>()
            .add_message::<JobRankingResponse>()
            .add_systems(Update, on_job_ranking);
        app.world_mut().write_message(
            JobRankingResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).unwrap(),
        );
        app.update();
        assert!(lines(&app).is_empty());
        assert_eq!(app.world().resource::<JobRanking>().0, None);
    }

    /// The success body from `packets`' own round-trip test, shown as the
    /// descriptor's three labelled rows. A job with no archived figures at all
    /// still has a `(0, 0)` pair on the wire, so all three rows appear — the
    /// original's window has three gauges for the same reason.
    #[test]
    fn prev_job_info_reports_the_three_descriptor_labels() {
        let mut app = app_with_chat();
        app.init_resource::<PrevJobInfo>()
            .add_message::<JobPrevInfoResponse>()
            .add_systems(Update, on_prev_job_info);
        // 01 | merchant (3, 1000) | thief (0, 0) | hunter (1, 7)
        let body = Bytes::from_static(&[
            0x01, 0x03, 0xE8, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00,
            0x00, 0x00,
        ]);
        app.world_mut()
            .write_message(JobPrevInfoResponse::try_from(body).unwrap());
        app.update();
        assert_eq!(
            lines(&app),
            vec![
                "Check previous job information".to_string(),
                "Trader Grade 3, Experience 1000".to_string(),
                "Hunter Grade 1, Experience 7".to_string(),
                "Thief Grade 0, Experience 0".to_string(),
            ],
            "descriptor order MERCHANT, HUNTER, THIEF — the bytes are read in \
             the [S] wire order MERCHANT, THIEF, HUNTER"
        );
        let state = app.world().resource::<PrevJobInfo>();
        assert_eq!(state.merchant, Some((3, 1000)));
        assert_eq!(state.thief, Some((0, 0)));
        assert_eq!(state.hunter, Some((1, 7)));
    }

    /// The measured refusal `02 03 00` (§11.14 #1): no chat line at all, because
    /// the error-code text table is unread — the code goes to the log.
    #[test]
    fn a_refused_prev_job_info_says_nothing_in_chat() {
        let mut app = app_with_chat();
        app.init_resource::<PrevJobInfo>()
            .add_message::<JobPrevInfoResponse>()
            .add_systems(Update, on_prev_job_info);
        app.world_mut().write_message(
            JobPrevInfoResponse::try_from(Bytes::from_static(&[0x02, 0x03, 0x00])).unwrap(),
        );
        app.update();
        assert!(lines(&app).is_empty());
        assert_eq!(
            *app.world().resource::<PrevJobInfo>(),
            PrevJobInfo::default()
        );
    }
}

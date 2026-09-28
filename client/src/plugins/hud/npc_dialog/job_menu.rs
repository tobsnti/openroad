//! The job NPC's menu lines — the opener for the two job windows.
//!
//! Idea: the original builds this menu from **client strings**, and the string
//! set is enumerable, which is what makes the menu reconstructable without a
//! capture: `docs/re/systems/job-trade-system.md` §11.8 lists every
//! `UIIT_STT_NPC_CHATTING_*` key that x-refs the one menu builder `0x6fcd60`
//! — per job `{TRADER,HUNTER,THIEF}MENU_{JOIN,WITHD,ALIASCREATE,ALIASMODIFY,
//! JOBRANK}`, plus **only** for hunters `HUNTERMENU_{OUTCOME,CONTRIBUTERANK}`,
//! **only** for traders `TRADERMENU_DONATIONRANK`, and the job-neutral
//! `JOBINFO_OLD`. That absence claim carries a positive control on the same
//! read path (`THIEFMENU_*` does exist for JOIN/WITHD/ALIAS/JOBRANK), so
//! "thieves have no contribution ranking" is a fact about the data, not a gap
//! in the query.
//!
//! **Which NPC is a job NPC** is decided the way this dialog already decides
//! shop, ferry and warehouse lines: from the user's own data, by codename —
//! `hud::npc_dialog::ui`'s `codename.contains("WAREHOUSE")` is the precedent.
//! The rule here is a *suffix* rule, and the census behind it is
//! `characterdata_*.txt` joined to `textdata_object` (English column), all 17
//! NPC codenames that mention a job:
//!
//! | codename | name in the user's data | job menu? |
//! |---|---|---|
//! | `NPC_CA_MERCHANT`, `NPC_EU_MERCHANT` | "Merchant Associate …" | trader |
//! | `NPC_SD_M_AREA_MERCHANT` | "Trader Union President Naunakt" | trader |
//! | `NPC_CA_HUNTER`, `NPC_EU_HUNTER` | "Hunter Associate …" | hunter |
//! | `NPC_SD_M_AREA_HUNTER` | "Hunter Union President Narmer" | hunter |
//! | `NPC_SD_T_AREA_THIEF` | "Thief Union President Tausert" | thief |
//! | `NPC_EU_UNION1/2` | "Association Boss …" | job-neutral only |
//! | `NPC_SD_MERCHANT_CHANGER`, `NPC_SD_THIEF_AGENT_CHANGER` | "… Item Exchange manager …" | no |
//! | `NPC_TD_THIEF_{A,B,C,D,BUY,SELL}` | bandit bands / "Stolen Goods Dealer" | no |
//!
//! The suffix rule `_MERCHANT` / `_HUNTER` / `_THIEF` at the **end** of the
//! codename selects exactly the seven union/associate NPCs and rejects the two
//! `_CHANGER`s and the six thief-den shops — the test below pins that against
//! the census, so widening the rule cannot silently put a job menu on a shop.
//!
//! What is deliberately **not** here yet: the alias pair (`ALIASCREATE` /
//! `ALIASMODIFY`) and `HUNTERMENU_OUTCOME`. JOIN / WITHD (`0x70E1`/`0x70E2`)
//! are here, but never as a send-on-click: joining costs gold, leaving locks
//! the league for seven days, and the success shape of the `0xB0E1`/`0xB0E2`
//! acks is `[U]` (`packets::agent::job`), so each raises the original's own
//! shipped confirmation and goes out on the answer —
//! `docs/planning/JOB-etappe3.md`.

use bevy::prelude::*;

use packets::agent::job::{
    JobJoinRequest, JobLeaveRequest, JobPrevInfoRequest, JobRankingRequest, JOB_RANK_KIND_ACTIVITY,
    JOB_RANK_KIND_CONTRIBUTION,
};
use packets::Packet;

use crate::assets::textdata::job::JobType;
use crate::net::connection::SilkroadConnection;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::ui_v2::choice_confirm::ChoiceConfirmed;

/// What clicking one of these lines asks the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobMenuAction {
    /// `0x70E4` — a ranking page for this job and kind.
    Ranking { job_type: u8, rank_kind: u8 },
    /// `0x70E6` — the archived figures. Carries no job type (the request is a
    /// bare `u32 npc_gid`), which is why it is offered on the job-neutral
    /// union bosses too.
    PreviousInfo,
    /// `0x70E1` — join this NPC's league. **Costs gold**, so this one never
    /// sends on the click: it raises the original's own confirmation first
    /// (see [`confirm_text`]).
    Join { job_type: u8 },
    /// `0x70E2` — leave it. Confirmed too: the original's prompt states the
    /// seven-day lockout, and a prompt without the consequence is not one.
    Leave,
}

/// One menu line: its textdata key, the string that key carries in the user's
/// own `textuisystem.txt` as fallback, and what it asks for.
pub struct JobMenuLine {
    pub key: &'static str,
    pub fallback: &'static str,
    pub action: JobMenuAction,
}

/// The job an NPC's codename names, by the suffix rule (module doc).
pub fn job_of_npc(codename: &str) -> Option<JobType> {
    if codename.ends_with("_MERCHANT") {
        Some(JobType::Trader)
    } else if codename.ends_with("_HUNTER") {
        Some(JobType::Hunter)
    } else if codename.ends_with("_THIEF") {
        Some(JobType::Thief)
    } else {
        None
    }
}

/// Whether an NPC is one of the job-neutral union bosses (`NPC_EU_UNION1/2`,
/// "Association Boss" in the user's data). They get the archived-info line but
/// no ranking line: a ranking request needs a `job_type` byte and nothing in
/// the codename or in `0x70E6` says which one they stand for.
fn is_union_boss(codename: &str) -> bool {
    codename.starts_with("NPC_") && codename.contains("_UNION")
}

/// The lines this NPC offers, in the original's own order: the activity
/// ranking, then the job's second ranking where the data has one, then the
/// archived figures.
pub fn job_menu_lines(codename: &str) -> Vec<JobMenuLine> {
    let mut lines = Vec::new();
    if let Some(job) = job_of_npc(codename) {
        let (rank_key, rank_fallback) = match job {
            JobType::Trader => (
                "UIIT_STT_NPC_CHATTING_TRADERMENU_JOBRANK",
                "Ranking  - merchant activity",
            ),
            JobType::Hunter => (
                "UIIT_STT_NPC_CHATTING_HUNTERMENU_JOBRANK",
                "Ranking  - hunter activity",
            ),
            JobType::Thief => (
                "UIIT_STT_NPC_CHATTING_THIEFMENU_JOBRANK",
                "Ranking  - thief activity",
            ),
        };
        lines.push(JobMenuLine {
            key: rank_key,
            fallback: rank_fallback,
            action: JobMenuAction::Ranking {
                job_type: job as u8,
                rank_kind: JOB_RANK_KIND_ACTIVITY,
            },
        });
        // The second ranking exists for traders and hunters only (§11.8's
        // absence claim); a thief line here would be an invented key.
        let second = match job {
            JobType::Trader => Some((
                "UIIT_STT_NPC_CHATTING_TRADERMENU_DONATIONRANK",
                "Ranking  - weekly donation",
            )),
            JobType::Hunter => Some((
                "UIIT_STT_NPC_CHATTING_HUNTERMENU_CONTRIBUTERANK",
                "Ranking  - weekly contribution",
            )),
            JobType::Thief => None,
        };
        if let Some((key, fallback)) = second {
            lines.push(JobMenuLine {
                key,
                fallback,
                action: JobMenuAction::Ranking {
                    job_type: job as u8,
                    rank_kind: JOB_RANK_KIND_CONTRIBUTION,
                },
            });
        }
        // Join / leave. Both are confirmed before anything goes out.
        let (join, withd) = match job {
            JobType::Trader => (
                (
                    "UIIT_STT_NPC_CHATTING_TRADERMENU_JOIN",
                    "Join merchant guild",
                ),
                (
                    "UIIT_STT_NPC_CHATTING_TRADERMENU_WITHD",
                    "Leave merchant guild",
                ),
            ),
            JobType::Hunter => (
                ("UIIT_STT_NPC_CHATTING_HUNTERMENU_JOIN", "Join hunter guild"),
                (
                    "UIIT_STT_NPC_CHATTING_HUNTERMENU_WITHD",
                    "Leave hunter guild",
                ),
            ),
            JobType::Thief => (
                ("UIIT_STT_NPC_CHATTING_THIEFMENU_JOIN", "Join thief guild"),
                ("UIIT_STT_NPC_CHATTING_THIEFMENU_WITHD", "Leave thief guild"),
            ),
        };
        lines.push(JobMenuLine {
            key: join.0,
            fallback: join.1,
            action: JobMenuAction::Join {
                job_type: job as u8,
            },
        });
        lines.push(JobMenuLine {
            key: withd.0,
            fallback: withd.1,
            action: JobMenuAction::Leave,
        });
    }
    if job_of_npc(codename).is_some() || is_union_boss(codename) {
        lines.push(JobMenuLine {
            key: "UIIT_STT_NPC_CHATTING_JOBINFO_OLD",
            fallback: "Check previous job information",
            action: JobMenuAction::PreviousInfo,
        });
    }
    lines
}

/// Send the request a clicked line stands for.
///
/// Direct send rather than a message, for the same reason `EndConversation`
/// sends its `0x704B` here: the answer (`0xB0E4` / `0xB0E6`) is consumed by
/// `plugins::net::job`, which already owns the state and opens the window —
/// a HUD-side message in between would only forward bytes.
pub fn send_job_request(
    conn: &Query<&SilkroadConnection, With<AgentConnection>>,
    npc_gid: u32,
    action: JobMenuAction,
) {
    let Ok(conn) = conn.single() else {
        return;
    };
    let packet = match action {
        JobMenuAction::Ranking {
            job_type,
            rank_kind,
        } => Packet::from(JobRankingRequest {
            npc_gid,
            job_type,
            rank_kind,
        }),
        JobMenuAction::PreviousInfo => Packet::from(JobPrevInfoRequest { npc_gid }),
        // Join and leave never take this path: they go through the
        // confirmation and `send_confirmed_job_request`.
        JobMenuAction::Join { .. } | JobMenuAction::Leave => return,
    };
    if let Err(e) = conn.get_sender().send(packet.into()) {
        error!("network: failed to send a job menu request: {}", e.0);
    }
}

// --- join / leave: confirm first, then send ---------------------------------
//
// Idea: joining costs gold and leaving locks the league for a week, so neither
// may ride on a single click. The original asks first, and its two prompts are
// *shipped strings* that already name the consequences:
// `UIIT_STT_JOBGUILD_JOIN_WINDOW` ("… Other previous job information will reset
// … You need certain gold depending on your level") and
// `UIIT_STT_JOBGUILD_WITHD_WINDOW` ("… cannot be rejoined for 7 days").
//
// **What we add on top, deliberately (ADR-0009):** the join prompt names the
// actual price. The original leaves it at "certain gold depending on your
// level" because the number lives on the server; we can state it, because the
// formula was read out of the server binary on 2026-08-23 — and a confirmation
// that names the price is strictly better than one that does not:
//
// ```text
// SR_GameServer_Clean.exe, JOIN handler @0x512100 (read as data, never run)
// 51218b  mov edx,[ebx] ; mov eax,[edx+0x124] ; call eax   ; virtual getter -> u8
// 512197  cmp al,0x14 / jae                                ; < 20 -> error 0x4819
// 5121c8  call eax                                          ; the same getter
// 5121d0  movzx eax,al ; sub eax,0x14 ; imul eax,eax,0x1388 ; (x - 20) * 5000
// 5121dd  cmp against the 64-bit gold at [[ebx+0x34]+0x78/0x7c]; short -> 0x4807
// ```
//
// `x` is the **character level**: the same vtable slot `+0x124` is called at
// `0x4e53f4` and its `u8` result compared with `0x8c = 140` — above that, the
// routine skips adding a 64-bit quantity to `[[esi+0x34]+0x68/0x6c]`, i.e. "no
// EXP at max level". Nothing else in this game is capped at 140. (Positive
// control for the whole read: every immediate `job-trade-system.md` §11.5
// publishes — `3`@512136, `0x4828`@512151, `0x14/0x15/0x16`@512169/512162/51215b,
// `0x480F`@512181, `0x4819`@51219b, `0x4820`@5121b4, `0x4807`@5121ea — came back
// at exactly those VAs.) §11.5 calls the getter `GetJobType()`; that name is
// wrong, and the `< 0x14` refusal is the level gate the client's own
// `UIIT_MSG_JOBGUILD_JOIN_ERR_LEVEL` states: "Can join the league from level 20
// or above."
//
// The acks are *not* interpreted: `0xB0E1`/`0xB0E2` are not in `packets!` at
// all (their success tail is `[U]`, Etappe 2), so the outcome shows up as the
// server's own follow-up state, and an invented "you joined!" line is exactly
// what ADR-0009 forbids.

/// Join fee in gold for a character level, or `None` below the level gate.
///
/// `(level - 20) * 5000`, from the JOIN handler (module comment above).
pub fn join_fee(level: u8) -> Option<u32> {
    if level < JOB_JOIN_MIN_LEVEL {
        return None;
    }
    Some(u32::from(level - JOB_JOIN_MIN_LEVEL) * JOB_JOIN_FEE_PER_LEVEL)
}

/// `cmp al,0x14` @0x512197 — below this the server answers error `0x4819`.
pub const JOB_JOIN_MIN_LEVEL: u8 = 20;
/// `imul eax,eax,0x1388` @0x5121d6.
pub const JOB_JOIN_FEE_PER_LEVEL: u32 = 5000;

/// Tag on the join confirmation, echoed back by `ChoiceConfirmed`.
pub const JOIN_CONFIRM_TAG: &str = "job-join";
/// Tag on the leave confirmation.
pub const LEAVE_CONFIRM_TAG: &str = "job-leave";

/// The NPC and action a raised confirmation belongs to.
///
/// A resource because the answer arrives one frame later through
/// `ChoiceConfirmed`, and the dialog may be gone by then — the request is
/// addressed by `npc_gid`, which is all we need to keep.
#[derive(Resource, Default, Debug, Clone, PartialEq, Eq)]
pub struct PendingJobRequest(pub Option<(u32, JobMenuAction)>);

/// The prompt for a join/leave confirmation: the shipped string, with the
/// `<sml2>`/`<center>`/`<br>` markup flattened (we have no PML renderer in this
/// dialog), plus the priced line on the join side.
pub fn confirm_text(action: JobMenuAction, prompt: &str, level: Option<u8>) -> String {
    let mut text = flatten_pml(prompt);
    if let JobMenuAction::Join { .. } = action {
        if let Some(fee) = level.and_then(join_fee) {
            text.push_str(&format!("\n({fee} gold)"));
        }
    }
    text
}

/// `<sml2><center>a<br>b</center></sml2>` -> `a\nb`. Deliberately minimal: it
/// only has to handle the two job strings, and a real PML renderer is its own
/// change.
fn flatten_pml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        if rest[start..start + end].eq_ignore_ascii_case("<br") {
            out.push('\n');
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Sends the join/leave request once its confirmation came back with OK.
pub fn send_confirmed_job_request(
    mut answered: MessageReader<ChoiceConfirmed>,
    mut pending: ResMut<PendingJobRequest>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    for answer in answered.read() {
        if answer.tag != JOIN_CONFIRM_TAG && answer.tag != LEAVE_CONFIRM_TAG {
            continue;
        }
        let Some((npc_gid, action)) = pending.0.take() else {
            continue;
        };
        let Ok(conn) = conn.single() else {
            return;
        };
        let packet = match action {
            JobMenuAction::Join { job_type } => Packet::from(JobJoinRequest {
                npc_gid,
                // `enroll_job_union_type` — the server maps 1/2/3 onto its
                // internal job ids 0x14/0x15/0x16 (`512142..512169`); "1 =
                // trader" is `[S]` and matches `JobType`'s own numbering.
                enroll_job_union_type: job_type,
            }),
            JobMenuAction::Leave => Packet::from(JobLeaveRequest { npc_gid }),
            // The two request-on-click actions never raise a confirmation.
            JobMenuAction::Ranking { .. } | JobMenuAction::PreviousInfo => continue,
        };
        if let Err(e) = conn.get_sender().send(packet.into()) {
            error!("network: failed to send a job join/leave request: {}", e.0);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The fee, from the JOIN handler's own arithmetic (module comment): the
    /// gate is level 20 and every level above it costs 5000 more.
    #[test]
    fn the_join_fee_is_the_level_formula_from_the_server_binary() {
        assert_eq!(join_fee(19), None, "below the 0x512197 gate");
        assert_eq!(join_fee(20), Some(0));
        assert_eq!(join_fee(21), Some(5000));
        assert_eq!(join_fee(40), Some(100_000));
        // the cap the same getter is checked against at 0x4e53f4
        assert_eq!(join_fee(140), Some(600_000));
    }

    /// A job NPC offers join and leave, and both are *confirmed* actions —
    /// neither may be a plain click-to-send.
    #[test]
    fn join_and_leave_are_offered_and_both_need_a_confirmation() {
        let lines = job_menu_lines("NPC_CA_HUNTER");
        let actions: Vec<JobMenuAction> = lines.iter().map(|l| l.action).collect();
        assert!(actions.contains(&JobMenuAction::Join { job_type: 3 }));
        assert!(actions.contains(&JobMenuAction::Leave));
        // The union bosses stay job-neutral: no join line without a job.
        for boss in ["NPC_EU_UNION1", "NPC_EU_UNION2"] {
            for line in job_menu_lines(boss) {
                assert_eq!(line.action, JobMenuAction::PreviousInfo, "{boss}");
            }
        }
    }

    /// The prompt is the shipped string, flattened, and the join side gains the
    /// price the original leaves as "certain gold depending on your level".
    #[test]
    fn the_join_prompt_carries_the_shipped_words_and_the_price() {
        let shipped = "<sml2><center>Join job league?<br>Other previous job information will \
                       reset when you join a league.</center></sml2>";
        let text = confirm_text(JobMenuAction::Join { job_type: 1 }, shipped, Some(30));
        assert!(text.starts_with("Join job league?"));
        assert!(
            text.contains("\nOther previous job information"),
            "<br> -> newline"
        );
        assert!(!text.contains('<'), "no markup left: {text}");
        assert!(text.ends_with("(50000 gold)"), "{text}");
        // No level, no price — an unknown number is not printed as 0.
        let unknown = confirm_text(JobMenuAction::Join { job_type: 1 }, shipped, None);
        assert!(!unknown.contains("gold)"), "{unknown}");
        // Leaving has no price at all, only the shipped consequence.
        let leave = confirm_text(
            JobMenuAction::Leave,
            "<sml2><center>Leave job league?<br>… cannot be rejoined for 7 days.</center></sml2>",
            Some(30),
        );
        assert!(leave.contains("7 days"));
        assert!(!leave.contains("gold)"));
    }

    /// The census from the user's own `characterdata_*.txt` × `textdata_object`
    /// (module doc). The suffix rule must hit the seven union/associate NPCs
    /// and miss every shop and exchange manager that merely *mentions* a job.
    #[test]
    fn the_suffix_rule_selects_exactly_the_union_npcs() {
        for (codename, expected) in [
            ("NPC_CA_MERCHANT", Some(JobType::Trader)),
            ("NPC_EU_MERCHANT", Some(JobType::Trader)),
            ("NPC_SD_M_AREA_MERCHANT", Some(JobType::Trader)),
            ("NPC_CA_HUNTER", Some(JobType::Hunter)),
            ("NPC_EU_HUNTER", Some(JobType::Hunter)),
            ("NPC_SD_M_AREA_HUNTER", Some(JobType::Hunter)),
            ("NPC_SD_T_AREA_THIEF", Some(JobType::Thief)),
            // "… Item Exchange manager …" — not a job menu.
            ("NPC_SD_MERCHANT_CHANGER", None),
            ("NPC_SD_THIEF_AGENT_CHANGER", None),
            // the thief den: four bandit bands and two shops.
            ("NPC_TD_THIEF_A", None),
            ("NPC_TD_THIEF_D", None),
            ("NPC_TD_THIEF_BUY", None),
            ("NPC_TD_THIEF_SELL", None),
            // and the neighbours this dialog already serves.
            ("NPC_CH_WAREHOUSE_M", None),
            ("NPC_CH_FERRY", None),
        ] {
            assert_eq!(job_of_npc(codename), expected, "{codename}");
        }
    }

    /// Traders and hunters get two ranking lines, thieves one — the §11.8
    /// absence claim, and the reason a thief NPC must not grow a third line.
    #[test]
    fn only_traders_and_hunters_have_a_second_ranking() {
        let kinds = |codename: &str| -> Vec<u8> {
            job_menu_lines(codename)
                .iter()
                .filter_map(|line| match line.action {
                    JobMenuAction::Ranking { rank_kind, .. } => Some(rank_kind),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            kinds("NPC_CA_MERCHANT"),
            vec![JOB_RANK_KIND_ACTIVITY, JOB_RANK_KIND_CONTRIBUTION]
        );
        assert_eq!(
            kinds("NPC_CA_HUNTER"),
            vec![JOB_RANK_KIND_ACTIVITY, JOB_RANK_KIND_CONTRIBUTION]
        );
        assert_eq!(kinds("NPC_SD_T_AREA_THIEF"), vec![JOB_RANK_KIND_ACTIVITY]);
        // The job type on the wire is the one `JobType` numbers.
        let trader = job_menu_lines("NPC_CA_MERCHANT");
        assert!(matches!(
            trader[0].action,
            JobMenuAction::Ranking { job_type: 1, .. }
        ));
        let hunter = job_menu_lines("NPC_CA_HUNTER");
        assert!(matches!(
            hunter[0].action,
            JobMenuAction::Ranking { job_type: 3, .. }
        ));
    }

    /// The archived-info line is job-neutral: every job NPC has it, including
    /// the two union bosses, which get *only* that line.
    #[test]
    fn the_union_bosses_get_the_job_neutral_line_only() {
        for boss in ["NPC_EU_UNION1", "NPC_EU_UNION2"] {
            let lines = job_menu_lines(boss);
            assert_eq!(lines.len(), 1, "{boss}");
            assert_eq!(lines[0].action, JobMenuAction::PreviousInfo);
            assert_eq!(lines[0].key, "UIIT_STT_NPC_CHATTING_JOBINFO_OLD");
        }
        for job_npc in ["NPC_CA_MERCHANT", "NPC_CA_HUNTER", "NPC_SD_T_AREA_THIEF"] {
            let actions: Vec<JobMenuAction> = job_menu_lines(job_npc)
                .iter()
                .map(|line| line.action)
                .collect();
            assert!(
                actions.contains(&JobMenuAction::PreviousInfo),
                "{job_npc} must offer the job-neutral line too"
            );
        }
        // Everyone else keeps the menu they had.
        assert!(job_menu_lines("NPC_CH_WAREHOUSE_M").is_empty());
        assert!(job_menu_lines("NPC_TD_THIEF_BUY").is_empty());
    }
}

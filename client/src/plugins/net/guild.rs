//! Client-side guild record: the 0x34B3 / 0x3101 / 0x34B4 chunked push folded
//! into one resource.
//!
//! Idea: `packets::agent::guild` parses the wire, but a single 0x3101 body is
//! not a record — the original accumulates every chunk between the two markers
//! and only then decodes ([`GuildData::parse`]). This module is the one place
//! that does the accumulation, exactly like `net::party` does for 0x3065.
//!
//! Deliberately minimal: it keeps the decoded record and answers two questions
//! about it (guild level, a member's permission bits). That is what the guild
//! storage consumer needs to gate itself (#744); the guild *window* — roster,
//! notice, log — is a separate piece of work (#25, #252) and nothing here
//! anticipates it.
//!
//! The 0x38F5 incremental update (`GuildUpdate`) is **not** applied: only its
//! discriminator is known and every arm's payload is `[U]`
//! (`docs/net-guild-0x3101.md`), so folding it in would mean inventing a
//! layout. A permission change therefore only lands on the next full push —
//! stated rather than papered over.

use bevy::prelude::*;

use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::textdata::ClientUiStrings;
use packets::agent::guild::{
    GuildCreatedData, GuildData, GuildDataBegin, GuildDataBody, GuildDataEnd, GuildDisbandAck,
    GuildDonateAck, GuildInviteAck, GuildKickAck, GuildKickRequest, GuildLeaveAck,
    GuildNoticeEditAck, GuildNoticeEditRequest, GuildPermissionUpdateAck, GuildPermissions,
    GuildPromoteAck,
};
use packets::agent::guild_union::{
    UnionExpelAck, UnionGuild, UnionInviteAck, UnionLeaveAck, UnionLeaveRequest, UnionRoster,
};
use packets::agent::guild_war::{GuildWarEndAck, GuildWarStartAck};
use packets::agent::ingame::GuildInviteRequest;
use packets::Packet;

use crate::plugins::net::agent::AgentConnection;

/// The local player's guild, as the server last pushed it.
#[derive(Resource, Default, Debug)]
pub struct GuildRoster {
    /// The assembled record; `None` while the player is guildless (no push).
    pub data: Option<GuildData>,
}

impl GuildRoster {
    /// Guild level — the gate `UIIT_MSG_GUILD_WAREHOUSE_LIMIT` ("Guild level
    /// must be 2 or higher to use guild storage.") talks about.
    pub fn level(&self) -> Option<u8> {
        self.data.as_ref().map(|d| d.level)
    }

    /// The permission bits the server gave this member, matched by name.
    ///
    /// Name-matched because the record carries no "this is you" marker: the
    /// roster's `member_id` is a guild-internal id, not the character id the
    /// client knows itself by. Matching is case-sensitive — SRO names are
    /// unique and the server echoes the same spelling in CHARACTER_DATA.
    pub fn permissions_for(&self, name: &str) -> Option<GuildPermissions> {
        self.data
            .as_ref()?
            .members
            .iter()
            .find(|m| m.name == name)
            .map(|m| m.permissions())
    }
}

/// The alliance (union) the local player's guild belongs to, as the server last
/// pushed it in 0x3102.
///
/// A resource of its own rather than a field of [`GuildRoster`], because the two
/// pushes are independent: 0x3102 arrives (and re-arrives) whenever the alliance
/// changes, without a guild record around it, and the original keeps them in
/// two separate maps for the same reason (`(mgr+0x148)+0x94` for the alliance,
/// `+0x88` for the members).
#[derive(Resource, Default, Debug)]
pub struct GuildUnion {
    /// `None` while the guild is in no alliance — no push has arrived.
    pub data: Option<UnionRoster>,
}

impl GuildUnion {
    /// The alliance's leading guild, looked up the way the original's header
    /// line does it: `leader_guild_id` is a **key** into the entry list, not an
    /// index — the original does `map.find` on the same map.
    pub fn leader(&self) -> Option<&UnionGuild> {
        let data = self.data.as_ref()?;
        data.guilds
            .iter()
            .find(|g| g.guild_id == data.leader_guild_id)
    }
}

/// 0x3102 — the alliance roster. Replaces the previous one wholesale: the push
/// is a full list (the original clears its map and refills it before redrawing
/// the window), so merging would keep guilds that have left.
pub fn on_union_roster(mut reader: MessageReader<UnionRoster>, mut union: ResMut<GuildUnion>) {
    for msg in reader.read() {
        info!(
            "guild: alliance {} — {} guild(s), leader id {}",
            msg.union_id,
            msg.guilds.len(),
            msg.leader_guild_id
        );
        // Louder than a silent truncation, and not a rejection: the ceiling of 8
        // is the *client window's* readout (`"%d/%d"`), the server is
        // authoritative, and a list we cannot fully draw is still a list we want
        // to see in a log.
        if msg.guilds.len() > usize::from(UnionRoster::MAX_GUILDS) {
            warn!(
                "guild: alliance carries {} guilds, above the original's ceiling of {}",
                msg.guilds.len(),
                UnionRoster::MAX_GUILDS
            );
        }
        union.data = Some(msg.clone());
    }
}

/// The 0x3101 chunks between a 0x34B3 begin and its 0x34B4 end.
#[derive(Resource, Default)]
pub struct GuildDataBuffer(pub Vec<u8>);

/// 0x34B3 — a new record starts; drop whatever a previous, truncated transfer
/// left behind.
pub fn on_guild_begin(
    mut reader: MessageReader<GuildDataBegin>,
    mut buffer: ResMut<GuildDataBuffer>,
) {
    for _ in reader.read() {
        buffer.0.clear();
    }
}

/// 0x3101 — accumulate; the record can span several packets.
pub fn on_guild_chunk(
    mut reader: MessageReader<GuildDataBody>,
    mut buffer: ResMut<GuildDataBuffer>,
) {
    for msg in reader.read() {
        buffer.0.extend_from_slice(&msg.data);
    }
}

/// 0x34B4 — the record is complete: decode it.
pub fn on_guild_end(
    mut reader: MessageReader<GuildDataEnd>,
    mut buffer: ResMut<GuildDataBuffer>,
    mut roster: ResMut<GuildRoster>,
) {
    for _ in reader.read() {
        let raw = std::mem::take(&mut buffer.0);
        if raw.is_empty() {
            warn!("guild: 0x34B4 end with no 0x3101 chunks — roster stays empty");
            continue;
        }
        match GuildData::parse(raw.clone().into()) {
            Ok(data) => {
                info!(
                    "guild: record for '{}' (level {}, {} members)",
                    data.name,
                    data.level,
                    data.members.len()
                );
                roster.data = Some(data);
            }
            Err(e) => warn!(
                "guild: record parse failed ({e:?}) — {} bytes: {} — capture for decode",
                raw.len(),
                packets::hexdump(&raw, 64)
            ),
        }
    }
}

/// 0xB0F0 — a guild the player just founded carries the same record inline,
/// so there is no push to wait for.
///
/// The refusal arm goes through the same [`guild_ack_line`] the other eleven
/// guild acks use, because 0xB0F0 has the same body shape (`u8 result`, and on
/// `result == 2` a `u16` error). Reading
/// only `msg.data` — which is what this did — made a refused founding
/// *silent*: the parsed code was dropped on the floor, so a player whose guild
/// was rejected saw nothing at all. Same defect class as the mount ack (#882)
/// and the stall ack (#884).
///
/// `ChatHistory` and `ClientUiStrings` are HUD resources and are therefore
/// `Option`: a missing `ResMut` fails Bevy's parameter validation and panics
/// the schedule instead of skipping the system, and everything under `net/**`
/// must run in the headless netcheck harness (AGENTS.md).
pub fn on_guild_created(
    mut reader: MessageReader<GuildCreatedData>,
    mut roster: ResMut<GuildRoster>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    let strings = strings.as_deref().unwrap_or(&default_strings);

    for msg in reader.read() {
        if let Some(data) = msg.data.clone() {
            info!("guild: created '{}' (level {})", data.name, data.level);
            roster.data = Some(data);
            continue;
        }
        info!(
            "guild: creation ack result={} error={:?}",
            msg.result, msg.error
        );
        // The only refusal seen so far is `02 0300` (error `0x0003`), and
        // `0x0003` does **not** fall in `0x4C06..=0x4C7A` — the space the
        // original's category-0x10 router has strings for ([`guild_error_key`]).
        // So there is no sourced
        // text for it and it is shown as the number it is; putting a GUILDERR
        // sentence on it would be inventing a mapping the original does not
        // make.
        if let Some(text) = guild_ack_line("creation", msg.result, msg.error, strings) {
            report(&mut history, text);
        }
    }
}

// --- Commands (C->S) --------------------------------------------------------
//
// Idea: the same seam `net::party` uses — the UI states an *intent* and this
// module turns it into a packet, so a button never touches the connection and
// the mapping stays testable without one.
//
// Only the three commands whose request body is fully sourced are here. Leave
// (0x70F2), disband (0x70F1) and promote (0x70FA) each carry one `u32` whose
// meaning the decompile does not give (`docs/net-guild-lifecycle.md` — the
// builder shows a width and the window accessor, not a meaning),
// so sending one would mean inventing its value. They stay out until a capture
// resolves the slot.

/// A guild command the UI wants sent.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub enum GuildAction {
    /// 0x70F3 — invite the selected player's spawn id into the guild.
    Invite(u32),
    /// 0x70F4 — expel a member, addressed **by name**: the only membership op
    /// in the family that is not id-addressed.
    Kick(String),
    /// 0x70FC — secede from the alliance. **No body**: the builder
    /// writes no field and the server accepted the empty
    /// frame — the sender's guild is inferred
    /// server-side. An empty layout is a layout, not a missing one.
    UnionLeave,
    /// 0x70F9 — replace the guild notice. Both fields are named in the
    /// request (`title` = the strip's subject, `message` = the read pane's
    /// body), so nothing here is invented; the permission gate
    /// ([`GuildPermissions::can_edit_notice`]) is checked by the window that
    /// writes this, the same way guild storage gates its own open.
    EditNotice { title: String, message: String },
}

/// The intent-to-opcode mapping, split out so it is testable without a live
/// connection — the same shape `party_action_packet` has.
pub fn guild_action_packet(action: &GuildAction) -> Packet {
    match action {
        GuildAction::Invite(unique_id) => Packet::from(GuildInviteRequest {
            unique_id: *unique_id,
        }),
        GuildAction::Kick(member_name) => Packet::from(GuildKickRequest {
            member_name: member_name.clone(),
        }),
        GuildAction::UnionLeave => Packet::from(UnionLeaveRequest),
        GuildAction::EditNotice { title, message } => Packet::from(GuildNoticeEditRequest {
            title: title.clone(),
            message: message.clone(),
        }),
    }
}

/// Turn [`GuildAction`]s into wire packets.
pub fn send_guild_actions(
    mut reader: MessageReader<GuildAction>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let mut pending = reader.read().peekable();
    if pending.peek().is_none() {
        return;
    }
    let Ok(conn) = conn.single() else {
        // Draining without a connection is the point: a queued invite must not
        // fire at a later, unrelated session.
        for action in pending {
            warn!("guild: no agent connection, dropping {action:?}");
        }
        return;
    };
    for action in pending {
        info!("guild: sending {action:?}");
        if let Err(e) = conn.get_sender().send(guild_action_packet(action).into()) {
            error!("network: failed to send guild action: {}", e.0);
        }
    }
}

// --- Refusals the player can read ------------------------------------------
//
// Idea: every ack in this family carries `{result:u8, error_code:u16}` and the
// original does not print the number — it hands the code to its system-message
// router the original's routine, which resolves it to a
// **textdata key** and shows the localized sentence. So the client never had to
// name these errors itself; it only had to keep the mapping. That mapping is
// decoded and reproducible: outer jump table
//, entry `[0x10]` →, code normalised `code - 0x4C06`,
// bounded `<= 0x74`, indexed through the 117-byte table at into the
// jump table at; each case block pushes its wide key. That is where
// the 72 rows below come from — they are a transcription of the original's own
// table, not wording we chose, which is why the strings themselves stay in the
// player's `textuisystem.txt` and only the *keys* live here.
//
// The guild error space is `0x4C06..=0x4C7A`; the 45 slots inside it that are
// missing below fall to the router's default arm (no message at all) and must
// therefore stay unnamed. Two codes deliberately share `PERMISSION_DENIED`
// (`0x4C1E` actor-side, `0x4C52` a second actor-side denial): they are distinct
// codes and are not collapsed.
//
// **Not applicable to the election family.** `0xB106`/`0xB107` report under
// category **0x15**, a different error space, and `0xB103`/`0xB252`/
// `0xB25A` never call the router at all — none of them is reported here.

/// The textdata key the original's category-0x10 router renders for a guild
/// error code, or `None` for the codes that fall to its default arm.
///
/// Positive control for the transcription: all 72 keys resolve in
/// `Media/server_dep/silkroad/textdata/textuisystem.txt` at exactly the line
/// they were read from; a fabricated key does not.
pub fn guild_error_key(code: u16) -> Option<&'static str> {
    match code {
        0x4C06 => Some("UIIT_MSG_GUILDERR_TARGET_BUSY"), // textuisystem L1399
        0x4C07 => Some("UIIT_MSG_GUILDERR_IM_DEAD"),     // textuisystem L1400
        0x4C0A => Some("UIIT_MSG_GUILDERR_TOO_LOW_CREATOR_LEVEL"), // textuisystem L1410
        0x4C0C => Some("UIIT_MSG_GUILDERR_NOT_ENOUGH_GOLD"), // textuisystem L1411
        0x4C12 => Some("UIIT_MSG_GUILDERR_EXISTING_MEMBER"), // textuisystem L1401
        0x4C13 => Some("UIIT_MSG_GUILDERR_MEMBER_FULL"), // textuisystem L1402
        0x4C16 => Some("UIIT_MSG_GUILDERR_JOIN_GUILD_REFUSED"), // textuisystem L1403
        0x4C17 => Some("UIIT_MSG_GUILDERR_MEMBER_OF_ANOTHER_GUILD"), // textuisystem L1404
        0x4C18 => Some("UIIT_MSG_GUILDERR_INVALID_GUILDNAME_LEN"), // textuisystem L1405
        0x4C19 => Some("UIIT_MSG_GUILDERR_NOT_ALLOWED_GUILDNAME"), // textuisystem L1413
        0x4C1A => Some("UIIT_MSG_GUILDERR_SAME_GUILDNAME_EXIST"), // textuisystem L1412
        0x4C1B => Some("UIIT_MSG_GUILDERR_CANT_CREATE_GUILD_IN_DB"), // textuisystem L1407
        0x4C1C => Some("UIIT_MSG_GUILDERR_CANT_ADD_MEMBER_IN_DB"), // textuisystem L1408
        0x4C1D => Some("UIIT_MSG_GUILDERR_CANT_FIND_MEMBER"), // textuisystem L1409
        0x4C1E => Some("UIIT_MSG_GUILDERR_PERMISSION_DENIED"), // textuisystem L1406
        0x4C1F => Some("UIIT_MSG_GUILD_EXPULSION_NOTEXP"), // textuisystem L1350
        0x4C21 => Some("UIIT_MSG_ERROR_GUILD_LEVEL_UP_FULL"), // textuisystem L1312
        0x4C22 => Some("UIIT_MSG_GUILDERR_INVALID_MASTER_COMMENT_TITLE"), // textuisystem L1277
        0x4C23 => Some("UIIT_MSG_GUILDERR_INVALID_MASTER_COMMENT"), // textuisystem L1278
        0x4C24 => Some("UIIT_MSG_GUILDERR_TARGET_PERMISSION_DENIED"), // textuisystem L1279
        0x4C25 => Some("UIIT_MSG_GUILDERR_CANT_FIND_TARGET_GUILD"), // textuisystem L2815
        0x4C26 => Some("UIIT_MSG_GUILDERR_CANT_ALLY_TO_OWN_GUILD"), // textuisystem L1290
        0x4C27 => Some("UIIT_MSG_GUILDERR_ALREADY_ALLIED"), // textuisystem L1291
        0x4C28 => Some("UIIT_MSG_GUILDERR_TARGET_GUILD_HAS_ALLIANCE_ALREADY"), // textuisystem L1292
        0x4C29 => Some("UIIT_MSG_GUILDERR_ALLIANCE_FULL"), // textuisystem L1293
        0x4C2A => Some("UIIT_MSG_GUILDERR_TOO_LOW_CREATOR_ALLIANCE"), // textuisystem L1294
        0x4C2B => Some("UIIT_MSG_GUILDERR_TOO_LOW_JOINER_ALLIANCE"), // textuisystem L1295
        0x4C31 => Some("UIIT_MSG_ERROR_GUILD_LEVEL_UP_GOLD_DEFICIT"), // textuisystem L1311
        0x4C32 => Some("UIIT_MSG_ERROR_GUILD_LEVEL_UP_GP_DEFICIT"), // textuisystem L1310
        0x4C36 => Some("UIIT_MSG_GUILD_EXPULSION_NOTEXP"), // textuisystem L1350
        0x4C38 => Some("UIIT_MSG_MRELEASEERR_NOTSECEDE"), // textuisystem L2783
        0x4C39 => Some("UIIT_MSG_MRELEASEERR_NOTREMOVE"), // textuisystem L2784
        0x4C3A => Some("UIIT_MSG_GUILDWARERR_ENEMYGUILD"), // textuisystem L2799
        0x4C3B => Some("UIIT_MSG_GUILDERR_TARGET_GUILD_MASTER_IS_NOT_IN_GAME"), // textuisystem L2816
        0x4C3C => Some("UIIT_MSG_GUILDWARERR_GUILD_CREATE_PENALTY"), // textuisystem L2845
        0x4C3D => Some("UIIT_MSG_GUILD_PENALTY"),                    // textuisystem L1500
        0x4C3E => Some("UIIT_MSG_GUILDWARERR_MAX_HOSTILE_GUILD"),    // textuisystem L2847
        0x4C3F => Some("UIIT_MSG_GUILDWARERR_TARGET_MAX_HOSTILE_GUILD"), // textuisystem L2848
        0x4C41 => Some("UIIT_MSG_GUILDWARERR_ALLYGUILD"),            // textuisystem L2820
        0x4C42 => Some("UIIT_MSG_QUESTION_GUILD_RESPECT_ALLY_NOTALLY"), // textuisystem L1285
        0x4C43 => Some("UIIT_MSG_GUILDWAR_SUGGESTIONS_02"),          // textuisystem L2822
        0x4C44 => Some("UIIT_MSG_PARTYERR_UNKNOWN_ERROR"),           // textuisystem L2064
        0x4C46 => Some("UIIT_MSG_GUILD_ERROR_BREAK_WAR"),            // textuisystem L1395
        0x4C48 => Some("UIIT_MSG_GUILD_WAREHOUSE_INTERRUPT"),        // textuisystem L1428
        0x4C4A => Some("UIIT_MSG_GUILD_WAREHOUSE_LIMIT"),            // textuisystem L1424
        0x4C4B => Some("UIIT_MSG_GUILD_UNION_CHAT_FULL"),            // textuisystem L1418
        0x4C4C => Some("UIIT_MSG_GUILD_WAREHOUSE_USE_ING"),          // textuisystem L1427
        0x4C4D => Some("UIIT_MSG_GUILD_ERROR_NAME_GRANT_SPECIAL_LETTER"), // textuisystem L1365
        0x4C52 => Some("UIIT_MSG_GUILDERR_PERMISSION_DENIED"),       // textuisystem L1406
        0x4C53 => Some("UIIT_MSG_GUILD_SOLDIER_ABILITY_OVER"),       // textuisystem L1482
        0x4C54 => Some("UIIT_MSG_GUILD_SOLDIER_ABILITY_SELECT_ERROR"), // textuisystem L1484
        0x4C55 => Some("UIIT_MSG_GUILD_SOLDIER_LEAVE_MASTER_ERROR"), // textuisystem L1485
        0x4C56 => Some("UIIT_CTL_GUILD_JOIN_JOB_ERROR"),             // textuisystem L1375
        0x4C57 => Some("UIIT_CTL_GUILD_CREATE_JOB_ERROR"),           // textuisystem L1374
        0x4C58 => Some("UIIT_MSG_FORT_POSITION_GRANT_FAIL_04"),      // textuisystem L3609
        0x4C59 => Some("UIIT_MSG_FORT_POSITION_GRANT_FAIL_03"),      // textuisystem L3608
        0x4C5A => Some("UIIT_MSG_FORT_POSITION_GRANT_FAIL_02"),      // textuisystem L3607
        0x4C5B => Some("UIIT_MSG_FORT_POSITION_GRANT_FAIL_05"),      // textuisystem L3610
        0x4C5C => Some("UIIT_MSG_FORT_POSITION_GRANT_FAIL_06"),      // textuisystem L3611
        0x4C5D => Some("UIIT_MSG_FORT_POSITION_GRANT_FAIL_01"),      // textuisystem L3606
        0x4C5E => Some("UIIT_MSG_GUILDWARERR_LIMIT_COMPENSATION"),   // textuisystem L2850
        0x4C60 => Some("UIIT_STT_ERR_COMMON_TOO_FAR"),               // textuisystem L2111
        0x4C70 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C71 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C72 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C73 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C74 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C75 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C76 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C77 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C78 => Some("UIIT_MSG_FORT_STRUCTURE_ACTION_ERROR_06"),   // textuisystem L3618
        0x4C7A => Some("UIIT_MGS_ARENA_ERR_FREQUEST_WAR_IN_BATTLE_ARENA"), // textuisystem L4105
        _ => None,
    }
}

/// The line an ack turns into, or `None` when it should stay silent.
///
/// Success is silent by construction: the original's success arms do not print
/// (the roster change arrives out of band), so a chat line per accepted
/// command would be our invention. The one exception is the notice edit, whose
/// success arm *does* show `UIIT_MSG_GUILD_COMMON_KNOW_REMIND_UPDATE`
/// (`packets/src/agent/guild.rs:328`), and it is passed in by the caller.
///
/// `result` values other than 1 and 2 are neither success nor failure — the
/// original silently ignores them, and so does this.
fn guild_ack_line(
    verb: &str,
    result: u8,
    error_code: Option<u16>,
    strings: &ClientUiStrings,
) -> Option<String> {
    if result == 1 {
        return None;
    }
    let code = error_code?;
    Some(match guild_error_key(code) {
        Some(key) => strings.get_plain_or(key, &fallback_wording(verb, code)),
        // An unnamed code keeps its number rather than borrowing a neighbour's
        // sentence: the router has no message for it either.
        None => fallback_wording(verb, code),
    })
}

/// What a refusal reads as when the original has no sentence for it. **Ours** —
/// the number is the only sourced part, so it is the part that is shown.
fn fallback_wording(verb: &str, code: u16) -> String {
    format!("Guild {verb} refused (error {code:#06x}).")
}

/// Push one line into the chat log when there is one; see `net/party.rs` for
/// why the HUD resource is optional in a `net` system.
fn report(history: &mut Option<ResMut<ChatHistory>>, text: String) {
    match history {
        Some(history) => history.push(ChatLine::system(text)),
        None => info!("guild (headless): {text}"),
    }
}

/// Drain one ack reader into the chat log. A macro rather than a generic
/// because the eleven ack types share a body *shape*, not a trait — the shape
/// is generated per opcode by `guild_op_ack!` so that the opcode table can map
/// one opcode to one type (`packets/src/agent/guild.rs:255`).
macro_rules! report_guild_acks {
    ($history:expr, $strings:expr, $( $reader:ident => $verb:expr ),+ $(,)?) => {
        $(
            for ack in $reader.read() {
                info!(
                    "guild: {} ack result={} error={:?}",
                    $verb, ack.result, ack.error_code
                );
                if let Some(text) = guild_ack_line($verb, ack.result, ack.error_code, $strings) {
                    report(&mut $history, text);
                }
            }
        )+
    };
}

/// The membership half of the ack cluster: 0xB0F1 / 0xB0F2 / 0xB0F3 / 0xB0F4 /
/// 0xB0F6 / 0xB0F9 / 0xB0FA.
///
/// Split from [`on_guild_authority_acks`] only to keep each system's parameter
/// list short — Bevy's system tuples have a hard arity ceiling and a system
/// that crosses it fails to compile without naming a type.
#[allow(clippy::too_many_arguments)] // a Bevy system's params; the tree's convention (`scenes/game_scene.rs:381`)
pub fn on_guild_membership_acks(
    mut disbands: MessageReader<GuildDisbandAck>,
    mut leaves: MessageReader<GuildLeaveAck>,
    mut invites: MessageReader<GuildInviteAck>,
    mut kicks: MessageReader<GuildKickAck>,
    mut donations: MessageReader<GuildDonateAck>,
    mut notices: MessageReader<GuildNoticeEditAck>,
    mut promotions: MessageReader<GuildPromoteAck>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    let strings = strings.as_deref().unwrap_or(&default_strings);

    // The notice edit is the one command whose *success* the original reports.
    for ack in notices.read() {
        info!(
            "guild: notice ack result={} error={:?}",
            ack.result, ack.error_code
        );
        let line = if ack.result == 1 {
            Some(
                strings
                    .get_plain_or(NOTICE_UPDATED_KEY, "The guild notice has been updated.")
                    .to_string(),
            )
        } else {
            guild_ack_line("notice edit", ack.result, ack.error_code, strings)
        };
        if let Some(text) = line {
            report(&mut history, text);
        }
    }

    report_guild_acks!(
        history,
        strings,
        disbands => "disband",
        leaves => "secession",
        invites => "invitation",
        kicks => "expulsion",
        donations => "donation",
        promotions => "promotion",
    );
}

/// `UIIT_MSG_GUILD_COMMON_KNOW_REMIND_UPDATE` — the notice edit's success
/// message in the original (`packets/src/agent/guild.rs:328`).
const NOTICE_UPDATED_KEY: &str = "UIIT_MSG_GUILD_COMMON_KNOW_REMIND_UPDATE";

/// The authority/relations half: 0xB104 (permissions), the three union acks
/// 0xB0FB / 0xB0FC / 0xB0FD, and the two war acks 0xB110 / 0xB112.
///
/// The three union acks are the only ones in this file with a body off the
/// wire behind them: 0xB0FC, 0xB0FD and 0xB104 all read `02 0d4c` — result 2,
/// error `0x4C0D` — from a guildless character, which is exactly the shape this
/// reader decodes.
/// `0x4C0D` itself is one of the router's unnamed slots, so those three lines
/// come out as the number: the honest rendering of a code the original has no
/// sentence for either.
#[allow(clippy::too_many_arguments)] // a Bevy system's params; the tree's convention (`scenes/game_scene.rs:381`)
pub fn on_guild_authority_acks(
    mut permissions: MessageReader<GuildPermissionUpdateAck>,
    mut union_invites: MessageReader<UnionInviteAck>,
    mut union_leaves: MessageReader<UnionLeaveAck>,
    mut union_expels: MessageReader<UnionExpelAck>,
    mut war_starts: MessageReader<GuildWarStartAck>,
    mut war_ends: MessageReader<GuildWarEndAck>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    let default_strings = ClientUiStrings::default();
    let strings = strings.as_deref().unwrap_or(&default_strings);

    report_guild_acks!(
        history,
        strings,
        permissions => "authority change",
        union_invites => "alliance proposal",
        union_leaves => "alliance secession",
        union_expels => "alliance expulsion",
        war_starts => "war declaration",
        war_ends => "war conclusion",
    );
}

/// The guild record consumer. Registered next to [`crate::plugins::net::party`]
/// because it is wire-state, not a window.
pub struct GuildPlugin;

impl Plugin for GuildPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GuildRoster>()
            .init_resource::<GuildUnion>()
            .init_resource::<GuildDataBuffer>()
            .add_systems(
                Update,
                (
                    // chained for the same reason the storage push is
                    // (`hud::storage::mod`): begin/chunk/end land in ONE frame
                    // and an unordered tuple lets `end` decode an empty buffer
                    (on_guild_begin, on_guild_chunk, on_guild_end).chain(),
                    on_guild_created,
                    on_union_roster,
                    send_guild_actions,
                    on_guild_membership_acks,
                    on_guild_authority_acks,
                ),
            )
            .add_message::<GuildAction>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use packets::agent::guild::GuildMember;

    fn member(name: &str, permissions: u32) -> GuildMember {
        GuildMember {
            member_id: 7,
            name: name.to_string(),
            unk_u8_01: 0,
            level: 40,
            guild_points: 0,
            permissions,
            unk_u32_01: 0,
            unk_u32_02: 0,
            unk_u32_03: 0,
            nickname: String::new(),
            model_id: 1907,
            is_master: permissions == GuildPermissions::MASTER,
            // Present in a live guild record; its meaning is open.
            unk_u8_02: 0,
            is_offline: false,
        }
    }

    /// 0x3102 lands in `GuildUnion`, and the leader is found by **key**: the
    /// leading guild is the last entry here, so a parser that treated
    /// `leader_guild_id` as an index (the easy misreading — it sits exactly
    /// where a count would) would name the wrong guild.
    #[test]
    fn the_alliance_push_lands_and_the_leader_is_keyed_not_indexed() {
        let mut app = App::new();
        app.init_resource::<GuildUnion>()
            .add_message::<UnionRoster>()
            .add_systems(Update, on_union_roster);
        app.world_mut().write_message(UnionRoster {
            union_id: 4,
            union_crest_rev: 0,
            leader_guild_id: 78,
            count: 2,
            guilds: vec![
                union_guild(77, "OpenRoad", "Mira"),
                union_guild(78, "Roadmen", "Grunt"),
            ],
        });
        app.update();

        let union = app.world().resource::<GuildUnion>();
        assert_eq!(union.data.as_ref().unwrap().guilds.len(), 2);
        assert_eq!(union.leader().unwrap().guild_name, "Roadmen");
        assert_eq!(union.leader().unwrap().master_name, "Grunt");
    }

    /// A second push replaces the first wholesale — merging would keep a guild
    /// that has left the alliance on screen forever, and the original refills
    /// its map before redrawing.
    #[test]
    fn a_second_alliance_push_replaces_the_first() {
        let mut app = App::new();
        app.init_resource::<GuildUnion>()
            .add_message::<UnionRoster>()
            .add_systems(Update, on_union_roster);
        for guilds in [
            vec![
                union_guild(77, "OpenRoad", "Mira"),
                union_guild(78, "Roadmen", "Grunt"),
            ],
            vec![union_guild(77, "OpenRoad", "Mira")],
        ] {
            app.world_mut().write_message(UnionRoster {
                union_id: 4,
                union_crest_rev: 0,
                leader_guild_id: 77,
                count: guilds.len() as u8,
                guilds,
            });
            app.update();
        }
        assert_eq!(
            app.world()
                .resource::<GuildUnion>()
                .data
                .as_ref()
                .unwrap()
                .guilds
                .len(),
            1,
            "the second push must not be merged into the first"
        );
    }

    /// A leader id that names no entry leaves the header blank rather than
    /// falling back to entry 0 — the disagreement should be visible.
    #[test]
    fn an_unknown_leader_id_resolves_to_nothing() {
        let union = GuildUnion {
            data: Some(UnionRoster {
                union_id: 1,
                union_crest_rev: 0,
                leader_guild_id: 999,
                count: 1,
                guilds: vec![union_guild(77, "OpenRoad", "Mira")],
            }),
        };
        assert!(union.leader().is_none());
    }

    fn union_guild(guild_id: u32, name: &str, master: &str) -> UnionGuild {
        UnionGuild {
            guild_id,
            guild_name: name.to_string(),
            // Both unnamed: display-only bytes, see the type docs.
            unknown_a: 0,
            master_name: master.to_string(),
            master_object_id: 0,
            unknown_b: 0,
        }
    }

    fn record(level: u8, members: Vec<GuildMember>) -> GuildData {
        GuildData {
            guild_id: 42,
            name: "Wanderers".into(),
            level,
            guild_points: 0,
            notice: String::new(),
            message: String::new(),
            unk_u32_00: 0,
            unk_u8_00: 0,
            member_count: members.len() as u8,
            members,
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(GuildPlugin);
        app.add_message::<GuildDataBegin>()
            .add_message::<GuildDataBody>()
            .add_message::<GuildDataEnd>()
            .add_message::<GuildCreatedData>()
            // the ack consumers' inputs: in the real app these come from the
            // `packets!` registration, which a bare test App does not run —
            // and Bevy 0.19 *panics* a system whose `MessageReader` type was
            // never registered, so the list has to be complete
            .add_message::<GuildDisbandAck>()
            .add_message::<GuildLeaveAck>()
            .add_message::<GuildInviteAck>()
            .add_message::<GuildKickAck>()
            .add_message::<GuildDonateAck>()
            .add_message::<GuildNoticeEditAck>()
            .add_message::<GuildPromoteAck>()
            .add_message::<GuildPermissionUpdateAck>()
            .add_message::<UnionInviteAck>()
            .add_message::<UnionLeaveAck>()
            .add_message::<UnionExpelAck>()
            // Rule 8c: a `MessageReader` on a type this list forgets panics the
            // whole test app in Bevy 0.19 ("Message not initialized") — the
            // production path gets it from `packets!`, a test App does not.
            // Adding 0x3102's reader without this line reddened six unrelated
            // tests in this module.
            .add_message::<UnionRoster>()
            .add_message::<GuildWarStartAck>()
            .add_message::<GuildWarEndAck>();
        app
    }

    /// The intent-to-opcode mapping, pinned by the encoded bytes rather than by
    /// the type name: invite is the 4-byte id form, expel is the name form,
    /// the notice edit is the two-string form.
    #[test]
    fn the_three_commands_encode_as_their_sourced_bodies() {
        let invite = guild_action_packet(&GuildAction::Invite(0x2A)).into_serialize();
        assert_eq!(invite.0, 0x70F3);
        assert_eq!(invite.1.as_ref(), &[0x2A, 0x00, 0x00, 0x00]);

        let kick = guild_action_packet(&GuildAction::Kick("Grunt".into())).into_serialize();
        assert_eq!(kick.0, 0x70F4);
        assert_eq!(kick.1.as_ref(), b"\x05\x00Grunt");

        // Field order is the request's own (`title` then `message`); a swap
        // here would put the body in the strip and the subject in the pane on
        // every other client in the guild.
        let notice = guild_action_packet(&GuildAction::EditNotice {
            title: "Raid".into(),
            message: "Tonight".into(),
        })
        .into_serialize();
        assert_eq!(notice.0, 0x70F9);
        assert_eq!(notice.1.as_ref(), b"\x04\x00Raid\x07\x00Tonight");
    }

    /// Without a connection the queue must drain rather than hold an invite
    /// that would fire into a later, unrelated session.
    #[test]
    fn actions_are_dropped_when_there_is_no_connection() {
        let mut app = app();
        app.add_message::<GuildAction>()
            .add_systems(Update, send_guild_actions);
        app.world_mut().write_message(GuildAction::Invite(7));
        app.update();
        app.update();
        // nothing to assert beyond "it did not panic without a connection" —
        // the point is the drain path, which a queued message would survive.
        assert!(app
            .world()
            .resource::<Messages<GuildAction>>()
            .iter_current_update_messages()
            .next()
            .is_none());
    }

    /// The record only exists concatenated: a two-packet split must decode to
    /// the same roster a single packet would. This is the defect
    /// `net-storage-0x3047-0x3049.md:162-165` records for the personal family
    /// — an unordered/unbuffered consumer decodes "0 bytes" instead.
    #[test]
    fn a_guild_record_split_across_two_chunks_decodes_as_one() {
        let wire: Bytes = record(
            2,
            vec![
                member("Master", GuildPermissions::MASTER),
                member("Grunt", GuildPermissions::JOIN | GuildPermissions::STORAGE),
            ],
        )
        .into();
        let (head, tail) = wire.split_at(wire.len() / 2);

        let mut app = app();
        app.world_mut().write_message(GuildDataBegin);
        app.world_mut().write_message(GuildDataBody {
            data: Bytes::copy_from_slice(head),
        });
        app.world_mut().write_message(GuildDataBody {
            data: Bytes::copy_from_slice(tail),
        });
        app.world_mut().write_message(GuildDataEnd);
        app.update();

        let roster = app.world().resource::<GuildRoster>();
        assert_eq!(roster.level(), Some(2));
        assert_eq!(
            roster.permissions_for("Grunt").map(|p| p.can_use_storage()),
            Some(true)
        );
        // the master sentinel is every bit set, not just the named ones
        assert_eq!(
            roster
                .permissions_for("Master")
                .map(|p| p.can_use_storage()),
            Some(true)
        );
        // a name that is not in the roster is not "no permissions", it is
        // "not a member" — the storage gate distinguishes the two
        assert!(roster.permissions_for("Stranger").is_none());
    }

    /// A member without the `Storage` bit reads as refused rather than as an
    /// absent roster — the two produce different messages at the gate.
    #[test]
    fn a_member_without_the_storage_bit_is_refused_not_unknown() {
        let wire: Bytes = record(3, vec![member("Rookie", GuildPermissions::JOIN)]).into();
        let mut app = app();
        app.world_mut().write_message(GuildDataBegin);
        app.world_mut().write_message(GuildDataBody { data: wire });
        app.world_mut().write_message(GuildDataEnd);
        app.update();

        let perms = app
            .world()
            .resource::<GuildRoster>()
            .permissions_for("Rookie")
            .expect("member is in the roster");
        assert!(!perms.can_use_storage());
    }

    // --- Ack reporting ------------------------------------------------------

    /// An app with a chat log and the two ack readers, but **no**
    /// `ClientUiStrings` — that is the interesting case twice over: it proves
    /// the `Option<Res<_>>` degradation the netcheck harness needs, and it is
    /// the arm that shows the raw code.
    fn ack_app() -> App {
        let mut app = app();
        app.init_resource::<ChatHistory>();
        app
    }

    fn chat_lines(app: &App) -> Vec<String> {
        app.world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| line.display())
            .collect()
    }

    /// The transcription is the router's, not ours: every named code sits
    /// inside the range `0x4C06..=0x4C7A`, there are exactly 72 of them, and
    /// the slots that fall to the router's default arm stay unnamed. The last part is the one that matters — a table that
    /// answered *every* code would be a table we invented.
    #[test]
    fn the_error_table_is_the_routers_own_range_and_no_wider() {
        let named: Vec<u16> = (0x0000..=0xFFFFu16)
            .filter(|c| guild_error_key(*c).is_some())
            .collect();
        assert_eq!(named.len(), 72);
        assert!(named.iter().all(|c| (0x4C06..=0x4C7A).contains(c)));
        // the router's default-arm slots, which stay unnamed
        for code in [0x4C08, 0x4C0D, 0x4C33, 0x4C45, 0x4C79] {
            assert!(
                guild_error_key(code).is_none(),
                "{code:#06x} must stay unnamed"
            );
        }
        // two distinct codes deliberately share one message
        assert_eq!(guild_error_key(0x4C1E), guild_error_key(0x4C52));
        assert_eq!(
            guild_error_key(0x4C52),
            Some("UIIT_MSG_GUILDERR_PERMISSION_DENIED")
        );
    }

    /// The refusal a guildless character gets, byte for byte: `020d4c`.
    /// It must decode as
    /// `result 2 / 0x4C0D` and reach the chat log — and because `0x4C0D` is one
    /// of the router's unnamed slots, it must come out as the **number**, not
    /// as a neighbouring sentence.
    #[test]
    fn the_union_refusal_reaches_the_chat_log_as_its_code() {
        let wire = Bytes::from_static(&[0x02, 0x0D, 0x4C]);
        let ack = UnionLeaveAck::try_from(wire).expect("the body decodes");
        assert_eq!((ack.result, ack.error_code), (2, Some(0x4C0D)));

        let mut app = ack_app();
        app.world_mut().write_message(ack);
        app.update();

        let lines = chat_lines(&app);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("0x4c0d"), "{}", lines[0]);
        assert!(lines[0].contains("alliance secession"), "{}", lines[0]);
    }

    /// The **creation** refusal, byte for byte: `020300` — the answer to a
    /// `0x70F0` sent with no NPC dialogue open. It must decode as
    /// `result 2 / 0x0003` **and reach the chat log**.
    ///
    /// Red control for this lane: `on_guild_created` used to read only
    /// `msg.data` and returned without touching the failure arm, so this
    /// assertion had nothing to read — the model could represent the code, but
    /// nobody looked at it. Deleting the `guild_ack_line` call restores the
    /// silence and fails exactly here.
    ///
    /// `0x0003` is outside `0x4C06..=0x4C7A`, so [`guild_error_key`] has no
    /// name for it and the number itself is what the player sees.
    #[test]
    fn the_creation_refusal_reaches_the_chat_log_as_its_code() {
        let wire = Bytes::from_static(&[0x02, 0x03, 0x00]);
        let ack = GuildCreatedData::try_from(wire).expect("the body decodes");
        assert_eq!((ack.result, ack.error), (2, Some(0x0003)));
        assert!(ack.data.is_none());
        assert!(guild_error_key(0x0003).is_none(), "not a GUILDERR code");

        let mut app = ack_app();
        app.world_mut().write_message(ack);
        app.update();

        let lines = chat_lines(&app);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("0x0003"), "{}", lines[0]);
        assert!(lines[0].contains("creation"), "{}", lines[0]);
        assert!(
            app.world().resource::<GuildRoster>().data.is_none(),
            "a refusal must not install a roster"
        );
    }

    /// The same arm with no HUD at all — the headless netcheck shape. A missing
    /// `ChatHistory`/`ClientUiStrings` must degrade to a log line, not fail
    /// Bevy's parameter validation and panic the schedule (AGENTS.md).
    #[test]
    fn a_refused_creation_without_a_chat_log_does_not_panic() {
        let mut app = app();
        app.world_mut().write_message(GuildCreatedData {
            result: 2,
            data: None,
            error: Some(0x0003),
            tail: Bytes::new(),
        });
        app.update();
    }

    /// Success is silent, because the original's success arms are silent —
    /// and so is a `result` that is neither 1 nor 2, which the
    /// original ignores outright.
    #[test]
    fn an_accepted_command_says_nothing_and_neither_does_a_third_result() {
        let mut app = ack_app();
        app.world_mut().write_message(GuildKickAck {
            result: 1,
            error_code: None,
        });
        app.world_mut().write_message(GuildPermissionUpdateAck {
            result: 7,
            error_code: None,
        });
        app.update();
        assert!(chat_lines(&app).is_empty());
    }

    /// The one documented exception: the notice edit reports its **success**.
    /// Without a loaded `textuisystem.txt` the fallback wording shows, which is
    /// exactly what a preview scene sees.
    #[test]
    fn the_notice_edit_is_the_one_ack_that_reports_success() {
        let mut app = ack_app();
        app.world_mut().write_message(GuildNoticeEditAck {
            result: 1,
            error_code: None,
        });
        app.update();
        let lines = chat_lines(&app);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("notice"), "{}", lines[0]);
    }

    /// A named code resolves to a key that the player's own textdata carries.
    /// The lookup itself cannot be exercised here (`ClientUiStrings` has no
    /// public constructor from rows), so the assertion is on the key. All 72
    /// keys exist in `textuisystem.txt`.
    #[test]
    fn a_named_code_carries_the_originals_key() {
        assert_eq!(
            guild_error_key(0x4C4A),
            Some("UIIT_MSG_GUILD_WAREHOUSE_LIMIT")
        );
        assert_eq!(
            guild_error_key(0x4C06),
            Some("UIIT_MSG_GUILDERR_TARGET_BUSY")
        );
    }
}

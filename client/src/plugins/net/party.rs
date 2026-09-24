//! Client-side party roster: 0x3065's full list plus 0x3864's deltas, folded
//! into one resource.
//!
//! Idea: the whole party wire family is already parsed in
//! `packets::agent::party`, but nothing client-side kept it, so every consumer
//! (the world-map markers, the minimap signs, the party window) would have to
//! fold the full-roster packet and its five delta types itself. This module is
//! the single place that does it, and it keeps the wire shapes verbatim: a
//! member is stored as the wire's own [`PartyMemberCore`], and `hp_mp` stays the
//! raw byte the server sent ([`PartyHpMp`] packs both bars into nibbles).
//! Turning that byte into percentages is the view's job — the roster gauges are
//! a crop, not a stretch, so quantisation belongs where it is drawn, not here.
//!
//! # Records are partial, so the fold MERGES
//!
//! Every party record on the wire begins with a presence bitmask and carries
//! only the fields whose bit is set (see `packets::agent::party`'s module note).
//! A full roster push names everything; a 0x3864 delta typically names one
//! field. So a stored member is the **accumulation** of every record seen for
//! that id, not the last one: overwriting the record with a two-byte hp/mp delta
//! would blank the member's name, level, guild and position. `presence` is
//! OR-ed as it goes, so it always says what we actually know about a member.
//!
//! The leader is now modelled — `PartyData`'s header bit 0 carries
//! `master_join_id`, which the original compares against the local player's own
//! JID. That used to be an UNKNOWN here because the header was nine opaque
//! bytes; the original resolved it, and the wire agrees — a roster names the
//! creator's jid there. A 0x3864 **type 9** re-points it when the master
//! changes: the handler reads one `u32` and `_swprintf_s`es it into a notice
//! line, so that type is not bodyless.
//!
//! # The vitals byte is not a percentage
//!
//! `hp_mp` stays the raw byte, and the conversion lives in `PartyHpMp` —
//! **asymmetrically**, because the original's own setter is: HP is 1-based with
//! nibble 0 meaning *dead* and fills `(n-1)/9`, MP is a plain decile `m/10`
//! . A real vitals frame carries an HP nibble of 11, which a `nibble * 10`
//! reading turns into 110 %. `PartyHpMp::{hp_fill,mp_fill}` hand out the exact fraction and
//! the rounding happens at the pixel, because the roster gauges are a crop of
//! fixed art, not a stretch.
//!
//! # Our own jid has three sources
//!
//! [`PartyRoster::local_member_id`] is what the "am I the master?" test needs,
//! and the two acks that carry it (0xB060 create, 0xB067 join) only answer an
//! action *we* took — so the name match against [`CharacterInfo`] stays as the
//! fallback for the roster that arrives unasked after a relog.
//!
//! The second half of the file is the outbound side: the four verbs the player
//! can actually trigger (invite / create / leave / kick) and the two acks that
//! answer them. See the section comment there for why one message covers both
//! invite opcodes.

use bevy::prelude::*;

use packets::agent::ingame::PartyInviteRequest;
use packets::agent::party::{
    PartyCreateResponse, PartyCreationRequest, PartyData, PartyInviteResponse, PartyJoinResponse,
    PartyKickRequest, PartyLeave, PartyMemberCore, PartySetup, PartyUpdate,
};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::character_info::CharacterInfo;

/// The party the local player is in. Empty when there is no party.
#[derive(Resource, Default, Debug)]
pub struct PartyRoster {
    /// The server's id for this party. 0 when there is none.
    pub party_number: u32,
    /// The leader's JID, when the server has told us — `PartyData`'s header bit
    /// 0 carries it, and a roster-only push does not.
    pub master_join_id: Option<u32>,
    /// `SRParty.Setup` bitfield (EXP/item sharing, invite permission). Drives
    /// the party window's mode readout and the 4-vs-8 capacity.
    pub setup: PartySetup,
    /// Every member the server listed, in wire order. The local player is one
    /// of them — 0x3065 sends the full party, not "the others".
    pub members: Vec<PartyMemberCore>,
    /// **Our own** party member jid, `0` while it is unknown.
    ///
    /// The original keeps exactly this: a fixed slot (the party
    /// object's `this+0x38`) holds the local player's jid, and the 0x3065
    /// handler compares the wire's master jid against it to set a one-byte
    /// "I am the master" flag at `this+0x2d`
    /// ->. That flag is what gates the client's
    /// own kick path: the `/BanishFromParty` branch of calls
    ///, nine bytes reading `partyInfo+0x0d` — i.e. `this+0x2d` —
    /// before it looks the target up at all. So a master-only kick is the
    /// original's behaviour, not a precaution we invented; see
    /// [`PartyRoster::is_local_master`].
    ///
    /// Three sources, in falling order of authority, because the two acks only
    /// answer an action *we* took:
    /// 1. 0xB060's success `u32` — the party we formed;
    /// 2. 0xB067's success `u32` — the party we joined (see
    ///    [`PartyJoinResponse`]);
    /// 3. a name match against `CharacterInfo` on every 0x3065 — the fallback
    ///    that stays because 0x3065 also arrives *unasked*: logging back into a
    ///    party we were already in produces a roster with no ack in front of it,
    ///    and then neither (1) nor (2) ever fires.
    ///
    /// Whoever writes first wins: an id we already hold is never overwritten
    /// within a party, and it is cleared when the party is dismissed.
    pub local_member_id: u32,
}

impl PartyRoster {
    /// Whether the local player is in a party at all.
    pub fn is_active(&self) -> bool {
        !self.members.is_empty()
    }

    /// How many members this party can hold — a *derived* property of the
    /// EXP-share bit, not a separate wire field.
    pub fn capacity(&self) -> u8 {
        self.setup.capacity()
    }

    /// Whether `member_id` leads this party. `false` when the leader is
    /// unknown, which is the honest answer: no crown is better than a wrong one.
    pub fn is_leader(&self, member_id: u32) -> bool {
        self.master_join_id == Some(member_id)
    }

    /// Are **we** the party master? The client's own precondition for kicking
    /// (see [`PartyRoster::local_member_id`]), and deliberately false while our
    /// own jid is unknown: "we might be the master" is not a reason to expel
    /// somebody else.
    pub fn is_local_master(&self) -> bool {
        self.local_member_id != 0 && self.is_leader(self.local_member_id)
    }

    /// Learn our own jid from an ack that carries it.
    ///
    /// One function for both acks on purpose: 0xB060 (party formed) and 0xB067
    /// (party joined) have the *same* two-armed body and their success `u32` is
    /// the same field in the original — `partyInfo+0x18`, the one its own master
    /// test reads as our jid, the local RE notes
/// §0xB060/§0xB067). Splitting that into two nearly identical handlers is
    /// how the create side ended up authoritative and the join side a name
    /// guess; there is now one read and one place to reason about.
    fn learn_local_member_from_ack(&mut self, result: u8, join_id: Option<u32>) {
        if result != 1 {
            return;
        }
        if let Some(jid) = join_id.filter(|jid| *jid != 0) {
            self.local_member_id = jid;
        }
    }

    /// Learn our own jid from a full roster by name — the *fallback*.
    ///
    /// Why it survives now that 0xB067 is read: 0x3065 is also pushed without
    /// any request of ours. Logging into a character that is already in a party
    /// delivers the roster and no ack at all, so an ack-only client would sit
    /// with `local_member_id == 0` — and [`PartyRoster::is_local_master`] is
    /// deliberately `false` then, i.e. a master who relogs could not kick.
    /// A name is unique per server in this game, so the match is exact rather
    /// than a heuristic, and it never overwrites an id an ack already gave us.
    fn learn_local_member(&mut self, local_name: &str) {
        if self.local_member_id != 0 || local_name.is_empty() {
            return;
        }
        if let Some(me) = self
            .members
            .iter()
            .find(|m| m.name.as_deref() == Some(local_name))
        {
            self.local_member_id = me.member_id.unwrap_or(0);
        }
    }

    pub fn member(&self, member_id: u32) -> Option<&PartyMemberCore> {
        self.members.iter().find(|m| m.member_id == Some(member_id))
    }

    fn member_mut(&mut self, member_id: u32) -> Option<&mut PartyMemberCore> {
        self.members
            .iter_mut()
            .find(|m| m.member_id == Some(member_id))
    }

    /// Clear everything — the party is gone.
    fn dismiss(&mut self) {
        self.members.clear();
        self.setup = PartySetup::default();
        self.master_join_id = None;
        self.party_number = 0;
        // Our jid was scoped to that party: the next one assigns a new one
        // (0xB060/0xB067), so keeping it would let a stale value decide the
        // master test in the following party.
        self.local_member_id = 0;
    }

    /// 0x3065 — the authoritative push. Each of its two halves is independently
    /// present: the header bit 0 names the party's attributes, bit 1 names the
    /// roster. A roster-only push must not be read as "sharing was turned off",
    /// and an info-only push must not empty the party.
    pub fn apply_data(&mut self, data: &PartyData) {
        self.party_number = data.party_number;
        if data.has_party_info() {
            self.setup = data.setup();
            self.master_join_id = data.master_join_id;
        }
        if data.has_roster() {
            self.members = data.members.clone();
        }
    }

    /// 0x3864 — delta. An unrecognised update type is a no-op by design: the
    /// packet model decodes it to "no payload" instead of failing, and the
    /// roster must not invent state for it.
    pub fn apply_update(&mut self, update: &PartyUpdate) {
        match update.update_type {
            // 1 — party dismissed.
            1 => self.dismiss(),
            // 2 — member joined. A re-join of a known id merges onto the record
            // we already have rather than replacing it, for the same reason
            // type 6 does: the record is only as complete as its mask.
            2 => {
                if let Some(joined) = update.joined.as_ref() {
                    self.upsert(joined);
                }
            }
            // 3 — member left or was kicked.
            3 => {
                if let Some(member_id) = update.member_id {
                    self.members.retain(|m| m.member_id != Some(member_id));
                }
            }
            // 6 — some member fields changed. The id is in the envelope, not in
            // the record (whose own 0x10 bit is usually clear here).
            6 => {
                if let (Some(member_id), Some(delta)) =
                    (update.member_id, update.member_update.as_ref())
                {
                    if let Some(member) = self.member_mut(member_id) {
                        merge_member(member, delta);
                    }
                }
            }
            // 9 — the master changed. It DOES have a body: the handler reads
            // one `u32` and `_swprintf_s`es it into a notice line,
            // so "type 9 has no known body" was wrong.
            9 => {
                if let Some(master_id) = update.new_master_id {
                    self.master_join_id = Some(master_id);
                }
            }
            _ => {}
        }
    }

    /// Merge a record onto the member it names, or append it as a new one.
    fn upsert(&mut self, record: &PartyMemberCore) {
        match record.member_id.and_then(|id| self.member_mut(id)) {
            Some(slot) => merge_member(slot, record),
            None => self.members.push(record.clone()),
        }
    }
}

/// Fold one presence-masked record onto a stored member.
///
/// Only fields the record's mask actually named are copied — a `None` here
/// means "this record did not mention it", never "it became empty". The
/// position is the one group that moves together, because one mask bit gates
/// the region, the coordinates and the trailing word, and the region's high bit
/// decides which of the two coordinate flavours arrived.
fn merge_member(member: &mut PartyMemberCore, delta: &PartyMemberCore) {
    member.presence |= delta.presence;
    if delta.member_id.is_some() {
        member.member_id = delta.member_id;
    }
    if delta.name.is_some() {
        member.name = delta.name.clone();
        member.model_id = delta.model_id;
    }
    if delta.level.is_some() {
        member.level = delta.level;
    }
    if delta.hp_mp.is_some() {
        member.hp_mp = delta.hp_mp;
    }
    if delta.region.is_some() {
        member.region = delta.region;
        member.position_dungeon = delta.position_dungeon.clone();
        member.position_world = delta.position_world.clone();
        member.position_tail = delta.position_tail;
    }
    if delta.guild_name.is_some() {
        member.guild_name = delta.guild_name.clone();
    }
    if delta.flag.is_some() {
        member.flag = delta.flag;
    }
    if delta.mastery_primary.is_some() {
        member.mastery_primary = delta.mastery_primary;
        member.mastery_secondary = delta.mastery_secondary;
    }
}

/// 0x3065 — fold the roster, and take the chance to learn our own jid from it if
/// the acks have not told us yet (see [`PartyRoster::local_member_id`]).
pub fn on_party_data(
    mut reader: MessageReader<PartyData>,
    mut roster: ResMut<PartyRoster>,
    me: Query<&CharacterInfo>,
) {
    for data in reader.read() {
        roster.apply_data(data);
        // `CharacterInfo` is a **component**, and its own module doc says only
        // the local player receives one today (0x3013) — so `single()` is the
        // local character, and the day remote players get one this stops
        // resolving rather than picking the wrong entity.
        if let Some(name) = me.single().ok().and_then(|me| me.name.as_deref()) {
            roster.learn_local_member(name);
        }
    }
}

pub fn on_party_update(mut reader: MessageReader<PartyUpdate>, mut roster: ResMut<PartyRoster>) {
    for update in reader.read() {
        roster.apply_update(update);
    }
}

// --------------------------------------------------------------------------
// Outbound: the four party verbs
// --------------------------------------------------------------------------
//
// Idea: everything above folds what the server pushes; nothing sent one. The
// original has *two* different "invite" opcodes and picking between them is the
// only decision this half makes: `0x7060` forms a party around the target and
// raises petition type 2 (`PartyCreation`) on them, `0x7062` adds the target to
// a party that already exists and raises type 3 (`PartyInvitation`).
// One message type therefore carries the
// player's intent ("ask this person into my party") and the sender resolves it
// against the roster, so no caller has to know the split.

/// A party verb the local player triggered. Written by the UI (the target
/// menu today, the party window next), consumed by [`send_party_actions`].
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartyAction {
    /// Ask the player with this **spawn id** into the party. Becomes 0x7060 or
    /// 0x7062 depending on whether we are already in one.
    Invite(u32),
    /// Leave the party — 0x7061, empty body.
    Leave,
    /// Expel the member with this **join id** — 0x7063.
    Kick(u32),
}

/// The `SRParty.Setup` flags a freshly created party is opened with.
///
/// Deliberately the empty set rather than a guessed sharing policy. The
/// original picks these in `ifsetpartymode.txt` (two radio groups plus the
/// "can invite without master status" checkbox) *before* the create request goes
/// out, and that dialog does not exist here yet (#32). Sending flags the player
/// never chose would invent a policy — and a visible one, since `EXP_SHARED`
/// alone changes the party's capacity from 4 to 8. This resource is the seam
/// the dialog writes to when it lands.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct PartyCreateSetup(pub PartySetup);

/// Turn [`PartyAction`]s into wire packets.
pub fn send_party_actions(
    mut reader: MessageReader<PartyAction>,
    roster: Res<PartyRoster>,
    setup: Res<PartyCreateSetup>,
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
            warn!("party: no agent connection, dropping {action:?}");
        }
        return;
    };
    let in_party = roster.is_active();
    for action in pending {
        let Some(packet) = party_action_packet_checked(*action, in_party, setup.0, &roster) else {
            continue;
        };
        info!("party: sending {action:?} (in_party={in_party})");
        if let Err(e) = conn.get_sender().send(packet.into()) {
            error!("network: failed to send party action: {}", e.0);
        }
    }
}

/// The verb-to-opcode rule, split out so the 0x7060/0x7062 fork is testable
/// without a live connection — it is the only decision on this path.
pub fn party_action_packet(action: PartyAction, in_party: bool, setup: PartySetup) -> Packet {
    match action {
        // Already in a party -> 0x7062 adds to it; otherwise 0x7060 forms one
        // around the target (and `setup` is only read on that branch).
        PartyAction::Invite(unique_id) if in_party => {
            Packet::from(PartyInviteRequest { unique_id })
        }
        PartyAction::Invite(unique_id) => Packet::from(PartyCreationRequest {
            unique_id,
            setup: setup.0,
        }),
        PartyAction::Leave => Packet::from(PartyLeave),
        PartyAction::Kick(join_id) => Packet::from(PartyKickRequest { join_id }),
    }
}
/// [`party_action_packet`] plus the one permission the client itself checks:
/// **only the party master may kick**, and `None` is the refusal.
///
/// Split out from the sender so "a non-master never puts a 0x7063 on the wire"
/// is a testable statement rather than a comment. The precondition is the
/// original's, not a precaution of ours: the `/BanishFromParty` branch of
/// calls — nine bytes reading `partyInfo+0x0d`,
/// the byte the 0x3065 handler sets to 1 exactly when the wire's master jid
/// equals our own ->, `this+0x2d`) — *before* it
/// resolves the target name at all. What the original shows on refusal is not
/// known (the branch is a plain early-out and references no string), so we log
/// and print nothing rather than invent a confirmation dialog; seeing the
/// original refuse is what would fill that in.
pub fn party_action_packet_checked(
    action: PartyAction,
    in_party: bool,
    setup: PartySetup,
    roster: &PartyRoster,
) -> Option<Packet> {
    if let PartyAction::Kick(member_id) = action {
        if !roster.is_local_master() {
            warn!(
                "party: refusing to kick {member_id} — not the party master (master={:?}, us={})",
                roster.master_join_id, roster.local_member_id
            );
            return None;
        }
    }
    Some(party_action_packet(action, in_party, setup))
}

/// The error codes this family answers with, as values from the wiki's
/// create/join pages quoted in `docs/net-party.md` §5. Wording is ours: the
/// original's own strings are `textuisystem` keys we cannot bind to a code
/// without the wire (the client resolves them through its category-2 error
/// box).
pub fn party_error_text(code: u16) -> Option<&'static str> {
    match code {
        11276 => Some("The party request was declined."),
        11280 => Some("The invitation expired without an answer."),
        11288 => Some("That player is already in another party."),
        11292 => Some("That party no longer exists."),
        11301 => Some("That player has a party registered in party matching."),
        _ => None,
    }
}

/// One line of feedback for an ack, or `None` when it succeeded.
///
/// `0xB060` and `0xB062` share this shape: `result == 1` is success (0xB062
/// carries nothing at all on success, so silence is correct — the party itself
/// arrives as the separate 0x3065/0x3864 push), `result == 2` carries the code.
fn ack_feedback(verb: &str, result: u8, error_code: Option<u16>) -> Option<String> {
    if result == 1 {
        return None;
    }
    Some(match error_code.and_then(party_error_text) {
        Some(text) => text.to_string(),
        // An unmapped code is reported verbatim rather than swallowed: the
        // table above is wiki-derived, not captured, so it will have holes.
        None => format!(
            "Party {verb} failed (code {}).",
            error_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| format!("result {result}"))
        ),
    })
}

/// Push one ack line into the chat log, when there is a chat log.
///
/// `ChatHistory` is a **HUD** resource and this is the **net** layer. The
/// headless netcheck harness builds the networking core with no HUD at all, and
/// Bevy 0.19 does not skip a system whose `ResMut` is missing — it fails
/// parameter validation and panics the schedule, which killed the harness
/// outright. So the dependency is optional by construction: without a HUD the
/// ack still reaches the log, it just has no window to print in.
fn report_ack(history: &mut Option<ResMut<ChatHistory>>, text: String) {
    match history {
        Some(history) => history.push(ChatLine::system(text)),
        None => info!("party (headless): {text}"),
    }
}

/// 0xB060 — ack for our own 0x7060. Success also carries a `u32` with no
/// established meaning, so it is logged, not modelled.
pub fn on_party_create_response(
    mut reader: MessageReader<PartyCreateResponse>,
    mut roster: ResMut<PartyRoster>,
    mut history: Option<ResMut<ChatHistory>>,
) {
    for msg in reader.read() {
        info!(
            "party: 0xB060 create ack result={} value={:?} error={:?}",
            msg.result, msg.leader_join_id, msg.error_code
        );
        // The u32 is **our own party jid**, and that is read rather than
        // assumed: it used to be called "party number or member count", but the
        // original stores it at `partyInfo+0x18` — the field its own master test
        // reads as *our* jid — and the wire says the same. A `0105000000` is
        // followed 20 ms later by a 0x3065 whose master is 5 while its party
        // *number* is 1. Neither a number nor a count; the creator's jid, which
        // for a create ack is ours.
        roster.learn_local_member_from_ack(msg.result, msg.leader_join_id);
        if let Some(text) = ack_feedback("creation", msg.result, msg.error_code) {
            report_ack(&mut history, text);
        }
    }
}

/// 0xB067 — ack for joining a party that already existed. Its success `u32` is
/// **our own party jid**, read rather than guessed: see [`PartyJoinResponse`]
/// for the four dumps that pin it against the `0x3065` of the same instant.
///
/// Why this matters beyond tidiness: the name match in
/// [`PartyRoster::learn_local_member`] only works once [`CharacterInfo`] exists,
/// and it is fed by 0x3013 — a packet that has no ordering guarantee against
/// 0x3065. Until it lands, `local_member_id` stayed 0 and
/// [`PartyRoster::is_local_master`] answered `false` for a joiner *and* for a
/// master; the ack closes that window because it arrives with the roster.
pub fn on_party_join_response(
    mut reader: MessageReader<PartyJoinResponse>,
    mut roster: ResMut<PartyRoster>,
    mut history: Option<ResMut<ChatHistory>>,
) {
    for msg in reader.read() {
        info!(
            "party: 0xB067 join ack result={} jid={:?} error={:?}",
            msg.result, msg.local_join_id, msg.error_code
        );
        roster.learn_local_member_from_ack(msg.result, msg.local_join_id);
        if let Some(text) = ack_feedback("join", msg.result, msg.error_code) {
            report_ack(&mut history, text);
        }
    }
}

/// 0xB062 — ack for our own 0x7062.
pub fn on_party_invite_response(
    mut reader: MessageReader<PartyInviteResponse>,
    mut history: Option<ResMut<ChatHistory>>,
) {
    for msg in reader.read() {
        info!(
            "party: 0xB062 invite ack result={} error={:?}",
            msg.result, msg.error_code
        );
        if let Some(text) = ack_feedback("invitation", msg.result, msg.error_code) {
            report_ack(&mut history, text);
        }
    }
}

/// Owns [`PartyRoster`]; part of the networking core so the roster exists
/// wherever party packets can arrive.
pub struct PartyPlugin;

impl Plugin for PartyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartyRoster>()
            .init_resource::<PartyCreateSetup>()
            .add_message::<PartyAction>()
            // Also registered by `add_network_events`; repeated so the plugin
            // stands alone in a test app.
            .add_message::<PartyJoinResponse>()
            .add_systems(
                Update,
                (
                    on_party_data,
                    on_party_update,
                    on_party_create_response,
                    on_party_join_response,
                    on_party_invite_response,
                    send_party_actions,
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use bytes::Bytes;
    use packets::agent::party::{PartyHpMp, PartyMemberMask, PartyPositionWorld};

    /// A member record with every field present — what a full roster push sends.
    pub(crate) fn core(member_id: u32, name: &str, region: u16) -> PartyMemberCore {
        PartyMemberCore {
            presence: PartyMemberMask::ALL,
            member_id: Some(member_id),
            name: Some(name.to_string()),
            model_id: Some(1907),
            level: Some(20),
            // 0x5A = HP nibble 10 -> 100 %, MP nibble 5 -> 50 %.
            hp_mp: Some(0x5A),
            region: Some(region),
            position_dungeon: None,
            position_world: Some(PartyPositionWorld {
                x: 100,
                y: 0,
                z: 200,
            }),
            position_tail: Some(0),
            guild_name: Some(String::new()),
            flag: Some(0),
            mastery_primary: Some(0),
            mastery_secondary: Some(0),
        }
    }

    /// A record naming exactly one field, the shape a 0x3864 delta really has.
    fn delta(mask: u8, apply: impl FnOnce(&mut PartyMemberCore)) -> PartyMemberCore {
        let mut record = PartyMemberCore {
            presence: mask,
            ..Default::default()
        };
        apply(&mut record);
        record
    }

    /// One `PartyUpdate` builder every type-specific test below builds on, so a
    /// new wire field cannot be forgotten in six places.
    fn update(update_type: u8) -> PartyUpdate {
        PartyUpdate {
            update_type,
            dismiss_code: None,
            joined: None,
            member_id: None,
            leave_reason: None,
            member_update: None,
            new_master_id: None,
        }
    }

    pub(crate) fn party_data(members: Vec<PartyMemberCore>) -> PartyData {
        PartyData {
            presence: 0x03,
            party_number: 1,
            master_join_id: members.first().and_then(|m| m.member_id),
            setup: Some(PartySetup::EXP_SHARED),
            member_count: Some(members.len() as u8),
            members,
        }
    }

    #[test]
    fn full_roster_replaces_state() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![
            core(1, "Alice", 25000),
            core(2, "Bob", 25000),
        ]));
        assert_eq!(roster.members.len(), 2);
        assert!(roster.setup.is_exp_shared());
        assert_eq!(roster.capacity(), 8);
        // The first member is the leader in this fixture, and the crown follows
        // the wire field rather than wire order.
        assert!(roster.is_leader(1));
        assert!(!roster.is_leader(2));
        // A second 0x3065 carrying the roster bit is authoritative, not additive.
        roster.apply_data(&party_data(vec![core(3, "Carol", 25000)]));
        assert_eq!(roster.members.len(), 1);
        assert_eq!(roster.members[0].member_id, Some(3));
    }

    /// A push that carries only the party's attributes must not empty the
    /// party, and one that carries only the roster must not reset the sharing
    /// flags. Under the old fixed 9-byte header both halves were always assumed
    /// present.
    #[test]
    fn each_half_of_the_roster_push_is_independent() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));

        // roster only — sharing and leader survive
        roster.apply_data(&PartyData {
            presence: 0x02,
            party_number: 1,
            master_join_id: None,
            setup: None,
            member_count: Some(2),
            members: vec![core(1, "Alice", 25000), core(2, "Bob", 25000)],
        });
        assert!(roster.setup.is_exp_shared());
        assert_eq!(roster.master_join_id, Some(1));
        assert_eq!(roster.members.len(), 2);

        // info only — the members survive
        roster.apply_data(&PartyData {
            presence: 0x01,
            party_number: 1,
            master_join_id: Some(2),
            setup: Some(PartySetup::ITEM_SHARED),
            member_count: None,
            members: Vec::new(),
        });
        assert_eq!(roster.members.len(), 2);
        assert!(roster.is_leader(2));
        assert_eq!(roster.capacity(), 4, "EXP sharing was switched off");
    }

    #[test]
    fn hp_mp_stays_the_raw_wire_byte() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));
        assert_eq!(roster.members[0].hp_mp, Some(0x5A));
        // the roster keeps the byte; the fractions are derived, not stored
        let vitals = roster.members[0].hp_mp().unwrap();
        assert_eq!(vitals.hp_fill().as_f32(), 1.0);
        assert_eq!(vitals.mp_fill().percent_rounded(), 50);
        assert_eq!(Some(PartyHpMp(0x5A)), roster.members[0].hp_mp());
    }

    #[test]
    fn deltas_join_leave_dismiss_and_update() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));

        // type 2 — joined.
        roster.apply_update(&PartyUpdate {
            joined: Some(core(2, "Bob", 25000)),
            ..update(2)
        });
        assert_eq!(roster.members.len(), 2);

        // type 6 — a level-only delta.
        roster.apply_update(&PartyUpdate {
            member_id: Some(2),
            member_update: Some(delta(PartyMemberMask::LEVEL, |r| r.level = Some(42))),
            ..update(6)
        });
        assert_eq!(roster.member(2).unwrap().level, Some(42));

        // type 3 — left/kicked.
        roster.apply_update(&PartyUpdate {
            member_id: Some(2),
            ..update(3)
        });
        assert!(roster.member(2).is_none());

        // An unknown update type must not touch the roster. 9 is NOT one any
        // more — it re-points the master, see
        // `the_master_comes_from_the_roster_header_and_from_type_nine` — so the
        // negative control is 7.
        roster.apply_update(&update(7));
        assert_eq!(roster.members.len(), 1);

        // type 1 — dismissed.
        roster.apply_update(&update(1));
        assert!(roster.members.is_empty());
        assert!(!roster.is_active());
        assert_eq!(roster.master_join_id, None);
    }

    /// The regression the presence-mask rewrite exists to prevent: a delta names
    /// one field, so folding it must not blank the rest of the member. Replacing
    /// the stored record — which is what the old code did for type 2, and what
    /// the obvious implementation of type 6 would do — would drop the name, the
    /// guild and the position on every hp/mp tick.
    #[test]
    fn a_partial_delta_merges_instead_of_replacing() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));

        roster.apply_update(&PartyUpdate {
            member_id: Some(1),
            member_update: Some(delta(PartyMemberMask::HP_MP, |r| r.hp_mp = Some(0x12))),
            ..update(6)
        });

        let member = roster.member(1).unwrap();
        assert_eq!(member.hp_mp, Some(0x12), "the delta applied");
        assert_eq!(member.name.as_deref(), Some("Alice"), "the name survived");
        assert_eq!(member.level, Some(20), "the level survived");
        assert!(member.position_world.is_some(), "the position survived");
        assert_eq!(member.model_id, Some(1907), "the model survived");
    }

    /// The position group moves together: one mask bit gates the region, the
    /// coordinates and the trailing word, and a member who walked into a dungeon
    /// must not keep a stale overworld position beside the new dungeon one.
    #[test]
    fn a_position_delta_swaps_the_coordinate_flavour() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));
        assert!(roster.member(1).unwrap().position_world.is_some());

        roster.apply_update(&PartyUpdate {
            member_id: Some(1),
            member_update: Some(delta(PartyMemberMask::POSITION, |r| {
                r.region = Some(0x8001);
                r.position_dungeon =
                    Some(packets::agent::party::PartyPositionDungeon { x: 1, y: 2, z: 3 });
            })),
            ..update(6)
        });

        let member = roster.member(1).unwrap();
        assert_eq!(member.region, Some(0x8001));
        assert!(member.position_dungeon.is_some());
        assert!(
            member.position_world.is_none(),
            "the overworld position must not linger"
        );
    }

    /// The file's one real decision: the same player-facing verb is two
    /// different opcodes, picked by whether a party already exists. Getting
    /// this backwards is silently wrong on the wire (the server answers the
    /// other ack and the invitee sees the other petition type).
    #[test]
    fn invite_picks_create_or_invite_by_roster_state() {
        let uid = 0x0001_60AA;
        // no party yet -> 0x7060 with the setup byte appended
        let (opcode, body) =
            party_action_packet(PartyAction::Invite(uid), false, PartySetup(0)).into_serialize();
        assert_eq!(opcode, 0x7060);
        assert_eq!(&body[..], &[0xAA, 0x60, 0x01, 0x00, 0x00]);

        // in a party -> 0x7062, a bare uid and no setup byte
        let (opcode, body) =
            party_action_packet(PartyAction::Invite(uid), true, PartySetup(0)).into_serialize();
        assert_eq!(opcode, 0x7062);
        assert_eq!(&body[..], &[0xAA, 0x60, 0x01, 0x00]);

        // the setup byte only exists on the create branch
        let (_, body) = party_action_packet(
            PartyAction::Invite(uid),
            false,
            PartySetup(PartySetup::EXP_SHARED | PartySetup::ITEM_SHARED),
        )
        .into_serialize();
        assert_eq!(body[4], 0x03);
    }

    #[test]
    fn leave_is_empty_and_kick_carries_the_join_id() {
        let (opcode, body) =
            party_action_packet(PartyAction::Leave, true, PartySetup(0)).into_serialize();
        assert_eq!(opcode, 0x7061);
        assert!(body.is_empty());

        let (opcode, body) =
            party_action_packet(PartyAction::Kick(0x0201), true, PartySetup(0)).into_serialize();
        assert_eq!(opcode, 0x7063);
        assert_eq!(&body[..], &[0x01, 0x02, 0x00, 0x00]);
    }

    /// A successful ack says nothing (the party arrives as its own push); a
    /// failure always says *something*, even for a code the table misses.
    #[test]
    fn acks_are_silent_on_success_and_never_swallow_a_failure() {
        assert_eq!(ack_feedback("invitation", 1, None), None);
        assert_eq!(
            ack_feedback("invitation", 2, Some(11288)).as_deref(),
            Some("That player is already in another party.")
        );
        // unmapped code -> reported verbatim rather than dropped
        assert_eq!(
            ack_feedback("creation", 2, Some(9999)).as_deref(),
            Some("Party creation failed (code 9999).")
        );
        // a failure result with no code at all still surfaces
        assert!(ack_feedback("creation", 2, None).is_some());
    }

    /// The regression the headless netcheck harness caught: these two systems
    /// live in the **net** layer but reported through `ChatHistory`, a **HUD**
    /// resource. The harness builds networking with no HUD, and Bevy 0.19 does
    /// not skip a system whose `ResMut` is missing — it fails parameter
    /// validation and panics the schedule, so the harness died before login.
    ///
    /// This builds exactly that shape: the ack systems, no HUD resource, a real
    /// ack delivered. It must survive the update.
    #[test]
    fn the_ack_systems_run_without_a_hud() {
        let mut app = App::new();
        // `PartyRoster` is a NET resource, not a HUD one — the create ack learns
        // our own jid from its success arm, so it needs it. The point of the
        // test is the absent `ChatHistory` below.
        app.init_resource::<PartyRoster>()
            .add_message::<PartyCreateResponse>()
            .add_message::<PartyInviteResponse>()
            .add_message::<PartyJoinResponse>()
            .add_systems(
                Update,
                (
                    on_party_create_response,
                    on_party_join_response,
                    on_party_invite_response,
                ),
            );
        assert!(
            !app.world().contains_resource::<ChatHistory>(),
            "this test is only meaningful without the HUD resource"
        );

        // a failure arm, so the reporting path is actually taken
        app.world_mut().write_message(PartyCreateResponse {
            result: 2,
            leader_join_id: None,
            error_code: Some(11288),
        });
        app.world_mut().write_message(PartyInviteResponse {
            result: 2,
            error_code: Some(11276),
        });
        app.world_mut().write_message(PartyJoinResponse {
            result: 2,
            local_join_id: None,
            error_code: Some(11280),
        });
        app.update();
    }

    /// ...and with a HUD present the same acks really do reach the chat log,
    /// so making the dependency optional did not quietly drop the feedback.
    #[test]
    fn the_ack_systems_still_write_to_the_chat_log_when_there_is_one() {
        let mut app = App::new();
        app.init_resource::<ChatHistory>()
            .add_message::<PartyInviteResponse>()
            .add_systems(Update, on_party_invite_response);
        app.world_mut().write_message(PartyInviteResponse {
            result: 2,
            error_code: Some(11288),
        });
        app.update();
        let history = app.world().resource::<ChatHistory>();
        assert_eq!(history.iter().count(), 1);

        // and a success stays silent — the party arrives as its own push
        app.world_mut().write_message(PartyInviteResponse {
            result: 1,
            error_code: None,
        });
        app.update();
        assert_eq!(app.world().resource::<ChatHistory>().iter().count(), 1);
    }

    /// The codes are the wiki values quoted in `docs/net-party.md` §5 — pinned
    /// so a later edit of the prose cannot silently re-map them.
    #[test]
    fn the_error_table_is_the_documented_five() {
        for code in [11276, 11280, 11288, 11292, 11301] {
            assert!(party_error_text(code).is_some(), "{code} lost its text");
        }
        assert_eq!(party_error_text(0), None);
    }
    /// The defect this slice fixes on the client side: a delta with several
    /// mask bits set must fold **all** of them. Under the old equality model
    /// (`kind == 2` / `== 4` / `== 0x20`) a combined mask folded nothing.
    #[test]
    fn a_combined_delta_folds_every_field_it_carries() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));

        roster.apply_update(&PartyUpdate {
            member_id: Some(1),
            member_update: Some(delta(
                PartyMemberMask::LEVEL | PartyMemberMask::HP_MP | PartyMemberMask::POSITION,
                |r| {
                    r.level = Some(42);
                    r.hp_mp = Some(0x8B);
                    r.region = Some(0x61a8);
                    r.position_world = Some(PartyPositionWorld {
                        x: 0x03c1,
                        y: -31,
                        z: 0x0086,
                    });
                    r.position_tail = Some(0x0001_0001);
                },
            )),
            ..update(6)
        });

        let member = &roster.members[0];
        assert_eq!(member.level, Some(42));
        assert_eq!(member.hp_mp, Some(0x8B));
        assert_eq!(member.region, Some(0x61a8));
        assert_eq!(member.position_world.as_ref().map(|p| p.y), Some(-31));
        // the u32 that used to be left on the wire lands in `position_tail`
        assert_eq!(member.position_tail, Some(0x0001_0001));
        // ...and the nibbles are the real server's, so no 110 %
        let vitals = member.hp_mp().unwrap();
        assert_eq!(vitals.hp_nibble(), 11);
        assert_eq!(vitals.hp_fill().percent_rounded(), 100);
        assert_eq!(vitals.mp_fill().percent_rounded(), 80);
        // and the fields the delta did NOT name survived
        assert_eq!(member.name.as_deref(), Some("Alice"));
    }
    /// End to end from real bytes: a vitals delta (`06 02000000 04 8b`) and an
    /// 18-byte position delta, decoded by the packet model and folded by the
    /// roster.
    #[test]
    fn real_deltas_fold_into_the_roster() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![core(2, "1234", 25000)]));

        let vitals = PartyUpdate::try_from(bytes::Bytes::from_static(&[
            0x06, 0x02, 0x00, 0x00, 0x00, 0x04, 0x8B,
        ]))
        .unwrap();
        roster.apply_update(&vitals);
        assert_eq!(roster.members[0].hp_mp, Some(0x8B));
        assert!(!roster.members[0].hp_mp().unwrap().is_dead());

        let position = PartyUpdate::try_from(bytes::Bytes::from_static(&[
            0x06, 0x02, 0x00, 0x00, 0x00, 0x20, 0xa8, 0x61, 0x00, 0x05, 0xe1, 0xff, 0xc5, 0x01,
            0x01, 0x00, 0x01, 0x00,
        ]))
        .unwrap();
        roster.apply_update(&position);
        assert_eq!(
            roster.members[0].position_world,
            Some(PartyPositionWorld {
                x: 0x0500,
                y: -31,
                z: 0x01c5
            })
        );
        // the vitals byte survived a position delta: each bit is its own field
        assert_eq!(roster.members[0].hp_mp, Some(0x8B));
    }
    /// The leader: 0x3065 names it, a type 9 re-points it, a dismiss clears it.
    #[test]
    fn the_master_comes_from_the_roster_header_and_from_type_nine() {
        let mut roster = PartyRoster::default();
        assert_eq!(roster.master_join_id, None);
        assert!(!roster.is_leader(0), "an unknown leader leads nobody");

        roster.apply_data(&party_data(vec![core(1, "Alice", 25000)]));
        assert_eq!(roster.master_join_id, Some(1));
        assert!(roster.is_leader(1));

        roster.apply_update(&PartyUpdate {
            new_master_id: Some(6),
            ..update(9)
        });
        assert_eq!(roster.master_join_id, Some(6));
        assert!(!roster.is_leader(1));
    }
    /// Where our own jid comes from — both sources, and the fallback order.
    #[test]
    fn our_own_party_jid_is_learned_from_the_ack_and_from_the_roster() {
        // (a) 0xB060: the success u32 is our jid (see `on_party_create_response`).
        let mut app = App::new();
        app.init_resource::<PartyRoster>()
            .add_message::<PartyCreateResponse>()
            .add_systems(Update, on_party_create_response);
        app.world_mut().write_message(PartyCreateResponse {
            result: 1,
            leader_join_id: Some(5),
            error_code: None,
        });
        app.update();
        assert_eq!(app.world().resource::<PartyRoster>().local_member_id, 5);

        // a *failed* ack teaches nothing
        app.world_mut()
            .resource_mut::<PartyRoster>()
            .local_member_id = 0;
        app.world_mut().write_message(PartyCreateResponse {
            result: 2,
            leader_join_id: None,
            error_code: Some(11288),
        });
        app.update();
        assert_eq!(app.world().resource::<PartyRoster>().local_member_id, 0);

        // (b) a joined party: the name match on 0x3065 is what fills it in
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![
            core(1, "Alice", 25000),
            core(2, "Bob", 25000),
        ]));
        roster.learn_local_member("Bob");
        assert_eq!(roster.local_member_id, 2);
        assert!(!roster.is_local_master(), "Bob is not the master");

        // an id we already know is never overwritten, and an empty name is a
        // no-op rather than a match against a nameless member
        roster.learn_local_member("Alice");
        assert_eq!(roster.local_member_id, 2);
        let mut fresh = PartyRoster::default();
        fresh.apply_data(&party_data(vec![core(1, "Alice", 25000)]));
        fresh.learn_local_member("");
        assert_eq!(fresh.local_member_id, 0);

        // and the party dissolving forgets it (the next party assigns a new one)
        roster.apply_update(&update(1));
        assert_eq!(roster.local_member_id, 0);
    }
    /// 0xB067, from real bytes — the party we *joined* now reads our own jid
    /// instead of waiting for a name to compare against.
    ///
    /// The wire and the roster in this test are a real pair: `01 04000000`, and
    /// the `0x3065` 19 ms later lists jids 6, 5 and 4 with master 6 — we are
    /// jid 4. No `CharacterInfo` is registered here on purpose: that is exactly
    /// the tick in which the name match cannot answer yet.
    #[test]
    fn the_join_ack_names_our_own_jid_without_character_info() {
        // the body decodes to the jid, not to a count or a party number
        let joined = PartyJoinResponse::try_from(Bytes::from_static(&[1, 4, 0, 0, 0])).unwrap();
        assert_eq!(joined.local_join_id, Some(4));
        assert_eq!(joined.error_code, None);
        // ...and the failure arm: 0x2C10 = 11280
        let refused = PartyJoinResponse::try_from(Bytes::from_static(&[2, 0x10, 0x2C])).unwrap();
        assert_eq!(refused.error_code, Some(11280));
        assert!(party_error_text(11280).is_some(), "the code has a message");

        let mut app = App::new();
        app.init_resource::<PartyRoster>()
            .add_message::<PartyJoinResponse>()
            .add_systems(Update, on_party_join_response);
        app.world_mut()
            .resource_mut::<PartyRoster>()
            .apply_data(&PartyData {
                master_join_id: Some(6),
                ..party_data(vec![
                    core(6, "Trader6", 25000),
                    core(5, "Mira", 25000),
                    core(4, "Brigand", 25000),
                ])
            });
        app.world_mut().write_message(joined);
        app.update();

        let roster = app.world().resource::<PartyRoster>();
        assert_eq!(
            roster.local_member_id, 4,
            "0xB067's success u32 is our own party jid"
        );
        assert!(
            !roster.is_local_master(),
            "the joiner is not the master — master 6, us 4"
        );

        // the refusal teaches nothing, and cannot un-learn what we know
        app.world_mut().write_message(refused);
        app.update();
        assert_eq!(app.world().resource::<PartyRoster>().local_member_id, 4);
    }
    /// The kick permission, end to end on the model side: we must **know** we
    /// are the master, and "not sure" is a refusal.
    ///
    /// This is the test the lane asked for by name — a non-master never puts a
    /// 0x7063 on the wire — and it is expressible only because the check lives
    /// in [`party_action_packet_checked`] instead of inside the sender.
    #[test]
    fn a_non_master_never_sends_a_kick() {
        let mut roster = PartyRoster::default();
        roster.apply_data(&party_data(vec![
            core(1, "Alice", 25000),
            core(2, "Bob", 25000),
        ]));
        // `party_data` makes jid 1 the master.
        let kick = PartyAction::Kick(2);
        let setup = PartySetup::default();

        // our own jid unknown -> refused, even though we may well be the master
        assert_eq!(roster.local_member_id, 0);
        assert!(!roster.is_local_master());
        assert!(party_action_packet_checked(kick, true, setup, &roster).is_none());

        // a member who is not the master -> refused
        roster.local_member_id = 2;
        assert!(!roster.is_local_master());
        assert!(party_action_packet_checked(kick, true, setup, &roster).is_none());

        // the master -> the packet goes out, and it is the kick
        roster.local_member_id = 1;
        assert!(roster.is_local_master());
        let (opcode, body) = party_action_packet_checked(kick, true, setup, &roster)
            .expect("the master may kick")
            .into_serialize();
        assert_eq!(opcode, 0x7063);
        // ...and it carries the roster jid, which is what the original's own
        // kick path sends (see `party_action_packet_checked`).
        assert_eq!(&body[..], &[0x02, 0x00, 0x00, 0x00]);

        // ...and the two verbs that act on nobody else are never gated by it
        assert!(party_action_packet_checked(PartyAction::Leave, true, setup, &roster).is_some());
        roster.local_member_id = 2;
        assert!(party_action_packet_checked(PartyAction::Leave, true, setup, &roster).is_some());
        assert!(
            party_action_packet_checked(PartyAction::Invite(7), false, setup, &roster).is_some()
        );
    }
}

//! The one owner of the server's action slot.
//!
//! Idea: vSRO runs one action per character and keeps at most one waiting
//! behind it; a newer request replaces the waiting one, and the server starts
//! it itself when the running action ends (`0xB074`'s second byte is that
//! queue's length, [`ObjectActionResponse`]). Four call sites used to write to
//! that slot independently —
//! the skill cast, the auto-attack order, the pickup order and the cast
//! retry — so nothing arbitrated between them and every keypress went
//! straight onto the wire. In a fight the player mashes hotkeys, and the
//! captures show what that costs: of 463 requests in `packet_dump/c2s/
//! 0x7074.log`, **178 (38%) landed in the server's queue** (`01 02`), where
//! the next one replaced them, and the log holds 29 runs of three-or-more
//! byte-identical sends less than 600 ms apart — the worst being 14 identical
//! `CastSkill` requests over 2.2 s.
//!
//! So every producer now hands its intent here instead of sending, and this
//! module owns the wire. At most **one** intent is held: a newer press
//! replaces the one waiting, because the thing the player pressed last is the
//! thing they want. Mashing one skill therefore collapses to a single
//! request, and pressing B then C casts C. Replaying the capture through this
//! policy turns those 463 requests into 361.
//!
//! It is the same shape [`PlayerCommands::move_to`](crate::plugins::player::PlayerCommands)
//! already uses for the movement lane: a new order replaces the queued one
//! rather than appending to it.
//!
//! **Stated deviation (ADR 0009).** No capture of the *original* client's
//! outbound traffic exists in this repository and no note records what it does
//! with a re-press, so this policy is ours, not vanilla's. Its rationale is
//! the paragraph above: fewer requests against someone else's server, and the
//! player's latest intent winning.
//! The one duration it needs is sourced rather than invented — see
//! [`CombatSettings::action_buffer_seconds`](crate::plugins::config::combat::CombatSettings).
//!
//! Item use (`0x704C`) is deliberately **not** routed through here: it is a
//! different opcode with its own gate, and nothing in the corpus shows it
//! contending for the action slot. A potion pressed mid-cast must still go out
//! at once.

use bevy::prelude::*;

use packets::agent::prelude::{
    ActionCommand, ActionTarget, ObjectActionRequest, ObjectActionResponse,
};
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::config::ClientConfig;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::entities::NetworkId;

/// One thing the player can ask the action slot to do. Everything that turns
/// into a `0x7074 Execute` is one of these.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Cast {
        skill_id: u32,
        target: Option<Entity>,
    },
    Attack(Entity),
    Pickup(Entity),
}

impl Action {
    /// The world entity this action names, if any. A self-buff
    /// (`Cast { target: None }`) names nothing.
    fn target(&self) -> Option<Entity> {
        match *self {
            Action::Cast { target, .. } => target,
            Action::Attack(entity) | Action::Pickup(entity) => Some(entity),
        }
    }
}

/// A cast/attack/pickup the server refused. Emitted so the presentation layer
/// can retract whatever it optimistically played for it — the skills plugin
/// reads this to drop a click-predicted swing.
///
/// This is the *only* honest way to attribute a refusal. The ack's second byte
/// cannot do it — it is the server's queue length, not the action type.
/// Because this module keeps at most one request in flight, the refused
/// action is simply known.
#[derive(Message)]
pub struct ActionRefused(pub Action);

/// A request sent and awaiting its first ack.
struct InFlight {
    action: Action,
    sent_at: f64,
}

/// The intent waiting for the slot to free.
struct Pending {
    action: Action,
    /// When the player asked for it, so a stale one can be dropped.
    at: f64,
}

/// How long a request may wait for its FIRST ack before we stop counting the
/// slot as ours.
///
/// Origin: send→next-ack latency over the whole `packet_dump/c2s/0x7074.log` ×
/// `packet_dump/0xb074.log` correlation is median 107 ms, p90 219 ms. Two
/// seconds is ~9× that p90 — long enough that a live round trip never trips
/// it, short enough that an ack the server never sends cannot park the slot.
const ACK_TIMEOUT_SECS: f64 = 2.0;

/// How long the client believes a `Started` it was never told the end of.
///
/// Generous on purpose: the server frees its own slot in a median 1.38 s
/// (`packet_dump/0xb074.log`) and the longest cast chain runs 5.1 s, so this
/// cannot cut a real action short. It only stops a missing `Ended` from
/// latching the slot shut for the rest of the session.
///
/// **What this covers is a server defect, not a gap in the protocol.** Vanilla
/// really does send the end: a capture of a real server holds 391 `0xB074`
/// frames split 181 `Started` / 142 `Ended` / 68 `Failed` — an `Ended` almost
/// as often as a `Started`. So the correct reading of this timeout is "the
/// peer owes us an `Ended` and did not send one", never "the protocol has no
/// end". A peer that sends them clears the flag in the same frame and never
/// reaches this bound.
const SERVER_BUSY_TIMEOUT_SECS: f64 = 8.0;

/// The client's model of the server's single action slot, plus the one thing
/// the player most recently asked it to do.
#[derive(Resource, Default)]
pub struct ActionSlot {
    /// An action the server told us it started and has not ended. Tracked from
    /// the `0xB074` acks, not from our own sends, since we have no visibility
    /// into server state until it answers — the server also runs actions we
    /// never requested (it restarts its own auto-attack loop after a cast).
    server_busy: bool,
    /// When the running action was accepted, for [`SERVER_BUSY_TIMEOUT_SECS`].
    busy_since: Option<f64>,
    /// Our request between the send and its first ack.
    in_flight: Option<InFlight>,
    next: Option<Pending>,
}

impl ActionSlot {
    /// Ask the slot to do `action`. Replaces whatever was waiting — the newest
    /// press wins.
    pub fn request(&mut self, action: Action, now: f64) {
        if let Some(displaced) = self.next.replace(Pending { action, at: now }) {
            debug!(
                "action slot: {:?} replaces the waiting {:?}",
                action, displaced.action
            );
        }
    }

    /// Whether the slot cannot take a new request right now. Gates the skill
    /// plugin's click prediction: predicting while the slot is occupied plays
    /// a throwaway swing that the server's refusal then contradicts.
    pub fn occupied(&self) -> bool {
        self.server_busy || self.in_flight.is_some()
    }

    /// Drop the waiting intent and send a `Cancel`'s worth of forgetting — a
    /// deliberate stop must not be followed by the thing it interrupted.
    pub fn clear_pending(&mut self) {
        self.next = None;
    }

    /// Drop the waiting intent if it names `entity` — called when that entity
    /// dies, so the slot frees for the player's next press instead of holding
    /// an intent that can no longer do anything.
    pub fn forget_target(&mut self, entity: Entity) {
        if self.next.as_ref().and_then(|p| p.action.target()) == Some(entity) {
            let dropped = self.next.take().expect("checked above");
            debug!(
                "action slot: {:?} dropped — its target died",
                dropped.action
            );
        }
    }

    /// Stop counting a request as ours once its ack window has passed, so an
    /// ack the server never sends cannot park the slot forever.
    fn release_stalled_send(&mut self, now: f64) {
        if self
            .in_flight
            .as_ref()
            .is_some_and(|f| now - f.sent_at > ACK_TIMEOUT_SECS)
        {
            let flight = self.in_flight.take().expect("checked above");
            debug!("action slot: no ack for {:?} — releasing", flight.action);
        }
        // The same bound on the other half of `occupied()`. `server_busy` is
        // set by a `Started` and cleared ONLY by an `Ended`, so a server that
        // never sends one latches the slot shut for the rest of the session:
        // every later intent then waits out the buffer and is dropped as
        // stale, which is what "went stale" means in a log with no sends after
        // it. Measured 2026-10-23 against our own stub: 375 `0xB074` frames in
        // one session, every one of them `Started`, not a single `Ended`;
        // `0xB071` never arrived either.
        //
        // The stub was the one at fault, and that is now settled rather than
        // suspected: a capture of a real server holds 391 `0xB074` frames,
        // 181 `Started` / 142 `Ended` / 68 `Failed`. The end is part of the
        // protocol, so this timeout is a guard against a peer that owes one
        // and withholds it — not a stand-in for a message that does not exist.
        // A peer that sends `Ended` clears the flag in the same frame and
        // never reaches this bound.
        if self
            .busy_since
            .is_some_and(|at| now - at > SERVER_BUSY_TIMEOUT_SECS)
        {
            self.busy_since = None;
            self.server_busy = false;
            debug!("action slot: no Ended for the running action — releasing the slot");
        }
    }

    /// The waiting intent, if it may go out now.
    ///
    /// Two reasons to **drop** it, checked first so a doomed intent frees the
    /// slot for the player's next press rather than occupying it:
    ///
    /// - its target died while it waited. The buffer made this reachable: the
    ///   press is held for seconds where it used to go out at once, and a
    ///   corpse keeps its network id for [`CORPSE_LINGER_SECS`] — so without
    ///   this the follow-up skill fires at a dead monster.
    /// - it has gone stale. The fight has moved on and firing it now would act
    ///   on an intent the player has forgotten about.
    ///
    /// Two reasons to **hold** it: the server's slot is occupied, or our own
    /// body is still playing the cast this one follows. The second is what
    /// makes "first finishes, second follows" true on the wire and not just in
    /// the animation queue — the server frees its slot (median 1.38 s) well
    /// before a cast clip ends (2.4-5.1 s for a chain).
    ///
    /// Every drop is logged: a press that vanishes without a trace is what
    /// this module exists to stop.
    fn take_ready(
        &mut self,
        now: f64,
        buffer_secs: f64,
        mid_cast: bool,
        is_dead: impl Fn(Entity) -> bool,
    ) -> Option<Pending> {
        let pending = self.next.as_ref()?;
        if pending.action.target().is_some_and(is_dead) {
            let dropped = self.next.take().expect("checked above");
            info!(
                "action slot: {:?} — its target is dead, dropped",
                dropped.action
            );
            return None;
        }
        if now - pending.at > buffer_secs {
            let dropped = self.next.take().expect("checked above");
            info!("action slot: {:?} went stale — dropped", dropped.action);
            return None;
        }
        if self.occupied() || mid_cast {
            return None; // keep waiting
        }
        self.next.take()
    }

    /// Record that `pending` is now on the wire.
    fn mark_sent(&mut self, pending: &Pending, now: f64) {
        self.in_flight = Some(InFlight {
            action: pending.action,
            sent_at: now,
        });
    }
}

/// Fold the `0xB074` acks into the slot's state and announce every refusal as
/// an [`ActionRefused`].
pub fn apply_action_acks(
    time: Res<Time>,
    mut acks: MessageReader<ObjectActionResponse>,
    mut slot: ResMut<ActionSlot>,
    mut refusals: MessageWriter<ActionRefused>,
) {
    let now = time.elapsed_secs_f64();
    for ack in acks.read() {
        match ack {
            // Accepted — running now, or waiting behind the running action
            // (`01 02`). A waiting one is the server's to start: it says so
            // with `02 01`, and sending it again would run it twice. Either
            // way the server's slot is taken.
            ObjectActionResponse::Started { .. } => {
                slot.in_flight = None;
                slot.server_busy = true;
                slot.busy_since = Some(now);
            }
            // An action ended with `queue_len` left: one left means the
            // waiting action has just started, so the slot stays taken.
            ObjectActionResponse::Ended { queue_len } => {
                slot.server_busy = *queue_len > 0;
                slot.busy_since = slot.server_busy.then_some(now);
            }
            // A refusal on the request's own merits (a cooldown the client's
            // execution-time mirror never saw start, a precondition we do not
            // model). It ends nothing and starts nothing, so `server_busy` is
            // untouched — but the request will never execute, and whatever was
            // played for it has to come back off.
            ObjectActionResponse::Failed { .. } => {
                if let Some(flight) = slot.in_flight.take() {
                    info!("action slot: {:?} refused by the server", flight.action);
                    refusals.write(ActionRefused(flight.action));
                }
            }
            ObjectActionResponse::Unknown { .. } => {}
        }
    }
}

/// Send the waiting intent as soon as the slot and the caster's body are both
/// free, or drop it once its target has died or it has gone stale.
///
/// Ordered after every producer so a press onto a free slot still reaches the
/// wire in the same frame it was made — the buffer costs latency only when
/// something really is busy.
pub fn drive_action_slot(
    time: Res<Time>,
    config: Res<ClientConfig>,
    mut slot: ResMut<ActionSlot>,
    ids: Query<&NetworkId>,
    liveness: super::TargetLiveness,
    players: Query<Entity, With<crate::plugins::player::Player>>,
    children: Query<&Children>,
    casting: crate::plugins::player::CastingWrappers,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let now = time.elapsed_secs_f64();
    slot.release_stalled_send(now);
    let buffer_secs = config.combat.action_buffer_seconds.max(0.0) as f64;
    // A cast still on the body holds the next action back. Not `is_mid_one_shot`
    // — that also reports a flinch, and stalling the wire on the most
    // interruptible clip there is would be worse than sending early.
    let mid_cast = players
        .single()
        .is_ok_and(|player| crate::plugins::player::is_mid_cast(player, &children, &casting));
    let Some(pending) = slot.take_ready(now, buffer_secs, mid_cast, |entity| {
        liveness.is_known_dead(entity)
    }) else {
        return;
    };
    let Ok(conn) = conn.single() else {
        // Offline sandbox: no agent connection, so nothing to send to.
        return;
    };
    let target = |entity: Entity| match ids.get(entity) {
        Ok(NetworkId(uid)) => Some(ActionTarget::Entity { unique_id: *uid }),
        Err(_) => None,
    };
    let command = match pending.action {
        Action::Cast {
            skill_id,
            target: t,
        } => {
            // A self-buff goes out untargeted on purpose; only an entity we
            // cannot resolve at all is a reason to drop the request.
            let wire_target = match t {
                Some(entity) => match target(entity) {
                    Some(resolved) => resolved,
                    None => {
                        warn!("action slot: cast {skill_id} target is gone — dropped");
                        return;
                    }
                },
                None => ActionTarget::None,
            };
            ActionCommand::CastSkill {
                ref_skill_id: skill_id,
                target: wire_target,
            }
        }
        Action::Attack(entity) => match target(entity) {
            Some(resolved) => ActionCommand::Attack(resolved),
            None => return,
        },
        Action::Pickup(entity) => match target(entity) {
            Some(resolved) => ActionCommand::Pickup(resolved),
            None => return,
        },
    };
    info!("action slot: sending {:?} (0x7074)", pending.action);
    if let Err(e) = conn
        .get_sender()
        .send(Packet::from(ObjectActionRequest::Execute(command)).into())
    {
        error!("network: failed to send ObjectActionRequest: {}", e.0);
        return;
    }
    slot.mark_sent(&pending, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast(id: u32) -> Action {
        Action::Cast {
            skill_id: id,
            target: None,
        }
    }

    /// The whole point: mashing one hotkey is one request, not fourteen. The
    /// capture's worst run is exactly this shape — 14 identical `CastSkill`
    /// sends over 2.2 s.
    #[test]
    fn mashing_one_skill_leaves_a_single_intent() {
        let mut slot = ActionSlot::default();
        for step in 0..14 {
            slot.request(cast(7185), step as f64 * 0.15);
        }
        assert_eq!(
            slot.next.as_ref().map(|p| p.action),
            Some(cast(7185)),
            "one intent survives the burst"
        );
    }

    /// Newest wins: press B then C and C is what casts.
    #[test]
    fn a_newer_press_replaces_the_waiting_one() {
        let mut slot = ActionSlot::default();
        slot.request(cast(1), 0.0);
        slot.request(cast(2), 0.1);
        assert_eq!(slot.next.as_ref().map(|p| p.action), Some(cast(2)));
    }

    /// `occupied` is the union of both signals: our own request awaiting its
    /// first ack, and an action the server told us it started. The second
    /// matters because the server runs actions we never asked for.
    #[test]
    fn the_slot_is_occupied_by_either_signal() {
        let mut slot = ActionSlot::default();
        assert!(!slot.occupied(), "an idle slot takes a request at once");
        slot.server_busy = true;
        assert!(slot.occupied());
        slot.server_busy = false;
        slot.in_flight = Some(InFlight {
            action: cast(1),
            sent_at: 0.0,
        });
        assert!(slot.occupied(), "a sent request still owns the slot");
    }

    /// A free slot must not cost the press any latency — the buffer exists
    /// for the busy case only.
    #[test]
    fn a_press_onto_a_free_slot_goes_out_at_once() {
        let mut slot = ActionSlot::default();
        slot.request(cast(7185), 10.0);
        assert_eq!(
            slot.take_ready(10.0, 3.0, false, |_| false)
                .map(|p| p.action),
            Some(cast(7185))
        );
    }

    /// While the slot is occupied the intent waits rather than being dropped
    /// or sent into a refusal, and goes out on the frame it frees.
    #[test]
    fn a_press_waits_for_the_slot_and_then_goes() {
        let mut slot = ActionSlot::default();
        slot.server_busy = true;
        slot.request(cast(7185), 10.0);
        assert!(
            slot.take_ready(10.1, 3.0, false, |_| false).is_none(),
            "still waiting"
        );
        assert!(slot.next.is_some(), "and still held, not dropped");
        slot.server_busy = false;
        assert_eq!(
            slot.take_ready(10.2, 3.0, false, |_| false)
                .map(|p| p.action),
            Some(cast(7185))
        );
    }

    /// Past the buffer window the fight has moved on: firing then would act on
    /// an intent the player has forgotten, possibly at a dead target. Dropped,
    /// not sent late.
    /// The defect this bound exists for: a `Started` with no `Ended` behind it
    /// used to latch `occupied()` for the rest of the session, so every later
    /// press waited out the buffer and was dropped as stale. Measured
    /// 2026-10-23: 375 `0xB074` frames in one session, all `Started`, none
    /// `Ended`.
    #[test]
    fn a_started_without_an_ended_stops_latching_the_slot() {
        let mut slot = ActionSlot::default();
        slot.server_busy = true;
        slot.busy_since = Some(0.0);
        assert!(slot.occupied(), "the action really is running at first");

        slot.release_stalled_send(SERVER_BUSY_TIMEOUT_SECS - 0.1);
        assert!(slot.occupied(), "a running action is not cut short");

        slot.release_stalled_send(SERVER_BUSY_TIMEOUT_SECS + 0.1);
        assert!(
            !slot.occupied(),
            "an end we were never told about is assumed"
        );

        // An `Ended` that does arrive still clears it the same frame, and the
        // timeout must not then fire on a stale timestamp.
        slot.server_busy = true;
        slot.busy_since = Some(100.0);
        slot.server_busy = false;
        slot.busy_since = None;
        slot.release_stalled_send(200.0);
        assert!(!slot.occupied());
    }

    #[test]
    fn a_stale_intent_is_dropped_rather_than_fired_late() {
        let mut slot = ActionSlot::default();
        slot.request(cast(7185), 10.0);
        assert!(slot.take_ready(13.5, 3.0, false, |_| false).is_none());
        assert!(
            slot.next.is_none(),
            "and it is gone, not left to fire later"
        );
    }

    /// A request whose ack never arrives must not park the slot forever.
    #[test]
    fn a_send_with_no_ack_releases_the_slot() {
        let mut slot = ActionSlot::default();
        slot.mark_sent(
            &Pending {
                action: cast(7185),
                at: 0.0,
            },
            0.0,
        );
        slot.release_stalled_send(ACK_TIMEOUT_SECS - 0.1);
        assert!(
            slot.occupied(),
            "a live round trip must not trip the release"
        );
        slot.release_stalled_send(ACK_TIMEOUT_SECS + 0.1);
        assert!(!slot.occupied());
    }

    /// Feed one ack to a slot whose request is in flight; returns whether the
    /// request is held to be sent again, the refusals announced, and whether
    /// the server's slot counts as taken.
    fn answer(ack: ObjectActionResponse) -> (bool, usize, bool) {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ActionSlot>()
            .add_message::<ObjectActionResponse>()
            .add_message::<ActionRefused>()
            .add_systems(Update, apply_action_acks);
        app.world_mut().resource_mut::<ActionSlot>().mark_sent(
            &Pending {
                action: cast(7185),
                at: 0.0,
            },
            0.0,
        );
        app.world_mut().write_message(ack);
        app.update();
        let slot = app.world().resource::<ActionSlot>();
        let (held, busy) = (slot.next.is_some(), slot.server_busy);
        let refusals = app
            .world_mut()
            .resource_mut::<bevy::ecs::message::Messages<ActionRefused>>()
            .drain()
            .count();
        (held, refusals, busy)
    }

    /// `01 02` is not a refusal: the server queued the request behind the
    /// running action and will start it itself. It used to be re-parked and
    /// sent again once the running action ended — the moment the server had
    /// just started it, so it ran twice.
    #[test]
    fn a_queued_request_is_not_sent_again() {
        assert_eq!(
            answer(ObjectActionResponse::Started { queue_len: 2 }),
            (false, 0, true)
        );
    }

    /// A refusal on the merits is announced and never retried.
    #[test]
    fn a_refusal_is_announced_but_not_retried() {
        assert_eq!(
            answer(ObjectActionResponse::Failed {
                queue_len: 0,
                error: 0x4004
            }),
            (false, 1, false)
        );
    }

    /// `02 01`: the running action ended and the waiting one started, so the
    /// slot stays taken until the `02 00` that ends it.
    #[test]
    fn the_waiting_action_starting_keeps_the_slot_taken() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ActionSlot>()
            .add_message::<ObjectActionResponse>()
            .add_message::<ActionRefused>()
            .add_systems(Update, apply_action_acks);
        app.world_mut().resource_mut::<ActionSlot>().server_busy = true;
        app.world_mut()
            .write_message(ObjectActionResponse::Ended { queue_len: 1 });
        app.update();
        assert!(app.world().resource::<ActionSlot>().occupied());
        app.world_mut()
            .write_message(ObjectActionResponse::Ended { queue_len: 0 });
        app.update();
        assert!(!app.world().resource::<ActionSlot>().occupied());
    }

    /// The reported defect: a press held against a busy slot must not go out
    /// at a monster that died while it waited. The buffer is what made this
    /// reachable — a corpse keeps its network id for 4 s, so resolving the id
    /// at send time is not enough on its own.
    #[test]
    fn a_press_whose_target_died_is_dropped() {
        let victim = Entity::from_raw_u32(7).unwrap();
        let mut slot = ActionSlot::default();
        slot.request(
            Action::Cast {
                skill_id: 7185,
                target: Some(victim),
            },
            10.0,
        );
        assert!(slot.take_ready(10.1, 3.0, false, |e| e == victim).is_none());
        assert!(slot.next.is_none(), "and it does not linger in the buffer");
    }

    /// ...but only the ones that actually name the dead thing. A self-buff
    /// names nothing, and an action at some other entity is unaffected.
    #[test]
    fn an_unrelated_intent_survives_a_death() {
        let corpse = Entity::from_raw_u32(7).unwrap();
        let mut slot = ActionSlot::default();
        slot.request(cast(7185), 10.0); // self-buff, target: None
        assert!(
            slot.take_ready(10.1, 3.0, false, |e| e == corpse).is_some(),
            "an untargeted cast has nothing to be dead"
        );
        slot.request(Action::Pickup(Entity::from_raw_u32(9).unwrap()), 10.0);
        assert!(
            slot.take_ready(10.1, 3.0, false, |e| e == corpse).is_some(),
            "another entity's death is not this action's business"
        );
    }

    /// `forget_target` frees the slot the moment the target dies, rather than
    /// leaving a doomed intent to occupy it until the send-time check.
    #[test]
    fn forgetting_a_target_drops_only_its_own_intent() {
        let victim = Entity::from_raw_u32(7).unwrap();
        let other = Entity::from_raw_u32(8).unwrap();
        let mut slot = ActionSlot::default();
        slot.request(Action::Attack(victim), 10.0);
        slot.forget_target(other);
        assert!(slot.next.is_some(), "someone else's death changes nothing");
        slot.forget_target(victim);
        assert!(slot.next.is_none());
    }

    /// The second reported defect: the server frees its slot (median 1.38 s)
    /// well before a cast clip ends (2.4-5.1 s for a chain), so releasing on
    /// the server alone sent the follow-up mid-animation.
    #[test]
    fn a_press_waits_for_the_cast_clip_to_finish() {
        let mut slot = ActionSlot::default();
        slot.request(cast(7185), 10.0);
        assert!(
            slot.take_ready(11.4, 5.5, true, |_| false).is_none(),
            "the server slot is free but our cast is still playing"
        );
        assert!(slot.next.is_some(), "held, not dropped");
        assert_eq!(
            slot.take_ready(12.8, 5.5, false, |_| false)
                .map(|p| p.action),
            Some(cast(7185)),
            "and it goes out when the clip ends"
        );
    }

    /// The window has to outlast the clip it now waits on, or a press made
    /// during the longest authored chain (5.1 s) would expire before the body
    /// ever freed. This is what ties `action_buffer_seconds` to the clip
    /// length rather than to the server's action window.
    #[test]
    fn a_press_survives_the_longest_chain_clip() {
        /// `skill_ch_sword_chain_h.ban`, the longest chain animation in the
        /// corpus (see `skills::cast::MAX_SWING_QUEUE_LAG_SECS`).
        const LONGEST_CLIP_SECS: f64 = 5.1;
        let mut slot = ActionSlot::default();
        slot.request(cast(7185), 0.0);
        assert!(
            slot.take_ready(LONGEST_CLIP_SECS, 5.5, true, |_| false)
                .is_none(),
            "still playing"
        );
        assert!(
            slot.take_ready(LONGEST_CLIP_SECS + 0.01, 5.5, false, |_| false)
                .is_some(),
            "the press outlives the clip it was waiting for"
        );
    }

    /// A deliberate stop must not be chased by the thing it interrupted.
    #[test]
    fn cancelling_forgets_the_waiting_intent() {
        let mut slot = ActionSlot::default();
        slot.request(cast(7185), 0.0);
        slot.clear_pending();
        assert!(slot.next.is_none());
    }
}

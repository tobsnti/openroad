//! NPCs stand where the user's data puts them: the server places one per NPC
//! row of `npcpos.txt` when it starts. The vSRO server does the same; every NPC
//! measured in its captures stands at its `npcpos.txt` row
//! [V capture, peer vSRO: 0x3019, refs 2003-2014 in region 25000].
//!
//! The file places more than NPCs: of its 12,367 rows in the user's data,
//! 12,110 are monsters and 12 name models that are not in the client's data
//! (the user's characterdata). The original client reads a monster's record
//! with more bytes [V original handler], and what is not in the client's data
//! is not sent to it. So only NPC models (`RefData::is_npc`) are placed; the
//! other rows are counted and skipped, and monsters come with their own
//! system.

use bytes::{BufMut, Bytes, BytesMut};
use packets::agent::character_data::{EntityMovement, EntityState, SpawnPosition};
use packets::agent::prelude::{CloseTalkResponse, SelectEntityResponse, TalkResponse};
use packets::Packet;
use sro_macro::Serialize;

use crate::store::Position;
use crate::world::{Phase, Reason, SessionId, World};

#[derive(Clone, Debug, PartialEq)]
pub struct Npc {
    pub ref_id: u32,
    pub position: Position,
}

/// What a click on an NPC offers, as the vSRO server sends it in the NPC's
/// spawn and in its selection (0xB045) [V capture, peer vSRO]. Where the
/// original takes it from is open [U]: the shop groups of `refshopgroup.txt`
/// fit option 02 for five of these but also name the warehouse, which has
/// none. An NPC without a row offers nothing.
const OPTIONS: [(u32, &[u8]); 10] = [
    (2003, &[0x01, 0x02, 0x04, 0x20]), // NPC_CH_SMITH
    (2004, &[0x01, 0x02, 0x04]),       // NPC_CH_ARMOR
    (2005, &[0x01, 0x02]),             // NPC_CH_POTION
    (2008, &[0x01, 0x02]),             // NPC_CH_ACCESSORY
    (2009, &[0x01, 0x02, 0x0B]),       // NPC_CH_HORSE
    (2013, &[0x01, 0x03]),             // NPC_CH_WAREHOUSE_M
    (2014, &[0x01, 0x03]),             // NPC_CH_WAREHOUSE_W
    (2021, &[0x01, 0x02, 0x0F]),       // NPC_CH_GENARAL_SP, selection only
    (2037, &[0x08]),                   // NPC_CH_SOLDIER_SO1, selection only
    (3861, &[0x02]),                   // NPC_CH_EVENT_KISAENG1, selection only
];

pub(crate) fn options(ref_id: u32) -> &'static [u8] {
    OPTIONS
        .iter()
        .find(|(id, _)| *id == ref_id)
        .map_or(&[], |(_, options)| options)
}

/// The NPC's record in a group spawn, in the order the original client reads
/// it [V original handler]. A standing NPC as the vSRO server sends it
/// [V capture, peer vSRO]: alive, speeds 0, 0 and 100, no buffs, and when it
/// offers something, interaction flag 2 with its options. `npcpos.txt` has no
/// heading; it is 0 [U].
pub(crate) fn record(uid: u32, npc: &Npc) -> Bytes {
    let at = npc.position;
    let mut b = BytesMut::new();
    b.put_u32_le(npc.ref_id);
    b.put_u32_le(uid);
    let position = SpawnPosition {
        region: at.region,
        x: at.x,
        y: at.y,
        z: at.z,
        heading: at.heading,
    };
    position.serialize_to(&mut b);
    let standing = EntityMovement {
        has_destination: false,
        move_type: 1,
        source: Some(0),
        angle: Some(at.heading),
        ..Default::default()
    };
    standing
        .write_with_region(&mut b, at.region)
        .expect("a standing block names its source and angle");
    let state = EntityState {
        life_state: 1,
        unk: 0,
        motion_state: 0,
        body_state: 0,
        walk_speed: 0.0,
        run_speed: 0.0,
        hwan_speed: 100.0,
        buffs: Vec::new(),
    };
    state.serialize_to(&mut b);
    match options(npc.ref_id) {
        [] => b.put_u8(0),
        options => {
            b.put_u8(2);
            b.put_u8(u8::try_from(options.len()).expect("a handful of options"));
            b.put_slice(options);
        }
    }
    b.freeze()
}

/// 0x7045. Selecting an NPC the client was shown is answered with what the
/// original client reads for one [V original handler]: no vital mask, the
/// NPC's options, the NPC byte 0, and for an NPC of a listed fortress two
/// more bytes, 0, as the vSRO server sent for the Jangan smith
/// [V capture, peer vSRO]. Selecting anything else is not handled yet.
pub(crate) fn select(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::SelectEntityRequest(request) = packet else {
        unreachable!("0x7045 decodes to SelectEntityRequest");
    };
    let uid = request.unique_id;
    let ref_id = shown(world, session, uid).ok_or(Reason::NotHandled)?;
    let options = options(ref_id);
    let mut tail = BytesMut::new();
    tail.put_u8(0);
    tail.put_u8(u8::try_from(options.len()).expect("a handful of options"));
    tail.put_slice(options);
    tail.put_u8(0);
    if world.refdata().in_listed_fortress(ref_id) {
        tail.put_u16_le(0);
    }
    let answer = SelectEntityResponse {
        result: 1,
        unique_id: uid,
        tail: tail.freeze(),
    };
    world.send(session, Packet::SelectEntityResponse(answer));
    Ok(())
}

/// 0x7046. Talking to an NPC the client was shown, about an option it offers,
/// is answered `01` and the option, as the vSRO server answered
/// [V capture, peer vSRO]; option 0x0C carries one byte more (A3). Anything
/// else is refused.
pub(crate) fn talk(world: &mut World, session: SessionId, packet: Packet) -> Packet {
    let Packet::TalkRequest(request) = packet else {
        unreachable!("0x7046 decodes to TalkRequest");
    };
    let option = request.talk_flag;
    let offered = shown(world, session, request.unique_id)
        .is_some_and(|ref_id| options(ref_id).contains(&option));
    if !offered {
        return talk_refusal();
    }
    let tail = match option {
        0x0C => Bytes::from_static(&[0x0C, 0x00]),
        _ => Bytes::copy_from_slice(&[option]),
    };
    Packet::TalkResponse(TalkResponse::Success { tail })
}

/// Code 0, which the client words as "invalid target" [V original handler].
pub(crate) fn talk_refusal() -> Packet {
    Packet::TalkResponse(TalkResponse::Failure(0))
}

/// 0x704B. Closing the talk with an NPC the client was shown is answered
/// `01`, as the vSRO server answered [V capture, peer vSRO]. Anything else is
/// not handled yet.
pub(crate) fn close(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::CloseTalkRequest(request) = packet else {
        unreachable!("0x704B decodes to CloseTalkRequest");
    };
    shown(world, session, request.unique_id).ok_or(Reason::NotHandled)?;
    world.send(
        session,
        Packet::CloseTalkResponse(CloseTalkResponse::Success),
    );
    Ok(())
}

/// The model of NPC `uid`, if the client of `session` is in the game and
/// was shown it.
fn shown(world: &World, session: SessionId, uid: u32) -> Option<u32> {
    let Some(Phase::Game { .. }) = world.phase(session) else {
        return None;
    };
    let npc = world.npc(uid)?;
    world.knows(session, uid).then_some(npc.ref_id)
}

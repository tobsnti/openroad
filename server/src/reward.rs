//! What a kill gives: the killer the monster's EXP, and the ground the
//! monster's gold ([`ground`]). The EXP is the monster's ExpToGive
//! (characterdata), given as EXP and as SP-EXP alike. This is a
//! deliberate simplification (decision 0021 E1) and differs from the vSRO
//! server. For a Mangnyang (ExpToGive 24), vSRO gave a level 1 character 23
//! EXP and 119 SP-EXP, and gave less the higher the killer's level was above
//! the monster's [V capture, peer vSRO]. Its formula is [U] until the re-team
//! reads it. The frame names the monster as the origin, and every full 400
//! SP-EXP turns into a skill point, which the client is told [V capture,
//! peer vSRO].

use bytes::Bytes;
use packets::agent::ingame::{CharacterPointsUpdate, ReceiveExperience};
use packets::Packet;

use crate::ground;
use crate::world::World;

/// SP-EXP per skill point [V capture, peer vSRO: 0x304E 2 followed exactly
/// the 68 of 147 EXP frames that took the SP-EXP past a multiple of 400].
const SP_EXP_PER_POINT: u32 = 400;

/// Player `killer` killed monster `uid`: it gains the monster's EXP and
/// SP-EXP (0x3056, the monster as origin [V capture, peer vSRO: 142 of 142
/// EXP frames after a kill]), the client hears of a new skill point (0x304E
/// 2), and the monster drops its gold for the killer.
pub(crate) fn killed(world: &mut World, killer: u32, uid: u32) {
    let model = world.monster(uid).map(|m| m.ref_id);
    let exp = model
        .and_then(|m| world.refdata().monster(m))
        .map_or(0, |m| m.exp);
    let Some(player) = world.player_mut(killer) else {
        return;
    };
    let character = &mut player.character;
    character.exp += u64::from(exp);
    let sp_exp = character.skill_exp + exp;
    let points = sp_exp / SP_EXP_PER_POINT;
    character.skill_exp = sp_exp % SP_EXP_PER_POINT;
    character.skill_points += points;
    let (session, skill_points, owner) = (player.session, character.skill_points, character.id);
    let gained = ReceiveExperience {
        exp_origin: uid,
        experience: i64::from(exp),
        sp_exp: u64::from(exp),
        unknown: 0,
        tail: Bytes::new(),
    };
    world.send(session, Packet::ReceiveExperience(gained));
    if points > 0 {
        let sp = CharacterPointsUpdate::Sp {
            amount: skill_points,
            display: 0,
        };
        world.send(session, Packet::CharacterPointsUpdate(sp));
    }
    ground::monster_died(world, uid, owner);
}

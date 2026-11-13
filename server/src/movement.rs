//! Walking: a client asks to go somewhere (0x7021), and the world moves its
//! character there, a step at a time, at its run speed.
//!
//! An accepted request gets exactly one 0xB021, a refused one none: the
//! original server builds 0xB021 only at the end of its success path
//! [V original server]. The answer carries no start position; the captured
//! vSRO server leaves it out in a quarter of its frames (1,543 of 6,180)
//! [V capture, peer vSRO], and the client walks from where it is.
//!
//! This slice moves within the character's region: a target in another
//! region or in a dungeon is refused until the region arithmetic is shared
//! with the client.

use packets::agent::ingame::is_dungeon;
use packets::agent::prelude::{MovementRequest, MovementResponse};
use packets::Packet;

use crate::tick::STEPS_PER_SECOND;
use crate::world::{Phase, Player, Point, Reason, SessionId, Walk, World};

/// The edge of a region in world units, the grid of the terrain files.
const REGION_SIZE: i32 = 1920;

/// 0x7021.
pub(crate) fn request(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::MovementRequest(request) = packet else {
        unreachable!("0x7021 decodes to MovementRequest");
    };
    let Some(Phase::Game { uid, .. }) = world.phase(session) else {
        return Err(Reason::NotHandled);
    };
    let player = world
        .player_mut(uid)
        .expect("a character in the game has a player");
    let answer = match request {
        MovementRequest::ToLocation { region, x, y, z } => {
            let inside = |v: i32| (0..=REGION_SIZE).contains(&v);
            if region != player.position.region || is_dungeon(region) || !inside(x) || !inside(z) {
                return Ok(());
            }
            let at = player.position;
            player.walk = Some(Walk {
                from: Point {
                    x: at.x,
                    y: at.y,
                    z: at.z,
                },
                to: Point {
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                },
                steps: 0,
            });
            MovementResponse {
                unique_id: uid,
                has_destination: true,
                region,
                x,
                y,
                z,
                angle: 0,
            }
        }
        MovementRequest::ToAngle { angle } => {
            player.walk = None;
            player.position.heading = angle;
            MovementResponse {
                unique_id: uid,
                has_destination: false,
                region: 0,
                x: 0,
                y: 0,
                z: 0,
                angle,
            }
        }
    };
    world.send(session, Packet::MovementResponse(answer));
    Ok(())
}

/// One step of a walking character. Its position follows from the steps the
/// walk has taken at its run speed, not from adding up per-step moves, so the
/// float error does not grow with the walk. Within a millionth of a unit of
/// the target counts as there.
pub(crate) fn advance(player: &mut Player) {
    let Some(walk) = &mut player.walk else {
        return;
    };
    walk.steps += 1;
    let (from, to) = (walk.from, walk.to);
    let (dx, dy, dz) = (to.x - from.x, to.y - from.y, to.z - from.z);
    let length = f64::from(dx).hypot(f64::from(dz));
    let speed = f64::from(player.run_speed) / STEPS_PER_SECOND as f64;
    let covered = f64::from(walk.steps) * speed;
    let at = &mut player.position;
    if covered + 1e-6 >= length {
        (at.x, at.y, at.z) = (to.x, to.y, to.z);
        player.walk = None;
    } else {
        let share = (covered / length) as f32;
        (at.x, at.y, at.z) = (
            from.x + dx * share,
            from.y + dy * share,
            from.z + dz * share,
        );
    }
}

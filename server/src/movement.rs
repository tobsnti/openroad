//! Walking: a client asks to go somewhere (0x7021), and the world moves its
//! character there, a step at a time, at its run speed.
//!
//! An accepted request gets exactly one 0xB021, to the client and to every
//! client the character is shown to; a refused one gets none: the
//! original server builds 0xB021 only at the end of its success path
//! [V original server]. The answer carries no start position; the captured
//! vSRO server leaves it out in a quarter of its frames (1,543 of 6,180)
//! [V capture, peer vSRO], and the client walks from where it is.
//!
//! A walk may cross into other regions, so it runs in world coordinates
//! (`packets::region`). Dungeons are refused: their coordinates are local to
//! the dungeon, outside the region grid.

use packets::agent::prelude::{MovementRequest, MovementResponse};
use packets::region::{is_dungeon, region_id, sector, REGION_SIZE};
use packets::Packet;

use crate::tick::STEPS_PER_SECOND;
use crate::world::{Phase, Player, Point, Reason, SessionId, Walk, World};

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
            let at = player.character.position;
            let inside = |v: i32| (0..=REGION_SIZE as i32).contains(&v);
            if is_dungeon(region) || is_dungeon(at.region) || !inside(x) || !inside(z) {
                return Ok(());
            }
            let here = Point {
                x: at.x,
                y: at.y,
                z: at.z,
            };
            let there = Point {
                x: x as f32,
                y: y as f32,
                z: z as f32,
            };
            player.walk = Some(Walk {
                from: in_world(at.region, here),
                to: in_world(region, there),
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
            player.character.position.heading = angle;
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
    let observers = world.observers(uid);
    world.broadcast(&observers, Packet::MovementResponse(answer));
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
    let now = if covered + 1e-6 >= length {
        player.walk = None;
        to
    } else {
        let share = (covered / length) as f32;
        Point {
            x: from.x + dx * share,
            y: from.y + dy * share,
            z: from.z + dz * share,
        }
    };
    let (region, local) = in_region(now);
    let at = &mut player.character.position;
    (at.region, at.x, at.y, at.z) = (region, local.x, local.y, local.z);
}

/// A point of an overworld region in world coordinates: the region's sector
/// times [`REGION_SIZE`], plus the point.
pub(crate) fn in_world(region: u16, local: Point) -> Point {
    let (x, z) = sector(region);
    Point {
        x: f32::from(x) * REGION_SIZE + local.x,
        y: local.y,
        z: f32::from(z) * REGION_SIZE + local.z,
    }
}

/// The region a world point lies in, and the point in it.
pub(crate) fn in_region(world: Point) -> (u16, Point) {
    let (x, z) = (
        (world.x / REGION_SIZE).floor(),
        (world.z / REGION_SIZE).floor(),
    );
    let local = Point {
        x: world.x - x * REGION_SIZE,
        y: world.y,
        z: world.z - z * REGION_SIZE,
    };
    (region_id(x as u8, z as u8), local)
}

//! The consignment trade, as far as the server holds it: no listings yet.

use packets::agent::prelude::ConsignmentListResponse;
use packets::Packet;

use crate::world::{Phase, Reason, SessionId, World};

/// 0x750E, which the original client sends after the entry. There are no
/// listings, so the answer is success with none, as the vSRO server answered
/// both captured requests (`01 00`) [V capture, peer vSRO].
pub(crate) fn list(world: &mut World, session: SessionId, _: Packet) -> Result<(), Reason> {
    let Some(Phase::Game { .. }) = world.phase(session) else {
        return Err(Reason::NotHandled);
    };
    let none = ConsignmentListResponse {
        result: 1,
        count: Some(0),
        listings: Vec::new(),
        error: None,
    };
    world.send(session, Packet::ConsignmentListResponse(none));
    Ok(())
}

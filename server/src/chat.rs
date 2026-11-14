//! Chat in the game, the local channel (type 1, "all"): a line goes to the
//! speaker and to every client the speaker is shown to (0x3026), then the
//! speaker gets the acknowledgement (0xB025). The vSRO server does it in that
//! order for a GM's local line, as type 3 [V capture, peer vSRO: two GM
//! accounts, each received its own line]; for an ordinary player the type
//! stays 1 [S]. The other channels come with parties, guilds and whispers.

use packets::agent::chat::chat_type;
use packets::agent::prelude::{ChatRequest, ChatResponse, ChatUpdate};
use packets::Packet;

use crate::world::{Phase, Reason, SessionId, World};

/// 0x7025.
pub(crate) fn say(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::ChatRequest(request) = packet else {
        unreachable!("0x7025 decodes to ChatRequest");
    };
    let Some(Phase::Game { uid, .. }) = world.phase(session) else {
        return Err(Reason::NotHandled);
    };
    let ChatRequest {
        chat_type: chat_type::ALL,
        chat_index,
        message,
        ..
    } = request
    else {
        return Err(Reason::NotHandled);
    };
    let line = ChatUpdate::All {
        sender_id: uid,
        message,
    };
    let listeners = world.observers(uid);
    world.broadcast(&listeners, Packet::ChatUpdate(line));
    let ack = ChatResponse {
        result: 1,
        error: None,
        chat_type: chat_type::ALL,
        chat_index,
    };
    world.send(session, Packet::ChatResponse(ack));
    Ok(())
}

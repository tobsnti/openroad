//! Before the world: the client names itself, logs in with the ticket the
//! gateway gave it, and lists its characters.
//!
//! A session starts in [`Phase::Title`] and reaches [`Phase::Lobby`] with a
//! good ticket. A ticket is good for one login, under the account name the
//! gateway checked the password of; a refused login closes the session.

use packets::agent::prelude::*;
use packets::global::{KeepAlive, ModuleIdentification};
use packets::Packet;

use crate::store::Character;
use crate::world::{Phase, Reason, SessionId, World};

/// 0x2001. The agent names itself as the original does: `AgentServer` and a
/// zero [V capture, peer vSRO].
/// A heartbeat (0x2002) gets an empty one back, in every phase. The original
/// client hangs up 10 s after the server's last frame, measured twice at the
/// character selection; the stub answers every heartbeat, and the original
/// client stayed. A deliberate, provisional deviation [S]: not one of 44
/// captures against the real vSRO server carries an s2c 0x2002, although our
/// dumper records the stub's; the real behaviour in the lobby is open [U].
pub(crate) fn keep_alive(world: &mut World, session: SessionId, _: Packet) -> Result<(), Reason> {
    world.send(session, Packet::KeepAlive(KeepAlive));
    Ok(())
}

pub(crate) fn identify(world: &mut World, session: SessionId, _: Packet) -> Result<(), Reason> {
    if world.phase(session) != Some(Phase::Title) {
        return Err(Reason::NotHandled);
    }
    let me = ModuleIdentification {
        module: "AgentServer".to_owned(),
        tail: Some(0),
        extra: None,
    };
    world.send(session, Packet::ModuleIdentification(me));
    Ok(())
}

/// 0x6103. A refusal carries code 1, which the client shows as "Failed to
/// connect to server" [V original handler]: it has no text for a bad ticket,
/// and a code it does not know leaves it waiting.
pub(crate) fn login(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::AgentLoginRequest(request) = packet else {
        unreachable!("0x6103 decodes to AgentLoginRequest");
    };
    if world.phase(session) != Some(Phase::Title) {
        return Err(Reason::NotHandled);
    }
    let (result, error_code, phase) = match world.take_ticket(request.token) {
        Some(ticket) if ticket.username == request.username => {
            (1, None, Phase::Lobby(ticket.account))
        }
        _ => (2, Some(1), Phase::Closed),
    };
    let answer = AgentLoginResponse {
        result,
        error_code,
        extra: None,
    };
    world.send(session, Packet::AgentLoginResponse(answer));
    world.enter(session, phase);
    Ok(())
}

/// 0x7007. Listing is all this lobby does yet; creating, deleting, checking a
/// name and restoring come with writes to the store.
pub(crate) fn lobby(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::CharacterSelectionActionRequest(request) = packet else {
        unreachable!("0x7007 decodes to CharacterSelectionActionRequest");
    };
    let (Some(Phase::Lobby(account)), CharacterSelectionAction::List) =
        (world.phase(session), request.action)
    else {
        return Err(Reason::NotHandled);
    };
    let characters: Vec<LobbyCharacter> = world
        .store()
        .characters(account)
        .iter()
        .map(lobby_character)
        .collect();
    let count = u8::try_from(characters.len()).expect("a lobby lists at most 255 characters");
    let answer = CharacterSelectionActionResponse {
        action: CharacterSelectionAction::List,
        result: 1,
        characters: Some(CharacterData { count, characters }),
        error_code: None,
    };
    world.send(session, Packet::CharacterSelectionActionResponse(answer));
    Ok(())
}

/// No guild or academy yet: class 0, which 529 of the 531 characters in the
/// captured vSRO lists carry [V capture, peer vSRO]. Equipment comes with the
/// inventory.
fn lobby_character(c: &Character) -> LobbyCharacter {
    LobbyCharacter {
        ref_obj_id: c.ref_id,
        name: c.name.clone(),
        scale: c.scale,
        level: c.level,
        exp_offset: c.exp,
        str: c.strength,
        int: c.intellect,
        stat_points: c.stat_points,
        hp: c.hp,
        mp: c.mp,
        is_deleting: false,
        deletion_time: None,
        guild_member_class: 0,
        is_guild_rename_required: false,
        current_guild_name: None,
        academy_member_class: 0,
        char_items: Vec::new(),
        avatar_items: Vec::new(),
    }
}

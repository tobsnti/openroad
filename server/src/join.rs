//! From the lobby into the world.
//!
//! 0x7001 names a character of the account. The answer is 0xB001, then the
//! character record as the client reads it (0x34A5, 0x3013, 0x34A6) and the
//! in-game time (0x3020). The client then loads the world and says so with
//! 0x3012. Until then the session gets no other frame: a chat or notice frame
//! in that window crashes the original client, whose chat path reads the
//! guild window before it exists [V original handler].
//!
//! The record is built from the stored character. What the store does not
//! hold yet is set to what a character without it carries in the 243 captured
//! vSRO records [V capture, peer vSRO], each value marked where it is set.

use std::time::{SystemTime, UNIX_EPOCH};

use packets::agent::character_data::{
    write_character_info, CharacterStats, CollectionBook, EntityMovement, EntityState, ItemClass,
    ItemClassResolver, JobInfo, Mastery, ParsedCharacterData, ParsedCharacterInfo, PlayerExtras,
};
use packets::agent::prelude::{
    CelestialPosition, CharacterDataBegin, CharacterDataBody, CharacterDataEnd,
    CharacterJoinResponse, CharacterStatsUpdate,
};
use packets::Packet;

use crate::clock::{game_time, server_time};
use crate::refdata::CharacterRef;
use crate::sight;
use crate::store::{AccountId, Character};
use crate::world::{Event, Phase, Player, Reason, SessionId, World};

/// The client words 0x40D as "Invalid character name." [V original handler];
/// it has no text for a character of another account.
const NOT_THE_ACCOUNTS: u16 = 0x40D;
/// "Login failed" with the code appended [V original handler].
const LOGIN_FAILED: u16 = 0x415;

/// 0x7001.
pub(crate) fn join(world: &mut World, session: SessionId, packet: Packet) -> Result<(), Reason> {
    let Packet::CharacterJoinRequest(request) = packet else {
        unreachable!("0x7001 decodes to CharacterJoinRequest");
    };
    let Some(Phase::Lobby(account)) = world.phase(session) else {
        return Err(Reason::NotHandled);
    };
    let characters = world.store().characters(account);
    let Some(character) = characters.iter().find(|c| c.name == request.character_name) else {
        world.send(session, refusal(NOT_THE_ACCOUNTS));
        return Ok(());
    };
    let Some(speeds) = world.refdata().character(character.ref_id) else {
        let ref_id = character.ref_id;
        world.report(Event::NoRefData { session, ref_id });
        world.send(session, refusal(LOGIN_FAILED));
        return Ok(());
    };
    let uid = world.next_uid();
    let info = record(character, uid, speeds, account);
    let raw = write_character_info(&info, &NoItems).expect("the record has every section");
    let (day, hour, minute) = game_time(world.steps());
    let joined = CharacterJoinResponse {
        result: 1,
        error: None,
    };
    world.send(session, Packet::CharacterJoinResponse(joined));
    world.send(session, Packet::CharacterDataBegin(CharacterDataBegin));
    world.send(
        session,
        Packet::CharacterDataBody(CharacterDataBody { raw }),
    );
    world.send(session, Packet::CharacterDataEnd(CharacterDataEnd));
    let time = CelestialPosition {
        unique_id: uid,
        day,
        hour,
        minute,
    };
    world.send(session, Packet::CelestialPosition(time));
    let player = Player {
        session,
        name: character.name.clone(),
        position: character.position,
        hp: character.hp,
        mp: character.mp,
        ref_id: character.ref_id,
        scale: character.scale,
        inventory_size: character.inventory_size,
        walk_speed: speeds.walk_speed,
        run_speed: speeds.run_speed,
        walk: None,
    };
    world.add_player(uid, player);
    world.enter(session, Phase::Joining { account, uid });
    Ok(())
}

/// 0x3012: the client has loaded the world.
pub(crate) fn game_ready(world: &mut World, session: SessionId, _: Packet) -> Result<(), Reason> {
    let Some(Phase::Joining { account, uid }) = world.phase(session) else {
        return Err(Reason::NotHandled);
    };
    world.enter(session, Phase::Game { account, uid });
    let name = world.player(uid).map(|p| p.name.clone());
    let characters = world.store().characters(account);
    if let Some(c) = characters.iter().find(|c| Some(&c.name) == name.as_ref()) {
        world.send(session, Packet::CharacterStatsUpdate(values(c)));
    }
    sight::enter(world, uid);
    world.report(Event::Joined { session, uid });
    Ok(())
}

/// The character's values (0x303D), which the vSRO server sends after the
/// entry [V capture, peer vSRO: 388 frames]; without them the original
/// client shows its HP and MP as 0/0. There are no items and no formulas for
/// attack, defence, hit and parry yet, so these are 0 [U], and the maximum HP
/// and MP are the stored ones, as if the character entered in full [U].
fn values(c: &Character) -> CharacterStatsUpdate {
    CharacterStatsUpdate {
        phys_attack_min: 0,
        phys_attack_max: 0,
        mag_attack_min: 0,
        mag_attack_max: 0,
        phys_defense: 0,
        mag_defense: 0,
        hit_rate: 0,
        parry_rate: 0,
        max_hp: c.hp,
        max_mp: c.mp,
        strength: c.strength,
        intelligence: c.intellect,
    }
}

fn refusal(code: u16) -> Packet {
    let refused = CharacterJoinResponse {
        result: 2,
        error: Some(code),
    };
    Packet::CharacterJoinResponse(refused)
}

/// The server holds no items yet, so no item class is ever asked for.
struct NoItems;

impl ItemClassResolver for NoItems {
    fn item_class(&self, _: u32) -> ItemClass {
        ItemClass::Unknown
    }
}

fn record(
    c: &Character,
    uid: u32,
    speeds: CharacterRef,
    account: AccountId,
) -> ParsedCharacterInfo {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let at = c.position;
    let stats = CharacterStats {
        server_time: server_time(now),
        ref_id: c.ref_id,
        scale: c.scale,
        level: c.level,
        // The highest level reached; the store keeps no other yet.
        max_level: c.level,
        exp: c.exp,
        skill_exp: c.skill_exp,
        gold: c.gold,
        skill_points: c.skill_points,
        stat_points: c.stat_points,
        // No berserk points, auto-invest or PK record yet.
        berserk_points: 0,
        unk_u32: 0, // 243 of 243
        hp: c.hp,
        mp: c.mp,
        auto_invest_exp: 0,
        daily_pk: 0,
        total_pk: 0,
        pk_penalty: 0,
        berserk_level: 0,
        free_pvp: 0,
    };
    let spawn = ParsedCharacterData {
        unique_id: uid,
        ref_id: c.ref_id,
        region: at.region,
        x: at.x,
        y: at.y,
        z: at.z,
        heading: at.heading,
    };
    // Standing and facing its heading: 243 of 243.
    let movement = EntityMovement {
        has_destination: false,
        move_type: 1,
        source: Some(0),
        angle: Some(at.heading),
        ..Default::default()
    };
    let state = EntityState {
        life_state: 0, // with the next three: 243 of 243
        unk: 0,
        motion_state: 0,
        body_state: 0,
        walk_speed: speeds.walk_speed,
        run_speed: speeds.run_speed,
        hwan_speed: 100.0, // 243 of 243; no characterdata column is known for it
        buffs: Vec::new(),
    };
    // No job: 158 of 243.
    let job = JobInfo {
        job_name: String::new(),
        job_type: 0,
        job_level: 1,
        job_exp: 0,
        job_contribution: 0,
        job_reward: 0,
    };
    let extras = PlayerExtras {
        pvp_state: 0, // with the next two: 243 of 243
        transport_flag: 0,
        in_combat: 0,
        transport_id: None,
        pvp_flag: 255,  // 243 of 243
        guide_flag: 0,  // no guide seen yet [S]
        jid: account.0, // the account; the captures carry 1 to 8 [S]
        gm: false,
        activation_flag: 7, // 234 of 243, meaning unknown
        hotkeys: Vec::new(),
        auto_hp: 0,
        auto_mp: 0,
        auto_universal: 0,
        auto_potion_delay: 0,
        blocked_whispers: Vec::new(),
        unk_u32: 0x0001_0001, // 243 of 243, meaning unknown
        unk_u8: 0,            // 243 of 243
    };
    let masteries = c.masteries.iter();
    ParsedCharacterInfo {
        stats: Some(stats),
        inventory_size: Some(c.inventory_size),
        inventory: Some(Vec::new()),
        avatar_inventory_size: Some(5), // 243 of 243
        avatar_items: Some(Vec::new()),
        masteries: Some(
            masteries
                .map(|&(id, level)| Mastery { id, level })
                .collect(),
        ),
        skills: Some(Vec::new()),
        mastery_skill_unknown: Some([0, 0]), // 243 of 243
        completed_quests: Some(Vec::new()),
        // Active quests and buffs stay out until their layout is read [U].
        active_quests: Some(Vec::new()),
        collection: Some(CollectionBook::default()), // 243 of 243 have none started
        spawn: Some(spawn),
        movement: Some(movement),
        state: Some(state),
        name: Some(c.name.clone()),
        job: Some(job),
        extras: Some(extras),
        ..Default::default()
    }
}

//! NPCs and monsters in-process: placed from the reference data, seen in
//! groups by the characters around them.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::character_data::{EntityMovement, EntityState, SpawnPosition};
use packets::agent::entity_spawn::{
    parse_group_spawn, parse_single_spawn, Interaction, SingleTail, SpawnKind, SpawnedEntity,
};
use packets::agent::ingame::{
    ActionCommand, ActionKind, ActionTarget, CharacterPointsUpdate, EntityBarsUpdate, HitEffect,
    ObjectActionRequest, ObjectActionResponse, ObjectActionUpdate,
};
use packets::agent::prelude::{
    AgentLoginRequest, CharacterJoinRequest, CloseTalkRequest, MovementRequest, MovementResponse,
    SelectEntityRequest, TalkRequest,
};
use packets::region::{sector, REGION_SIZE};
use packets::Packet;
use server::refdata::RefData;
use server::store::{login, Character, MemoryStore, Position, Store};
use server::world::{Phase, Point, SessionId, World};
use sro_textdata::characterdata::CharacterData;
use sro_textdata::spawn::SpawnTables;

const REGION: u16 = 25000;

/// Rows as `npcpos.txt` has them: the smith in the start region (4 options),
/// a warehouse one region east (2 options), an NPC without options two
/// regions east, one in a dungeon; and, not placed, a model characterdata
/// does not have. Monsters have tests of their own (`tests/monsters.rs`).
const NPCPOS: &str = "2003\t25000\t332.73001\t0.0\t1406.7\r\n\
                      2013\t25001\t980.79999\t-32.43\t989.13\r\n\
                      2028\t25002\t100.0\t0.0\t100.0\r\n\
                      15004\t-32764\t100.0\t0.0\t200.0\r\n\
                      9252\t25000\t950.0\t0.0\t1400.0\r\n";

/// The models' characterdata rows as far as the server reads them: the type
/// ids, NPC (1/2/2) for the four NPCs, and the fortress column, which names
/// Jangan for the smith only; and Jangan's row of `siegefortress.txt`.
fn model_rows(refdata: &mut RefData) {
    for (ref_id, tid3, fortress) in [
        ("2003", "2", "FORTRESS_JANGAN"),
        ("2013", "2", "xxx"),
        ("2028", "2", "xxx"),
        ("15004", "2", "xxx"),
    ] {
        let mut row = vec![String::from("0"); 60];
        (row[1], row[9], row[10], row[11], row[55]) = (
            ref_id.into(),
            "1".into(),
            "2".into(),
            tid3.into(),
            fortress.into(),
        );
        refdata.add_characters(&row.join("\t"));
    }
    refdata.add_fortresses(
        "1\t1\tFORTRESS_JANGAN\t?\tSN_FORTRESS_JANGAN\tGATE_CH\t0\t300\t30\t25\t63\t1000000\tetc\\fort_jangan.ddj\tNPC_CH_FORTRESS_OFFICIAL",
    );
}

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 0x22,
        level: 10,
        exp: 0,
        skill_exp: 0,
        gold: 0,
        skill_points: 0,
        strength: 20,
        intellect: 20,
        stat_points: 0,
        hp: 1000,
        mp: 1000,
        inventory_size: 109,
        masteries: Vec::new(),
        position: Position {
            region: REGION,
            x: 969.0,
            y: 0.0,
            z: 1419.0,
            heading: 0,
        },
        equipment: Vec::new(),
        avatar: Vec::new(),
    }
}

fn world() -> (World, Arc<MemoryStore>) {
    world_with(refdata())
}

/// A world whose chance repeats from run to run.
fn world_with(refdata: RefData) -> (World, Arc<MemoryStore>) {
    seeded_with(refdata, 19)
}

fn seeded_with(refdata: RefData, seed: u64) -> (World, Arc<MemoryStore>) {
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![hero()]);
    let store = Arc::new(store);
    (World::seeded(store.clone(), refdata, seed), store)
}

/// The hero's model, the rows of [`model_rows`] and the NPCs of [`NPCPOS`].
fn refdata() -> RefData {
    let mut refdata = hero_model();
    model_rows(&mut refdata);
    refdata.add_npcs(NPCPOS);
    refdata
}

/// The model 1907, a player's (type ids 1/1) with its walk and run speeds.
fn hero_model() -> RefData {
    let mut refdata = RefData::default();
    let mut row = vec![String::from("0"); 48];
    (row[1], row[9], row[10], row[46], row[47]) = (
        "1907".into(),
        "1".into(),
        "1".into(),
        "16".into(),
        "50".into(),
    );
    refdata.add_characters(&row.join("\t"));
    refdata
}

/// Monster rows as `npcpos.txt` has them: a Mangnyang and a Tiger Woman in
/// the start region, a Mangnyang two regions east.
const MONSTERPOS: &str = "1933\t25000\t900.0\t0.0\t1400.0\r\n\
                          1954\t25000\t1000.0\t0.0\t1500.0\r\n\
                          1933\t25002\t100.0\t0.0\t100.0\r\n";

/// The monsters' characterdata rows with the values of the user's data:
/// MOB_CH_MANGNYANG, a normal monster (1/2/1, rarity 0, speeds 8 and 22,
/// 54 HP, 24 EXP), and MOB_CH_TIGERWOMAN, a unique (rarity 3, speeds 20 and
/// 90, 598720 HP, 451200 EXP).
fn monster_rows() -> String {
    let row = |id: &str, rarity: &str, (walk, run): (&str, &str), hp: &str, exp: &str| {
        let mut row = vec![String::from("0"); 80];
        (row[1], row[9], row[10], row[11], row[12]) =
            (id.into(), "1".into(), "2".into(), "1".into(), "1".into());
        (row[15], row[46], row[47], row[59]) = (rarity.into(), walk.into(), run.into(), hp.into());
        row[79] = exp.into();
        row.join("\t")
    };
    let rows = [
        row("1933", "0", ("8", "22"), "54", "24"),
        row("1954", "3", ("20", "90"), "598720", "451200"),
    ];
    rows.join("\n")
}

/// The hero's model and the monsters of [`MONSTERPOS`].
fn monster_world() -> (World, Arc<MemoryStore>) {
    monsters_at(MONSTERPOS)
}

/// The hero's model and the monsters of the rows `npcpos`.
fn monsters_at(npcpos: &str) -> (World, Arc<MemoryStore>) {
    world_with(monster_data(npcpos))
}

fn monster_data(npcpos: &str) -> RefData {
    let mut refdata = hero_model();
    refdata.add_characters(&monster_rows());
    refdata.add_npcs(npcpos);
    refdata
}

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

/// Hero in the game; what the session was sent before 0x3012 is dropped.
fn enter((world, store): &mut (World, Arc<MemoryStore>)) -> (SessionId, u32) {
    let account = login(store.as_ref(), "a", "a").unwrap();
    let token = world
        .admit(account, "a", store.characters(account))
        .unwrap();
    let s = world.connect();
    let login = AgentLoginRequest {
        token,
        username: "a".to_owned(),
        password: String::new(),
        content_id: 22,
        mac_address: [0; 6],
    };
    send(world, s, Packet::AgentLoginRequest(login));
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let join = CharacterJoinRequest {
        character_name: "Hero".to_owned(),
    };
    send(world, s, Packet::CharacterJoinRequest(join));
    world.take_frames(s);
    world.receive(s, 0x3012, Bytes::new());
    let Some(Phase::Game { uid, .. }) = world.phase(s) else {
        panic!("Hero is in the game");
    };
    (s, uid)
}

fn uid_of(world: &World, ref_id: u32) -> u32 {
    world.npcs().find(|(_, n)| n.ref_id == ref_id).unwrap().0
}

/// The kind of the one group in `frames` (1 spawn, 2 despawn) and the uids it
/// names: a despawn lists them, a spawn's records start with ref and uid and
/// are as long as their options make them.
fn group(frames: &[(u16, Bytes)]) -> (u8, Vec<u32>) {
    let codes: Vec<u16> = frames.iter().map(|f| f.0).collect();
    assert_eq!(codes, [0x3017, 0x3019, 0x3018]);
    let (kind, count) = (
        frames[0].1[0],
        u16::from_le_bytes([frames[0].1[1], frames[0].1[2]]),
    );
    let data = &frames[1].1;
    let u32_at = |at: usize| u32::from_le_bytes(data[at..at + 4].try_into().unwrap());
    let mut uids = Vec::new();
    let mut at = 0;
    while at < data.len() {
        if kind == 2 {
            uids.push(u32_at(at));
            at += 4;
            continue;
        }
        uids.push(u32_at(at + 4));
        // ref, uid, position 16, standing movement 5, state 17, then the
        // interaction: a 0, or 2, the count and the options.
        let interaction = at + 8 + 16 + 5 + 17;
        at = interaction
            + if data[interaction] == 0 {
                1
            } else {
                2 + data[interaction + 1] as usize
            };
    }
    assert_eq!((at, uids.len()), (data.len(), count as usize));
    (kind, uids)
}

/// The frames `session` was sent, but for the entering character's own values
/// (0x303D), which are not about sight.
fn frames(world: &mut World, session: SessionId) -> Vec<(u16, Bytes)> {
    let frames = world.take_frames(session).into_iter();
    frames
        .filter(|f| f.opcode != 0x303D)
        .map(|f| (f.opcode, f.body))
        .collect()
}

/// NPC models are placed as NPCs; the model not in the client's data is
/// counted and skipped.
#[test]
fn npcs_stand_where_the_data_puts_them() {
    let (world, _) = world();
    assert_eq!(world.skipped_npc_rows(), 1);
    let placed: Vec<(u32, u16, f32)> = world
        .npcs()
        .map(|(_, n)| (n.ref_id, n.position.region, n.position.x))
        .collect();
    assert_eq!(
        placed,
        [
            (2003, 25000, 332.73),
            (2013, 25001, 980.8),
            (2028, 25002, 100.0),
            (15004, 0x8004, 100.0)
        ]
    );
}

#[test]
fn a_character_entering_sees_the_npcs_around_it_in_one_group() {
    let mut shard = world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let expected = vec![uid_of(world, 2003), uid_of(world, 2013)];
    assert_eq!(group(&frames(world, s)), (1, expected));
}

/// The group comes from the shared writer: the spawn reader in `packets`
/// reads it back as the NPCs, with their options, to its last byte.
#[test]
fn the_npc_group_reads_back_as_the_npcs() {
    let mut shard = world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let (smith, warehouse) = (uid_of(world, 2003), uid_of(world, 2013));
    let seen = frames(world, s);
    let data = &seen[1].1;
    let parsed = parse_group_spawn(data, true, 2, &refdata().spawn_tables());
    assert!(parsed.notes.is_empty(), "{:?}", parsed.notes);
    let read: Vec<_> = parsed
        .spawns
        .iter()
        .map(|e| (e.unique_id, e.kind.clone(), e.talk_options.clone()))
        .collect();
    let expected = vec![
        (smith, SpawnKind::Npc, vec![0x01, 0x02, 0x04, 0x20]),
        (warehouse, SpawnKind::Npc, vec![0x01, 0x03]),
    ];
    assert_eq!((read, parsed.end), (expected, data.len()));
}

/// One region east, the NPC two regions east comes into sight; back home it
/// goes out of it again, each time as a group of one.
#[test]
fn walking_brings_npcs_into_sight_and_takes_them_back() {
    let mut shard = world();
    let (s, hero) = enter(&mut shard);
    let world = &mut shard.0;
    frames(world, s);
    let far = uid_of(world, 2028);
    for (region, x, kind) in [(REGION + 1, 100, 1), (REGION, 969, 2)] {
        let walk = MovementRequest::ToLocation {
            region,
            x,
            y: 0,
            z: 1419,
        };
        send(world, s, Packet::MovementRequest(walk));
        while world.player(hero).unwrap().walk.is_some() {
            world.step();
        }
        let seen = frames(world, s);
        assert_eq!(seen[0].0, 0xB021);
        assert_eq!(group(&seen[1..]), (kind, vec![far]));
    }
}

/// With the user's data (OPENROAD_TEXTDATA_DIR): Hero enters at the bench's
/// start point and is shown the NPCs and monsters around Jangan in one group.
/// Run it with OPENROAD_WIRE_LOG to have the reading contract check the real
/// records.
#[test]
#[ignore = "needs the user's data: OPENROAD_TEXTDATA_DIR=<textdata> cargo test -p server --test npcs -- --ignored"]
fn the_users_npcs_around_the_start_point_come_in_one_group() {
    let textdata = std::env::var("OPENROAD_TEXTDATA_DIR").expect("OPENROAD_TEXTDATA_DIR");
    let refdata = RefData::load(std::path::Path::new(&textdata)).expect("the textdata loads");
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![hero()]);
    let store = Arc::new(store);
    let mut shard = (World::new(store.clone(), refdata), store);
    let (s, _) = enter(&mut shard);
    // The group holds the monsters south of the town too, read as the client
    // reads them with the user's tables.
    let seen = frames(&mut shard.0, s);
    let count = u16::from_le_bytes([seen[0].1[1], seen[0].1[2]]);
    let tables = RefData::load(std::path::Path::new(&textdata)).expect("the textdata loads");
    let parsed = parse_group_spawn(&seen[1].1, true, count, &tables.spawn_tables());
    assert_eq!(
        (parsed.end, parsed.spawns.len()),
        (seen[1].1.len(), count as usize)
    );
    let world = &shard.0;
    let npcs = parsed
        .spawns
        .iter()
        .filter(|e| world.npc(e.unique_id).is_some());
    let monsters = parsed
        .spawns
        .iter()
        .filter(|e| world.monster(e.unique_id).is_some());
    let (npcs, monsters) = (npcs.count(), monsters.count());
    assert!(npcs > 0 && monsters > 0 && npcs + monsters == count as usize);
    // The smith of Jangan, selected, talked to and left, with the user's
    // characterdata and siegefortress.txt deciding the fortress bytes.
    let smith = uid_of(&shard.0, 2003);
    send(
        &mut shard.0,
        s,
        Packet::SelectEntityRequest(SelectEntityRequest { unique_id: smith }),
    );
    let talk = TalkRequest {
        unique_id: smith,
        talk_flag: 1,
    };
    send(&mut shard.0, s, Packet::TalkRequest(talk));
    send(
        &mut shard.0,
        s,
        Packet::CloseTalkRequest(CloseTalkRequest { unique_id: smith }),
    );
    let codes: Vec<u16> = frames(&mut shard.0, s).iter().map(|f| f.0).collect();
    assert_eq!(codes, [0xB045, 0xB046, 0xB04B]);
}

/// The smith (a Jangan fortress NPC) and the warehouse, selected, talked to
/// and closed: the selection carries the options and, for the smith only,
/// the fortress's two bytes; a talk about an option the NPC does not offer,
/// or about an id the client was not shown, is refused.
#[test]
fn an_npc_is_selected_talked_to_and_closed() {
    let mut shard = world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    frames(world, s);
    let (smith, warehouse) = (uid_of(world, 2003), uid_of(world, 2013));
    let ask = |world: &mut World, packet| {
        send(world, s, packet);
        frames(world, s)
    };
    let select = |uid| Packet::SelectEntityRequest(SelectEntityRequest { unique_id: uid });
    let talk = |uid, talk_flag| {
        Packet::TalkRequest(TalkRequest {
            unique_id: uid,
            talk_flag,
        })
    };
    let id = |uid: u32| uid.to_le_bytes().to_vec();
    let selected = |uid, tail: &[u8]| (0xB045, Bytes::from([&[1][..], &id(uid), tail].concat()));
    assert_eq!(
        ask(world, select(smith)),
        [selected(smith, &[0, 4, 1, 2, 4, 0x20, 0, 0, 0])]
    );
    assert_eq!(
        ask(world, select(warehouse)),
        [selected(warehouse, &[0, 2, 1, 3, 0])]
    );
    let opened = (0xB046, Bytes::from_static(&[0x01, 0x01]));
    let refused = || (0xB046, Bytes::from_static(&[0x02, 0x00, 0x00]));
    assert_eq!(ask(world, talk(smith, 1)), [opened]);
    assert_eq!(ask(world, talk(smith, 3)), [refused()]);
    assert_eq!(ask(world, talk(uid_of(world, 2028), 1)), [refused()]);
    let close = Packet::CloseTalkRequest(CloseTalkRequest { unique_id: smith });
    assert_eq!(ask(world, close), [(0xB04B, Bytes::from_static(&[0x01]))]);
}

/// One row is one monster, at the HP of its characterdata row.
#[test]
fn monsters_stand_where_the_data_puts_them() {
    let (world, _) = monster_world();
    let placed: Vec<(u32, u16, f32, u32)> = world
        .monsters()
        .map(|(_, m)| (m.ref_id, m.position.region, m.position.x, m.hp))
        .collect();
    let expected = [
        (1933, 25000, 900.0, 54),
        (1954, 25000, 1000.0, 598720),
        (1933, 25002, 100.0, 54),
    ];
    assert_eq!(
        (placed.as_slice(), world.skipped_npc_rows()),
        (&expected[..], 0)
    );
}

/// The monsters around the entering character come in one group, read as the
/// client reads it, with tables of its own: standing, alive, the speeds and
/// the rarity of their rows, option 5.
#[test]
fn the_monsters_around_come_in_one_group_as_the_client_reads_it() {
    let mut shard = monster_world();
    let (s, _) = enter(&mut shard);
    let near: Vec<(u32, u32)> = (shard.0.monsters())
        .filter(|(_, m)| m.position.region == REGION)
        .map(|(uid, m)| (uid, m.ref_id))
        .collect();
    let seen = frames(&mut shard.0, s);
    let codes: Vec<u16> = seen.iter().map(|f| f.0).collect();
    assert_eq!(codes, [0x3017, 0x3019, 0x3018]);
    let characters = CharacterData::parse_shard(&monster_rows());
    let client = SpawnTables {
        characters: Some(&characters),
        items: None,
        gates: None,
    };
    let parsed = parse_group_spawn(&seen[1].1, true, 2, &client);
    assert_eq!(parsed.end, seen[1].1.len());
    let monster = |(unique_id, ref_id), (x, z), rarity, (walk_speed, run_speed)| SpawnedEntity {
        ref_id,
        unique_id,
        position: SpawnPosition {
            region: REGION,
            x,
            y: 0.0,
            z,
            heading: 0,
        },
        name: None,
        guild: None,
        movement: Some(EntityMovement {
            has_destination: false,
            move_type: 1,
            source: Some(0),
            angle: Some(0),
            ..Default::default()
        }),
        state: Some(EntityState {
            life_state: 1,
            unk: 0,
            motion_state: 0,
            body_state: 0,
            walk_speed,
            run_speed,
            hwan_speed: 100.0,
            buffs: Vec::new(),
        }),
        spawn_rarity: Some(rarity),
        talk_options: vec![5],
        interaction: Some(Interaction {
            flags: 2,
            ..Interaction::default()
        }),
        kind: SpawnKind::Monster { extra: None },
    };
    let expected = vec![
        monster(near[0], (900.0, 1400.0), 0, (8.0, 22.0)),
        monster(near[1], (1000.0, 1500.0), 3, (20.0, 90.0)),
    ];
    assert_eq!(parsed.spawns, expected);
}

/// Selecting a monster the client was shown answers its HP, also for one the
/// bench placed; a monster the client was not shown is not answered, and a
/// model that is no monster's is not placed.
#[test]
fn a_monster_is_selected_with_its_hp() {
    let mut shard = monster_world();
    let at = Position {
        region: REGION,
        x: 969.0,
        y: 0.0,
        z: 369.0,
        heading: 0,
    };
    let placed = shard.0.place_monster(1933, at).unwrap();
    assert_eq!(shard.0.place_monster(1907, at), None);
    let (s, _) = enter(&mut shard);
    frames(&mut shard.0, s);
    let at_x = |x: f32| {
        shard
            .0
            .monsters()
            .find(|(_, m)| m.position.x == x)
            .unwrap()
            .0
    };
    let (mangnyang, far) = (at_x(900.0), at_x(100.0));
    let mut select = |uid: u32| {
        let select = SelectEntityRequest { unique_id: uid };
        send(&mut shard.0, s, Packet::SelectEntityRequest(select));
        frames(&mut shard.0, s)
    };
    let selected = |uid: u32| {
        let body = [
            &[1][..],
            &uid.to_le_bytes(),
            &[1],
            &54u32.to_le_bytes(),
            &[1, 5],
        ]
        .concat();
        vec![(0xB045, Bytes::from(body))]
    };
    assert_eq!(select(mangnyang), selected(mangnyang));
    assert_eq!(select(placed), selected(placed));
    assert_eq!(select(far), []);
}

/// The monster's walk speed: a Mangnyang (8 units per second) sent 88 units
/// east walks 11 s, 330 steps; the client that sees it is told where it
/// goes, as for a character.
#[test]
fn a_monster_walks_at_its_walk_speed_and_is_seen_walking() {
    let mut shard = monster_world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    frames(world, s);
    let mangnyang = world
        .monsters()
        .find(|(_, m)| m.position.x == 900.0)
        .unwrap()
        .0;
    let (sx, sz) = sector(REGION);
    let to = Point {
        x: f32::from(sx) * REGION_SIZE + 988.0,
        y: 0.0,
        z: f32::from(sz) * REGION_SIZE + 1400.0,
    };
    assert!(world.walk_monster(mangnyang, to));
    let told = MovementResponse {
        unique_id: mangnyang,
        has_destination: true,
        region: REGION,
        x: 988,
        y: 0,
        z: 1400,
        angle: 0,
    };
    let (opcode, body) = Packet::MovementResponse(told).into_serialize();
    assert_eq!(frames(world, s), [(opcode, body)]);
    for _ in 0..329 {
        world.step();
    }
    assert!(world.monster(mangnyang).unwrap().walk.is_some());
    world.step();
    let m = world.monster(mangnyang).unwrap();
    assert_eq!(
        (m.walk, m.position.region, m.position.x),
        (None, REGION, 988.0)
    );
}

/// A monster walking into a region the hero sees comes alone, walking, in a
/// single spawn whose last byte is 4, and is taken back alone (0x3016) when it
/// walks out again. Before it comes, the hero is not told where it goes.
#[test]
fn a_monster_walking_into_sight_comes_alone_and_walking() {
    let mut shard = monster_world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    frames(world, s);
    let far = world
        .monsters()
        .find(|(_, m)| m.position.x == 100.0)
        .unwrap()
        .0;
    let point = |region: u16, x: f32| {
        let (sx, sz) = sector(region);
        let (x, z) = (
            f32::from(sx) * REGION_SIZE + x,
            f32::from(sz) * REGION_SIZE + 100.0,
        );
        Point { x, y: 0.0, z }
    };
    // What the hero was sent about this monster; the others near it wander.
    let walk = |world: &mut World, to| {
        assert!(world.walk_monster(far, to));
        while world.monster(far).unwrap().walk.is_some() {
            world.step();
        }
        let about = |f: &(u16, Bytes)| f.0 != 0xB021 || f.1[..4] == far.to_le_bytes();
        frames(world, s)
            .into_iter()
            .filter(about)
            .collect::<Vec<_>>()
    };
    let seen = walk(world, point(REGION + 1, 1800.0));
    let codes: Vec<u16> = seen.iter().map(|f| f.0).collect();
    assert_eq!(codes, [0x3015]);
    let characters = CharacterData::parse_shard(&monster_rows());
    let client = SpawnTables {
        characters: Some(&characters),
        items: None,
        gates: None,
    };
    let (entity, tail) = parse_single_spawn(&seen[0].1, &client).unwrap();
    let walking = EntityMovement {
        has_destination: true,
        move_type: 0,
        dest_region: Some(REGION + 1),
        dest_x: Some(1800),
        dest_y: Some(0),
        dest_z: Some(100),
        ..Default::default()
    };
    let read = (entity.unique_id, entity.movement, tail);
    assert_eq!(read, (far, Some(walking), SingleTail::Figure(4)));
    // Walking out, the hero is told where it goes, then sees it taken back.
    let seen = walk(world, point(REGION + 2, 100.0));
    let codes: Vec<u16> = seen.iter().map(|f| f.0).collect();
    assert_eq!(codes, [0xB021, 0x3016]);
    assert_eq!(seen[1].1[..], far.to_le_bytes());
}

/// Monsters near the hero wander: in the first minute each takes a few
/// steps, each at most 111 units long and ending at most 250 units from its
/// row, at least 9.2 s apart; the Tiger Woman, walking 20 units a second,
/// rests most of that time.
#[test]
fn monsters_near_a_character_wander_around_their_rows() {
    let mut shard = monster_world();
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    frames(world, s);
    let in_world = |region: u16, x: f32, z: f32| {
        let (sx, sz) = sector(region);
        (
            f32::from(sx) * REGION_SIZE + x,
            f32::from(sz) * REGION_SIZE + z,
        )
    };
    let placed = |world: &World, x: f32, z: f32| {
        let uid = world.monsters().find(|(_, m)| m.position.x == x).unwrap().0;
        (uid, in_world(REGION, x, z))
    };
    let near = [placed(world, 900.0, 1400.0), placed(world, 1000.0, 1500.0)];
    let mut moves: Vec<(u32, u64, (f32, f32))> = Vec::new();
    for step in 0..60 * 30 {
        world.step();
        for (opcode, body) in frames(world, s) {
            let Ok(Packet::MovementResponse(m)) = Packet::deserialize(opcode, body) else {
                continue;
            };
            moves.push((
                m.unique_id,
                step,
                in_world(m.region, m.x as f32, m.z as f32),
            ));
        }
    }
    let dist = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).hypot(a.1 - b.1);
    for (uid, home) in near {
        let mine: Vec<(u64, (f32, f32))> = (moves.iter())
            .filter(|m| m.0 == uid)
            .map(|m| (m.1, m.2))
            .collect();
        assert!(mine.len() >= 2, "{uid}: {mine:?}");
        let mut from = (0, home);
        for (step, to) in mine {
            // The destinations are whole units on the wire.
            assert!(dist(to, home) <= 250.0 + 1.0, "{to:?}");
            assert!(dist(from.1, to) <= 111.0 + 1.0, "{from:?} {to:?}");
            assert!(from.0 == 0 || step - from.0 >= 276, "{from:?} {step}");
            from = (step, to);
        }
    }
}

/// Monsters think a region beyond what the hero sees. One whose row lies at
/// the edge of sight and that walked out comes back within ten minutes,
/// alone; one three regions away does not move.
#[test]
fn a_monster_that_walked_out_of_sight_walks_back_in() {
    const EDGE: &str = "1933\t25001\t1880.0\t0.0\t100.0\r\n\
                        1933\t25003\t100.0\t0.0\t100.0\r\n";
    let mut shard = monsters_at(EDGE);
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let placed = |world: &World, region| {
        let mut monsters = world.monsters();
        monsters
            .find(|(_, m)| m.position.region == region)
            .unwrap()
            .0
    };
    let (edge, beyond) = (placed(world, REGION + 1), placed(world, REGION + 3));
    let (sx, sz) = sector(REGION + 2);
    let out = Point {
        x: f32::from(sx) * REGION_SIZE + 60.0,
        y: 0.0,
        z: f32::from(sz) * REGION_SIZE + 100.0,
    };
    assert!(world.walk_monster(edge, out));
    while world.monster(edge).unwrap().walk.is_some() {
        world.step();
    }
    frames(world, s);
    // A single spawn whose record, after its ref, names the monster.
    let back = (0..10 * 60 * 30).any(|_| {
        world.step();
        let seen: Vec<_> = (frames(world, s).into_iter())
            .filter(|f| f.0 != 0xB021)
            .collect();
        let codes: Vec<u16> = seen.iter().map(|f| f.0).collect();
        codes == [0x3015] && seen[0].1[4..8] == edge.to_le_bytes()
    });
    assert!(back);
    let unmoved = world.monster(beyond).unwrap();
    assert_eq!((unmoved.walk, unmoved.position.x), (None, 100.0));
}

/// The world's chance repeats from its seed: in a minute, two worlds of the
/// same seed send the hero the same monster moves, and a world of another
/// seed other ones.
#[test]
fn a_seeded_world_repeats_its_monsters_moves() {
    let moves = |seed: u64| {
        let mut shard = seeded_with(monster_data(MONSTERPOS), seed);
        let (s, _) = enter(&mut shard);
        let world = &mut shard.0;
        frames(world, s);
        let mut moves = Vec::new();
        for _ in 0..60 * 30 {
            world.step();
            moves.extend(frames(world, s).into_iter().filter(|f| f.0 == 0xB021));
        }
        moves
    };
    let once = moves(19);
    assert!(!once.is_empty());
    assert_eq!(moves(19), once);
    assert_ne!(moves(20), once);
}

fn armed_at(npcpos: &str) -> (World, Arc<MemoryStore>) {
    armed(npcpos, hero())
}

/// `hero` with the bench's sword, and the monsters of `npcpos`; the sword
/// and the basic attacks of the fist and the sword as the user's data holds
/// them (itemdata_5000.txt:1683, skilldata_5000.txt:1 and :2).
fn armed(npcpos: &str, hero: Character) -> (World, Arc<MemoryStore>) {
    let mut refdata = monster_data(npcpos);
    let row = |width: usize, values: &[(usize, &str)]| {
        let mut row = vec!["0"; width];
        for (at, value) in values {
            row[*at] = value;
        }
        row.join("\t")
    };
    let sword = [
        (1, "3632"),
        (9, "3"),
        (10, "1"),
        (11, "6"),
        (12, "2"),
        (94, "6"),
    ];
    let white = [(95, "15"), (96, "16"), (97, "16"), (98, "18")];
    refdata.add_items(&row(160, &[&sword[..], &white[..]].concat()));
    let basic = |id, code, duration, weapons: [&'static str; 2], params: &[&'static str]| {
        let mut values = vec![(1, id), (3, code), (7, "1"), (8, "2"), (13, duration)];
        values.extend([(50, weapons[0]), (51, weapons[1]), (57, "255")]);
        values.extend(params.iter().enumerate().map(|(at, p)| (69 + at, *p)));
        row(119, &values)
    };
    let att = |share| ["6386804", "5", share, "0", "0", share];
    refdata.add_skills(&basic(
        "1",
        "SKILL_PUNCH_01",
        "1500",
        ["1", "255"],
        &att("150"),
    ));
    let sword_base = [&att("60")[..], &["28003", "2", "2"]].concat();
    refdata.add_skills(&basic(
        "2",
        "SKILL_CH_SWORD_BASE_01",
        "1200",
        ["2", "3"],
        &sword_base,
    ));
    let mut store = MemoryStore::default();
    let armed = Character {
        equipment: vec![(6, 3632)],
        ..hero
    };
    store.add_account("a", "a", vec![armed]);
    let store = Arc::new(store);
    (World::seeded(store.clone(), refdata, 19), store)
}

fn attack(target: u32) -> Packet {
    let target = ActionTarget::Entity { unique_id: target };
    Packet::ObjectActionRequest(ObjectActionRequest::Execute(ActionCommand::Attack(target)))
}

/// What the hero's client is sent, decoded, with the step and the opcode:
/// from an attack on the Mangnyang next to it (10 units east), having
/// selected it or not, until the attack ends; and the hero's and the
/// Mangnyang's uids.
fn fight(select: bool) -> (Vec<(u64, u16, Packet)>, u32, u32) {
    let mut shard = armed_at("1933\t25000\t979.0\t0.0\t1419.0\r\n");
    let (s, hero) = enter(&mut shard);
    let world = &mut shard.0;
    let mob = world.monsters().next().unwrap().0;
    if select {
        send(
            world,
            s,
            Packet::SelectEntityRequest(SelectEntityRequest { unique_id: mob }),
        );
    }
    frames(world, s);
    send(world, s, attack(mob));
    let mut seen = Vec::new();
    for step in 0..60 * 30 {
        for (op, body) in frames(world, s) {
            seen.push((step, op, Packet::deserialize(op, body).unwrap()));
        }
        let end = ObjectActionResponse::Ended { queue_len: 0 };
        if matches!(seen.last(), Some((_, _, Packet::ObjectActionResponse(r))) if *r == end) {
            break;
        }
        world.step();
    }
    (seen, hero, mob)
}

/// The hero selects the Mangnyang next to it and attacks it with the sword.
/// It is answered `01 01`; a swing every 39 steps (1200 ms and the pause)
/// lands two hits, each in the span the sword's rows give, and the client
/// that selected the Mangnyang is told the HP left after each. The hit that
/// kills carries 0x80 and one after it lands on nothing (arm 8); the death
/// follows (0x30BF `00 02`) with the Mangnyang's EXP (0x3056), and the end of
/// that swing ends the attack.
#[test]
fn a_monster_in_reach_is_hit_until_it_dies() {
    let (seen, hero, mob) = fight(true);
    let started = ObjectActionResponse::Started { queue_len: 1 };
    assert!(matches!(&seen[0], (0, _, Packet::ObjectActionResponse(r)) if *r == started));
    let share = |attack: f32| (attack * 60.0 / 100.0).round() as u32;
    let span = share(15.0)..=share(16.0);
    let (mut hp, mut swings) = (54u32, Vec::new());
    let (mut died, mut gained) = (None, 0);
    for (step, opcode, packet) in &seen[1..] {
        match packet {
            Packet::ObjectActionUpdate(ObjectActionUpdate::Success {
                skill_id,
                source,
                instance,
                target,
                kind: ActionKind::Attack { damage: Some(d) },
                ..
            }) => {
                assert_eq!((*skill_id, *source, *target), (2, hero, mob));
                assert_eq!((d.instance_count, d.entities.len()), (2, 1));
                for hit in &d.entities[0].hits {
                    match hit.effect {
                        HitEffect::Damage { value, .. } => {
                            assert!(hp > 0 && span.contains(&value.amount), "{value:?}");
                            hp = hp.saturating_sub(value.amount);
                            assert_eq!(hit.killing_blow, hp == 0);
                        }
                        HitEffect::NoPayload { arm: 8 } => assert_eq!(hp, 0),
                        other => panic!("{other:?}"),
                    }
                }
                swings.push((*step, *instance));
            }
            Packet::EntityBarsUpdate(EntityBarsUpdate {
                unique_id,
                hp: told,
                ..
            }) => {
                assert_eq!((*unique_id, *told), (mob, Some(hp)));
            }
            Packet::EntityStateUpdate(dead) => {
                assert_eq!((dead.unique_id, dead.is_death(), hp), (mob, true, 0));
                died = Some(*step);
            }
            Packet::ReceiveExperience(exp) => {
                let read = (exp.exp_origin, exp.experience, exp.sp_exp, exp.unknown);
                assert_eq!((read, exp.tail.is_empty()), ((mob, 24, 24, 0), true));
                assert_eq!(Some(*step), died);
                gained += 1;
            }
            Packet::ObjectActionResponse(ObjectActionResponse::Ended { queue_len: 0 }) => {
                assert_eq!(*step, swings.last().unwrap().0 + 39);
            }
            _ => panic!("unexpected {opcode:#06x}"),
        }
    }
    assert_eq!((hp, gained), (0, 1));
    let numbered: Vec<u32> = swings.iter().map(|s| s.1).collect();
    assert_eq!(numbered, (1..=swings.len() as u32).collect::<Vec<_>>());
    assert!(
        swings.windows(2).all(|w| w[1].0 - w[0].0 == 39),
        "{swings:?}"
    );
}

/// A client that has not selected the monster is not told its HP.
#[test]
fn only_a_selecting_client_is_told_the_hp() {
    let (seen, _, _) = fight(false);
    let opcodes: Vec<u16> = seen.iter().map(|(_, op, _)| *op).collect();
    assert!(
        !opcodes.contains(&0x3057) && opcodes.contains(&0x30BF),
        "{opcodes:x?}"
    );
}

/// The Tiger Woman stands while the hero attacks it (decision 0020, E4). A
/// cancel during a swing ends the attack with two `02 00`, and no swing
/// follows.
#[test]
fn a_cancel_during_a_swing_ends_the_attack() {
    let mut shard = armed_at("1954\t25000\t979.0\t0.0\t1419.0\r\n");
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let tiger = world.monsters().next().unwrap().0;
    frames(world, s);
    send(world, s, attack(tiger));
    for _ in 0..30 * 30 {
        world.step();
    }
    let opcodes: Vec<u16> = frames(world, s).iter().map(|f| f.0).collect();
    assert!(
        opcodes.contains(&0xB070) && !opcodes.contains(&0xB021),
        "{opcodes:x?}"
    );
    send(
        world,
        s,
        Packet::ObjectActionRequest(ObjectActionRequest::Cancel),
    );
    let ended = (0xB074, Bytes::from_static(&[2, 0]));
    assert_eq!(frames(world, s), [ended.clone(), ended]);
    for _ in 0..3 * 30 {
        world.step();
    }
    let opcodes: Vec<u16> = frames(world, s).iter().map(|f| f.0).collect();
    assert!(!opcodes.contains(&0xB070), "{opcodes:x?}");
}

/// A Mangnyang 200 units south: the hero is answered `01 01`, told where it
/// walks, a unit inside reach of it (the sword's 6 and 10 for the bodies),
/// and swings when it gets there, at its run speed (50 units a second).
#[test]
fn an_attacker_walks_into_reach_first() {
    let mut shard = armed_at("1933\t25000\t969.0\t0.0\t1219.0\r\n");
    let (s, hero) = enter(&mut shard);
    let world = &mut shard.0;
    let mob = world.monsters().next().unwrap().0;
    frames(world, s);
    send(world, s, attack(mob));
    assert_eq!(frames(world, s), [(0xB074, Bytes::from_static(&[1, 1]))]);
    world.step();
    let walks = MovementResponse {
        unique_id: hero,
        has_destination: true,
        region: REGION,
        x: 969,
        y: 0,
        z: 1219 + 15,
        angle: 0,
    };
    let (opcode, body) = Packet::MovementResponse(walks).into_serialize();
    assert_eq!(frames(world, s), [(opcode, body)]);
    let walked = (185.0f32 / 50.0 * 30.0).ceil() as usize;
    for _ in 0..walked {
        world.step();
    }
    let opcodes: Vec<u16> = frames(world, s).iter().map(|f| f.0).collect();
    assert_eq!(opcodes, [0xB070]);
}

/// A request for a second Mangnyang during a swing at the first waits behind
/// it (`01 02`); when the swing ends (39 steps), it starts (`02 01`) and the
/// next swing hits the second.
#[test]
fn a_request_during_a_swing_waits_behind_it() {
    let rows = "1933\t25000\t979.0\t0.0\t1419.0\r\n1933\t25000\t969.0\t0.0\t1429.0\r\n";
    let mut shard = armed_at(rows);
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let at = |world: &World, x: f32| world.monsters().find(|(_, m)| m.position.x == x).unwrap().0;
    let (first, second) = (at(world, 979.0), at(world, 969.0));
    frames(world, s);
    send(world, s, attack(first));
    world.step();
    frames(world, s);
    send(world, s, attack(second));
    assert_eq!(frames(world, s), [(0xB074, Bytes::from_static(&[1, 2]))]);
    for _ in 0..38 {
        world.step();
    }
    assert_eq!(frames(world, s), []);
    world.step();
    let sent = frames(world, s);
    assert_eq!(sent[0], (0xB074, Bytes::from_static(&[2, 1])));
    let Ok(Packet::ObjectActionUpdate(ObjectActionUpdate::Success { target, .. })) =
        Packet::deserialize(sent[1].0, sent[1].1.clone())
    else {
        panic!("a swing follows");
    };
    assert_eq!(target, second);
}

/// The killed Mangnyang's body lies 150 steps (5 s); then the hero is told it
/// is gone (0x3016), and it leaves the world. 420 to 510 steps (14 to 17 s)
/// after its death, the monster of its row comes back under a new uid,
/// standing at the row with full HP, as a single spawn whose last byte is 1.
#[test]
fn a_killed_monster_is_taken_away_and_its_row_comes_back() {
    let mut shard = armed_at("1933\t25000\t979.0\t0.0\t1419.0\r\n");
    let (s, _) = enter(&mut shard);
    let world = &mut shard.0;
    let mob = world.monsters().next().unwrap().0;
    frames(world, s);
    send(world, s, attack(mob));
    let died = (0..60 * 30u64).find(|_| {
        world.step();
        frames(world, s).iter().any(|f| f.0 == 0x30BF)
    });
    let died = died.unwrap();
    let (mut gone, mut back) = (None, None);
    for step in died + 1..died + 600 {
        world.step();
        for (opcode, body) in frames(world, s) {
            match opcode {
                0x3016 => gone = Some((step, body)),
                0x3015 => back = Some((step, body)),
                _ => {}
            }
        }
    }
    let gone = gone.unwrap();
    assert_eq!((gone.0, &gone.1[..]), (died + 150, &mob.to_le_bytes()[..]));
    assert!(world.monster(mob).is_none());
    let (at, spawn) = back.unwrap();
    assert!((died + 420..=died + 510).contains(&at), "{at}");
    let (new, m) = world.monsters().next().unwrap();
    assert_ne!(new, mob);
    let placed = (m.ref_id, m.position.x, m.position.z, m.hp, m.walk);
    assert_eq!(placed, (1933, 979.0, 1419.0, 54, None));
    let tail = spawn[spawn.len() - 1];
    assert_eq!((&spawn[4..8], tail), (&new.to_le_bytes()[..], 1));
}

/// A kill gives the hero the Mangnyang's 24 EXP and 24 SP-EXP, the monster
/// named as their origin. From 390 SP-EXP that makes a skill point, which its
/// client is told (0x304E 2), with 14 SP-EXP left over.
#[test]
fn a_kill_gives_the_monsters_exp_and_400_sp_exp_make_a_skill_point() {
    let hero = Character {
        exp: 100,
        skill_exp: 390,
        skill_points: 7,
        ..hero()
    };
    let mut shard = armed("1933\t25000\t979.0\t0.0\t1419.0\r\n", hero);
    let (s, uid) = enter(&mut shard);
    let world = &mut shard.0;
    let mob = world.monsters().next().unwrap().0;
    frames(world, s);
    send(world, s, attack(mob));
    let (mut exp, mut points) = (Vec::new(), Vec::new());
    for _ in 0..60 * 30 {
        world.step();
        for (op, body) in frames(world, s) {
            match Packet::deserialize(op, body).unwrap() {
                Packet::ReceiveExperience(p) => exp.push((p.exp_origin, p.experience, p.sp_exp)),
                Packet::CharacterPointsUpdate(p) => points.push(p),
                _ => {}
            }
        }
    }
    let sp = CharacterPointsUpdate::Sp {
        amount: 8,
        display: 0,
    };
    assert_eq!((exp, points), (vec![(mob, 24, 24)], vec![sp]));
    let c = &world.player(uid).unwrap().character;
    assert_eq!((c.exp, c.skill_exp, c.skill_points), (124, 14, 8));
}

//! NPCs and monsters in-process: placed from the reference data, seen in
//! groups by the characters around them.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::character_data::{EntityMovement, EntityState, SpawnPosition};
use packets::agent::entity_spawn::{parse_group_spawn, Interaction, SpawnKind, SpawnedEntity};
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
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![hero()]);
    let store = Arc::new(store);
    (World::seeded(store.clone(), refdata, 19), store)
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
/// 54 HP), and MOB_CH_TIGERWOMAN, a unique (rarity 3, speeds 20 and 90,
/// 598720 HP).
fn monster_rows() -> String {
    let row = |id: &str, rarity: &str, (walk, run): (&str, &str), hp: &str| {
        let mut row = vec![String::from("0"); 60];
        (row[1], row[9], row[10], row[11], row[12]) =
            (id.into(), "1".into(), "2".into(), "1".into(), "1".into());
        (row[15], row[46], row[47], row[59]) = (rarity.into(), walk.into(), run.into(), hp.into());
        row.join("\t")
    };
    let rows = [
        row("1933", "0", ("8", "22"), "54"),
        row("1954", "3", ("20", "90"), "598720"),
    ];
    rows.join("\n")
}

/// The hero's model and the monsters of [`MONSTERPOS`].
fn monster_world() -> (World, Arc<MemoryStore>) {
    let mut refdata = hero_model();
    refdata.add_characters(&monster_rows());
    refdata.add_npcs(MONSTERPOS);
    world_with(refdata)
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

/// A monster walking into a region the hero sees comes as a group of one,
/// walking, and is taken back when it walks out again. Before it comes, the
/// hero is not told where it goes.
#[test]
fn a_monster_walking_into_sight_comes_as_a_group_of_one() {
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
    assert_eq!(codes, [0x3017, 0x3019, 0x3018]);
    let characters = CharacterData::parse_shard(&monster_rows());
    let client = SpawnTables {
        characters: Some(&characters),
        items: None,
        gates: None,
    };
    let parsed = parse_group_spawn(&seen[1].1, true, 1, &client);
    let walking = EntityMovement {
        has_destination: true,
        move_type: 0,
        dest_region: Some(REGION + 1),
        dest_x: Some(1800),
        dest_y: Some(0),
        dest_z: Some(100),
        ..Default::default()
    };
    let read = (parsed.spawns[0].unique_id, parsed.spawns[0].movement);
    assert_eq!(read, (far, Some(walking)));
    // Walking out, the hero is told where it goes, then sees it taken back.
    let seen = walk(world, point(REGION + 2, 100.0));
    let codes: Vec<u16> = seen.iter().map(|f| f.0).collect();
    assert_eq!(codes, [0xB021, 0x3017, 0x3019, 0x3018]);
    let gone = (
        seen[2].1.clone(),
        Bytes::copy_from_slice(&far.to_le_bytes()),
    );
    assert_eq!(
        (seen[1].1.clone(), gone.0),
        (Bytes::from_static(&[2, 1, 0]), gone.1)
    );
}

/// Monsters near the hero wander: in the first minute each takes a few
/// steps, each at most 111 units long and ending at most 250 units from its
/// row, at least 9.2 s apart; the Tiger Woman, walking 20 units a second,
/// rests most of that time. The monster two regions away, which nobody sees,
/// does not move.
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
    let far = world
        .monsters()
        .find(|(_, m)| m.position.x == 100.0)
        .unwrap()
        .0;
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
    let unmoved = world.monster(far).unwrap();
    assert_eq!((unmoved.walk, unmoved.position.x), (None, 100.0));
}

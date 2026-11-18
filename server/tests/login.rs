//! Login, lobby and world entry in-process: the gateway's ticket, the
//! agent's answers.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::character_data::{
    parse_character_info, EquipmentData, InventoryItem, ItemClass, ItemClassResolver, ItemTypeData,
    RentInfo,
};
use packets::agent::prelude::*;
use packets::global::{KeepAlive, ModuleIdentification};
use packets::Packet;
use server::refdata::RefData;
use server::store::{login, AccountId, Character, MemoryStore, Position};
use server::world::{Event, Phase, Reason, SessionId, World};

mod common;
use common::item_row;

fn hero() -> Character {
    Character {
        name: "Hero".to_owned(),
        ref_id: 1907,
        scale: 34,
        level: 10,
        exp: 120,
        skill_exp: 0,
        gold: 1000,
        skill_points: 0,
        strength: 29,
        intellect: 29,
        stat_points: 3,
        hp: 489,
        mp: 489,
        inventory_size: 45,
        // The seven Chinese masteries, as every captured record lists them.
        masteries: [257, 258, 259, 273, 274, 275, 276]
            .map(|id| (id, 0))
            .to_vec(),
        position: Position {
            region: 25000,
            x: 969.0,
            y: 0.0,
            z: 1419.0,
            heading: 0,
        },
        equipment: Vec::new(),
        avatar: Vec::new(),
    }
}

/// A characterdata line of a player's model (type ids 1/1) with the model's
/// id and speeds, at the columns the game's table has them
/// (`docs/formats/textdata-characterdata.md` §3).
fn character_row(ref_id: u32, walk: f32, run: f32) -> String {
    let mut cols = vec![String::from("0"); 48];
    (cols[1], cols[9], cols[10]) = (ref_id.to_string(), "1".into(), "1".into());
    (cols[46], cols[47]) = (walk.to_string(), run.to_string());
    cols.join("\t")
}

/// A world whose store holds `admin` with one character, and a ticket the
/// gateway would have issued after checking the password.
fn admitted() -> (World, u32) {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![hero()]);
    let account = login(&store, "admin", "secret").expect("the seeded password logs in");
    let mut refdata = RefData::default();
    refdata.add_characters(&character_row(hero().ref_id, 16.0, 50.0));
    let mut world = World::new(Arc::new(store), refdata);
    let ticket = world
        .admit(account, "admin", vec![hero()])
        .expect("no agent session yet");
    (world, ticket)
}

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

fn log_in(world: &mut World, session: SessionId, token: u32, username: &str) {
    let request = AgentLoginRequest {
        token,
        username: username.to_owned(),
        password: "secret".to_owned(),
        content_id: 0,
        mac_address: [0; 6],
    };
    send(world, session, Packet::AgentLoginRequest(request));
}

fn frames(world: &mut World, session: SessionId) -> Vec<(u16, Vec<u8>)> {
    let frames = world.take_frames(session);
    frames
        .into_iter()
        .map(|f| (f.opcode, f.body.to_vec()))
        .collect()
}

#[test]
fn a_client_with_a_ticket_logs_in_and_lists_its_characters() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    send(
        &mut world,
        s,
        Packet::ModuleIdentification(ModuleIdentification::client()),
    );
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));

    let mut out = frames(&mut world, s).into_iter();
    assert_eq!(
        out.next(),
        Some((0x2001, b"\x0b\x00AgentServer\x00".to_vec()))
    );
    assert_eq!(out.next(), Some((0xA103, vec![0x01])));
    let (opcode, body) = out.next().expect("the character list");
    assert_eq!(opcode, 0xB007);
    let Ok(Packet::CharacterSelectionActionResponse(list)) =
        Packet::deserialize(opcode, body.into())
    else {
        panic!("0xB007 does not decode");
    };
    let listed = list.characters.expect("a list on success").characters;
    let listed: Vec<_> = listed.iter().map(|c| (c.name.as_str(), c.level)).collect();
    assert_eq!(listed, [("Hero", 10)]);
    assert_eq!(out.next(), None);
    assert!(world.take_events().is_empty());
}

#[test]
fn a_ticket_is_good_for_one_login() {
    let (mut world, ticket) = admitted();
    let first = world.connect();
    log_in(&mut world, first, ticket, "admin");
    let second = world.connect();
    log_in(&mut world, second, ticket, "admin");
    assert_eq!(frames(&mut world, second), [(0xA103, vec![0x02, 0x01])]);
    assert_eq!(world.take_events(), [Event::Closed(second)]);
    assert_eq!(world.phase(second), Some(Phase::Closed));
}

#[test]
fn a_ticket_is_bound_to_the_name_it_was_issued_for() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    log_in(&mut world, s, ticket, "someone");
    assert_eq!(frames(&mut world, s), [(0xA103, vec![0x02, 0x01])]);
    assert_eq!(world.phase(s), Some(Phase::Closed));
}

#[test]
fn the_lobby_answers_only_after_login() {
    let (mut world, _) = admitted();
    let s = world.connect();
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    assert!(frames(&mut world, s).is_empty());
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x7007,
        len: 1,
        reason: Reason::NotHandled,
    };
    assert_eq!(world.take_events(), [rejected]);
}

fn join(world: &mut World, session: SessionId, name: &str) {
    let request = CharacterJoinRequest {
        character_name: name.to_owned(),
    };
    send(world, session, Packet::CharacterJoinRequest(request));
}

/// The server holds no items, so the reader never asks for a class.
struct NoItems;

impl ItemClassResolver for NoItems {
    fn item_class(&self, _: u32) -> ItemClass {
        ItemClass::Unknown
    }
}

/// The record parses as the client reads it, carries the stored level the
/// lobby shows too, and nothing follows it until the client's 0x3012: a chat
/// frame in that window crashes the original client. Then the character's
/// values follow; without them the original shows HP and MP as 0/0.
#[test]
fn a_character_enters_the_world_and_nothing_follows_until_game_ready() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    world.take_frames(s);
    join(&mut world, s, "Hero");

    let out = frames(&mut world, s);
    let opcodes: Vec<u16> = out.iter().map(|(opcode, _)| *opcode).collect();
    assert_eq!(opcodes, [0xB001, 0x34A5, 0x3013, 0x34A6, 0x3020]);
    assert_eq!(out[0].1, [0x01]);
    let Some(Phase::Joining { uid, .. }) = world.phase(s) else {
        panic!("the session is joining");
    };
    assert_eq!(out[4].1[..4], uid.to_le_bytes());
    let info = parse_character_info(&out[2].1, Some(uid), &NoItems);
    assert!(info.fully_parsed, "failed at {:?}", info.failed_stage);
    assert_eq!(info.name.as_deref(), Some("Hero"));
    assert_eq!(info.stats.map(|stats| stats.level), Some(hero().level));

    world.receive(s, 0x3012, Bytes::new());
    // Then only the character's values: no items and formulas yet, the
    // stored HP and MP as the maximum.
    let values = CharacterStatsUpdate {
        phys_attack_min: 0,
        phys_attack_max: 0,
        mag_attack_min: 0,
        mag_attack_max: 0,
        phys_defense: 0,
        mag_defense: 0,
        hit_rate: 0,
        parry_rate: 0,
        max_hp: 489,
        max_mp: 489,
        strength: 29,
        intelligence: 29,
    };
    let (opcode, body) = Packet::CharacterStatsUpdate(values).into_serialize();
    assert_eq!(frames(&mut world, s), [(opcode, body.to_vec())]);
    assert!(matches!(world.phase(s), Some(Phase::Game { uid: u, .. }) if u == uid));
    assert_eq!(world.take_events(), [Event::Joined { session: s, uid }]);
    let players: Vec<_> = world
        .players()
        .map(|(u, p)| (u, p.character.name.as_str()))
        .collect();
    assert_eq!(players, [(uid, "Hero")]);
    world.disconnect(s);
    assert_eq!(world.players().count(), 0);
}

/// The hero's model, a sword (durability 62 to 76), a heavy armour's body
/// (48 to 59) and an avatar flag, as the user's itemdata has them.
fn refdata_with_items() -> RefData {
    let mut refdata = RefData::default();
    refdata.add_characters(&character_row(hero().ref_id, 16.0, 50.0));
    for (id, code, tid, durability) in [
        (3632, "ITEM_CH_SWORD_01_A_DEF", [3, 1, 6, 2], (62, 76)),
        (3637, "ITEM_CH_M_HEAVY_01_BA_A_DEF", [3, 1, 3, 3], (48, 59)),
        (
            5893,
            "ITEM_ETC_E051021_BLUEDRAGON_BUNTING_M",
            [3, 1, 13, 4],
            (0, 0),
        ),
    ] {
        refdata.add_items(&item_row(id, code, tid, durability));
    }
    refdata
}

/// A character wearing a sword, an armour's body and an avatar flag shows
/// them in the lobby list, and its record (0x3013) carries them in slot order
/// as new items: no rent, plus 0, variance 0, the durability its itemdata
/// starts it with, no options.
#[test]
fn a_character_wears_its_items_in_the_lobby_and_in_its_record() {
    let mut worn = hero();
    (worn.equipment, worn.avatar) = (vec![(6, 3632), (1, 3637)], vec![(0, 5893)]);
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![worn.clone()]);
    let account = login(&store, "admin", "secret").expect("the seeded password logs in");
    let mut world = World::new(Arc::new(store), refdata_with_items());
    let ticket = world
        .admit(account, "admin", vec![worn])
        .expect("no agent session yet");
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    world.take_frames(s);
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let (opcode, body) = frames(&mut world, s).remove(0);
    let Ok(Packet::CharacterSelectionActionResponse(list)) =
        Packet::deserialize(opcode, Bytes::from(body))
    else {
        panic!("a lobby list");
    };
    let lobby = &list.characters.expect("the list").characters[0];
    let shown =
        |items: &[LobbyItem]| -> Vec<(u32, u8)> { items.iter().map(|i| (i.id, i.plus)).collect() };
    let listed = (shown(&lobby.char_items), shown(&lobby.avatar_items));
    assert_eq!(listed, (vec![(3637, 0), (3632, 0)], vec![(5893, 0)]));

    join(&mut world, s, "Hero");
    let out = frames(&mut world, s);
    let Some(Phase::Joining { uid, .. }) = world.phase(s) else {
        panic!("the session is joining");
    };
    let items = refdata_with_items();
    let info = parse_character_info(&out[2].1, Some(uid), items.items());
    assert!(info.fully_parsed, "failed at {:?}", info.failed_stage);
    let new = |slot, ref_id, durability| InventoryItem {
        slot,
        rent: RentInfo::default(),
        ref_id,
        data: ItemTypeData::Equipment(EquipmentData {
            opt_level: 0,
            variance: 0,
            durability,
            mag_params: Vec::new(),
            socket_tag: 1,
            sockets: Vec::new(),
            elixir_tag: 2,
            adv_elixirs: Vec::new(),
        }),
    };
    let worn = (info.inventory, info.avatar_items);
    let expected = (
        vec![new(1, 3637, 48), new(6, 3632, 62)],
        vec![new(0, 5893, 0)],
    );
    assert_eq!(worn, (Some(expected.0), Some(expected.1)));
}

#[test]
fn only_a_character_of_the_account_enters() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    world.take_frames(s);
    join(&mut world, s, "Nobody");
    assert_eq!(frames(&mut world, s), [(0xB001, vec![0x02, 0x0D, 0x04])]);
    assert!(matches!(world.phase(s), Some(Phase::Lobby(_))));
}

/// A character whose model is no player's in the reference data could not be
/// spawned as one, so its join is refused like one whose model has no row.
#[test]
fn a_character_whose_model_is_no_players_is_refused() {
    let mut store = MemoryStore::default();
    store.add_account("admin", "secret", vec![hero()]);
    let account = login(&store, "admin", "secret").expect("the seeded password logs in");
    let mut refdata = RefData::default();
    // the hero's model id and speeds, with an NPC's type ids (1/2/2)
    let mut npc: Vec<String> = character_row(hero().ref_id, 16.0, 50.0)
        .split('\t')
        .map(String::from)
        .collect();
    (npc[10], npc[11]) = ("2".into(), "2".into());
    refdata.add_characters(&npc.join("\t"));
    let mut world = World::new(Arc::new(store), refdata);
    let ticket = world
        .admit(account, "admin", vec![hero()])
        .expect("no agent session yet");
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    world.take_frames(s);
    join(&mut world, s, "Hero");
    assert_eq!(frames(&mut world, s), [(0xB001, vec![0x02, 0x15, 0x04])]);
    let refused = Event::NoRefData {
        session: s,
        ref_id: hero().ref_id,
    };
    assert!(world.take_events().contains(&refused));
    assert!(matches!(world.phase(s), Some(Phase::Lobby(_))));
}

#[test]
fn game_ready_counts_only_while_joining() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x3012, Bytes::new());
    let rejected = Event::Rejected {
        session: s,
        opcode: 0x3012,
        len: 0,
        reason: Reason::NotHandled,
    };
    assert_eq!(world.take_events(), [rejected]);
}

/// The speeds come from the user's own table, through the client's row type.
#[test]
#[ignore = "needs the user's data: OPENROAD_TEXTDATA_DIR=<textdata> cargo test -p server --test login -- --ignored"]
fn the_speeds_come_from_the_users_table() {
    let dir = std::env::var("OPENROAD_TEXTDATA_DIR").expect("OPENROAD_TEXTDATA_DIR is set");
    let refdata = RefData::load(std::path::Path::new(&dir)).expect("the folder is readable");
    let speeds = refdata
        .character(hero().ref_id)
        .expect("the model is in the table");
    assert_eq!((speeds.walk_speed, speeds.run_speed), (16.0, 50.0));
}

/// Every phase takes a heartbeat and stays silent: before the login, in the
/// lobby and in the game. Nothing is answered, and nothing is reported as
/// rejected.
#[test]
fn a_heartbeat_is_taken_and_left_unanswered_in_every_phase() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    let beat = |world: &mut World| {
        send(world, s, Packet::KeepAlive(KeepAlive));
        assert!(frames(world, s).is_empty(), "{:?}", world.phase(s));
    };
    beat(&mut world);
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    frames(&mut world, s);
    beat(&mut world);
    let join = CharacterJoinRequest {
        character_name: "Hero".to_owned(),
    };
    send(&mut world, s, Packet::CharacterJoinRequest(join));
    world.receive(s, 0x3012, Bytes::new());
    frames(&mut world, s);
    assert!(matches!(world.phase(s), Some(Phase::Game { .. })));
    beat(&mut world);
    assert!(world
        .take_events()
        .iter()
        .all(|e| !matches!(e, Event::Rejected { .. })));
}

/// The consignment listing (0x750E), which the original client asks for
/// after the entry: success with no rows, as the vSRO server answered. Before
/// the game it is not handled.
#[test]
fn the_consignment_listing_is_empty() {
    let (mut world, ticket) = admitted();
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    frames(&mut world, s);
    world.receive(s, 0x750E, Bytes::new());
    assert!(frames(&mut world, s).is_empty());
    join(&mut world, s, "Hero");
    world.receive(s, 0x3012, Bytes::new());
    frames(&mut world, s);
    world.receive(s, 0x750E, Bytes::new());
    assert_eq!(frames(&mut world, s), [(0xB50E, vec![0x01, 0x00])]);
}

/// The step does not read the store: the lobby lists, and the join finds, the
/// characters the ticket brought. Here the store has none.
#[test]
fn the_lobby_and_the_join_use_the_characters_of_the_ticket() {
    let mut refdata = RefData::default();
    refdata.add_characters(&character_row(hero().ref_id, 16.0, 50.0));
    let mut world = World::new(Arc::new(MemoryStore::default()), refdata);
    let ticket = world.admit(AccountId(1), "admin", vec![hero()]).unwrap();
    let s = world.connect();
    log_in(&mut world, s, ticket, "admin");
    frames(&mut world, s);
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let listed = frames(&mut world, s);
    let name = b"\x04\x00Hero";
    assert!(
        listed[0].1.windows(name.len()).any(|w| w == name),
        "{listed:?}"
    );
    join(&mut world, s, "Hero");
    assert_eq!(frames(&mut world, s)[0], (0xB001, vec![0x01]));
}

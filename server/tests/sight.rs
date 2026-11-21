//! Who sees whom, in-process: characters in one region are shown to each
//! other, see each other walk, and are taken back when they leave.

use std::sync::Arc;

use bytes::Bytes;
use packets::agent::entity_spawn::{parse_single_spawn, SingleTail, SpawnKind};
use packets::agent::prelude::{
    AgentLoginRequest, CharacterJoinRequest, ChatRequest, MovementRequest,
};
use packets::Packet;
use server::refdata::RefData;
use server::store::{login, Character, MemoryStore, Position, Store};
use server::world::{Event, Phase, Reason, SessionId, World};
use sro_textdata::characterdata::CharacterData;
use sro_textdata::itemdata::ItemData;
use sro_textdata::spawn::SpawnTables;

mod common;
use common::item_row;

const REGION: u16 = 25000;

fn character(name: &str, region: u16) -> Character {
    Character {
        id: 0,
        name: name.to_owned(),
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
            region,
            x: 969.0,
            y: 0.0,
            z: 1419.0,
            heading: 0,
        },
        equipment: Vec::new(),
        avatar: Vec::new(),
    }
}

/// Accounts a, b, c, d (each with its name as password): Hero and Kiro in
/// one region, Next in the region east of it and Far two regions east; the
/// store, for the gateway's login.
fn world() -> (World, Arc<MemoryStore>) {
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![character("Hero", REGION)]);
    store.add_account("b", "b", vec![character("Kiro", REGION)]);
    store.add_account("c", "c", vec![character("Far", REGION + 2)]);
    store.add_account("d", "d", vec![character("Next", REGION + 1)]);
    let store = Arc::new(store);
    (World::new(store.clone(), refdata()), store)
}

/// The model 1907: a player's (type ids 1/1), with its walk and run speeds.
fn model_row() -> String {
    let mut row = vec![String::from("0"); 48];
    (row[1], row[9], row[10], row[46], row[47]) = (
        "1907".into(),
        "1".into(),
        "1".into(),
        "16".into(),
        "50".into(),
    );
    row.join("\t")
}

/// Three items to wear: a sword, a heavy armour's body and an avatar flag.
fn item_rows() -> String {
    let items = [
        (3632, "ITEM_CH_SWORD_01_A_DEF", [3, 1, 6, 2]),
        (3637, "ITEM_CH_M_HEAVY_01_BA_A_DEF", [3, 1, 3, 3]),
        (5893, "ITEM_ETC_E051021_BLUEDRAGON_BUNTING_M", [3, 1, 13, 4]),
    ];
    items
        .map(|(id, code, tid)| item_row(id, code, tid, (0, 0)))
        .join("\n")
}

fn refdata() -> RefData {
    let mut refdata = RefData::default();
    refdata.add_characters(&model_row());
    refdata.add_items(&item_rows());
    refdata
}

fn send(world: &mut World, session: SessionId, packet: Packet) {
    let (opcode, body) = packet.into_serialize();
    world.receive(session, opcode, body);
}

/// `user`'s character in the game, as its client gets it there; what the
/// session was sent on the way is dropped.
fn enter(shard: &mut (World, Arc<MemoryStore>), user: &str, name: &str) -> (SessionId, u32) {
    let s = loading(shard, user, name);
    let world = &mut shard.0;
    world.receive(s, 0x3012, Bytes::new());
    let Some(Phase::Game { uid, .. }) = world.phase(s) else {
        panic!("{name} is in the game");
    };
    (s, uid)
}

/// `user`'s character joined, its client loading the world: after the
/// entry's 0x34A6, before its own 0x3012. What it was sent is dropped.
fn loading((world, store): &mut (World, Arc<MemoryStore>), user: &str, name: &str) -> SessionId {
    let account = login(store.as_ref(), user, user).unwrap();
    let token = world
        .admit(account, user, store.characters(account))
        .unwrap();
    let s = world.connect();
    let login = AgentLoginRequest {
        token,
        username: user.to_owned(),
        password: String::new(),
        content_id: 22,
        mac_address: [0; 6],
    };
    send(world, s, Packet::AgentLoginRequest(login));
    world.receive(s, 0x7007, Bytes::from_static(&[0x02]));
    let join = CharacterJoinRequest {
        character_name: name.to_owned(),
    };
    send(world, s, Packet::CharacterJoinRequest(join));
    world.take_frames(s);
    s
}

/// The frames `session` was sent, but for the entering character's own values
/// (0x303D), which are not about sight.
fn opcodes(world: &mut World, session: SessionId) -> Vec<(u16, Bytes)> {
    let frames = world.take_frames(session).into_iter();
    frames
        .filter(|f| f.opcode != 0x303D)
        .map(|f| (f.opcode, f.body))
        .collect()
}

/// The unique id in a player's single spawn: behind the model id, five bytes
/// of build and inventory, and the four of its empty item sections.
fn spawned(body: &Bytes) -> u32 {
    u32::from_le_bytes(body[13..17].try_into().unwrap())
}

/// The character a spawn (0x3015), despawn (0x3016) or walk (0xB021) is about.
fn about((opcode, body): &(u16, Bytes)) -> u32 {
    match opcode {
        0x3015 => spawned(body),
        _ => u32::from_le_bytes(body[..4].try_into().unwrap()),
    }
}

#[test]
fn two_characters_in_one_region_are_shown_to_each_other() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    assert!(
        opcodes(&mut shard.0, a).is_empty(),
        "alone, nothing to show"
    );
    let (b, kiro) = enter(&mut shard, "b", "Kiro");
    let to_a = opcodes(&mut shard.0, a);
    let to_b = opcodes(&mut shard.0, b);
    assert_eq!(
        (to_a.len(), to_a[0].0, spawned(&to_a[0].1)),
        (1, 0x3015, kiro)
    );
    assert_eq!(
        (to_b.len(), to_b[0].0, spawned(&to_b[0].1)),
        (1, 0x3015, hero)
    );
    // A standing character without gear or guild: 90 bytes and its name,
    // ending in the PK flag and the single spawn's last byte.
    let body = &to_a[0].1;
    assert_eq!(
        (body.len(), &body[body.len() - 2..]),
        (90 + 4, &[0xFF, 0x01][..])
    );
}

/// The spawn comes from the shared writer: the spawn reader in `packets`
/// reads it back as the character, to its last byte.
#[test]
fn a_spawn_reads_back_as_the_character() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let (_, kiro) = enter(&mut shard, "b", "Kiro");
    let to_a = opcodes(&mut shard.0, a);
    let tables = refdata();
    let (entity, tail) =
        parse_single_spawn(&to_a[0].1, &tables.spawn_tables()).expect("Kiro's spawn reads");
    let read = (entity.unique_id, entity.name.as_deref(), tail);
    assert_eq!(read, (kiro, Some("Kiro"), SingleTail::Figure(1)));
    assert!(matches!(entity.kind, SpawnKind::Player { .. }));
}

/// A spawn names the items the character wears, its inventory slots in slot
/// order and each with plus 0. It is read as the client reads it, with tables
/// of its own: whether a plus byte follows an item is the item table's to say,
/// and the server's tables could agree with a writer that leaves it out.
#[test]
fn a_spawn_shows_the_items_the_character_wears() {
    let mut store = MemoryStore::default();
    store.add_account("a", "a", vec![character("Hero", REGION)]);
    let mut kiro = character("Kiro", REGION);
    (kiro.equipment, kiro.avatar) = (vec![(6, 3632), (1, 3637)], vec![(0, 5893)]);
    store.add_account("b", "b", vec![kiro]);
    let store = Arc::new(store);
    let mut shard = (World::new(store.clone(), refdata()), store);
    let (a, _) = enter(&mut shard, "a", "Hero");
    enter(&mut shard, "b", "Kiro");
    let to_a = opcodes(&mut shard.0, a);
    let characters = CharacterData::parse_shard(&model_row());
    let items = ItemData::parse_shard(&item_rows());
    let client = SpawnTables {
        characters: Some(&characters),
        items: Some(&items),
        gates: None,
    };
    let (entity, _) = parse_single_spawn(&to_a[0].1, &client).expect("Kiro's spawn reads");
    let SpawnKind::Player {
        equipment, avatar, ..
    } = entity.kind
    else {
        panic!("a player");
    };
    let worn = (equipment, avatar);
    assert_eq!(worn, (vec![(3637, 0), (3632, 0)], vec![(5893, 0)]));
}

#[test]
fn a_walk_is_seen_by_whoever_was_shown_the_walker() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    let (b, _) = enter(&mut shard, "b", "Kiro");
    opcodes(&mut shard.0, a);
    opcodes(&mut shard.0, b);
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut shard.0, a, Packet::MovementRequest(walk));
    for session in [a, b] {
        let seen = opcodes(&mut shard.0, session);
        assert_eq!(seen.len(), 1);
        assert_eq!(
            (seen[0].0, &seen[0].1[..4]),
            (0xB021, &hero.to_le_bytes()[..])
        );
    }
}

#[test]
fn a_character_that_enters_is_shown_a_walker_walking() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let walk = MovementRequest::ToLocation {
        region: REGION,
        x: 1119,
        y: 0,
        z: 1419,
    };
    send(&mut shard.0, a, Packet::MovementRequest(walk));
    let (b, _) = enter(&mut shard, "b", "Kiro");
    let to_b = opcodes(&mut shard.0, b);
    // Behind the 16-byte position, the movement block names the destination
    // (flag, type, region, three u16) instead of the standing source and
    // heading: five bytes more.
    let mut block = vec![1, 1];
    for v in [REGION, 1119, 0, 1419] {
        block.extend(v.to_le_bytes());
    }
    let body = &to_b[0].1;
    assert_eq!((to_b[0].0, body.len()), (0x3015, 94 + 5));
    assert_eq!(&body[33..43], &block[..]);
}

#[test]
fn a_character_in_a_neighbouring_region_is_shown() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    let (d, next) = enter(&mut shard, "d", "Next");
    assert_eq!(spawned(&opcodes(&mut shard.0, a)[0].1), next);
    assert_eq!(spawned(&opcodes(&mut shard.0, d)[0].1), hero);
}

/// Hero walks one region east, into sight of Far two regions east, and back
/// out of it. On the way back Far, who knows Hero by then, also sees the walk.
#[test]
fn walking_across_a_border_shows_and_takes_back_both_ways() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    let (c, far) = enter(&mut shard, "c", "Far");
    assert!(opcodes(&mut shard.0, a).is_empty());
    let east = (REGION + 1, 100, vec![0xB021, 0x3015], vec![0x3015]);
    let back = (REGION, 969, vec![0xB021, 0x3016], vec![0xB021, 0x3016]);
    for (region, x, to_hero, to_far) in [east, back] {
        let walk = MovementRequest::ToLocation {
            region,
            x,
            y: 0,
            z: 1419,
        };
        send(&mut shard.0, a, Packet::MovementRequest(walk));
        while shard.0.player(hero).unwrap().walk.is_some() {
            shard.0.step();
        }
        let (seen_by_hero, seen_by_far) = (opcodes(&mut shard.0, a), opcodes(&mut shard.0, c));
        let codes = |frames: &[(u16, Bytes)]| frames.iter().map(|f| f.0).collect::<Vec<_>>();
        assert_eq!(codes(&seen_by_hero), to_hero);
        assert_eq!(codes(&seen_by_far), to_far);
        assert_eq!(about(seen_by_hero.last().unwrap()), far);
        assert_eq!(about(seen_by_far.last().unwrap()), hero);
    }
}

#[test]
fn a_character_in_another_region_is_not_shown() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let (c, _) = enter(&mut shard, "c", "Far");
    assert!(opcodes(&mut shard.0, a).is_empty());
    assert!(opcodes(&mut shard.0, c).is_empty());
}

#[test]
fn a_character_that_leaves_is_taken_back_once() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let (b, kiro) = enter(&mut shard, "b", "Kiro");
    opcodes(&mut shard.0, a);
    shard.0.disconnect(b);
    let gone = opcodes(&mut shard.0, a);
    assert_eq!(
        gone,
        [(0x3016, Bytes::copy_from_slice(&kiro.to_le_bytes()))]
    );
    shard.0.disconnect(b);
    assert!(opcodes(&mut shard.0, a).is_empty());
}

/// A line said in the local channel reaches the speaker and whoever sees it,
/// as 0x3026 with the speaker's id, and the speaker then gets the
/// acknowledgement with its index; a character out of sight hears nothing,
/// and a whisper is not handled yet.
#[test]
fn a_line_said_is_heard_by_whoever_sees_the_speaker() {
    let mut shard = world();
    let (a, hero) = enter(&mut shard, "a", "Hero");
    let (b, _) = enter(&mut shard, "b", "Kiro");
    let (c, _) = enter(&mut shard, "c", "Far");
    for s in [a, b, c] {
        opcodes(&mut shard.0, s);
    }
    let hallo = ChatRequest {
        chat_type: 1,
        chat_index: 5,
        receiver: None,
        message: "Hallo".to_owned(),
    };
    send(&mut shard.0, a, Packet::ChatRequest(hallo));
    let mut line = vec![0x01];
    line.extend(hero.to_le_bytes());
    line.extend(b"\x05\x00Hallo");
    let heard = (0x3026, Bytes::from(line));
    let ack = (0xB025, Bytes::from_static(&[0x01, 0x01, 0x05]));
    assert_eq!(opcodes(&mut shard.0, a), [heard.clone(), ack]);
    assert_eq!(opcodes(&mut shard.0, b), [heard]);
    assert!(opcodes(&mut shard.0, c).is_empty());

    let whisper = ChatRequest {
        chat_type: 2,
        chat_index: 6,
        receiver: Some("Kiro".to_owned()),
        message: "psst".to_owned(),
    };
    send(&mut shard.0, a, Packet::ChatRequest(whisper));
    assert!(opcodes(&mut shard.0, b).is_empty());
    let rejected = shard.0.take_events().into_iter().any(|e| {
        matches!(
            e,
            Event::Rejected {
                opcode: 0x7025,
                reason: Reason::NotHandled,
                ..
            }
        )
    });
    assert!(rejected);
}

/// A client loading the world crashes on a chat frame before its own 0x3012
/// [V original handler], so a line said nearby while it loads does not reach
/// it, neither then nor once it is in the game.
#[test]
fn a_client_loading_the_world_does_not_hear_a_line() {
    let mut shard = world();
    let (a, _) = enter(&mut shard, "a", "Hero");
    let b = loading(&mut shard, "b", "Kiro");
    opcodes(&mut shard.0, a);
    let hallo = ChatRequest {
        chat_type: 1,
        chat_index: 1,
        receiver: None,
        message: "Hallo".to_owned(),
    };
    send(&mut shard.0, a, Packet::ChatRequest(hallo));
    assert!(opcodes(&mut shard.0, b).is_empty());
    shard.0.receive(b, 0x3012, Bytes::new());
    let codes: Vec<u16> = opcodes(&mut shard.0, b).iter().map(|f| f.0).collect();
    assert_eq!(codes, [0x3015], "only Hero's spawn, no chat");
}

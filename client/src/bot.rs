//! Clientless bot: a headless, remotely driven game session.
//!
//! Idea: the netcheck harness already proves that the login → join → in-world
//! packet flow runs under `MinimalPlugins` with no window and no GPU. A bot is
//! that same session made *steerable*: the login credentials come from the
//! environment (so N bots on N accounts run side by side), and a Bevy Remote
//! Protocol surface — the same transport the GUI client exposes — turns the
//! session into an API: `bot/status` reports what the character is and where,
//! `bot/entities` lists what the server has spawned around it, and
//! `bot/command` queues one action (move, select, attack, pickup, chat, party,
//! exchange) to be sent on the next frame.
//!
//! Why remote-controlled rather than scripted in Rust: what needs testing is
//! the *packet layer and the game mechanics*, and the decisions ("which of
//! these ref ids is a monster I should hit") need refdata plus judgement. Both
//! live better in the driving script than in the client, and keeping them out
//! here means the bot has no game logic of its own to be wrong about — it
//! reports what the server said and sends what it was told.
//!
//! Deliberately NOT reused from the GUI: the HUD, scenes and asset pipeline.
//! Deliberately reused verbatim: framing/handshake (`NetworkCorePlugin`), every
//! packet codec (`packets`), and the netcheck login/join driver — a bot that
//! logged in its own way would stop testing the client's login.
//!
//! ```bash
//! BOT=1 BOT_ACCOUNT=myaccount BOT_PASSWORD=1234 BOT_PORT=15810 cargo run -p client
//! ```
//!
//! Give each session its own `PACKET_DUMP_DIR` when several run at once, so
//! their packet logs do not interleave.

use std::collections::{HashMap, VecDeque};
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use bevy::remote::{error_codes, http::RemoteHttpPlugin, BrpError, BrpResult, RemotePlugin};
use serde::Serialize;
use serde_json::{json, Value};

use packets::agent::lobby::{
    CharacterCreate, CharacterSelectionAction, CharacterSelectionActionRequest,
    CharacterSelectionActionResponse,
};

use packets::agent::character_data::{
    parse_character_data, parse_character_info, ItemClass, ItemClassResolver, ItemTypeData,
    ParsedCharacterInfo,
};
use packets::agent::inventory::{EntityEquip, InventoryOperationResult, INVENTORY_SLOT_GOLD};
use packets::agent::party::{
    PartyCreateResponse, PartyCreationRequest, PartyData, PartyInviteResponse, PartyLeave,
    PartyUpdate,
};
use packets::agent::prelude::{
    ActionCommand, ActionTarget, CelestialPosition, CharacterDataBody, CharacterPointsUpdate,
    CharacterStatsUpdate, ChatRequest, ChatUpdate, CloseTalkRequest, EntityBarsUpdate,
    ExchangeApproveRequest, ExchangeCanceled, ExchangeCompleted, ExchangeConfirmRequest,
    ExchangeExitRequest, ExchangeInviteRequest, ExchangeInviteResponse, ExchangeStarted,
    GameInvite, GmResponse, GroupEntitySpawnBegin, GroupEntitySpawnData, InventoryOperationRequest,
    InventoryOperationResponse, InviteResponse, ItemUseRequest, LogoutRequest,
    MovementPositionUpdate, MovementRequest, MovementResponse, ObjectActionRequest,
    ObjectActionResponse, ObjectActionUpdate, PartyInviteRequest, ReceiveExperience,
    SelectEntityRequest, SelectEntityResponse, SingleEntityDespawn, SingleEntitySpawn, TalkRequest,
    TalkResponse,
};
use packets::Packet;

use bevy_pk2::prelude::{Archive, Pk2Key};

use crate::assets::textdata::characterdata::CharacterDataRow;
use crate::assets::textdata::decode::decode_textdata;
use crate::assets::textdata::itemdata::ItemDataRow;
use crate::net::connection::SilkroadConnection;
use crate::net::entity_spawn::{parse_group_spawn, RefResolver, RefType};
use crate::net::frame::SilkroadFrame;
use crate::netcheck::{self, LoginIdentity};
use crate::plugins::config::division::DivisionInfo;
use crate::plugins::config::ClientConfig;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::gateway::systems::init_gateway_service;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::net::packet_dump::dump_root;
use crate::plugins::net::plugin::NetworkCorePlugin;

/// Default control port. One bot per port; `BOT_PORT` picks it.
const DEFAULT_BOT_PORT: u16 = 15810;

/// How many event lines the rolling log keeps. Enough for a driving script that
/// polls every second or two to never miss a kill, a level-up or an error.
const LOG_CAPACITY: usize = 400;

/// The control methods `run_bot` registers. Named once so the README section
/// and the registration cannot drift apart (there is a test for that).
const BRP_METHODS: [&str; 6] = [
    "bot/status",
    "bot/entities",
    "bot/inventory",
    "bot/log",
    "bot/command",
    "bot/send",
];

/// Smallest gap between two commands leaving the queue, in seconds.
///
/// Origin, not taste: 210 ms is the shortest `Action_ActionDuration` (col 13)
/// of any castable player skill in the v1.188 corpus — measured over all
/// `Media.pk2` `server_dep/silkroad/textdata/skilldata_*.txt` shards (30661
/// rows, activity 1 and 2 of the `SKILL_CH_*`/`SKILL_EU_*` rows); the minimum
/// is `skilldata_5000.txt` line 22, `SKILL_CH_SWORD_CHAIN_E_1S_01`, col 13 =
/// 210 (next values 262, 265, 283 ms). Nothing the character can do finishes
/// faster, so commands issued faster than this cannot be answered by the
/// session — they only make an error impossible to attribute. `BOT_COMMAND_INTERVAL`
/// (seconds, `0` = as fast as frames allow) overrides it for opcode probing.
const DEFAULT_COMMAND_INTERVAL: f64 = 0.210;

/// Build and run the bot. Blocks until the process is killed.
pub fn run_bot(config: ClientConfig) {
    let port = env::var("BOT_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_BOT_PORT);
    let identity = LoginIdentity {
        username: env::var("BOT_ACCOUNT")
            .unwrap_or_else(|_| config.dev_fast_login.username.clone()),
        password: env::var("BOT_PASSWORD")
            .unwrap_or_else(|_| config.dev_fast_login.password.clone()),
        character: env::var("BOT_CHAR").ok().filter(|c| !c.trim().is_empty()),
        captcha_answer: config.dev_fast_login.captcha_answer.clone(),
    };
    info!(
        "bot: account '{}', character {:?}, control port {}",
        identity.username, identity.character, port
    );

    App::new()
        .add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(5))))
        .add_plugins(LogPlugin {
            level: Level::TRACE,
            // `RUST_LOG` wins so a driving script can turn on the packet traces
            // (`packets_in=trace`) or the bot's own debug lines for one run.
            filter: env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
            ..default()
        })
        .add_plugins((
            RemotePlugin::default()
                .with_method_main(BRP_METHODS[0], brp_status)
                .with_method_main(BRP_METHODS[1], brp_entities)
                .with_method_main(BRP_METHODS[2], brp_inventory)
                .with_method_main(BRP_METHODS[3], brp_log)
                .with_method_main(BRP_METHODS[4], brp_command)
                .with_method_main(BRP_METHODS[5], brp_send),
            RemoteHttpPlugin::default().with_port(port),
        ))
        .insert_resource(config)
        .insert_resource(DivisionInfo::load_or_fallback())
        .insert_resource(identity)
        .init_resource::<BotState>()
        .init_resource::<BotWorld>()
        .init_resource::<PendingCharacterData>()
        .init_resource::<BotWalk>()
        .init_resource::<BotInventory>()
        .init_resource::<BotQueue>()
        .insert_resource(BotPace::from_env())
        .insert_resource(BotRefdata::load())
        .add_plugins(NetworkCorePlugin)
        .add_systems(Startup, init_gateway_service)
        .add_plugins(netcheck::LoginDriverPlugin)
        .add_systems(
            Update,
            (
                track_identity,
                track_position,
                track_entities,
                track_actions,
                track_progress,
                track_social,
                track_inventory.after(track_identity),
                execute_commands,
            ),
        )
        .run();
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// What the bot knows about itself, as reported by the server. Everything here
/// comes out of a packet — nothing is inferred or simulated locally, so a wrong
/// value is a decoding defect worth reporting rather than bot drift.
#[derive(Resource, Default, Serialize)]
pub struct BotState {
    pub account: String,
    pub character: String,
    pub unique_id: u32,
    /// characterdata ref id of the character itself (its race row), which is
    /// where the run speed comes from.
    pub ref_id: u32,
    pub level: u8,
    /// Experience *within the current level*, as CHARACTER_DATA states it and
    /// 0x3056 moves it — not a lifetime total. A level-up restarts it, which is
    /// why the deltas cannot simply be summed across one (the HUD's own
    /// `on_experience_gain` has the same rule, and it is the reason a bot that
    /// added them reported 16791 where the server said 3631).
    pub exp: u64,
    /// Every positive 0x3056 delta this session.
    pub exp_gained: u64,
    pub skill_exp: u32,
    pub gold: u64,
    pub hp: u32,
    pub mp: u32,
    pub max_hp: u32,
    pub max_mp: u32,
    pub stat_points: u16,
    pub skill_points: u32,
    pub region: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Where the last movement order was headed, when one is outstanding. The
    /// position above is where the server said the character *is*; a driving
    /// script that walks by the destination believes it has arrived while the
    /// character is still walking.
    pub destination: Option<[f32; 4]>,
    /// Whether a walk is still running (dead-reckoned, see `track_position`).
    pub moving: bool,
    pub in_world: bool,
    pub target: Option<u32>,
    /// Pending 0x3080 petition (party/exchange/guild/academy) awaiting an answer.
    pub pending_invite: Option<PendingInvite>,
    pub party: Vec<String>,
    pub kills: u32,
    pub deaths: u32,
    pub commands_sent: u32,
}

#[derive(Serialize, Clone)]
pub struct PendingInvite {
    pub petition: u8,
    pub unique_id: u32,
}

/// One spawned entity as the server described it. `ref_id` is passed through
/// raw: classifying it needs the refdata tables, which the driving script has
/// and the headless bot deliberately does not load.
#[derive(Serialize, Clone)]
pub struct NearbyEntity {
    pub unique_id: u32,
    pub ref_id: u32,
    pub region: u16,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Seconds since this entity was last seen in a spawn/movement packet.
    pub age_secs: f32,
}

/// The last CHARACTER_DATA body, kept until a position could be pinned from it.
///
/// Same ordering hazard netcheck hit (#728): on the live server the body
/// (0x3013) arrives ~20 ms *before* `CelestialPosition` (0x3020) names the
/// local unique id, and the id-less anchor scan refuses a body that offers more
/// than one plausible position. Keeping the bytes lets the scan be retried the
/// moment the id lands instead of losing the character's position for the whole
/// session.
#[derive(Resource, Default)]
pub struct PendingCharacterData(Option<bytes::Bytes>);

/// The character's inventory, mirrored from what the *server* confirmed.
///
/// Idea: the bag is seeded from CHARACTER_DATA (0x3013) and then kept current
/// by folding every server-confirmed operation in — 0xB034 op 0 (move/swap),
/// op 6 (pickup), op 15 (slot cleared) and the 0x3038 equip broadcast. The
/// bookkeeping is not re-invented here: it is [`Inventory`], the same
/// slot-indexed component the GUI's net layer keeps, so a bot-side slot and a
/// HUD-side slot are wrong or right for the same reasons.
///
/// Why this replaced a "seed only, log the deltas" resource: that shape can
/// answer `bot/inventory` with `{"size":109,"items":[]}` after the server has
/// already confirmed a pickup, a move and an equip — and a reader concludes the
/// character carries nothing. Two separate defects produce that answer and both
/// are addressed here: the deltas were never folded in, and an item section that
/// could not be *read* was reported as an inventory that was *empty*. Hence
/// `defect`: when the seed is not trustworthy the RPC says so instead of showing
/// a short list.
#[derive(Resource, Default)]
pub struct BotInventory {
    items: Inventory,
    /// Did a CHARACTER_DATA item section ever reach this resource?
    seeded: bool,
    /// Set when the seed is known to be incomplete (no itemdata to classify
    /// records with, item section stopped mid-way, no item section at all).
    /// `None` means the section parsed to its last record.
    defect: Option<String>,
    /// Server-confirmed operations folded in since the last seed.
    updates: u32,
    /// The slot/ref id of an equip this mirror already carried out from a
    /// 0x3038 broadcast, waiting for the 0xB034 move that says the same thing.
    /// See [`Self::apply_equip`] for why it must not be applied twice.
    pending_equip: Option<(u8, u32)>,
}

impl BotInventory {
    /// (Re)seed from a parsed CHARACTER_DATA body.
    ///
    /// `item_rows` is the number of itemdata rows the bot could load: with zero
    /// of them every record is [`ItemClass::Unknown`], the forward pass stops on
    /// the very first one, and `info.inventory` is `Some(vec![])` — an empty
    /// list that means "unreadable", not "empty bag". That answer is a lie, so it
    /// is named as a defect rather than shown.
    fn seed(&mut self, info: &ParsedCharacterInfo, item_rows: usize) {
        self.items = Inventory::from_character(info);
        self.seeded = true;
        self.updates = 0;
        self.defect = if item_rows == 0 {
            Some(
                "itemdata could not be loaded (0 rows) — no item record can be classified, so \
                 the item section of CHARACTER_DATA was not read at all. Point SRO_PK2_PATH at \
                 the directory holding Media.pk2 and restart the bot."
                    .to_string(),
            )
        } else if info.inventory.is_none() {
            Some(format!(
                "CHARACTER_DATA carried no usable item section (failed_stage {:?}, parsed {} of \
                 the body)",
                info.failed_stage, info.forward_parsed_to
            ))
        } else {
            info.item_stop.map(|stop| {
                format!(
                    "item section stopped at record {} (offset {}, ref id {} is in no client \
                     item table) — every slot behind it is missing",
                    stop.index, stop.offset, stop.ref_id
                )
            })
        };
    }

    /// Fold one server-confirmed 0xB034 result in. Returns the line to log when
    /// something changed, so the caller decides where it is reported.
    fn apply_operation(
        &mut self,
        op: &InventoryOperationResult,
        resolver: &impl ItemClassResolver,
    ) -> Option<String> {
        match op {
            InventoryOperationResult::Move {
                source,
                target,
                amount,
                ..
            } => {
                // The 0x3038 broadcast of the same equip may have arrived
                // first and already carried this move out; applying it a
                // second time swaps the two slots straight back.
                let already_done = self.pending_equip.take().is_some_and(|(slot, ref_id)| {
                    slot == *target && self.items.get(*target).map(|i| i.ref_id) == Some(ref_id)
                });
                if already_done {
                    return None;
                }
                self.items
                    .apply_move(*source, *target, *amount, &bot_item_data());
                self.updates += 1;
                Some(format!("inventory: slot {source} -> slot {target}"))
            }
            // Gold pickups (slot 0xFE) carry no item record; the gold total
            // comes from CHARACTER_DATA/0x304E, not from here.
            InventoryOperationResult::Pickup { slot, .. } if *slot == INVENTORY_SLOT_GOLD => None,
            InventoryOperationResult::Pickup { slot, .. } => match op.pickup_item(resolver) {
                Some(item) => {
                    let ref_id = item.ref_id;
                    self.items.gain_item(item);
                    self.updates += 1;
                    Some(format!(
                        "inventory: picked up ref id {ref_id} into slot {slot}"
                    ))
                }
                None => Some(format!(
                    "inventory: pickup into slot {slot} could not be decoded — slot left unknown"
                )),
            },
            _ => None,
        }
    }

    /// Fold the 0x3038 equip broadcast for our own character in.
    ///
    /// The equip itself already arrives as a 0xB034 op 0 move a few
    /// milliseconds later, so the normal case is that the slot already holds the
    /// item and there is nothing to do. It is kept because it is the one packet
    /// that states slot *and* ref id together: if the slot disagrees, the item is
    /// relocated when exactly one slot holds that ref id, and otherwise the
    /// disagreement is reported rather than guessed away. A relocation carried
    /// out here is remembered in `pending_equip` so the move behind it is
    /// recognised as the same event and not applied a second time (which would
    /// swap the two slots straight back).
    fn apply_equip(&mut self, slot: u8, ref_id: u32) -> Option<String> {
        if self.items.get(slot).map(|i| i.ref_id) == Some(ref_id) {
            return None;
        }
        let holders: Vec<u8> = (0..self.items.size())
            .filter(|s| self.items.get(*s).map(|i| i.ref_id) == Some(ref_id))
            .collect();
        match holders.as_slice() {
            [source] => {
                self.items.apply_move(*source, slot, 1, &bot_item_data());
                self.pending_equip = Some((slot, ref_id));
                self.updates += 1;
                Some(format!(
                    "inventory: equipped ref id {ref_id} (slot {source} -> slot {slot})"
                ))
            }
            _ => Some(format!(
                "inventory: 0x3038 says slot {slot} holds ref id {ref_id}, but {} slot(s) here \
                 hold it — mirror out of sync",
                holders.len()
            )),
        }
    }
}

#[derive(Resource, Default)]
pub struct BotWorld {
    entities: std::collections::HashMap<u32, (NearbyEntity, f64)>,
    /// The `kind`/`count` headers (0x3017) of batches whose payload (0x3019)
    /// has not arrived yet. A queue rather than a single slot because several
    /// batches land in one frame while walking: keeping only the last header
    /// made every batch in that frame inherit it, and a despawn list
    /// (`[u32 uid]…`) parsed as spawn records is how a walk through Jangan
    /// produced "could not parse spawn record" for every NPC that scrolled out
    /// of range.
    batches: VecDeque<(bool, u16)>,
}

/// Rolling event log plus the queue of commands the control API accepted.
#[derive(Resource, Default)]
pub struct BotQueue {
    commands: VecDeque<Value>,
    log: VecDeque<String>,
}

/// The rate brake on the command queue: the queue is drained one command per
/// frame, and a 5 ms run loop means 200 frames a second, so "one per frame" on
/// its own is not a brake at all. See [`DEFAULT_COMMAND_INTERVAL`] for where
/// the spacing comes from.
#[derive(Resource)]
pub struct BotPace {
    interval: f64,
    /// When a command last left the queue; `None` until the first one does.
    last_sent: Option<f64>,
}

impl Default for BotPace {
    fn default() -> Self {
        Self {
            interval: DEFAULT_COMMAND_INTERVAL,
            last_sent: None,
        }
    }
}

impl BotPace {
    fn from_env() -> Self {
        Self {
            interval: env::var("BOT_COMMAND_INTERVAL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_COMMAND_INTERVAL),
            last_sent: None,
        }
    }

    /// Whether the next queued command may leave at `now` (seconds since start).
    fn due(&self, now: f64) -> bool {
        self.last_sent
            .is_none_or(|last| now - last >= self.interval)
    }

    fn mark(&mut self, now: f64) {
        self.last_sent = Some(now);
    }
}

impl BotQueue {
    fn note(&mut self, line: impl Into<String>) {
        if self.log.len() >= LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(line.into());
    }
}

// ---------------------------------------------------------------------------
// Refdata
// ---------------------------------------------------------------------------

/// The two textdata tables the *spawn parser* cannot work without: a record's
/// width depends on what its leading ref id is (player / monster / NPC / COS /
/// item / gold), and only characterdata and itemdata say which.
///
/// Read straight out of `Media.pk2` with [`Archive::read_file_bytes`] rather
/// than through the asset server, exactly as `DivisionInfo` does and for the
/// same reason: the headless session builds no asset server at all. Both master
/// files list their real data files, so the loader follows that indirection.
/// Empty itemdata for [`Inventory::apply_move`].
///
/// The bot's mirror has no `ClientItemData`: `BotRefdata` keeps its own
/// `ItemDataRow` map for the walk/attack logic, not the plugin resource the
/// inventory model uses. Without `max_stack` the merge path declines and
/// `apply_move` keeps the whole-slot relocate/swap it had before #862 — which
/// is also its production fail-safe for a row itemdata does not know
/// (`net::inventory::an_unknown_item_falls_back_to_swapping`). The bot logs
/// slot movements, not stack sizes, so nothing it reports depends on it.
fn bot_item_data() -> crate::plugins::textdata::ClientItemData {
    crate::plugins::textdata::ClientItemData::default()
}

#[derive(Resource, Default)]
pub struct BotRefdata {
    chars: HashMap<i32, CharacterDataRow>,
    items: HashMap<i32, ItemDataRow>,
}

impl BotRefdata {
    fn load() -> Self {
        let media = std::env::var_os("SRO_PK2_PATH")
            .or_else(|| std::env::var_os("SRO_PATH"))
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                let mut dir = std::env::current_dir().unwrap_or_default();
                dir.push("assets");
                dir
            })
            .join("Media.pk2");

        // The archive is optional here (the bot still logs in without it), so
        // its failures are reported, not unwound.
        let loaded = Pk2Key::resolve()
            .map_err(|e| e.to_string())
            .and_then(|key| Archive::open(&media, &key).map_err(|e| format!("{e:?}")))
            .map(|archive| {
                let mut chars = HashMap::new();
                for row in read_master_table(&archive, "characterdata.txt") {
                    if let Some((id, cells)) = keyed_row(row) {
                        chars.insert(id, CharacterDataRow(cells));
                    }
                }
                let mut items = HashMap::new();
                for row in read_master_table(&archive, "itemdata.txt") {
                    if let Some((id, cells)) = keyed_row(row) {
                        items.insert(id, ItemDataRow(cells));
                    }
                }
                Self { chars, items }
            });
        match loaded {
            Ok(data) => {
                info!(
                    "bot: refdata loaded — {} character rows, {} item rows",
                    data.chars.len(),
                    data.items.len()
                );
                data
            }
            Err(err) => {
                warn!(
                    "bot: could not read characterdata/itemdata from {}: {err} — every spawn \
                     record will be unclassifiable, so the nearby-entity table stays empty",
                    media.display()
                );
                Self::default()
            }
        }
    }
}

/// Read a textdata master file (a list of real data files, one per line) and
/// return every data row of every listed file, tab-split.
fn read_master_table(archive: &Archive, master: &str) -> Vec<Vec<String>> {
    let dir = "server_dep/silkroad/textdata";
    let Some(list) = archive.read_file_bytes(&PathBuf::from(format!("{dir}/{master}"))) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for file in decode_textdata(&list).lines() {
        let file = file.trim();
        if file.is_empty() {
            continue;
        }
        let Some(bytes) = archive.read_file_bytes(&PathBuf::from(format!("{dir}/{file}"))) else {
            continue;
        };
        for line in decode_textdata(&bytes).lines() {
            let cells: Vec<String> = line.split('\t').map(String::from).collect();
            if cells.len() > 10 {
                rows.push(cells);
            }
        }
    }
    rows
}

/// Column 1 is the ref id in both tables (the same `RefObjCommon` schema).
fn keyed_row(cells: Vec<String>) -> Option<(i32, Vec<String>)> {
    let id = cells.get(1)?.trim().parse::<i32>().ok()?;
    Some((id, cells))
}

/// The same classification the GUI client uses
/// (`net::entity_spawn::TextdataResolver`), over tables loaded here.
struct BotResolver<'a>(&'a BotRefdata);

impl RefResolver for BotResolver<'_> {
    fn resolve(&self, ref_id: u32) -> RefType {
        let id = ref_id as i32;
        if let Some(item) = self.0.items.get(&id) {
            return RefType::Item {
                equipment: item.is_equipment(),
                gold: item.is_gold(),
            };
        }
        if let Some(ch) = self.0.chars.get(&id) {
            if ch.is_player() {
                return RefType::Player;
            }
            if ch.is_monster() {
                return RefType::Monster;
            }
            if let Some(kind) = ch.cos_kind() {
                return RefType::Cos { kind };
            }
            return RefType::Npc;
        }
        RefType::Unknown
    }

    fn item_is_equipment(&self, ref_id: u32) -> bool {
        self.0
            .items
            .get(&(ref_id as i32))
            .map(|i| i.is_equipment())
            .unwrap_or(false)
    }
}

// Same TID mapping as `entity_spawn::TextdataResolver` (one rule, two callers).
impl ItemClassResolver for BotResolver<'_> {
    fn item_class(&self, ref_id: u32) -> ItemClass {
        match self
            .0
            .items
            .get(&(ref_id as i32))
            .and_then(|i| i.type_ids())
        {
            Some((3, 1, _, _)) => ItemClass::Equipment,
            Some((3, 2, tid3, tid4)) => ItemClass::Container { tid3, tid4 },
            Some((3, 3, tid3, tid4)) => ItemClass::Expendable { tid3, tid4 },
            _ => ItemClass::Unknown,
        }
    }
}

impl BotRefdata {
    /// The packed `type_id` the item-use packet wants. Same packing the HUD's
    /// own use path applies (`hud::underbar::cast::pack_type_id`) — one rule,
    /// two callers, so a bot-driven use and a quickslot use are the same bytes.
    fn packed_type_id(&self, ref_id: u32) -> Option<u16> {
        let (t1, t2, t3, t4) = self.items.get(&(ref_id as i32))?.type_ids()?;
        Some(((t1 << 2) | (t2 << 5) | (t3 << 7) | (t4 << 11)) as u16)
    }

    /// characterdata `Speed2` (run) for a ref id, in world units per second.
    fn run_speed(&self, ref_id: u32) -> Option<f32> {
        self.chars.get(&(ref_id as i32))?.run_speed()
    }

    /// What the driving script needs to decide whether a ref id is worth
    /// attacking: the kind, the code name, and the level/HP characterdata
    /// claims. Names come from the table's own `CodeName`, not the localized
    /// string table (which would need textdataname too).
    fn describe(&self, ref_id: u32) -> (String, String, u32, u32) {
        let id = ref_id as i32;
        if let Some(item) = self.items.get(&id) {
            return ("item".into(), item.code_name().clone(), 0, 0);
        }
        if let Some(ch) = self.chars.get(&id) {
            let kind = if ch.is_player() {
                "player"
            } else if ch.is_monster() {
                "monster"
            } else if ch.cos_kind().is_some() {
                "cos"
            } else {
                "npc"
            };
            return (
                kind.into(),
                ch.code_name().clone(),
                ch.level().unwrap_or(0),
                ch.max_hp().unwrap_or(0),
            );
        }
        ("unknown".into(), String::new(), 0, 0)
    }
}

// ---------------------------------------------------------------------------
// Trackers
// ---------------------------------------------------------------------------

/// Name, level, gold, hp/mp and inventory size out of CHARACTER_DATA, plus the
/// in-world unique id from the celestial-position packet.
#[allow(clippy::too_many_arguments)]
fn track_identity(
    mut state: ResMut<BotState>,
    mut walk: ResMut<BotWalk>,
    mut queue: ResMut<BotQueue>,
    identity: Res<LoginIdentity>,
    refdata: Res<BotRefdata>,
    mut pending: ResMut<PendingCharacterData>,
    mut inventory: ResMut<BotInventory>,
    mut bodies: MessageReader<CharacterDataBody>,
    mut positions: MessageReader<CelestialPosition>,
) {
    if state.account.is_empty() {
        state.account = identity.username.clone();
    }
    for p in positions.read() {
        if state.unique_id != p.unique_id {
            state.unique_id = p.unique_id;
            state.in_world = true;
            queue.note(format!("joined the world as unique_id {}", p.unique_id));
            // The body that arrived before the id could not be pinned; retry it.
            if let Some(raw) = pending.0.take() {
                if let Some(spawn) = parse_character_data(&raw, Some(p.unique_id)) {
                    walk.position = world_xz(spawn.region, spawn.x, spawn.z);
                    walk.region_y = spawn.y;
                    walk.destination = None;
                    state.region = spawn.region;
                    state.x = spawn.x;
                    state.y = spawn.y;
                    state.z = spawn.z;
                    queue.note(format!(
                        "position pinned from the stashed body: region {} ({}, {}, {})",
                        spawn.region, spawn.x, spawn.y, spawn.z
                    ));
                }
                if state.character.is_empty() {
                    let info =
                        parse_character_info(&raw, Some(p.unique_id), &BotResolver(&refdata));
                    if let Some(name) = info.name {
                        state.character = name;
                    }
                }
            }
        }
    }
    for body in bodies.read() {
        debug!("bot: CHARACTER_DATA body {} bytes", body.raw.len());
        let known_uid = Some(state.unique_id).filter(|id| *id != 0);
        let info = parse_character_info(&body.raw, known_uid, &BotResolver(&refdata));
        if let Some(name) = &info.name {
            state.character = name.clone();
        }
        inventory.seed(&info, refdata.items.len());
        if let Some(defect) = &inventory.defect {
            queue.note(format!("inventory NOT trustworthy: {defect}"));
        } else {
            queue.note(format!(
                "inventory seeded from CHARACTER_DATA: {} item(s) in {} slots",
                inventory.items.slots.iter().flatten().count(),
                inventory.items.size()
            ));
        }
        if let Some(stats) = &info.stats {
            state.ref_id = stats.ref_id;
            let levelled = state.level != 0 && stats.level > state.level;
            let gained = stats.exp.saturating_sub(state.exp);
            state.level = stats.level;
            state.exp = stats.exp;
            state.skill_exp = stats.skill_exp;
            state.gold = stats.gold;
            state.hp = stats.hp;
            state.mp = stats.mp;
            state.stat_points = stats.stat_points;
            state.skill_points = stats.skill_points;
            if levelled {
                queue.note(format!("LEVEL UP -> {}", stats.level));
            }
            if gained > 0 {
                queue.note(format!("exp {} (+{})", stats.exp, gained));
            }
        }
        // Position: the forward pass first, the anchor scanner as the fallback
        // — a character whose item section stops the forward pass (a big
        // inventory does) still has to know where it is standing.
        match info
            .spawn
            .or_else(|| parse_character_data(&body.raw, known_uid))
        {
            Some(spawn) => {
                walk.position = world_xz(spawn.region, spawn.x, spawn.z);
                walk.region_y = spawn.y;
                walk.destination = None;
                state.region = spawn.region;
                state.x = spawn.x;
                state.y = spawn.y;
                state.z = spawn.z;
            }
            // No id yet, so the scan refused: keep the bytes for the retry.
            None => pending.0 = Some(body.raw.clone()),
        }
        queue.note(format!(
            "CHARACTER_DATA parsed fully={} ({} bytes)",
            info.fully_parsed,
            body.raw.len()
        ));
    }
}

/// Where the character is, by dead reckoning.
///
/// The server states a position only when a movement *starts* (the optional
/// source tail of 0xB021, in tenths of a unit) and otherwise reports the
/// destination the walk is heading for. Everything in between is the client's
/// own business — the GUI interpolates it into the transform, and a bot that
/// does not do the same believes it is standing where it ordered itself to go,
/// walks past its target, and reports positions that are minutes old.
///
/// So: pin the position to the source when one arrives, then advance it toward
/// the destination at the character's own run speed from characterdata until it
/// arrives. Region-local coordinates are folded into a flat world grid
/// (1920 units per region sector) so a walk across a region border is one
/// straight line rather than a jump.
fn track_position(
    mut state: ResMut<BotState>,
    mut walk: ResMut<BotWalk>,
    refdata: Res<BotRefdata>,
    mut moves: MessageReader<MovementResponse>,
    mut syncs: MessageReader<MovementPositionUpdate>,
    mut world: ResMut<BotWorld>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    for m in moves.read() {
        if m.unique_id == state.unique_id && state.unique_id != 0 {
            if m.has_destination {
                walk.destination = Some(world_xz(m.region, m.x as f32, m.z as f32));
                walk.dest_y = m.y as f32;
            } else {
                // A turn in place: the walk is over where the source said.
                walk.destination = None;
            }
            continue;
        }
        // Somebody else moved: keep their last known position fresh so the
        // driving script can pick a target that is actually nearby.
        if let Some((entity, seen)) = world.entities.get_mut(&m.unique_id) {
            if m.has_destination {
                entity.region = m.region;
                entity.x = m.x as f32;
                entity.y = m.y as f32;
                entity.z = m.z as f32;
            }
            *seen = now;
        }
    }

    // 0xB023 is the server's absolute answer about where an entity *is*, and it
    // outranks the dead reckoning: it arrives for teleports and knockback, at
    // the end of a walk, and — this is what made it worth reading — when the
    // server refuses a move order it echoed a moment earlier and puts the
    // character back on the last position it validated. Ignoring it is what let
    // a refused walk look like a successful one in `bot/status` for three hours.
    // Coordinates here are plain units,
    // not the tenths of the 0xB021 source tail.
    for sync in syncs.read() {
        if sync.unique_id == state.unique_id && state.unique_id != 0 {
            walk.position = world_xz(sync.region, sync.x, sync.z);
            walk.region_y = sync.y;
            // A sync ends the walk: whatever we were interpolating toward, the
            // server has just stated the outcome.
            walk.destination = None;
            continue;
        }
        if let Some((entity, seen)) = world.entities.get_mut(&sync.unique_id) {
            entity.region = sync.region;
            entity.x = sync.x;
            entity.y = sync.y;
            entity.z = sync.z;
            *seen = now;
        }
    }

    // Advance the dead reckoning.
    if walk.speed <= 0.0 {
        walk.speed = refdata.run_speed(state.ref_id).unwrap_or(DEFAULT_RUN_SPEED);
    }
    let dt = time.delta_secs();
    if let Some(dest) = walk.destination {
        let (dx, dz) = (dest.0 - walk.position.0, dest.1 - walk.position.1);
        let dist = (dx * dx + dz * dz).sqrt();
        let step = walk.speed * dt;
        if dist <= step || dist == 0.0 {
            walk.position = dest;
            walk.destination = None;
        } else {
            walk.position = (
                walk.position.0 + dx / dist * step,
                walk.position.1 + dz / dist * step,
            );
        }
    }

    // Publish it in the wire's own terms.
    let (region, x, z) = region_local(walk.position);
    state.region = region;
    state.x = x;
    state.z = z;
    state.y = walk.region_y;
    state.moving = walk.destination.is_some();
    state.destination = walk.destination.map(|d| {
        let (r, x, z) = region_local(d);
        [r as f32, x, walk.dest_y, z]
    });
}

/// A region sector is 1920 units on both horizontal axes; the region id packs
/// the x sector in its low byte and the z sector in its high byte. Dungeon
/// regions (bit 0x8000) do not use this grid and are left to their own
/// coordinates — the bot never enters one on its own.
const REGION_SIZE: f32 = 1920.0;
/// Player run speed when characterdata has none for the character's ref id
/// (`Speed2` is 50 for every playable race row).
const DEFAULT_RUN_SPEED: f32 = 50.0;

fn world_xz(region: u16, x: f32, z: f32) -> (f32, f32) {
    let (xs, zs) = ((region & 0xFF) as f32, ((region >> 8) & 0xFF) as f32);
    (xs * REGION_SIZE + x, zs * REGION_SIZE + z)
}

fn region_local(world: (f32, f32)) -> (u16, f32, f32) {
    let xs = (world.0 / REGION_SIZE).floor().clamp(0.0, 255.0);
    let zs = (world.1 / REGION_SIZE).floor().clamp(0.0, 255.0);
    let region = ((zs as u16) << 8) | xs as u16;
    (
        region,
        world.0 - xs * REGION_SIZE,
        world.1 - zs * REGION_SIZE,
    )
}

/// Dead-reckoning state: where the character is, where it is walking, and how
/// fast. Kept out of [`BotState`] because it is bookkeeping, not a report.
#[derive(Resource, Default)]
pub struct BotWalk {
    position: (f32, f32),
    destination: Option<(f32, f32)>,
    region_y: f32,
    dest_y: f32,
    speed: f32,
}

/// Maintain the nearby-entity table from the spawn stream.
#[allow(clippy::too_many_arguments)]
fn track_entities(
    mut world: ResMut<BotWorld>,
    mut queue: ResMut<BotQueue>,
    refdata: Res<BotRefdata>,
    mut singles: MessageReader<SingleEntitySpawn>,
    mut batch_begins: MessageReader<GroupEntitySpawnBegin>,
    mut groups: MessageReader<GroupEntitySpawnData>,
    mut despawns: MessageReader<SingleEntityDespawn>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    let ingest = |raw: &bytes::Bytes, spawning: bool, count: u16, world: &mut BotWorld| {
        let parsed = parse_group_spawn(raw, spawning, count, &BotResolver(&refdata));
        debug!(
            "bot: ingest {} bytes -> {} spawns, {} despawns, {} unresolved",
            raw.len(),
            parsed.spawns.len(),
            parsed.despawns.len(),
            parsed.unresolved.len()
        );
        for e in parsed.spawns {
            if e.unique_id == 0 {
                continue;
            }
            debug!("bot: track uid {} ref {}", e.unique_id, e.ref_id);
            world.entities.insert(
                e.unique_id,
                (
                    NearbyEntity {
                        unique_id: e.unique_id,
                        ref_id: e.ref_id,
                        region: e.position.region,
                        x: e.position.x,
                        y: e.position.y,
                        z: e.position.z,
                        age_secs: 0.0,
                    },
                    now,
                ),
            );
        }
        for uid in parsed.despawns {
            world.entities.remove(&uid);
        }
    };
    for s in singles.read() {
        ingest(&s.raw, true, 1, &mut world);
    }
    for b in batch_begins.read() {
        // kind 1 = spawn, anything else = despawn (the payload is then a plain
        // uid list).
        world.batches.push_back((b.kind == 1, b.count));
    }
    for g in groups.read() {
        let (spawning, count) = world.batches.pop_front().unwrap_or((true, 1));
        ingest(&g.raw, spawning, count, &mut world);
    }
    for d in despawns.read() {
        if world.entities.remove(&d.unique_id).is_some() {
            queue.note(format!("despawn {}", d.unique_id));
        }
    }
    // Age the table so a stale entry is visible as stale rather than as truth.
    for (entity, seen) in world.entities.values_mut() {
        entity.age_secs = (now - *seen) as f32;
    }
}

/// Action acks and damage updates — the evidence that a fight is happening.
fn track_actions(
    mut state: ResMut<BotState>,
    mut queue: ResMut<BotQueue>,
    mut acks: MessageReader<ObjectActionResponse>,
    mut updates: MessageReader<ObjectActionUpdate>,
    mut selects: MessageReader<SelectEntityResponse>,
) {
    for ack in acks.read() {
        queue.note(format!("action ack: {ack:?}"));
    }
    for update in updates.read() {
        queue.note(format!("action update: {update:?}"));
    }
    for s in selects.read() {
        if s.result == 1 {
            state.target = Some(s.unique_id);
        }
        queue.note(format!(
            "select result={} uid={} tail={}B",
            s.result,
            s.unique_id,
            s.tail.len()
        ));
    }
}

/// The live counters a grind produces: experience, SP, gold, vitals and max
/// HP/MP. CHARACTER_DATA states them once at join; everything after that
/// arrives as its own update, and a bot that only reads the join packet reports
/// the login values forever.
fn track_progress(
    mut state: ResMut<BotState>,
    mut queue: ResMut<BotQueue>,
    mut experience: MessageReader<ReceiveExperience>,
    mut points: MessageReader<CharacterPointsUpdate>,
    mut bars: MessageReader<EntityBarsUpdate>,
    mut stats: MessageReader<CharacterStatsUpdate>,
) {
    for gain in experience.read() {
        // Signed: a kill grants, a death charges the penalty.
        state.exp = state.exp.saturating_add_signed(gain.experience);
        if gain.experience > 0 {
            state.exp_gained += gain.experience as u64;
        }
        state.skill_exp = state
            .skill_exp
            .saturating_add(gain.sp_exp.min(u32::MAX as u64) as u32);
        if gain.experience > 0 && gain.exp_origin != state.unique_id {
            state.kills += 1;
        }
        if let Some(total) = gain.stat_points() {
            // The trailing field is only present on a level-up, which is the
            // only trustworthy level-up signal: private servers run their own
            // exp curve, so no client-side threshold can derive it.
            state.stat_points = total;
            state.level += 1;
            // The within-level offset restarts; the server's real leftover
            // arrives with the next CHARACTER_DATA.
            state.exp = 0;
            queue.note(format!(
                "LEVEL UP — now level {} (stat points {total}), within-level exp restarted",
                state.level
            ));
        }
        queue.note(format!(
            "exp {:+} (sp-exp {:+}) from {}",
            gain.experience, gain.sp_exp, gain.exp_origin
        ));
    }
    for update in points.read() {
        match update {
            CharacterPointsUpdate::Gold { amount, .. } => {
                let before = state.gold;
                state.gold = *amount;
                queue.note(format!(
                    "gold {} ({:+})",
                    amount,
                    *amount as i64 - before as i64
                ));
            }
            CharacterPointsUpdate::Sp { amount, .. } => {
                queue.note(format!("skill points {amount}"));
            }
            CharacterPointsUpdate::StatPoints { amount } => {
                state.stat_points = *amount;
            }
            CharacterPointsUpdate::Berserk { .. } => {}
        }
    }
    for update in bars.read() {
        if update.unique_id != state.unique_id {
            continue;
        }
        if let Some(hp) = update.hp {
            if hp == 0 && state.hp > 0 {
                state.deaths += 1;
                queue.note("DIED".to_string());
            }
            state.hp = hp;
        }
        if let Some(mp) = update.mp {
            state.mp = mp;
        }
    }
    for update in stats.read() {
        state.max_hp = update.max_hp;
        state.max_mp = update.max_mp;
    }
}

/// Keep [`BotInventory`] level with what the server confirmed: 0xB034 results
/// and the 0x3038 equip broadcast for our own character. Runs after
/// `track_identity` so a CHARACTER_DATA and an operation arriving in the same
/// frame are applied in wire order (seed first, delta second) instead of the
/// seed wiping the delta.
fn track_inventory(
    state: Res<BotState>,
    mut inventory: ResMut<BotInventory>,
    refdata: Res<BotRefdata>,
    mut queue: ResMut<BotQueue>,
    mut ops: MessageReader<InventoryOperationResponse>,
    mut equips: MessageReader<EntityEquip>,
) {
    for res in ops.read() {
        if res.result != 1 {
            continue;
        }
        let Some(op) = &res.operation else { continue };
        if let Some(line) = inventory.apply_operation(op, &BotResolver(&refdata)) {
            queue.note(line);
        }
    }
    for equip in equips.read() {
        if equip.unique_id != state.unique_id {
            continue;
        }
        if let Some(line) = inventory.apply_equip(equip.slot, equip.ref_id) {
            queue.note(line);
        }
    }
}

/// Invites, party composition, trade and chat — the multi-session surface.
#[allow(clippy::too_many_arguments)]
fn track_social(
    mut state: ResMut<BotState>,
    mut queue: ResMut<BotQueue>,
    mut invites: MessageReader<GameInvite>,
    mut exchange_started: MessageReader<ExchangeStarted>,
    mut exchange_invites: MessageReader<ExchangeInviteResponse>,
    mut party_created: MessageReader<PartyCreateResponse>,
    mut party_invited: MessageReader<PartyInviteResponse>,
    mut exchange_done: MessageReader<ExchangeCompleted>,
    mut exchange_cancelled: MessageReader<ExchangeCanceled>,
    mut talk: MessageReader<TalkResponse>,
    mut inventory: MessageReader<InventoryOperationResponse>,
    mut gm: MessageReader<GmResponse>,
    mut lobby: MessageReader<CharacterSelectionActionResponse>,
    mut party_data: MessageReader<PartyData>,
    mut party_updates: MessageReader<PartyUpdate>,
    mut chat: MessageReader<ChatUpdate>,
) {
    for invite in invites.read() {
        if let GameInvite::Petition(p) = invite {
            state.pending_invite = Some(PendingInvite {
                petition: p.petition,
                unique_id: p.unique_id,
            });
            queue.note(format!(
                "INVITE petition type {} from uid {}",
                p.petition, p.unique_id
            ));
        }
    }
    for data in party_data.read() {
        state.party = data
            .members
            .iter()
            .map(|m| m.name.clone().unwrap_or_default())
            .collect::<Vec<_>>();
        queue.note(format!("party data: {:?}", state.party));
    }
    for update in party_updates.read() {
        // The roster is PartyData *plus* the deltas: a member who joins after
        // us only ever appears in a 0x3067 update, so a roster built from
        // 0x3065 alone is wrong the moment the party grows (the client's own
        // `PartyRoster::apply_update` does the same three cases).
        match update.update_type {
            1 => state.party.clear(),
            2 => {
                if let Some(joined) = update.joined.as_ref() {
                    let name = joined.name.clone().unwrap_or_default();
                    if !state.party.contains(&name) {
                        state.party.push(name);
                    }
                }
            }
            3 => {
                if let Some(member) = update.member_update.as_ref() {
                    let _ = member;
                }
            }
            _ => {}
        }
        queue.note(format!("party update: {update:?}"));
    }
    for line in chat.read() {
        queue.note(format!("chat: {line:?}"));
    }
    for started in exchange_started.read() {
        queue.note(format!("exchange started with {started:?}"));
    }
    // The acks are where a refused invite shows up: the reference server
    // answers 0xB081 with `02 0400` for an exchange invite between two
    // characters standing on the same spot, and never answers 0x7062 at all
    // unless the sender is already in a party.
    for ack in exchange_invites.read() {
        queue.note(format!("exchange invite ack: {ack:?}"));
    }
    for ack in party_created.read() {
        queue.note(format!("party create ack: {ack:?}"));
    }
    for ack in party_invited.read() {
        queue.note(format!("party invite ack: {ack:?}"));
    }
    for _ in exchange_done.read() {
        queue.note("exchange completed".to_string());
    }
    for _ in exchange_cancelled.read() {
        queue.note("exchange cancelled".to_string());
    }
    for res in talk.read() {
        queue.note(format!("talk response: {res:?}"));
    }
    for res in inventory.read() {
        queue.note(format!("inventory op: {res:?}"));
    }
    for res in gm.read() {
        queue.note(format!("gm response: {res:?}"));
    }
    for res in lobby.read() {
        queue.note(format!("lobby action: {res:?}"));
    }
}

// ---------------------------------------------------------------------------
// Command execution
// ---------------------------------------------------------------------------

/// The packed `type_id` a `use_item` command must carry, or why it may not be
/// sent at all.
///
/// Against a server this reads: `{"cmd":"use_item", "slot":22}` puts
/// `16 0000` on the wire — the right slot with a zero type — and the answer is
/// `0xB04C 02 03 00`. A packet that is certain to fail
/// is worse than a refused command, because it looks like a server defect. So
/// an explicit `type_id` wins (that is how a caller probes a type our itemdata
/// does not describe), otherwise the slot's ref id is looked up, and a lookup
/// that comes back empty stops the command with a reason in `bot/log`.
fn use_item_type_id(
    explicit: Option<u32>,
    slot: u8,
    inventory: &BotInventory,
    refdata: &BotRefdata,
) -> Result<u16, String> {
    if let Some(type_id) = explicit {
        return match type_id {
            0 => Err("type_id 0 is not a packed item type".to_string()),
            t if t <= u16::MAX as u32 => Ok(t as u16),
            t => Err(format!("type_id {t} does not fit the packet's u16")),
        };
    }
    let Some(item) = inventory.items.get(slot) else {
        return Err(format!(
            "slot {slot} holds no item the session has seen — pass type_id to send anyway"
        ));
    };
    match refdata.packed_type_id(item.ref_id) {
        Some(0) | None => Err(format!(
            "itemdata has no type ids for ref id {} in slot {slot}",
            item.ref_id
        )),
        Some(type_id) => Ok(type_id),
    }
}

/// Send the queued commands, at most one every [`DEFAULT_COMMAND_INTERVAL`].
/// The spacing is deliberate: the server rejects a second action while one is
/// running (`ObjectActionResponse` code 2), and a burst would make it
/// impossible to tell which command an error answered. One per *frame* is not
/// enough of a brake — the run loop ticks every 5 ms.
fn execute_commands(
    mut queue: ResMut<BotQueue>,
    mut state: ResMut<BotState>,
    mut pace: ResMut<BotPace>,
    inventory: Res<BotInventory>,
    refdata: Res<BotRefdata>,
    time: Res<Time>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let now = time.elapsed_secs_f64();
    if !pace.due(now) {
        return;
    }
    let Some(cmd) = queue.commands.pop_front() else {
        return;
    };
    // Marked whatever the outcome: a dropped command spent its slot too, which
    // is what keeps "command dropped: no agent connection yet" from filling the
    // whole rolling log 200 times a second before the session is up.
    pace.mark(now);
    let Ok(conn) = conn.single() else {
        queue.note("command dropped: no agent connection yet");
        return;
    };
    let name = cmd.get("cmd").and_then(Value::as_str).unwrap_or("");
    let uid = |key: &str| cmd.get(key).and_then(Value::as_u64).map(|v| v as u32);
    let num = |key: &str| cmd.get(key).and_then(Value::as_f64);

    // `raw` builds the frame itself: probing an opcode the `packets` crate has
    // no type for (GM sub-commands, unwired opcodes) is exactly what a live
    // session is for, and a typed-only API cannot express it.
    if name == "raw" {
        let opcode = cmd.get("opcode").and_then(Value::as_u64).unwrap_or(0) as u16;
        let body = cmd.get("body").and_then(Value::as_str).unwrap_or("");
        let bytes = match hex_bytes(body) {
            Some(bytes) => bytes,
            None => {
                queue.note(format!("raw rejected: '{body}' is not hex"));
                return;
            }
        };
        let frame = SilkroadFrame::Packet {
            count: 0,
            crc: 0,
            opcode,
            encrypted: 0,
            data: bytes::Bytes::from(bytes),
        };
        match conn.get_sender().send(frame) {
            Ok(()) => {
                state.commands_sent += 1;
                queue.note(format!("sent raw {opcode:#06x} {body}"));
            }
            Err(e) => queue.note(format!("send failed for raw {opcode:#06x}: {}", e.0)),
        }
        return;
    }

    let frame = match name {
        "move" => {
            let region = cmd
                .get("region")
                .and_then(Value::as_u64)
                .map(|v| v as u16)
                .unwrap_or(state.region);
            Some(
                Packet::from(MovementRequest {
                    region,
                    x: num("x").unwrap_or(state.x as f64) as i32,
                    y: num("y").unwrap_or(state.y as f64) as i32,
                    z: num("z").unwrap_or(state.z as f64) as i32,
                })
                .into(),
            )
        }
        "select" => {
            uid("uid").map(|unique_id| Packet::from(SelectEntityRequest { unique_id }).into())
        }
        "attack" => uid("uid").map(|unique_id| {
            Packet::from(ObjectActionRequest::Execute(ActionCommand::Attack(
                ActionTarget::Entity { unique_id },
            )))
            .into()
        }),
        "pickup" => uid("uid").map(|unique_id| {
            Packet::from(ObjectActionRequest::Execute(ActionCommand::Pickup(
                ActionTarget::Entity { unique_id },
            )))
            .into()
        }),
        "skill" => match (uid("skill_id"), uid("uid")) {
            (Some(ref_skill_id), Some(unique_id)) => Some(
                Packet::from(ObjectActionRequest::Execute(ActionCommand::CastSkill {
                    ref_skill_id,
                    target: ActionTarget::Entity { unique_id },
                }))
                .into(),
            ),
            _ => None,
        },
        "cancel" => Some(Packet::from(ObjectActionRequest::Cancel).into()),
        "chat" => {
            let message = cmd
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let kind = cmd.get("kind").and_then(Value::as_u64).unwrap_or(1) as u8;
            let target = cmd
                .get("target")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            Some(
                Packet::from(ChatRequest {
                    chat_type: kind,
                    chat_index: 0,
                    receiver: target,
                    message,
                })
                .into(),
            )
        }
        "party_create" => Some(
            Packet::from(PartyCreationRequest {
                // 0 = create an empty party (the invite follows); a uid here is
                // the "create with this player" form the UI uses.
                unique_id: uid("uid").unwrap_or(0),
                setup: cmd.get("setup").and_then(Value::as_u64).unwrap_or(0) as u8,
            })
            .into(),
        ),
        "party_invite" => {
            uid("uid").map(|unique_id| Packet::from(PartyInviteRequest { unique_id }).into())
        }
        "party_leave" => Some(Packet::from(PartyLeave).into()),
        "exchange_invite" => {
            uid("uid").map(|unique_id| Packet::from(ExchangeInviteRequest { unique_id }).into())
        }
        "exchange_confirm" => Some(Packet::from(ExchangeConfirmRequest).into()),
        "exchange_approve" => Some(Packet::from(ExchangeApproveRequest).into()),
        "exchange_exit" => Some(Packet::from(ExchangeExitRequest).into()),
        "talk" => match (uid("uid"), cmd.get("kind").and_then(Value::as_u64)) {
            (Some(unique_id), kind) => Some(
                Packet::from(TalkRequest {
                    unique_id,
                    // 2 = shop is the common one; the option index selects which
                    // of the NPC's talk options is opened.
                    talk_flag: kind.unwrap_or(2) as u8,
                })
                .into(),
            ),
            _ => None,
        },
        // Character creation is the one lobby action worth driving from a bot,
        // and its defaults are the original client's own: action 1, name, body
        // 1907, scale 0x22, items 3637/3638/3639/3632.
        //
        // An earlier default set was refused with 1027 for one reason: the
        // weapon. 3632 is ITEM_CH_SWORD_01_A_DEF, 71 is ITEM_CH_SWORD_01_A — the
        // creation registry only carries the `_DEF` starter rows, so a perfectly
        // valid non-`_DEF` item id is still not a *creation* item id. The
        // garments were `_DEF` rows already (CLOTHES where the original picks
        // HEAVY), which is why swapping the weapon alone is the fix.
        //
        // Note what the server then stores for such a character: 287/395/359/
        // 467/431/503 and 71 — it resolves the `_DEF` request rows into real
        // items itself. Sending the real ids directly is not the same thing.
        "create_char" => cmd.get("name").and_then(Value::as_str).map(|name| {
            Packet::from(CharacterSelectionActionRequest {
                action: CharacterSelectionAction::Create,
                name: None,
                create: Some(CharacterCreate {
                    name: name.to_string(),
                    ref_obj_id: uid("body").unwrap_or(1907),
                    scale: uid("scale").unwrap_or(0x22) as u8,
                    chest: uid("chest").unwrap_or(3637),
                    pants: uid("pants").unwrap_or(3638),
                    boots: uid("boots").unwrap_or(3639),
                    weapon: uid("weapon").unwrap_or(3632),
                }),
            })
            .into()
        }),
        "char_list" => Some(
            Packet::from(CharacterSelectionActionRequest {
                action: CharacterSelectionAction::List,
                name: None,
                create: None,
            })
            .into(),
        ),
        "close_talk" => {
            uid("uid").map(|unique_id| Packet::from(CloseTalkRequest { unique_id }).into())
        }
        "buy" => match (uid("npc"), uid("tab"), uid("slot")) {
            (Some(npc_unique_id), Some(tab), Some(slot)) => Some(
                Packet::from(InventoryOperationRequest::Buy {
                    tab: tab as u8,
                    slot: slot as u8,
                    quantity: uid("quantity").unwrap_or(1) as u16,
                    npc_unique_id,
                })
                .into(),
            ),
            _ => None,
        },
        "sell" => match (uid("npc"), uid("slot")) {
            (Some(npc_unique_id), Some(slot)) => Some(
                Packet::from(InventoryOperationRequest::Sell {
                    slot: slot as u8,
                    quantity: uid("quantity").unwrap_or(1) as u16,
                    npc_unique_id,
                })
                .into(),
            ),
            _ => None,
        },
        "item_move" => match (uid("source"), uid("target")) {
            (Some(source), Some(target)) => Some(
                Packet::from(InventoryOperationRequest::Move {
                    source: source as u8,
                    target: target as u8,
                    amount: uid("amount").unwrap_or(1) as u16,
                })
                .into(),
            ),
            _ => None,
        },
        // The wire slot is the +0x0D-biased one the packet documents; the
        // type_id is the item's packed itemdata type, taken from what the slot
        // holds unless the caller names one ([`use_item_type_id`]).
        "use_item" => match uid("slot") {
            None => None,
            Some(slot) => {
                let slot = slot as u8;
                match use_item_type_id(uid("type_id"), slot, &inventory, &refdata) {
                    Ok(type_id) => {
                        Some(Packet::from(ItemUseRequest::Simple { slot, type_id }).into())
                    }
                    Err(why) => {
                        queue.note(format!("use_item rejected: {why}"));
                        return;
                    }
                }
            }
        },
        "invite_accept" => {
            state.pending_invite = None;
            Some(Packet::from(GameInvite::Response(InviteResponse::Accept)).into())
        }
        "invite_decline" => {
            state.pending_invite = None;
            Some(Packet::from(GameInvite::Response(InviteResponse::Decline)).into())
        }
        "logout" => Some(
            Packet::from(LogoutRequest {
                mode: cmd.get("mode").and_then(Value::as_u64).unwrap_or(1) as u8,
            })
            .into(),
        ),
        other => {
            queue.note(format!("unknown command '{other}'"));
            None
        }
    };

    let Some(frame) = frame else {
        queue.note(format!("command '{name}' rejected: missing arguments"));
        return;
    };
    match conn.get_sender().send(frame) {
        Ok(()) => {
            state.commands_sent += 1;
            queue.note(format!("sent {name} {cmd}"));
        }
        Err(e) => queue.note(format!("send failed for {name}: {}", e.0)),
    }
}

// ---------------------------------------------------------------------------
// Control surface
// ---------------------------------------------------------------------------

/// Parse an even-length hex string (spaces allowed) into bytes.
fn hex_bytes(text: &str) -> Option<Vec<u8>> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !clean.len().is_multiple_of(2) {
        return None;
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok())
        .collect()
}

fn brp_status(In(_params): In<Option<Value>>, state: Res<BotState>) -> BrpResult {
    Ok(serde_json::to_value(&*state).unwrap_or(Value::Null))
}

/// The nearby-entity table, enriched with what characterdata/itemdata say about
/// each ref id — the driving script picks targets by kind and level, and doing
/// the lookup here keeps the two tables in one place.
fn brp_entities(
    In(_params): In<Option<Value>>,
    world: Res<BotWorld>,
    refdata: Res<BotRefdata>,
) -> BrpResult {
    let mut list: Vec<&NearbyEntity> = world.entities.values().map(|(e, _)| e).collect();
    list.sort_by_key(|e| e.unique_id);
    let enriched: Vec<Value> = list
        .into_iter()
        .map(|e| {
            let (kind, code, level, max_hp) = refdata.describe(e.ref_id);
            json!({
                "unique_id": e.unique_id,
                "ref_id": e.ref_id,
                "kind": kind,
                "code": code,
                "level": level,
                "max_hp": max_hp,
                "region": e.region,
                "x": e.x,
                "y": e.y,
                "z": e.z,
                "age_secs": e.age_secs,
            })
        })
        .collect();
    Ok(Value::Array(enriched))
}

/// The inventory, with each ref id described from itemdata so the caller can
/// tell a potion from a sword without a second table.
///
/// `defect` is the important field: while it is non-null the item list is
/// *incomplete for a known reason* and no conclusion ("the bot carries no
/// weapon") may be drawn from its absences.
fn brp_inventory(
    In(_params): In<Option<Value>>,
    inventory: Res<BotInventory>,
    refdata: Res<BotRefdata>,
) -> BrpResult {
    Ok(inventory_json(&inventory, &refdata))
}

/// The body of `bot/inventory`, as a plain function so a test can assert on the
/// answer the driving script actually reads.
fn inventory_json(inventory: &BotInventory, refdata: &BotRefdata) -> Value {
    let items: Vec<Value> = inventory
        .items
        .slots
        .iter()
        .flatten()
        .map(|item| {
            let (kind, code, _, _) = refdata.describe(item.ref_id);
            let stack = match &item.data {
                ItemTypeData::Expendable { stack_count, .. } => *stack_count,
                _ => 1,
            };
            json!({
                "slot": item.slot,
                "ref_id": item.ref_id,
                "stack": stack,
                "equipped": item.slot < crate::plugins::net::inventory::EQUIP_SLOT_COUNT,
                "kind": kind,
                "code": code,
                "type_id": refdata.packed_type_id(item.ref_id),
            })
        })
        .collect();
    // An un-seeded mirror is empty because nothing has been read yet, which is
    // just as unreportable as an unreadable one: say so instead of answering
    // with a bare empty list.
    let defect = inventory.defect.clone().or_else(|| {
        (!inventory.seeded)
            .then(|| "no CHARACTER_DATA has been parsed yet — nothing has been read".to_string())
    });
    json!({
        "size": inventory.items.size(),
        "seeded": inventory.seeded,
        "updates": inventory.updates,
        "defect": defect,
        "items": items,
    })
}

fn brp_log(In(params): In<Option<Value>>, mut queue: ResMut<BotQueue>) -> BrpResult {
    let drain = params
        .as_ref()
        .and_then(|p| p.get("drain"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let lines: Vec<String> = queue.log.iter().cloned().collect();
    if drain {
        queue.log.clear();
    }
    Ok(json!({ "lines": lines }))
}

// ---------------------------------------------------------------------------
// Raw packet path (`bot/send`)
// ---------------------------------------------------------------------------

/// Build the outbound frame for `bot/send`.
///
/// Idea: the frame is assembled by hand rather than through a `packets` type on
/// purpose — the whole point of the raw path is to put an opcode on the wire for
/// which no codec exists yet (an unwired C→S opcode, a GM sub-command, a
/// deliberately malformed body to see the server's error code). `count`/`crc`
/// stay zero because `SilkroadFrame::serialize` stamps them from the session's
/// security context, exactly as for a typed packet, so a raw frame is
/// indistinguishable on the wire from one the client built itself.
fn raw_frame(opcode: u16, payload: Vec<u8>) -> SilkroadFrame {
    SilkroadFrame::Packet {
        count: 0,
        crc: 0,
        opcode,
        encrypted: 0,
        data: bytes::Bytes::from(payload),
    }
}

/// Hex of a byte slice, in the same lower-case, unseparated form the packet log
/// uses — so a `bot/send` answer can be grepped against a log line directly.
fn hex_of(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// An opcode as JSON: either a number (`30032`) or a string (`"0x7150"`,
/// `"7150"`). Hex without the prefix is accepted because that is how the log
/// file names read, and a driving script pasting `0x7150` should not have to
/// convert.
fn parse_opcode(value: Option<&Value>) -> Result<u16, String> {
    match value {
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|v| *v <= u16::MAX as u64)
            .map(|v| v as u16)
            .ok_or_else(|| format!("opcode {n} is not a u16")),
        Some(Value::String(s)) => {
            let text = s.trim();
            let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))
            {
                Some(rest) => (rest, 16),
                // A bare string is hex too: `"7150"` is the opcode as written
                // everywhere in this repo, not decimal 7150.
                None => (text, 16),
            };
            u16::from_str_radix(digits, radix)
                .map_err(|e| format!("opcode '{text}' is not a u16 in hex: {e}"))
        }
        Some(other) => Err(format!("opcode must be a number or string, got {other}")),
        None => Err("missing 'opcode'".to_string()),
    }
}

fn invalid_params(message: impl Into<String>) -> BrpError {
    BrpError {
        code: error_codes::INVALID_PARAMS,
        message: message.into(),
        data: None,
    }
}

/// **One call sends exactly one frame.** No repetition, no retry, no loop: this
/// is a probe against the operator's *own* server, and a method that resent on
/// error would turn a typo into a flood. A failed send is reported, never
/// retried — the caller decides whether to try again.
///
/// `{"opcode": "0x7150" | 30032, "payload": "hex" (optional, default empty),
/// "massive": bool (optional)}`. The answer states the opcode, the body length
/// and the body hex in packet-log form, so the caller can prove what was on the
/// wire and find the same bytes again in the c2s log, which `send_packets`
/// writes for this frame like any other.
///
/// Sends synchronously from the BRP main-world handler rather than through
/// [`BotQueue`]: a queued command answers "queued: 1" and the caller learns
/// nothing about the bytes. The frame still goes out via the connection's
/// outbound channel, i.e. through `send_packets` — same encryption policy, same
/// CRC/sequence stamping, same c2s log entry as every typed packet.
fn brp_send(
    In(params): In<Option<Value>>,
    mut state: ResMut<BotState>,
    mut queue: ResMut<BotQueue>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) -> BrpResult {
    let params = params.ok_or_else(|| invalid_params("no params: expected {\"opcode\": ...}"))?;
    let opcode = parse_opcode(params.get("opcode")).map_err(invalid_params)?;
    let payload_hex = match params.get("payload") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(invalid_params(format!(
                "payload must be a hex string, got {other}"
            )));
        }
    };
    let payload = hex_bytes(&payload_hex).ok_or_else(|| {
        invalid_params(format!(
            "payload '{payload_hex}' is not hex (even number of hex digits, whitespace allowed)"
        ))
    })?;
    // Rejected rather than built: `SilkroadFrame::serialize` is
    // `todo!("massive packet handling")` for `MassiveHeader`
    // (`client/src/net/frame.rs`), so queueing a massive frame would panic
    // `send_packets` and kill the session — the opposite of a probe tool.
    if params
        .get("massive")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(invalid_params(
            "massive frames cannot be sent yet: SilkroadFrame::serialize has \
             todo!(\"massive packet handling\") for MassiveHeader (client/src/net/frame.rs). \
             Send the frames individually.",
        ));
    }

    let conn = conn
        .single()
        .map_err(|_| invalid_params("no agent connection yet — the bot is not in world"))?;
    let hex = hex_of(&payload);
    conn.get_sender()
        .send(raw_frame(opcode, payload.clone()))
        .map_err(|e| BrpError {
            code: error_codes::INTERNAL_ERROR,
            message: format!("send failed for {opcode:#06x}: {}", e.0),
            data: None,
        })?;
    state.commands_sent += 1;
    queue.note(format!(
        "bot/send {opcode:#06x} {} byte(s) {hex}",
        payload.len()
    ));
    Ok(json!({
        "sent": true,
        "opcode": format!("{opcode:#06x}"),
        "opcode_dec": opcode,
        "len": payload.len(),
        "hex": hex,
        // Names the file this frame lands in, honouring PACKET_DUMP_DIR.
        "dump": dump_root().join("c2s").join(format!("{opcode:#06x}.log")).display().to_string(),
    }))
}

fn brp_command(In(params): In<Option<Value>>, mut queue: ResMut<BotQueue>) -> BrpResult {
    let Some(params) = params else {
        return Ok(json!({ "queued": 0, "error": "no params" }));
    };
    let items = match params {
        Value::Array(items) => items,
        one => vec![one],
    };
    let queued = items.len();
    for item in items {
        queue.commands.push_back(item);
    }
    Ok(json!({ "queued": queued }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::security::SilkroadSecurityState;
    use std::sync::{Arc, RwLock};

    /// Verbatim CHARACTER_DATA (0x3013) body of a bot test character, taken from
    /// the join of a session that answered `bot/inventory` with
    /// `{"size":109,"items":[]}`. 1202 bytes.
    const CHARACTER_DATA_FIXTURE: &str = concat!(
        "1ad203de8b0700002015154ae60300000000002e010000c24801000000000014d101003c000500000000520200005202",
        "0000000000000000000000006d2f00000000000f0500000000000000000000002c000000000100020001000000007b05",
        "00000000000000000000002c00000000010002000200000000570500000000000000000000002c000000000100020003",
        "00000000c30500000000000000000000002c000000000100020004000000009f0500000000000000000000002c000000",
        "00010002000500000000e70500000000000000000000002c000000000100020006000000007300000005000000000000",
        "00000000000000010002000700000000fb0000000000000000000000002e000000000100020009000000002b07000000",
        "00000000000000000000000000010002000a000000004f0700000000000000000000000000000000010002000b000000",
        "00070700000000000000000000000000000000010002000c000000000707000000000000000000000000000000000100",
        "02000d00000000855f0000e7030e00000000895f0000e8030f00000000815f000014001000000000805f000014001100",
        "0000007e5f0000140012000000005e2e0000018929221b0000000095010000043b00000003000000800000000a000000",
        "2f0000000200000041000000000000000100020013000000005e5f000000000000000000000000000000000100020014",
        "000000000b0f0000030015000000000204000000642c650f000000003e000000000100020016000000005f0e00000100",
        "170000000059080000020018000000003800000003001900000000031a000001001a000000009608000007001b000000",
        "001400000001001c00000000eb19000001001d00000000af19000001001e000000003e00000078001f00000000060000",
        "00010020000000003d00000002002100000000631c00000100220000000079000000020810db8001000000ef01000002",
        "30000000040000004100000000000000010002002300000000620e0000010024000000008805000002a1a50006000000",
        "0031010000040b00000001000000500000003c00000037000000010000004100000000000000010002002500000000e3",
        "18000001002600000000b004000000840093010000000043000000000100020027000000002c0100000012b4c6000000",
        "00004a000000020b000000030000003000000005000000010002002800000000480300000180992014000000000e0100",
        "000237000000020000004100000000000000010002002a000000008f28000013002b00000000001c000001002c000000",
        "003700000001002d000000005e05000000270c291e000000003500000000010002002e000000000400000001002f0000",
        "0000d40f00000800000000000000004f000000000100020064000000008f280000010005000001010100000001020100",
        "000001030100000001110100000001120100000001130100000001140100000002000201000100000000000000000082",
        "9b0200a860ca189c44923e5d3fa7fabd445aa10001005aa100000000cdcc8c4100005c420000c8420007007465737462",
        "6f7400000001000000000000000000000000000000ff5708a00400000000060000000107000000000000000000010001",
        "0000",
    );

    /// The itemdata classes of every ref id that body contains, plus ref id 79
    /// (the sword picked up later). They come from `BotResolver` over the real
    /// `itemdata.txt` of `Media.pk2` (12061 rows): the values are itemdata's
    /// TID2/TID3/TID4 columns, not a guess — a wrong class picks the wrong record
    /// width and the section stops.
    /// They are inlined because `Media.pk2` is user-supplied and not in CI.
    const ITEM_CLASSES: &[(u32, ItemClass)] = &[
        (4, ItemClass::Expendable { tid3: 1, tid4: 1 }),
        (6, ItemClass::Expendable { tid3: 1, tid4: 1 }),
        (20, ItemClass::Expendable { tid3: 1, tid4: 3 }),
        (55, ItemClass::Expendable { tid3: 2, tid4: 6 }),
        (56, ItemClass::Expendable { tid3: 2, tid4: 6 }),
        (61, ItemClass::Expendable { tid3: 3, tid4: 1 }),
        (62, ItemClass::Expendable { tid3: 4, tid4: 1 }),
        (79, ItemClass::Equipment),
        (115, ItemClass::Equipment),
        (121, ItemClass::Equipment),
        (251, ItemClass::Equipment),
        (300, ItemClass::Equipment),
        (840, ItemClass::Equipment),
        (1026, ItemClass::Equipment),
        (1200, ItemClass::Equipment),
        (1295, ItemClass::Equipment),
        (1367, ItemClass::Equipment),
        (1374, ItemClass::Equipment),
        (1403, ItemClass::Equipment),
        (1416, ItemClass::Equipment),
        (1439, ItemClass::Equipment),
        (1475, ItemClass::Equipment),
        (1511, ItemClass::Equipment),
        (1799, ItemClass::Equipment),
        (1835, ItemClass::Equipment),
        (1871, ItemClass::Equipment),
        (2137, ItemClass::Expendable { tid3: 3, tid4: 2 }),
        (2198, ItemClass::Expendable { tid3: 3, tid4: 1 }),
        (3679, ItemClass::Expendable { tid3: 10, tid4: 1 }),
        (3682, ItemClass::Expendable { tid3: 10, tid4: 1 }),
        (3851, ItemClass::Expendable { tid3: 3, tid4: 5 }),
        (4052, ItemClass::Equipment),
        (6371, ItemClass::Expendable { tid3: 11, tid4: 3 }),
        (6575, ItemClass::Expendable { tid3: 11, tid4: 3 }),
        (6635, ItemClass::Expendable { tid3: 11, tid4: 3 }),
        (6659, ItemClass::Expendable { tid3: 11, tid4: 3 }),
        (7168, ItemClass::Expendable { tid3: 11, tid4: 4 }),
        (7267, ItemClass::Expendable { tid3: 11, tid4: 4 }),
        (10383, ItemClass::Expendable { tid3: 4, tid4: 2 }),
        (11870, ItemClass::Equipment),
        (24414, ItemClass::Equipment),
        (24446, ItemClass::Expendable { tid3: 13, tid4: 6 }),
        (24448, ItemClass::Expendable { tid3: 3, tid4: 1 }),
        (24449, ItemClass::Expendable { tid3: 3, tid4: 3 }),
        (24453, ItemClass::Expendable { tid3: 1, tid4: 1 }),
        (24457, ItemClass::Expendable { tid3: 1, tid4: 2 }),
    ];

    struct FixtureResolver;

    impl ItemClassResolver for FixtureResolver {
        fn item_class(&self, ref_id: u32) -> ItemClass {
            ITEM_CLASSES
                .iter()
                .find(|(id, _)| *id == ref_id)
                .map(|(_, class)| *class)
                .unwrap_or(ItemClass::Unknown)
        }
    }

    /// No itemdata at all — what the bot runs with when `assets/Media.pk2` does
    /// not exist and `SRO_PK2_PATH` is unset.
    struct NoItemdata;

    impl ItemClassResolver for NoItemdata {
        fn item_class(&self, _ref_id: u32) -> ItemClass {
            ItemClass::Unknown
        }
    }

    /// The raw path must be byte-identical to the typed one.
    ///
    /// `bot/send {"opcode":"0x704b","payload":"ba000000"}` is the *same* frame
    /// the client sends when it closes an NPC talk with a typed
    /// [`CloseTalkRequest`]: 0x704B is `unique_id u32` little-endian, and
    /// `ba000000` is a real body, unique id 0xBA. Comparing the
    /// *serialized* frames (not just the bodies) is what proves the raw path
    /// gets the header, the security bytes and the CRC from the session exactly
    /// like a typed packet — a raw frame the server would reject is a probe
    /// tool that lies. No network: both frames are serialized against a fresh
    /// security state.
    #[test]
    fn a_raw_frame_is_byte_identical_to_the_typed_packet() {
        let body = hex_bytes("ba000000").expect("hex fixture");
        assert_eq!(body, vec![0xBA, 0x00, 0x00, 0x00]);
        assert_eq!(parse_opcode(Some(&json!("0x704b"))), Ok(0x704B));
        assert_eq!(parse_opcode(Some(&json!("704b"))), Ok(0x704B));
        assert_eq!(parse_opcode(Some(&json!(0x704B))), Ok(0x704B));
        assert!(parse_opcode(Some(&json!(0x1704B))).is_err());
        assert!(parse_opcode(None).is_err());
        assert!(hex_bytes("ba0000f").is_none(), "odd digit count is not hex");
        assert!(hex_bytes("zz").is_none(), "non-hex is not hex");

        let raw = raw_frame(0x704B, body.clone())
            .serialize(Arc::new(RwLock::new(SilkroadSecurityState::new())))
            .expect("serialize raw");
        let typed = SilkroadFrame::from(Packet::from(CloseTalkRequest { unique_id: 0xBA }).into())
            .serialize(Arc::new(RwLock::new(SilkroadSecurityState::new())))
            .expect("serialize typed");

        assert_eq!(hex_of(&raw), hex_of(&typed));
        // And the frame itself: `0400` length, `4b70` opcode, then the two
        // security bytes and the body. `00c0` is what a *fresh* security state
        // stamps (`Sequence::from(0).next()` and `CRC::from(0)` over the six
        // header bytes plus body); a live session has a negotiated seed and
        // stamps different ones. Pinning it keeps the
        // header layout (length | opcode | count | crc | body) under test.
        assert_eq!(hex_of(&raw), "04004b7000c0ba000000");
        // The hex the RPC answer reports is the packet log's own form, so a
        // caller can grep the c2s log for it.
        assert_eq!(hex_of(&body), "ba000000");
    }

    fn unhex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    fn op(hex: &str) -> InventoryOperationResult {
        InventoryOperationResponse::try_from(bytes::Bytes::from(unhex(hex)))
            .unwrap()
            .operation
            .unwrap()
    }

    fn seeded_inventory() -> BotInventory {
        let raw = unhex(CHARACTER_DATA_FIXTURE);
        let info = parse_character_info(&raw, None, &FixtureResolver);
        assert!(
            info.fully_parsed,
            "fixture body must parse to its last byte"
        );
        let mut inventory = BotInventory::default();
        inventory.seed(&info, ITEM_CLASSES.len());
        inventory
    }

    /// The regression the whole resource exists for: replay real frames of a
    /// session and require the RPC to *name* the sword the server confirmed.
    /// Asserting "empty stays empty" would have passed on the
    /// broken code, so every step here asserts a positive slot content.
    #[test]
    fn the_rpc_reports_the_items_the_server_confirmed() {
        let mut inventory = seeded_inventory();

        // Seed: the body itself. Slot 6 is the equipped blade (ref 115,
        // ITEM_CH_BLADE_03_C), and the bag holds 47 records in 109 slots.
        assert_eq!(inventory.defect, None);
        assert_eq!(inventory.items.get(6).map(|i| i.ref_id), Some(115));
        assert_eq!(inventory.items.slots.iter().flatten().count(), 47);
        assert_eq!(inventory.items.size(), 109);
        assert!(inventory.items.get(41).is_none());

        // 0xB034 op 6: the sword (ref 79, ITEM_CH_SWORD_03_C) is picked up
        // into slot 41 (0x29).
        let line = inventory.apply_operation(
            &op("010629000000004f0000000000000000000000004b0000000001000200"),
            &FixtureResolver,
        );
        assert!(line.is_some());
        assert_eq!(inventory.items.get(41).map(|i| i.ref_id), Some(79));

        // 0x3038 equip broadcast, then the move 41 -> 6. The broadcast
        // arrives *first* and disagrees with our mirror, so it
        // relocates; the move behind it is then a no-op on an equal slot.
        let equip =
            EntityEquip::try_from(bytes::Bytes::from(unhex("829b0200064f00000000"))).unwrap();
        assert_eq!(equip.unique_id, 170882);
        inventory.apply_equip(equip.slot, equip.ref_id);
        assert_eq!(inventory.items.get(6).map(|i| i.ref_id), Some(79));
        assert_eq!(inventory.items.get(41).map(|i| i.ref_id), Some(115));
        inventory.apply_operation(&op("01002906010000"), &FixtureResolver);
        assert_eq!(inventory.items.get(6).map(|i| i.ref_id), Some(79));

        // Unequip: move slot 6 -> slot 48 (0x30).
        inventory.apply_operation(&op("01000630010000"), &FixtureResolver);
        assert_eq!(inventory.items.get(48).map(|i| i.ref_id), Some(79));
        assert!(inventory.items.get(6).is_none());

        // Re-equip: move slot 48 -> slot 6.
        inventory.apply_operation(&op("01003006010000"), &FixtureResolver);
        assert_eq!(inventory.items.get(6).map(|i| i.ref_id), Some(79));

        // …and that is what `bot/inventory` must now say.
        let answer = inventory_json(&inventory, &BotRefdata::default());
        assert_eq!(answer["defect"], Value::Null);
        assert_eq!(answer["size"], 109);
        let items = answer["items"].as_array().unwrap();
        let sword = items
            .iter()
            .find(|i| i["slot"] == 6)
            .expect("slot 6 must appear in the answer");
        assert_eq!(sword["ref_id"], 79);
        assert_eq!(sword["equipped"], true);
        assert_eq!(items.len(), 48);
    }

    /// Without itemdata no record can be classified, so the item section is
    /// unreadable — and the answer has to SAY so. Otherwise
    /// `{"size":109,"items":[]}` gets read as "the character carries nothing".
    #[test]
    fn an_unreadable_item_section_is_not_reported_as_an_empty_bag() {
        let raw = unhex(CHARACTER_DATA_FIXTURE);
        let info = parse_character_info(&raw, None, &NoItemdata);
        // Positive control on the same read path: with itemdata the same bytes
        // yield 47 items, so the emptiness below is the resolver's doing.
        assert_eq!(
            parse_character_info(&raw, None, &FixtureResolver)
                .inventory
                .map(|i| i.len()),
            Some(47)
        );
        assert_eq!(info.inventory.as_ref().map(Vec::len), Some(0));

        // Before any body arrives the answer must already refuse to look like
        // an empty bag.
        let fresh = inventory_json(&BotInventory::default(), &BotRefdata::default());
        assert!(fresh["defect"]
            .as_str()
            .unwrap()
            .contains("nothing has been read"));

        let mut inventory = BotInventory::default();
        inventory.seed(&info, 0);
        let answer = inventory_json(&inventory, &BotRefdata::default());
        assert_eq!(answer["items"].as_array().unwrap().len(), 0);
        assert!(
            answer["defect"].as_str().unwrap().contains("itemdata"),
            "an unreadable inventory must name its defect, got {answer}"
        );
    }

    /// Column 1 is the ref id in both master tables. A row whose column 1 is
    /// not a number is a header or a comment line, and keying on it would
    /// panic the loader — characterdata files do carry such lines.
    #[test]
    fn a_row_is_keyed_on_its_ref_id_column() {
        let row: Vec<String> = "1\t1907\tCHAR_CH_WOMAN_SCHOLAR\txxx"
            .split('\t')
            .map(String::from)
            .collect();
        assert_eq!(keyed_row(row).map(|(id, _)| id), Some(1907));

        let header: Vec<String> = "Service\tID\tCodeName"
            .split('\t')
            .map(String::from)
            .collect();
        assert!(keyed_row(header).is_none());
    }

    /// The command surface accepts one object or a batch, because a driving
    /// script wants to queue "select, attack" without two round trips.
    #[test]
    fn a_command_batch_queues_every_entry() {
        let mut queue = BotQueue::default();
        let batch = json!([{ "cmd": "select", "uid": 1 }, { "cmd": "attack", "uid": 1 }]);
        let items = match batch {
            Value::Array(items) => items,
            one => vec![one],
        };
        for item in items {
            queue.commands.push_back(item);
        }
        assert_eq!(queue.commands.len(), 2);
    }

    /// The border case: the server echoes a cross-border order (`0xB021`,
    /// destination region 25000 / (907, 100), source region 24744 / z = 1398.0)
    /// and then refuses it with a `0xB023`
    /// (`20a80200a86077c062447a6126be2c83c844ff3f` = region 24744,
    /// 907.01 / 1604.1). Without reading 0xB023 the bot dead-reckons on to the
    /// destination and reports a position the server never granted.
    #[test]
    fn a_refused_move_ends_where_the_server_says_it_ends() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<MovementResponse>()
            .add_message::<MovementPositionUpdate>()
            .init_resource::<BotState>()
            .init_resource::<BotWalk>()
            .init_resource::<BotWorld>()
            .init_resource::<BotRefdata>()
            .add_systems(Update, track_position);
        app.world_mut().resource_mut::<BotState>().unique_id = 174112;

        // The order and its echo, alone: the bot believes it is walking north.
        app.world_mut().write_message(MovementResponse {
            unique_id: 174112,
            has_destination: true,
            region: 25000,
            x: 907,
            y: 0,
            z: 100,
            angle: 0,
        });
        app.update();
        assert!(
            app.world().resource::<BotState>().moving,
            "the echo starts a walk"
        );

        // The refusal.
        app.world_mut().write_message(MovementPositionUpdate {
            unique_id: 174112,
            region: 24744,
            x: 907.01,
            y: -0.157,
            z: 1604.1,
            heading: 0x3fff,
        });
        app.update();
        let state = app.world().resource::<BotState>();
        assert_eq!(state.region, 24744, "the snap keeps us in our own region");
        assert!(
            (state.z - 1604.1).abs() < 1.0,
            "the bot must report the position the server validated, got {}",
            state.z
        );
        assert!(!state.moving, "a snap ends the walk");
        assert!(state.destination.is_none());
    }

    /// `bot/command {"cmd":"use_item","slot":22}` puts `16 0000` on the wire —
    /// right slot, zero type — and the server answers `0xB04C 02 03 00`. That
    /// happens when the refdata lookup comes back empty and the command is sent
    /// anyway. A
    /// packet that is certain to fail looks like a server fault, so an
    /// unresolvable type must stop the command instead.
    #[test]
    fn use_item_never_puts_a_zero_type_on_the_wire() {
        let inventory = seeded_inventory();
        let (slot, ref_id) = inventory
            .items
            .slots
            .iter()
            .flatten()
            .map(|item| (item.slot, item.ref_id))
            .next()
            .expect("the fixture body carries items");

        // No itemdata at all — the state that produces `16 0000`.
        let blind = BotRefdata::default();
        let why = use_item_type_id(None, slot, &inventory, &blind)
            .expect_err("an unresolvable type must not be sent");
        assert!(
            why.contains(&ref_id.to_string()),
            "the rejection has to name the ref id that could not be resolved, got {why}"
        );
        // The same garbage by the other route: an explicit zero.
        assert!(use_item_type_id(Some(0), slot, &inventory, &blind).is_err());
        // And a slot the session has never seen an item in.
        let free = (0..inventory.items.size())
            .find(|s| inventory.items.get(*s).is_none())
            .expect("the fixture leaves slots empty");
        assert!(use_item_type_id(None, free, &inventory, &blind).is_err());

        // Positive control: with itemdata the type resolves, and it is the
        // packing the HUD's own use path applies. An HP potion is TID
        // `3,3,1,1`, i.e. (3<<2)|(3<<5)|(1<<7)|(1<<11) = 2284.
        let mut known = BotRefdata::default();
        let mut cells = vec!["0".to_string(); 20];
        cells[9] = "3".to_string();
        cells[10] = "3".to_string();
        cells[11] = "1".to_string();
        cells[12] = "1".to_string();
        known.items.insert(ref_id as i32, ItemDataRow(cells));
        assert_eq!(use_item_type_id(None, slot, &inventory, &known), Ok(2284));
        // An explicit type still wins, so an unknown class can be probed.
        assert_eq!(
            use_item_type_id(Some(0x2C), slot, &inventory, &blind),
            Ok(0x2C)
        );
    }

    /// The brake the "one command per frame" comment promised but did not have:
    /// the bot's run loop ticks every 5 ms, so per-frame draining is 200
    /// commands a second. Three queued commands must take three *intervals*.
    #[test]
    fn the_command_queue_is_paced_not_merely_one_per_frame() {
        let mut app = App::new();
        app.init_resource::<BotQueue>()
            .init_resource::<BotState>()
            .init_resource::<BotPace>()
            .init_resource::<BotInventory>()
            .init_resource::<BotRefdata>()
            .init_resource::<Time>()
            .add_systems(Update, execute_commands);
        app.world_mut()
            .resource_mut::<BotQueue>()
            .commands
            .extend([json!({"cmd": "cancel"}), json!({"cmd": "cancel"})]);

        // Three frames at the run loop's own 5 ms tick (`run_bot`).
        for _ in 0..3 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(5));
            app.update();
        }
        assert_eq!(
            app.world().resource::<BotQueue>().commands.len(),
            1,
            "15 ms of frames may spend one command, not the whole queue"
        );

        // One interval later the next one may go.
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f64(DEFAULT_COMMAND_INTERVAL));
        app.update();
        assert_eq!(
            app.world().resource::<BotQueue>().commands.len(),
            0,
            "after one interval the next command leaves"
        );

        // The default is the game data's own floor, not a round number: the
        // shortest castable player action in v1.188 skilldata is 210 ms.
        const { assert!(DEFAULT_COMMAND_INTERVAL >= 0.210) };
    }

    /// A control method nobody documents is a method nobody can call, and
    /// AGENTS.md asks for the README to follow behavior. This is what keeps the
    /// registration in `run_bot` and the README section in step.
    #[test]
    fn the_readme_documents_the_control_surface() {
        let readme = include_str!("../../README.md");
        for method in BRP_METHODS {
            assert!(readme.contains(method), "README does not document {method}");
        }
        for var in [
            "BOT=1",
            "BOT_ACCOUNT",
            "BOT_PASSWORD",
            "BOT_CHAR",
            "BOT_PORT",
            "BOT_COMMAND_INTERVAL",
        ] {
            assert!(readme.contains(var), "README does not document {var}");
        }
        assert!(
            readme.contains("127.0.0.1"),
            "README must state that the control port is localhost-only"
        );
    }
}

//! The quest journal's HUD half: who fills [`QuestJournal`], who keeps it
//! current, and which of its entries the on-screen tracker shows.
//!
//! Idea, and why it lives here rather than next to the model: the model
//! (`plugins::net::quest`) must stay system-free, because everything under
//! `plugins::net::**` runs in the headless netcheck harness (AGENTS.md). So
//! the model parses, decorates and folds, and this file is what gives it
//! **reception -> state -> visible effect**, with no send at all (there is no
//! evidenced client->server quest builder to call — see the module header of
//! [`super`]).
//!
//! * **Reception** is the login record. `CHARACTER_DATA`'s active-quest block
//!   reaches us as `CharacterInfo::active_quests` on the local player entity —
//!   the same `Added<CharacterInfo>` idiom every other HUD model uses
//!   (`hud/underbar/model.rs`, `hud/autopotion/model.rs`).
//! * **State** is [`QuestJournal`] plus [`TrackedQuests`]. The journal is
//!   wire-first: the tables only decorate it, because two of the nine live ids
//!   a real login carries (220, 399) have no `questdata.txt` row at all
//!   (`docs/re/systems/quest.md` §11.2 — a table-first journal shows blanks for
//!   real quests).
//! * **Progress** is `0x30D5`, whose layout came out of the original's own
//!   handler. It is a `Message` here, so a counter tick rebuilds the entry and
//!   the tracker line re-renders.
//!
//! **What is deliberately NOT here:** accept, abandon and hand-in.
//! `docs/planning/QUEST-audit.md` §5 records the measurement that would close
//! them.
//!
//! **Seeding is ours, and it is a stated deviation** (ADR-0009): the original
//! starts with nothing tracked and the player ticks a journal row's id-8
//! checkbox (`questlist.2dt`, `docs/re/ui/hud-quest-windows.md` §3.4) — which
//! [`super::journal`] wires. We additionally seed the tracker with the first
//! [`TrackedQuests::CAPACITY`] active quests in **wire order** (the only
//! ordering we have evidence for), so the state the server sent is visible on
//! login instead of visible only after a click. Unticking is real: both the
//! journal checkbox and the strip's close button untrack.

use bevy::prelude::*;

use packets::agent::quest::QuestUpdate;

use crate::plugins::net::character_info::CharacterInfo;
use crate::plugins::net::quest::{JournalChange, QuestJournal};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientQuestRewards, ClientQuestTable, ClientSpeechText};

/// The quests the on-screen tracker shows, in the order it shows them.
///
/// Ids, not indices: an entry can be removed by `0x30D5` while another is
/// tracked, and an index would then point at a different quest.
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct TrackedQuests {
    ids: Vec<u32>,
}

impl TrackedQuests {
    /// `questminilist.2dt` has exactly three `CNIFMiniQuestSlot` rows (ids
    /// 115/17/121, y 267/297/327, measured from the user's own file and
    /// recorded in `docs/re/ui/hud-quest-windows.md` §3.5) — so three is the
    /// original's number, not a taste.
    pub const CAPACITY: usize = 3;

    pub fn ids(&self) -> &[u32] {
        &self.ids
    }

    pub fn is_tracked(&self, id: u32) -> bool {
        self.ids.contains(&id)
    }

    /// Track a quest unless it is already tracked or the strip is full.
    /// Returns whether anything changed.
    pub fn track(&mut self, id: u32) -> bool {
        if self.is_tracked(id) || self.ids.len() >= Self::CAPACITY {
            return false;
        }
        self.ids.push(id);
        true
    }

    pub fn untrack(&mut self, id: u32) -> bool {
        match self.ids.iter().position(|tracked| *tracked == id) {
            Some(at) => {
                self.ids.remove(at);
                true
            }
            None => false,
        }
    }

    /// Seed from a freshly built journal: the first `CAPACITY` ids in wire
    /// order (see the module header for why this is ours and why it is stated).
    pub fn seed_from(&mut self, journal: &QuestJournal) {
        self.ids = journal
            .entries
            .iter()
            .take(Self::CAPACITY)
            .map(|entry| entry.id)
            .collect();
    }

    /// Drop tracked ids the journal no longer has. Called after every update so
    /// an abandoned quest cannot keep a row alive.
    pub fn retain_present(&mut self, journal: &QuestJournal) -> bool {
        let before = self.ids.len();
        self.ids.retain(|id| journal.get(*id).is_some());
        self.ids.len() != before
    }
}

/// Login: build the journal from the character record the server sent.
///
/// The tables are passed as they are — absent ones simply decorate less
/// (`QuestJournal::from_active` takes `Option`s on purpose), so a client whose
/// textdata has not finished loading still lists the right quests.
pub fn fill_journal_at_login(
    fresh: Query<&CharacterInfo, (With<Player>, Added<CharacterInfo>)>,
    table: Res<ClientQuestTable>,
    strings: Res<ClientSpeechText>,
    rewards: Res<ClientQuestRewards>,
    mut journal: ResMut<QuestJournal>,
    mut tracked: ResMut<TrackedQuests>,
) {
    let Ok(info) = fresh.single() else { return };
    *journal = QuestJournal::from_active(
        &info.active_quests,
        table.table(),
        strings.strings(),
        rewards.values(),
    );
    tracked.seed_from(&journal);
    // "No quests" and "the section did not parse" look identical from here, so
    // the count is logged rather than assumed (§11.3: 47 of 79 captured logins
    // really do carry zero active quests). The undecorated count is logged with
    // it because it is the one number that says the client's tables are behind
    // the server: two of the nine live ids (220, 399) have no `questdata.txt`
    // row, and a reader who sees `2 without a questdata.txt row` knows the
    // journal recovered them from the wire rather than dropping them.
    let undecorated = journal
        .entries
        .iter()
        .filter(|entry| entry.is_undecorated())
        .count();
    info!(
        "quest journal: {} active quest(s) at login, {} tracked, {} without a questdata.txt row",
        journal.entries.len(),
        tracked.ids().len(),
        undecorated
    );
}

/// In-session: fold every `0x30D5` into the journal.
///
/// Wire-first the same way the login build is — an `add` for an id no table
/// knows still becomes an entry. The tracker follows: a new quest takes a free
/// row, a removed one frees its row.
pub fn apply_quest_updates(
    mut updates: MessageReader<QuestUpdate>,
    table: Res<ClientQuestTable>,
    strings: Res<ClientSpeechText>,
    rewards: Res<ClientQuestRewards>,
    mut journal: ResMut<QuestJournal>,
    mut tracked: ResMut<TrackedQuests>,
) {
    for update in updates.read() {
        let change =
            journal.apply_update(update, table.table(), strings.strings(), rewards.values());
        match change {
            JournalChange::Added => {
                tracked.track(update.quest_id);
            }
            JournalChange::Removed => {
                tracked.untrack(update.quest_id);
            }
            // Our reading of the original's handler switch says these read no
            // bytes and change nothing. If one ever arrives, the reading is
            // incomplete — that is a finding, so it is said out loud.
            JournalChange::Ignored => warn!(
                "quest update {:#04x} for quest {}: kind outside the handler's switch",
                update.kind, update.quest_id
            ),
            JournalChange::NotPresent => debug!(
                "quest update: removal for quest {} which the journal does not list",
                update.quest_id
            ),
            JournalChange::Updated => {}
        }
        tracked.retain_present(&journal);
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn journal_of(ids: &[u32]) -> QuestJournal {
        let quests: Vec<ActiveQuest> = ids
            .iter()
            .map(|id| ActiveQuest {
                id: *id,
                achievements: 16,
                autoshare: 0,
                quest_type: 24,
                remaining_time: None,
                state: 1,
                objectives: Vec::new(),
                npcs: Vec::new(),
            })
            .collect();
        QuestJournal::from_active(&quests, None, None, None)
    }

    /// The strip holds the original's three rows and no more — a fourth quest
    /// does not silently push one out.
    #[test]
    fn the_tracker_seeds_three_quests_in_wire_order() {
        let journal = journal_of(&[3, 6, 11, 48]);
        let mut tracked = TrackedQuests::default();
        tracked.seed_from(&journal);
        assert_eq!(tracked.ids(), &[3, 6, 11]);
        assert!(!tracked.track(48), "a full strip refuses a fourth row");
        assert_eq!(tracked.ids().len(), TrackedQuests::CAPACITY);
    }

    /// Untracking is what the row's close button does, and it frees the slot.
    #[test]
    fn untracking_frees_a_row_for_the_next_quest() {
        let journal = journal_of(&[3, 6, 11, 48]);
        let mut tracked = TrackedQuests::default();
        tracked.seed_from(&journal);
        assert!(tracked.untrack(6));
        assert!(!tracked.is_tracked(6));
        assert!(tracked.track(48));
        assert_eq!(tracked.ids(), &[3, 11, 48]);
    }

    /// A quest the journal lost cannot keep its row: the `retain_present` pass
    /// after every update is what stops a stale line from surviving an abandon.
    #[test]
    fn a_quest_that_leaves_the_journal_leaves_the_strip() {
        let mut tracked = TrackedQuests::default();
        tracked.seed_from(&journal_of(&[3, 6, 11]));
        let shrunk = journal_of(&[3, 11]);
        assert!(tracked.retain_present(&shrunk));
        assert_eq!(tracked.ids(), &[3, 11]);
    }

    // --- the wiring itself: reception -> state, driven through real systems ---
    //
    // Idea: the three tests above check `TrackedQuests` arithmetic, which would
    // still pass if neither system were registered. These run the *systems* in
    // an `App`, because "the login record reaches the journal" is the claim this
    // slice makes and a pure unit test of the model cannot make it.

    use crate::plugins::net::character_info::CharacterInfo;
    use packets::agent::quest::{
        ActiveQuest, QuestObjective, QUEST_UPDATE_ABANDON, QUEST_UPDATE_UPDATE,
    };

    /// The nine records a real `Devi` login carries, ids as
    /// `docs/re/systems/quest.md` §11.2 decodes them. Only the ids and the one
    /// `%d` objective matter here; the byte-level fixture is the parser's own
    /// (`packets::agent::character_data`).
    fn devi_records() -> Vec<ActiveQuest> {
        [3u32, 6, 11, 48, 53, 57, 58, 220, 399]
            .iter()
            .map(|id| ActiveQuest {
                id: *id,
                achievements: 16,
                autoshare: 0,
                quest_type: 24,
                remaining_time: None,
                state: 1,
                objectives: vec![QuestObjective {
                    id: 1,
                    enabled: 1,
                    name_key: format!("SN_CON_QUEST_{id}"),
                    tasks: vec![0],
                }],
                npcs: Vec::new(),
            })
            .collect()
    }

    fn wired_app() -> App {
        let mut app = App::new();
        app.add_message::<QuestUpdate>()
            .init_resource::<QuestJournal>()
            .init_resource::<TrackedQuests>()
            // `None` tables: the journal must exist before textdata has loaded,
            // and the ids/counters asserted below are wire values either way.
            .init_resource::<ClientQuestTable>()
            .init_resource::<ClientSpeechText>()
            .init_resource::<ClientQuestRewards>()
            .add_systems(Update, (fill_journal_at_login, apply_quest_updates).chain());
        app
    }

    /// Reception: the login record on the local player entity becomes the
    /// journal, in wire order, and the strip is seeded from it. Nothing else in
    /// the tree filled `QuestJournal` before this system.
    #[test]
    fn the_login_record_reaches_the_journal_and_seeds_the_strip() {
        let mut app = wired_app();
        app.world_mut().spawn((
            Player,
            CharacterInfo {
                active_quests: devi_records(),
                ..default()
            },
        ));
        app.update();

        let journal = app.world().resource::<QuestJournal>();
        let ids: Vec<u32> = journal.entries.iter().map(|entry| entry.id).collect();
        assert_eq!(
            ids,
            [3, 6, 11, 48, 53, 57, 58, 220, 399],
            "every wire record is listed, in wire order"
        );
        assert_eq!(
            app.world().resource::<TrackedQuests>().ids(),
            &[3, 6, 11],
            "the strip shows the first three in wire order"
        );

        // `Added<CharacterInfo>` fires once: a second frame must not rebuild
        // (and so must not re-seed over a player's own tracking choice).
        app.world_mut().resource_mut::<TrackedQuests>().untrack(6);
        app.update();
        assert_eq!(
            app.world().resource::<TrackedQuests>().ids(),
            &[3, 11],
            "the login build runs once, not every frame"
        );
    }

    /// State: a `0x30D5` `update` frame moves the counter the strip renders.
    /// Before the wiring the journal was only ever right at login.
    #[test]
    fn a_quest_update_packet_moves_the_journal_counter() {
        let mut app = wired_app();
        app.world_mut().spawn((
            Player,
            CharacterInfo {
                active_quests: devi_records(),
                ..default()
            },
        ));
        app.update();
        assert_eq!(
            app.world()
                .resource::<QuestJournal>()
                .get(57)
                .expect("id 57")
                .objectives[0]
                .counter(),
            Some(0)
        );

        let mut ticked = devi_records()
            .into_iter()
            .find(|quest| quest.id == 57)
            .expect("id 57");
        ticked.objectives[0].tasks = vec![4];
        app.world_mut().write_message(QuestUpdate {
            kind: QUEST_UPDATE_UPDATE,
            quest_id: 57,
            quest: Some(ticked),
        });
        app.update();

        let journal = app.world().resource::<QuestJournal>();
        assert_eq!(journal.entries.len(), 9, "an update adds no entry");
        assert_eq!(
            journal.get(57).expect("id 57").objectives[0].counter(),
            Some(4),
            "the wire counter reached the journal through the system"
        );
    }

    /// State: an `abandon` frame removes the entry *and* frees its strip row —
    /// the `retain_present` pass is what stops a stale line from surviving.
    #[test]
    fn an_abandon_packet_clears_the_entry_and_its_strip_row() {
        let mut app = wired_app();
        app.world_mut().spawn((
            Player,
            CharacterInfo {
                active_quests: devi_records(),
                ..default()
            },
        ));
        app.update();
        assert!(app.world().resource::<TrackedQuests>().is_tracked(6));

        app.world_mut().write_message(QuestUpdate {
            kind: QUEST_UPDATE_ABANDON,
            quest_id: 6,
            quest: None,
        });
        app.update();

        assert!(
            app.world().resource::<QuestJournal>().get(6).is_none(),
            "the abandoned quest left the journal"
        );
        assert_eq!(
            app.world().resource::<TrackedQuests>().ids(),
            &[3, 11],
            "and its strip row went with it"
        );
        assert_eq!(app.world().resource::<QuestJournal>().entries.len(), 8);
    }
}

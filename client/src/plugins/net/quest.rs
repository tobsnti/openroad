//! The quest journal's **model**: the active quest set the server sent, plus
//! whatever the client's own tables can say about it. No window, no layout —
//! the display is a later stage (`docs/re/systems/quest.md` §11.8, S4).
//!
//! Idea, and why it is built this way round: the wire is the spine and the
//! tables are decoration. Two of the nine active quests our own server sends
//! for char `Devi` (ids 220 and 399) have **no row** in this client's
//! `questdata.txt` — verified lookup with the other seven ids of the same
//! packet as the positive control (§11.2/§11.4). A journal keyed on the table
//! would silently drop two real quests, so [`QuestJournal::from_active`]
//! creates one entry per **wire record** and decorates it only as far as the
//! tables reach:
//!
//! 1. `questdata.txt` by id → codename, required level, title key `[V]`.
//! 2. if the id is unknown: the codename recovered from the wire's own
//!    objective key `SN_CON_<codename>[_NN]` via `questcontentsdata.txt`, and
//!    the title key by the `SN_<codename>` convention that holds in 704 of the
//!    706 questdata rows `[S]`. Both table-less live ids resolve this way.
//! 3. no codename at all → the entry keeps its id, its state and its raw
//!    objective keys. Still listed, never dropped.
//!
//! The objective line is "client text with one `%d` filled from wire state":
//! `SN_CON_QNO_CH_GENARAL_1_02` is `"Hunt 15 TIger (%d)"` in
//! `textquest_speech&name.txt` and the number in the parentheses is the wire's
//! `tasks[0]`, never a table value (891 of the 1383 `SN_CON_*` rows carry a
//! `%d`, §11.5). Whether the original passes one or several arguments to a
//! multi-`%d` row is `[U]` (order 206), so only the first `%d` is substituted
//! and the rest is left standing rather than invented.
//!
//! **In-session progress** arrives on `0x30D5`, whose parser
//! ([`packets::agent::quest::QuestUpdate`]) is already in this tree.
//! [`QuestJournal::apply_update`] folds one such packet into the journal, and
//! it is wire-first the same way the login build is: an `add`/`update` for an
//! id no table knows still becomes an entry, and a `remove` for an id we never
//! had leaves every other entry alone. Because `add` and `update` carry a
//! **whole record** (both switch arms of the original's handler call the same
//! record parser), applying one is "replace this entry's wire half,
//! re-decorate", not a field-by-field merge — there is no delta form on this
//! wire to merge.
//!
//! **This file is model only, with no system in it** — deliberately, because
//! everything under `plugins::net::**` has to run in the headless netcheck
//! harness (AGENTS.md) and a resource nothing registers panics the schedule
//! behind a non-optional `Res`. The two systems that fill and fold it live in
//! `plugins::hud::quest::model`, and `plugins::hud::quest::QuestPlugin` is
//! what registers [`QuestJournal`].

use bevy::prelude::Resource;

use packets::agent::quest::{ActiveQuest, QuestUpdate};

use crate::assets::textdata::quest::{conventional_title_key, QuestTable};
use crate::assets::textdata::questreward::{QuestRewardValue, QuestRewardValues};
use crate::assets::textdata::uisystem::UiSystemText;

/// Where an entry's codename/title came from — a journal that cannot say this
/// cannot be reviewed against ADR-0009 ("every value carries its origin").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecorationSource {
    /// `questdata.txt` had a row for this id.
    QuestData,
    /// The id was unknown; the codename came from the wire's objective key and
    /// the title key from the `SN_<codename>` convention.
    WireObjectiveKey,
    /// Neither route resolved: id-only entry.
    None,
}

/// One objective line of one journal entry.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalObjective {
    /// 1-based within the quest, as the wire sends it.
    pub id: u8,
    /// Raw wire byte (`enabled`); `0` on the one captured objective of the
    /// tutorial quest, meaning `[U]` — kept raw instead of guessed.
    pub enabled: u8,
    /// `SN_CON_*` — the wire's key, kept even when the string table misses it.
    pub name_key: String,
    /// The client's sentence for that key, `None` when the string table is not
    /// loaded or has no row.
    pub text: Option<String>,
    /// The wire's `tasks` list verbatim; `tasks[0]` is what fills the `%d`.
    pub tasks: Vec<u32>,
}

impl JournalObjective {
    /// The counter the server sent for this objective, if any.
    pub fn counter(&self) -> Option<u32> {
        self.tasks.first().copied()
    }

    /// The display line: the client's text with its first `%d` replaced by the
    /// **wire** counter. Falls back to the raw key when the string table has
    /// no row — a missing string must stay visible, not blank.
    pub fn line(&self) -> String {
        let template = self.text.as_deref().unwrap_or(&self.name_key);
        match (template.find("%d"), self.counter()) {
            (Some(at), Some(counter)) => {
                let mut line = String::with_capacity(template.len() + 8);
                line.push_str(&template[..at]);
                line.push_str(&counter.to_string());
                line.push_str(&template[at + 2..]);
                line
            }
            _ => template.to_string(),
        }
    }
}

/// One active quest, wire state plus resolved decoration.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    // --- wire state (always present) ---
    pub id: u32,
    /// `1`, `7` or `8` in the captures `[U]` — kept raw, semantics need
    /// observation orders 200/202.
    pub state: u8,
    /// The raw type byte; its bits are read by `ActiveQuest`'s helpers.
    pub quest_type: u8,
    /// Type bits our `[S]` reading does not explain — non-zero is a finding,
    /// not an error.
    pub unknown_type_bits: u8,
    /// Present only for the time-limited form; unit `[U]`.
    pub remaining_time: Option<u32>,
    /// Quest-giver/target NPC ref ids the record carried.
    pub npcs: Vec<u32>,
    pub objectives: Vec<JournalObjective>,
    // --- decoration (may be absent) ---
    pub codename: Option<String>,
    pub decoration: DecorationSource,
    /// The resolved title text, `None` when the key or the string table is
    /// missing.
    pub title: Option<String>,
    pub title_key: Option<String>,
    /// `questdata.txt` column 3 `[S]`.
    pub required_level: Option<u8>,
    /// `questcontentsdata.txt` column 3: the quest this one chains into.
    pub follow_up: Option<String>,
    /// `refqusetreward.txt` columns 10/11 `[V]`.
    pub reward: Option<QuestRewardValue>,
}

impl JournalEntry {
    /// What the list shows: the title when it resolved, otherwise the codename,
    /// otherwise the id. Never empty — an undecorated quest is still a quest.
    pub fn display_title(&self) -> String {
        self.title
            .clone()
            .or_else(|| self.codename.clone())
            .unwrap_or_else(|| format!("quest {}", self.id))
    }

    /// `true` when no table row was found for this quest's id (the 220/399
    /// case). The journal keeps the entry; callers can mark it.
    pub fn is_undecorated(&self) -> bool {
        self.decoration != DecorationSource::QuestData
    }
}

/// The active quest set of the local character.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct QuestJournal {
    /// In wire order — the server's order is the only ordering we have
    /// evidence for.
    pub entries: Vec<JournalEntry>,
}

impl QuestJournal {
    /// Build the journal from the wire records. Every table is optional
    /// (`None` = not loaded yet): the journal must exist without them, exactly
    /// as it must exist for an id no table knows.
    pub fn from_active(
        quests: &[ActiveQuest],
        table: Option<&QuestTable>,
        strings: Option<&UiSystemText>,
        rewards: Option<&QuestRewardValues>,
    ) -> Self {
        let entries = quests
            .iter()
            .map(|quest| build_entry(quest, table, strings, rewards))
            .collect();
        QuestJournal { entries }
    }

    pub fn get(&self, id: u32) -> Option<&JournalEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    /// Fold one `0x30D5` into the journal and say what it did.
    ///
    /// The three cases are the three switch arms of the original's handler, so
    /// the behaviour is the handler's, not a policy of ours:
    ///
    /// * `add` / `update` (kinds 1, 2) carry a whole record — the entry is
    ///   rebuilt from it and **replaced in place**, keeping its position in the
    ///   list, or appended when the id is new. Wire-first: an id no table knows
    ///   gets an entry all the same, decorated as far as the tables reach.
    /// * `remove` / `abandon` (kinds 3, 4) delete the entry. Both are one arm
    ///   in the original's handler, so they are one arm here.
    /// * any other kind reads no bytes in the original and changes nothing
    ///   here — the caller should log it, because it would mean the `[V]`
    ///   reading of the handler's switch is incomplete.
    pub fn apply_update(
        &mut self,
        update: &QuestUpdate,
        table: Option<&QuestTable>,
        strings: Option<&UiSystemText>,
        rewards: Option<&QuestRewardValues>,
    ) -> JournalChange {
        if update.is_removal() {
            return match self
                .entries
                .iter()
                .position(|entry| entry.id == update.quest_id)
            {
                Some(at) => {
                    self.entries.remove(at);
                    JournalChange::Removed
                }
                // Not an error: the server may drop a quest we never listed
                // (e.g. one that arrived before the journal existed). Losing
                // the other entries over it would be the defect.
                None => JournalChange::NotPresent,
            };
        }

        let Some(quest) = update.quest.as_ref() else {
            return JournalChange::Ignored;
        };

        let entry = build_entry(quest, table, strings, rewards);
        match self
            .entries
            .iter()
            .position(|existing| existing.id == entry.id)
        {
            Some(at) => {
                self.entries[at] = entry;
                JournalChange::Updated
            }
            None => {
                self.entries.push(entry);
                JournalChange::Added
            }
        }
    }
}

/// What [`QuestJournal::apply_update`] did — returned rather than logged so the
/// caller decides whether a surprise is worth a chat line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalChange {
    /// The id was new to the journal and is now listed.
    Added,
    /// An existing entry was rebuilt from the record, in place.
    Updated,
    /// The entry was dropped (`remove` or `abandon`).
    Removed,
    /// A removal for an id the journal does not have: nothing changed.
    NotPresent,
    /// A `kind` outside the original handler's switch: nothing changed, and it
    /// is worth a log line because our reading of the switch would be wrong.
    Ignored,
}

fn build_entry(
    quest: &ActiveQuest,
    table: Option<&QuestTable>,
    strings: Option<&UiSystemText>,
    rewards: Option<&QuestRewardValues>,
) -> JournalEntry {
    // Route 1: the id is in questdata.txt. Route 2: it is not, so recover the
    // codename from the wire's own objective key.
    let row = table.and_then(|table| table.quest(quest.id));
    let (codename, decoration, title_key, required_level) = match (row, table) {
        (Some(row), _) => (
            Some(row.codename.clone()),
            DecorationSource::QuestData,
            row.title_key.clone(),
            Some(row.required_level),
        ),
        (None, Some(table)) => {
            let recovered = quest
                .objectives
                .iter()
                .find_map(|objective| table.codename_for_objective_key(&objective.name_key))
                .map(str::to_string);
            match recovered {
                Some(codename) => {
                    let key = conventional_title_key(&codename);
                    (
                        Some(codename),
                        DecorationSource::WireObjectiveKey,
                        Some(key),
                        None,
                    )
                }
                None => (None, DecorationSource::None, None, None),
            }
        }
        (None, None) => (None, DecorationSource::None, None, None),
    };

    let title = title_key
        .as_deref()
        .zip(strings)
        .and_then(|(key, strings)| strings.get(key))
        .map(str::to_string);
    let follow_up = codename
        .as_deref()
        .zip(table)
        .and_then(|(codename, table)| table.contents(codename))
        .and_then(|contents| contents.follow_up.clone());
    let reward = rewards.and_then(|rewards| rewards.get(quest.id));

    let objectives = quest
        .objectives
        .iter()
        .map(|objective| JournalObjective {
            id: objective.id,
            enabled: objective.enabled,
            name_key: objective.name_key.clone(),
            text: strings
                .and_then(|strings| strings.get(&objective.name_key))
                .map(str::to_string),
            tasks: objective.tasks.clone(),
        })
        .collect();

    JournalEntry {
        id: quest.id,
        state: quest.state,
        quest_type: quest.quest_type,
        unknown_type_bits: quest.unknown_type_bits(),
        remaining_time: quest.remaining_time,
        npcs: quest.npcs.clone(),
        objectives,
        codename,
        decoration,
        title,
        title_key,
        required_level,
        follow_up,
        reward,
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use packets::agent::quest::{
        QuestObjective, QUEST_UPDATE_ABANDON, QUEST_UPDATE_ADD, QUEST_UPDATE_REMOVE,
        QUEST_UPDATE_UPDATE,
    };

    /// The real `questdata.txt` rows of the seven ids char `Devi`'s login
    /// carries that the table *does* know — the two it does not (220, 399) are
    /// absent here on purpose, exactly as in the file.
    const QUESTDATA: &str = "1\t3\tQNO_CH_SMITH_1\t0\t<KOR>\tSN_QNO_CH_SMITH_1\tSN_PAY_QNO_CH_SMITH_1\txxx\tSN_PAYCON_QNO_CH_SMITH_1\tSN_NN_QNO_CH_SMITH_1\tSN_NC_QNO_CH_SMITH_1\t\n\
         1\t6\tQNO_CH_POTION_1\t5\t<KOR>\tSN_QNO_CH_POTION_1\txxx\txxx\txxx\txxx\txxx\t\n\
         1\t11\tQNO_CH_GENARAL_SP_1\t13\t<KOR>\tSN_QNO_CH_GENARAL_SP_1\txxx\txxx\txxx\txxx\txxx\t\n\
         1\t48\tQNO_CH_SPECIAL_1\t13\t<KOR>\tSN_QNO_CH_SPECIAL_1\txxx\txxx\txxx\txxx\txxx\t\n\
         1\t53\tQNO_CH_POTION_3\t6\t<KOR>\tSN_QNO_CH_POTION_3\txxx\txxx\txxx\txxx\txxx\t\n\
         1\t57\tQNO_CH_GENARAL_1\t13\t<KOR>\tSN_QNO_CH_GENARAL_1\txxx\txxx\txxx\txxx\txxx\t\n\
         1\t58\tQNO_CH_SMITH_2\t15\t<KOR>\tSN_QNO_CH_SMITH_2\txxx\txxx\txxx\txxx\txxx\t";

    /// Real `questcontentsdata.txt` rows, including both codenames whose ids
    /// are missing above.
    const QUESTCONTENTS: &str = "QNO_CH_SMITH_1\t<KOR>\t0\txxx\t1\tSN_CON_QNO_CH_SMITH_1\txxx\txxx\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0\n\
         QNO_CH_GENARAL_1\t<KOR>\t0\tQNO_CH_GENARAL_2\t1\tSN_CON_QNO_CH_GENARAL_1_01\tSN_CON_QNO_CH_GENARAL_1_02\tSN_CON_QNO_CH_GENARAL_1_03\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0\n\
         QSP_CH_EXINVENTORY_1\t<KOR>\t0\tQSP_WC_EXINVENTORY_2\t1\tSN_CON_QSP_CH_EXINVENTORY_1\txxx\txxx\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0\n\
         QTUTORIAL2_CH_1\t<KOR>\t0\tQTUTORIAL2_CH_2\t0\tSN_CON_QTUTORIAL2_CH_1\txxx\txxx\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0";

    /// The real strings of the keys this fixture touches, from
    /// `textquest_speech&name.txt` (`service \t key \t … \t English`).
    const SPEECH: &str = "1\tSN_QNO_CH_SMITH_1\t\t\t\t\t\t\tWeapon Dealer's Letter\n\
         1\tSN_QNO_CH_GENARAL_1\t\t\t\t\t\t\tTiger Hunting Contest 1\n\
         1\tSN_QSP_CH_EXINVENTORY_1\t\t\t\t\t\t\tInventory Expansion 1 (China)\n\
         1\tSN_QTUTORIAL2_CH_1\t\t\t\t\t\t\tBasic Movement Tutorial\n\
         1\tSN_CON_QNO_CH_SMITH_1\t\t\t\t\t\t\tSend Weapon List\n\
         1\tSN_CON_QNO_CH_GENARAL_1_02\t\t\t\t\t\t\tHunt 15 TIger (%d)\n\
         1\tSN_CON_QSP_CH_EXINVENTORY_1\t\t\t\t\t\t\tCollect 10 strong straws (%d) \n\
         1\tSN_CON_QTUTORIAL2_CH_1\t\t\t\t\t\t\tTeleport to the [South Gate] and speak with Jingyo";

    /// `refqusetreward.txt` rows for two of the nine ids; 220/399 have no row
    /// in that file either.
    const REWARDS: &str = "3\tQNO_CH_SMITH_1\t1\t1\t0\t0\t0\t0\t0\t0\t205\t270\n\
         11\tQNO_CH_GENARAL_SP_1\t1\t1\t0\t0\t0\t0\t0\t0\t0\t14500";

    fn tables() -> (QuestTable, UiSystemText, QuestRewardValues) {
        (
            QuestTable::parse(QUESTDATA, QUESTCONTENTS),
            UiSystemText::parse(SPEECH),
            QuestRewardValues::parse(REWARDS),
        )
    }

    fn objective(id: u8, enabled: u8, key: &str, tasks: Vec<u32>) -> QuestObjective {
        QuestObjective {
            id,
            enabled,
            name_key: key.to_string(),
            tasks,
        }
    }

    fn quest(id: u32, quest_type: u8, state: u8, objectives: Vec<QuestObjective>) -> ActiveQuest {
        ActiveQuest {
            id,
            achievements: 16,
            autoshare: 0,
            quest_type,
            remaining_time: None,
            state,
            objectives,
            npcs: if quest_type & 0x40 != 0 {
                vec![2037]
            } else {
                vec![]
            },
        }
    }

    /// The nine records char `Devi`'s CHARACTER_DATA carries, field for field
    /// as `docs/re/systems/quest.md` §11.2 decodes them (the byte-level
    /// fixture is `packets/src/agent/character_data.rs`
    /// `DEVI_QUEST_SECTION_HEX`; this is the same nine records as structs, so
    /// the journal test does not re-test the parser).
    fn devi_quests() -> Vec<ActiveQuest> {
        vec![
            quest(
                3,
                0x58,
                1,
                vec![objective(1, 1, "SN_CON_QNO_CH_SMITH_1", vec![0])],
            ),
            quest(
                6,
                0x18,
                1,
                vec![objective(1, 1, "SN_CON_QNO_CH_POTION_1", vec![0])],
            ),
            quest(
                11,
                0x18,
                1,
                vec![objective(1, 1, "SN_CON_QNO_CH_GENARAL_SP_1", vec![0])],
            ),
            quest(
                48,
                0x18,
                1,
                vec![objective(1, 1, "SN_CON_QNO_CH_SPECIAL_1", vec![0])],
            ),
            quest(
                53,
                0x18,
                1,
                vec![objective(1, 1, "SN_CON_QNO_CH_POTION_3", vec![0])],
            ),
            quest(
                57,
                0x18,
                1,
                vec![
                    objective(1, 1, "SN_CON_QNO_CH_GENARAL_1_01", vec![0]),
                    objective(2, 1, "SN_CON_QNO_CH_GENARAL_1_02", vec![0]),
                    objective(3, 1, "SN_CON_QNO_CH_GENARAL_1_03", vec![0]),
                ],
            ),
            quest(
                58,
                0x18,
                1,
                vec![objective(1, 1, "SN_CON_QNO_CH_SMITH_2", vec![0])],
            ),
            // the two ids with no questdata.txt row
            quest(
                220,
                0x18,
                7,
                vec![objective(1, 1, "SN_CON_QSP_CH_EXINVENTORY_1", vec![0])],
            ),
            quest(
                399,
                0x58,
                8,
                vec![objective(1, 0, "SN_CON_QTUTORIAL2_CH_1", vec![1])],
            ),
        ]
    }

    /// The whole point of the stage: nine wire records → nine journal entries,
    /// including the two ids the table does not know, and those two still get
    /// their codename and title from the wire's own objective key.
    #[test]
    fn all_nine_captured_quests_reach_the_journal() {
        let (table, strings, rewards) = tables();
        let journal =
            QuestJournal::from_active(&devi_quests(), Some(&table), Some(&strings), Some(&rewards));

        let ids: Vec<u32> = journal.entries.iter().map(|entry| entry.id).collect();
        assert_eq!(
            ids,
            [3, 6, 11, 48, 53, 57, 58, 220, 399],
            "wire order, and nothing dropped"
        );

        let smith = journal.get(3).expect("id 3");
        assert_eq!(smith.decoration, DecorationSource::QuestData);
        assert_eq!(smith.display_title(), "Weapon Dealer's Letter");
        assert_eq!(smith.required_level, Some(0));
        assert_eq!(
            smith.reward,
            Some(QuestRewardValue {
                gold: 205,
                exp: 270
            })
        );
        assert_eq!(smith.npcs, vec![2037], "the type-0x58 record's NPC list");

        // the three-objective quest keeps its three lines and its chain
        let genaral = journal.get(57).expect("id 57");
        assert_eq!(genaral.objectives.len(), 3);
        assert_eq!(genaral.follow_up.as_deref(), Some("QNO_CH_GENARAL_2"));

        // and the two table-less ids: entry present, decorated via the wire
        let exinventory = journal.get(220).expect("id 220 must be listed");
        assert_eq!(exinventory.decoration, DecorationSource::WireObjectiveKey);
        assert_eq!(
            exinventory.codename.as_deref(),
            Some("QSP_CH_EXINVENTORY_1")
        );
        assert_eq!(exinventory.display_title(), "Inventory Expansion 1 (China)");
        assert_eq!(
            exinventory.required_level, None,
            "no questdata row, so no level — absent, not 0"
        );
        assert_eq!(exinventory.reward, None, "no refqusetreward row either");
        assert_eq!(exinventory.state, 7, "wire state survives the missing row");

        let tutorial = journal.get(399).expect("id 399 must be listed");
        assert_eq!(tutorial.decoration, DecorationSource::WireObjectiveKey);
        assert_eq!(tutorial.display_title(), "Basic Movement Tutorial");
        assert_eq!(tutorial.objectives[0].enabled, 0, "raw wire byte, kept");
        assert!(tutorial.is_undecorated(), "no questdata row for id 399");

        // seven of nine came from the table — the doc's own count
        let from_table = journal
            .entries
            .iter()
            .filter(|entry| entry.decoration == DecorationSource::QuestData)
            .count();
        assert_eq!(from_table, 7);
    }

    /// The `%d` is wire state, not table state: same table, same string, two
    /// different counters give two different lines, and the table has no
    /// number that could have produced either.
    #[test]
    fn the_percent_d_counter_comes_from_the_wire() {
        let (table, strings, rewards) = tables();
        let line_for = |count: u32| {
            let quests = vec![quest(
                57,
                0x18,
                1,
                vec![objective(2, 1, "SN_CON_QNO_CH_GENARAL_1_02", vec![count])],
            )];
            QuestJournal::from_active(&quests, Some(&table), Some(&strings), Some(&rewards))
                .get(57)
                .expect("id 57")
                .objectives[0]
                .line()
        };
        assert_eq!(line_for(0), "Hunt 15 TIger (0)");
        assert_eq!(line_for(7), "Hunt 15 TIger (7)");
        // Same tables, same string row, different line: the number can only
        // have come from the record. The template itself keeps its `%d` — the
        // tables hold no counter to substitute (the `15` in the sentence is
        // part of the authored text, not a field).
        let (table, strings, _) = tables();
        assert_eq!(
            strings.get("SN_CON_QNO_CH_GENARAL_1_02"),
            Some("Hunt 15 TIger (%d)")
        );
        assert_eq!(
            table
                .contents("QNO_CH_GENARAL_1")
                .expect("row")
                .objective_keys
                .len(),
            3,
            "questcontentsdata lists keys, never counters"
        );

        // a record without a counter keeps the template rather than inventing
        // a zero
        let no_tasks = vec![quest(
            57,
            0x18,
            1,
            vec![objective(2, 1, "SN_CON_QNO_CH_GENARAL_1_02", vec![])],
        )];
        let journal = QuestJournal::from_active(&no_tasks, Some(&table), Some(&strings), None);
        assert_eq!(
            journal.get(57).expect("id 57").objectives[0].line(),
            "Hunt 15 TIger (%d)"
        );

        // a line with no %d at all is passed through unchanged, counter or not
        let tutorial = vec![quest(
            399,
            0x58,
            8,
            vec![objective(1, 0, "SN_CON_QTUTORIAL2_CH_1", vec![1])],
        )];
        let journal = QuestJournal::from_active(&tutorial, Some(&table), Some(&strings), None);
        assert_eq!(
            journal.get(399).expect("id 399").objectives[0].line(),
            "Teleport to the [South Gate] and speak with Jingyo"
        );
    }

    /// An id no table knows, whose objective key no table knows either, and
    /// with no tables loaded at all: three ways to lose a quest silently, none
    /// of which may happen.
    #[test]
    fn an_unknown_id_neither_panics_nor_vanishes() {
        let (table, strings, rewards) = tables();
        let stranger = vec![quest(
            65535,
            0x18,
            3,
            vec![objective(1, 1, "SN_CON_QCX_KT_ARMOR_1_01", vec![4])],
        )];

        let journal =
            QuestJournal::from_active(&stranger, Some(&table), Some(&strings), Some(&rewards));
        let entry = journal
            .get(65535)
            .expect("an unknown quest is still listed");
        assert_eq!(entry.decoration, DecorationSource::None);
        assert_eq!(entry.codename, None);
        assert_eq!(entry.title, None);
        assert_eq!(entry.display_title(), "quest 65535");
        // the raw key stays visible, with the wire counter substituted where
        // the string table has nothing to substitute into
        assert_eq!(entry.objectives[0].line(), "SN_CON_QCX_KT_ARMOR_1_01");
        assert_eq!(entry.objectives[0].counter(), Some(4));
        assert_eq!(entry.state, 3);

        // and with no tables loaded (the textdata assets have not arrived yet)
        let bare = QuestJournal::from_active(&devi_quests(), None, None, None);
        assert_eq!(bare.entries.len(), 9, "the journal exists without tables");
        assert!(bare
            .entries
            .iter()
            .all(|entry| entry.decoration == DecorationSource::None));
        assert_eq!(bare.get(3).expect("id 3").display_title(), "quest 3");
        assert_eq!(
            bare.get(3).expect("id 3").objectives[0].line(),
            "SN_CON_QNO_CH_SMITH_1"
        );
    }

    fn update(kind: u8, quest: ActiveQuest) -> QuestUpdate {
        QuestUpdate {
            kind,
            quest_id: quest.id,
            quest: Some(quest),
        }
    }

    /// The reason this lane exists: after an update the journal carries the
    /// **new** counter, and the objective line renders it. Before the wiring the
    /// journal was only ever right at login.
    #[test]
    fn a_counter_update_moves_the_journal_line() {
        let (table, strings, rewards) = tables();
        let mut journal =
            QuestJournal::from_active(&devi_quests(), Some(&table), Some(&strings), Some(&rewards));
        assert_eq!(
            journal.get(57).expect("id 57").objectives[1].line(),
            "Hunt 15 TIger (0)"
        );

        // 0x30D5 kind 2 for quest 57 with the middle objective at 3 kills.
        let ticked = quest(
            57,
            0x18,
            1,
            vec![
                objective(1, 1, "SN_CON_QNO_CH_GENARAL_1_01", vec![0]),
                objective(2, 1, "SN_CON_QNO_CH_GENARAL_1_02", vec![3]),
                objective(3, 1, "SN_CON_QNO_CH_GENARAL_1_03", vec![0]),
            ],
        );
        let change = journal.apply_update(
            &update(QUEST_UPDATE_UPDATE, ticked),
            Some(&table),
            Some(&strings),
            Some(&rewards),
        );
        assert_eq!(change, JournalChange::Updated);
        assert_eq!(journal.entries.len(), 9, "an update adds no entry");
        let entry = journal.get(57).expect("id 57");
        assert_eq!(entry.objectives[1].counter(), Some(3));
        assert_eq!(entry.objectives[1].line(), "Hunt 15 TIger (3)");
        // the decoration survives the rebuild
        assert_eq!(entry.decoration, DecorationSource::QuestData);
        assert_eq!(entry.follow_up.as_deref(), Some("QNO_CH_GENARAL_2"));
        // and the entry kept its place in wire order
        let ids: Vec<u32> = journal.entries.iter().map(|entry| entry.id).collect();
        assert_eq!(ids, [3, 6, 11, 48, 53, 57, 58, 220, 399]);
    }

    /// An update for an id the journal does not have, and whose id no table
    /// knows: it must be **added**, and no existing entry may go missing. This
    /// is the wire-first rule applied to the update path — the same rule that
    /// keeps ids 220 and 399 in the login journal.
    #[test]
    fn an_update_for_an_unknown_quest_id_loses_no_entry() {
        let (table, strings, rewards) = tables();
        let mut journal =
            QuestJournal::from_active(&devi_quests(), Some(&table), Some(&strings), Some(&rewards));
        let before: Vec<u32> = journal.entries.iter().map(|entry| entry.id).collect();

        let stranger = quest(
            65535,
            0x18,
            3,
            vec![objective(1, 1, "SN_CON_QCX_KT_ARMOR_1_01", vec![4])],
        );
        let change = journal.apply_update(
            &update(QUEST_UPDATE_UPDATE, stranger),
            Some(&table),
            Some(&strings),
            Some(&rewards),
        );
        assert_eq!(change, JournalChange::Added, "an unknown id is appended");
        let after: Vec<u32> = journal.entries.iter().map(|entry| entry.id).collect();
        assert_eq!(&after[..before.len()], &before[..], "nothing was dropped");
        let entry = journal.get(65535).expect("the stranger is listed");
        assert_eq!(entry.decoration, DecorationSource::None);
        assert_eq!(entry.display_title(), "quest 65535");
        assert_eq!(entry.objectives[0].counter(), Some(4));
    }

    /// The other three arms: a newly accepted quest, a removal, a removal for
    /// an id we never had, and a kind the original's switch does not cover.
    #[test]
    fn accept_remove_and_an_unknown_kind_each_do_exactly_one_thing() {
        let (table, strings, rewards) = tables();
        let mut journal =
            QuestJournal::from_active(&devi_quests(), Some(&table), Some(&strings), Some(&rewards));

        // accept: quest 9 is not in the login set
        let accepted = quest(
            9,
            0x18,
            1,
            vec![objective(1, 1, "SN_CON_QNO_CH_SHAMAN_1", vec![0])],
        );
        assert_eq!(
            journal.apply_update(
                &update(QUEST_UPDATE_ADD, accepted),
                Some(&table),
                Some(&strings),
                Some(&rewards)
            ),
            JournalChange::Added
        );
        assert_eq!(journal.entries.len(), 10);
        assert_eq!(journal.get(9).expect("id 9").state, 1);

        // hand-in / abandon: kinds 3 and 4 are the same client-side effect
        for (kind, id) in [(QUEST_UPDATE_REMOVE, 9u32), (QUEST_UPDATE_ABANDON, 220)] {
            let removal = QuestUpdate {
                kind,
                quest_id: id,
                quest: None,
            };
            assert_eq!(
                journal.apply_update(&removal, Some(&table), Some(&strings), Some(&rewards)),
                JournalChange::Removed
            );
            assert!(journal.get(id).is_none(), "the entry is gone");
        }
        assert_eq!(journal.entries.len(), 8);

        // a removal for something we never listed changes nothing
        let unknown_removal = QuestUpdate {
            kind: QUEST_UPDATE_REMOVE,
            quest_id: 4242,
            quest: None,
        };
        assert_eq!(
            journal.apply_update(
                &unknown_removal,
                Some(&table),
                Some(&strings),
                Some(&rewards)
            ),
            JournalChange::NotPresent
        );
        assert_eq!(journal.entries.len(), 8);

        // and a kind outside the handler's switch: no record, no change
        let surprise = QuestUpdate {
            kind: 7,
            quest_id: 3,
            quest: None,
        };
        assert_eq!(
            journal.apply_update(&surprise, Some(&table), Some(&strings), Some(&rewards)),
            JournalChange::Ignored
        );
        assert_eq!(journal.entries.len(), 8);
        assert!(journal.get(3).is_some(), "an ignored kind touches nothing");
    }

    /// The update path must work with no tables loaded at all, exactly like the
    /// login path — the textdata assets can arrive after the first packet.
    #[test]
    fn an_update_without_tables_still_carries_the_wire_counter() {
        let mut journal = QuestJournal::from_active(&devi_quests(), None, None, None);
        let ticked = quest(
            6,
            0x18,
            1,
            vec![objective(1, 1, "SN_CON_QNO_CH_POTION_1", vec![2])],
        );
        assert_eq!(
            journal.apply_update(&update(QUEST_UPDATE_UPDATE, ticked), None, None, None),
            JournalChange::Updated
        );
        let entry = journal.get(6).expect("id 6");
        assert_eq!(entry.objectives[0].counter(), Some(2));
        assert_eq!(entry.objectives[0].line(), "SN_CON_QNO_CH_POTION_1");
        assert_eq!(journal.entries.len(), 9);
    }

    /// A CHARACTER_DATA whose quest section did not parse must produce an
    /// empty journal, not a panic. The component flattens the parser's
    /// `Option<Vec<_>>` with `unwrap_or_default()`
    /// (`plugins/net/character_info.rs`), so "did not parse" and "no quests"
    /// both arrive here as an empty slice — which is exactly why
    /// `fill_journal_at_login` logs the count instead of inferring which of
    /// the two happened.
    #[test]
    fn an_absent_quest_section_yields_an_empty_journal() {
        let journal = QuestJournal::from_active(&[], None, None, None);
        assert!(journal.entries.is_empty());
    }
}

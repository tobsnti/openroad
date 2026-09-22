//! Quest structure tables: `questdata.txt` (quest **id** → codename, level,
//! title key) and `questcontentsdata.txt` (**codename** → objective keys and
//! follow-up quest).
//!
//! Idea: these two tables are the *decoration* of a journal, never its spine.
//! The server sends the active set on the wire (`packets::agent::quest`), and
//! two of the nine ids our own server sends for one character (220, 399) have
//! **no row** in this 706-row `questdata.txt` at all — verified lookup, with
//! the other seven ids of the same packet as the positive control. So the
//! tables are joined by two
//! *independent* keys on purpose:
//!
//! * `by_id` — what `questdata.txt` can say about a wire id (may be nothing).
//! * `contents` — keyed by **codename**, which the wire itself always carries
//!   inside the objective key `SN_CON_<codename>[_NN]`. Both missing ids do
//!   have a `questcontentsdata.txt` row, so the codename route decorates a
//!   quest the id route cannot ([`QuestTable::codename_for_objective_key`]).
//!
//! Columns consumed, and the ones deliberately left alone (a journal needs four
//! of the 29):
//!
//! `questdata.txt` (12 columns): `[1] id`, `[2] codename`, `[3] required
//! level`, `[5] SN_<codename>` = title key. **Not read:** `[0] service` (only
//! used as the row filter — `1` in all 706 data rows, the one other line is
//! the literal `//DBtoMedia` comment), `[4]` the Korean name (we display the
//! string table, not the raw table text), `[6] SN_PAY_*` / `[8] SN_PAYCON_*` /
//! `[9] SN_NN_*` / `[10] SN_NC_*` — those four are the *detail pane* keys and
//! their file (`textquest_otherstring.txt`) is not loaded yet, so reading the
//! keys would buy nothing; `[7]` is `xxx` in all 706 rows and `[11]` is empty.
//!
//! `questcontentsdata.txt` (17 columns): `[0] codename`, `[3]` follow-up quest
//! codename (`xxx` = none; 373 of 985 rows carry one), `[5..=12]` up to eight
//! `SN_CON_*` objective keys. **Not read:** `[1]` Korean description, `[2]`
//! (∈ {0,1,3}, unlabelled), `[4]` a 0/1 flag that is *not* an objective count
//! (160 rows disagree with their own key count), `[13]`/`[16]` 0/1
//! flags, `[14]`/`[15]` `xxx`.

use std::collections::HashMap;

/// `questdata.txt` column indices.
const QD_SERVICE_COL: usize = 0;
const QD_ID_COL: usize = 1;
const QD_CODENAME_COL: usize = 2;
const QD_LEVEL_COL: usize = 3;
const QD_TITLE_KEY_COL: usize = 5;

/// `questcontentsdata.txt` column indices; the objective keys are a range.
const QC_CODENAME_COL: usize = 0;
const QC_FOLLOW_UP_COL: usize = 3;
const QC_OBJECTIVE_COLS: std::ops::RangeInclusive<usize> = 5..=12;

/// The placeholder both files use for "no value".
const NONE_MARKER: &str = "xxx";

/// One `questdata.txt` row, reduced to what a journal shows.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestRow {
    pub codename: String,
    /// Column 3, `0..=110` in this data: the seven live ids sit at
    /// 0/5/13/13/6/13/15 for a level-1..15 starting area, which is what makes
    /// "required level" the reading (the column is unnamed in the data).
    pub required_level: u8,
    /// Column 5 (`SN_<codename>`), `None` where the row says `xxx`
    /// (`QEVENT_GUIDE`, id 1 — the one titleless row in the corpus).
    pub title_key: Option<String>,
}

/// One `questcontentsdata.txt` row, reduced likewise.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestContents {
    /// Column 3: the quest this one chains into, `None` for `xxx`.
    pub follow_up: Option<String>,
    /// Columns 5..=12 in file order, `xxx` entries dropped. These are the same
    /// `SN_CON_*` keys the wire sends per objective, which is what lets a
    /// journal cross-check the wire against the table instead of trusting
    /// either alone.
    pub objective_keys: Vec<String>,
}

/// Both tables, joined the two ways a wire-first journal needs them.
#[derive(Debug, Clone, Default)]
pub struct QuestTable {
    pub by_id: HashMap<u32, QuestRow>,
    pub contents: HashMap<String, QuestContents>,
}

impl QuestTable {
    pub fn parse(questdata: &str, questcontentsdata: &str) -> Self {
        let mut by_id = HashMap::new();
        for cols in rows(questdata) {
            // `service` is the row filter, exactly as in the string tables:
            // it also drops the `//DBtoMedia` comment line, which has one
            // column and no id.
            if cols.get(QD_SERVICE_COL).map(|c| c.trim()) != Some("1") {
                continue;
            }
            let Some(id) = cols
                .get(QD_ID_COL)
                .and_then(|c| c.trim().parse::<u32>().ok())
            else {
                continue;
            };
            let Some(codename) = cols.get(QD_CODENAME_COL).map(|c| c.trim()) else {
                continue;
            };
            if codename.is_empty() || codename == NONE_MARKER {
                continue;
            }
            by_id.insert(
                id,
                QuestRow {
                    codename: codename.to_string(),
                    required_level: cols
                        .get(QD_LEVEL_COL)
                        .and_then(|c| c.trim().parse::<u8>().ok())
                        .unwrap_or(0),
                    title_key: optional(cols.get(QD_TITLE_KEY_COL)),
                },
            );
        }

        let mut contents = HashMap::new();
        for cols in rows(questcontentsdata) {
            let Some(codename) = cols.get(QC_CODENAME_COL).map(|c| c.trim()) else {
                continue;
            };
            if codename.is_empty() || codename == NONE_MARKER || codename.starts_with("//") {
                continue;
            }
            let objective_keys = QC_OBJECTIVE_COLS
                .filter_map(|i| optional(cols.get(i)))
                .collect();
            contents.insert(
                codename.to_string(),
                QuestContents {
                    follow_up: optional(cols.get(QC_FOLLOW_UP_COL)),
                    objective_keys,
                },
            );
        }

        QuestTable { by_id, contents }
    }

    /// What `questdata.txt` knows about a wire id — `None` for the ids it does
    /// not carry (220 and 399 among them).
    pub fn quest(&self, id: u32) -> Option<&QuestRow> {
        self.by_id.get(&id)
    }

    pub fn contents(&self, codename: &str) -> Option<&QuestContents> {
        self.contents.get(codename)
    }

    /// Recover the quest codename from a wire objective key
    /// `SN_CON_<codename>[_NN]`.
    ///
    /// The `_NN` suffix cannot be stripped blindly, because a codename may end
    /// in a number itself (`QNO_CH_SMITH_1`). So the rule is a table
    /// membership test, in this order: the whole remainder, then the remainder
    /// without one trailing `_<digits>`. Of the 1383 `SN_CON_*`
    /// rows of `textquest_speech&name.txt`: 261 hit as-is, 937 after the
    /// strip, 185 resolve to no `questcontentsdata.txt` row at all (e.g.
    /// `SN_CON_QCX_KT_ARMOR_1_01`) — those return `None` rather than a guessed
    /// codename, which is why the caller keeps the raw key too.
    pub fn codename_for_objective_key(&self, key: &str) -> Option<&str> {
        let remainder = key.strip_prefix("SN_CON_")?;
        if let Some((codename, _)) = self.contents.get_key_value(remainder) {
            return Some(codename.as_str());
        }
        let (stem, suffix) = remainder.rsplit_once('_')?;
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
            if let Some((codename, _)) = self.contents.get_key_value(stem) {
                return Some(codename.as_str());
            }
        }
        None
    }
}

/// The title-key convention, for the quests `questdata.txt` has no row for:
/// column 5 is literally `SN_<codename>` in **704 of the 706** rows (the two
/// exceptions are `QEVENT_GUIDE`, whose column is `xxx`, and
/// `QNO_RM_MEETROC_6`, which points at `SN_QNO_RM_MEETROC_3`). So this is a
/// convention, not a table lookup — a caller that uses it says so, and
/// both table-less live ids do resolve through it
/// (`SN_QSP_CH_EXINVENTORY_1` → "Inventory Expansion 1 (China)",
/// `SN_QTUTORIAL2_CH_1` → "Basic Movement Tutorial").
pub fn conventional_title_key(codename: &str) -> String {
    format!("SN_{codename}")
}

fn rows(content: &str) -> impl Iterator<Item = Vec<&str>> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').collect())
}

/// A column that is present, non-empty and not the `xxx` placeholder.
fn optional(col: Option<&&str>) -> Option<String> {
    let value = col?.trim();
    (!value.is_empty() && value != NONE_MARKER).then(|| value.to_string())
}

#[cfg(test)]
mod test {
    use super::*;

    /// Real rows, tabs as in the file: three ids the wire sends (3, 6, 57),
    /// the titleless `QEVENT_GUIDE` (id 1) and the comment line.
    const QUESTDATA: &str = "1\t3\tQNO_CH_SMITH_1\t0\t<KOR>\tSN_QNO_CH_SMITH_1\tSN_PAY_QNO_CH_SMITH_1\txxx\tSN_PAYCON_QNO_CH_SMITH_1\tSN_NN_QNO_CH_SMITH_1\tSN_NC_QNO_CH_SMITH_1\t\n\
         1\t6\tQNO_CH_POTION_1\t5\t<KOR>\tSN_QNO_CH_POTION_1\tSN_PAY_QNO_CH_POTION_1\txxx\tSN_PAYCON_QNO_CH_POTION_1\tSN_NN_QNO_CH_POTION_1\tSN_NC_QNO_CH_POTION_1\t\n\
         1\t57\tQNO_CH_GENARAL_1\t13\t<KOR>\tSN_QNO_CH_GENARAL_1\tSN_PAY_QNO_CH_GENARAL_1\txxx\tSN_PAYCON_QNO_CH_GENARAL_1\tSN_NN_QNO_CH_GENARAL_1\tSN_NC_QNO_CH_GENARAL_1\t\n\
         1\t1\tQEVENT_GUIDE\t0\t<KOR>\txxx\txxx\txxx\txxx\txxx\txxx\t\n\
         //DBtoMedia";

    /// Real rows: the three-objective quest, the single-objective one, and the
    /// two codenames whose **ids** are missing from `questdata.txt`.
    const QUESTCONTENTS: &str = "QNO_CH_GENARAL_1\t<KOR>\t0\tQNO_CH_GENARAL_2\t1\tSN_CON_QNO_CH_GENARAL_1_01\tSN_CON_QNO_CH_GENARAL_1_02\tSN_CON_QNO_CH_GENARAL_1_03\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0\n\
         QNO_CH_SMITH_1\t<KOR>\t0\txxx\t1\tSN_CON_QNO_CH_SMITH_1\txxx\txxx\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0\n\
         QSP_CH_EXINVENTORY_1\t<KOR>\t0\tQSP_WC_EXINVENTORY_2\t1\tSN_CON_QSP_CH_EXINVENTORY_1\txxx\txxx\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0\n\
         QTUTORIAL2_CH_1\t<KOR>\t0\tQTUTORIAL2_CH_2\t0\tSN_CON_QTUTORIAL2_CH_1\txxx\txxx\txxx\txxx\txxx\txxx\txxx\t0\txxx\txxx\t0";

    fn table() -> QuestTable {
        QuestTable::parse(QUESTDATA, QUESTCONTENTS)
    }

    #[test]
    fn questdata_rows_carry_codename_level_and_title_key() {
        let table = table();
        assert_eq!(table.by_id.len(), 4, "the comment line is not a row");
        let smith = table.quest(3).expect("id 3 is in the table");
        assert_eq!(smith.codename, "QNO_CH_SMITH_1");
        assert_eq!(smith.required_level, 0);
        assert_eq!(smith.title_key.as_deref(), Some("SN_QNO_CH_SMITH_1"));
        assert_eq!(table.quest(57).map(|r| r.required_level), Some(13));
        // the one row whose title column is the `xxx` placeholder
        assert_eq!(table.quest(1).expect("id 1").title_key, None);
        // and the ids the server really sends that this table does not have
        assert!(table.quest(220).is_none());
        assert!(table.quest(399).is_none());
    }

    #[test]
    fn questcontents_rows_carry_the_objective_keys_and_the_chain() {
        let table = table();
        let genaral = table.contents("QNO_CH_GENARAL_1").expect("row");
        assert_eq!(
            genaral.objective_keys,
            [
                "SN_CON_QNO_CH_GENARAL_1_01",
                "SN_CON_QNO_CH_GENARAL_1_02",
                "SN_CON_QNO_CH_GENARAL_1_03"
            ]
        );
        assert_eq!(genaral.follow_up.as_deref(), Some("QNO_CH_GENARAL_2"));
        // `xxx` in the follow-up column is "no follow-up", not a quest name
        assert_eq!(
            table.contents("QNO_CH_SMITH_1").expect("row").follow_up,
            None
        );
        assert_eq!(
            table
                .contents("QNO_CH_SMITH_1")
                .expect("row")
                .objective_keys
                .len(),
            1,
            "the seven trailing xxx columns are not objectives"
        );
    }

    /// The wire's objective key is the only id→codename route for the two live
    /// quests `questdata.txt` omits, so the recovery rule is load-bearing.
    #[test]
    fn a_codename_is_recovered_from_the_wire_objective_key() {
        let table = table();
        // remainder hits a row as-is (a codename that ends in a number)
        assert_eq!(
            table.codename_for_objective_key("SN_CON_QNO_CH_SMITH_1"),
            Some("QNO_CH_SMITH_1")
        );
        // remainder only hits after one trailing `_NN` is dropped
        assert_eq!(
            table.codename_for_objective_key("SN_CON_QNO_CH_GENARAL_1_02"),
            Some("QNO_CH_GENARAL_1")
        );
        // the two ids without a questdata row still resolve this way
        assert_eq!(
            table.codename_for_objective_key("SN_CON_QSP_CH_EXINVENTORY_1"),
            Some("QSP_CH_EXINVENTORY_1")
        );
        assert_eq!(
            table.codename_for_objective_key("SN_CON_QTUTORIAL2_CH_1"),
            Some("QTUTORIAL2_CH_1")
        );
        // no row, no guess (185 of the 1383 SN_CON_* keys are in this class)
        assert_eq!(
            table.codename_for_objective_key("SN_CON_QCX_KT_ARMOR_1_01"),
            None
        );
        // and a key that is not an objective key at all
        assert_eq!(table.codename_for_objective_key("SN_QNO_CH_SMITH_1"), None);
    }

    #[test]
    fn the_title_key_convention_matches_the_table_column() {
        let table = table();
        for id in [3, 6, 57] {
            let row = table.quest(id).expect("row");
            assert_eq!(
                row.title_key.as_deref(),
                Some(conventional_title_key(&row.codename).as_str()),
                "column 5 is SN_<codename> in 704 of 706 rows"
            );
        }
    }
}

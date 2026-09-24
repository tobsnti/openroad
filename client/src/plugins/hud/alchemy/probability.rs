//! Alchemy: the reinforce success chance, read out of the shipped `itemdata`.
//!
//! Idea: the enhancement odds are **not** server-only. Every ordinary elixir
//! row carries its own twelve-step success ladder, and every Lucky Powder row
//! carries a twelve-step bonus ladder, in the `(Param<N>, Param<N>_Desc)` pairs
//! at itemdata fields 118..157. So the
//! client can *show* the odds instead of making the player guess, and it can do
//! it without a single number of its own: this module only decodes the rows.
//!
//! The packing, and how the byte order follows from the data: where a
//! `Param<N>_Desc` reads `"1,2,3,4"`, the paired int is four values,
//! most-significant byte first. In an elixir recipe row, `Param2 = 420744970 =
//! 0x19140F0A` decodes to `25,20,15,10` beside a `Desc` of `"1,2,3,4"`,
//! `Param3` to `10,10,10,10` (`"5,6,7,8"`) and `Param4` to `10,5,5,5`
//! (`"9,10,11,12"`). Three independent columns whose decoded values line up
//! with the indices their own `Desc` names.
//!
//! **Data caveat, and it is why nothing here is a constant:** an archive may
//! be tuned. The *structure* (which column carries what) is the client's; the
//! *values* belong to whatever archive is loaded. Every number this module
//! reports therefore comes out of the row it was asked about — the module
//! holds column indices and type ids, never a probability.
//!
//! Deliberate deviation (ADR-0009), stated: v1.188 shows no percentage in the
//! alchemy box (its `resinfo` pages declare no such control, and
//! `textuisystem.txt` ships no key that formats one — `:4246` only *says* that
//! Lucky Powder "will increase percent of strengthing success rate"). Showing
//! the number is our improvement; the number itself stays the data's.

use crate::assets::textdata::itemdata::ItemDataRow;

/// RefItemData `Param1` column (the first of the 20 `(Param<N>, Param<N>_Desc)`
/// pairs at fields 118..157; `itemdata::ItemdataFields::Param1` names the same
/// index).
const PARAM1_FIELD: usize = 118;

/// `Param<n>` (1-based) as the raw authored integer: `Param<n>` is field
/// `118 + 2*(n - 1)`. Lives here until `ItemDataRow` grows the accessor.
fn param_of(row: &ItemDataRow, n: usize) -> Option<i64> {
    let index = PARAM1_FIELD + 2 * n.checked_sub(1)?;
    row.0.get(index)?.trim().parse().ok()
}

/// `Param<n>` as the big-endian byte view of the signed int: the alchemy rows
/// pack four values into one column (`MAGICSTONE_STR_01.Param2 = 169090560 =
/// 0x0A141E00` -> `10,20,30,0` beside a `Desc` reading `"10, 20, 30, 0"`). A
/// `-1` column is empty, not `[255,255,255,255]`, so callers check the raw
/// value first.
fn param_bytes_of(row: &ItemDataRow, n: usize) -> Option<[u8; 4]> {
    Some(i32::try_from(param_of(row, n)?).ok()?.to_be_bytes())
}

/// The ladder is twelve entries long, so +12 is the last reachable step. No
/// column states a ceiling; the twelve-entry tables imply it.
pub const REINFORCE_LADDER_LEN: usize = 12;

/// TID of an ordinary elixir (`3.3.10.1`) — the rows that carry a ladder.
const TID_ELIXIR: (u32, u32, u32, u32) = (3, 3, 10, 1);
/// TID of Lucky Powder (`3.3.10.2`).
const TID_LUCKY_POWDER: (u32, u32, u32, u32) = (3, 3, 10, 2);
/// TID of a magic stone (`3.3.11.1`).
const TID_MAGIC_STONE: (u32, u32, u32, u32) = (3, 3, 11, 1);
/// The three columns that hold the twelve ladder entries, in index order
/// (`Desc` labels `"1,2,3,4"`, `"5,6,7,8"`, `"9,10,11,12"`).
const LADDER_PARAMS: [usize; 3] = [2, 3, 4];
/// `Param1` and `Param5` are both permitted-`TypeID3` lists: `Param1` carries
/// the Chinese classes, `Param5` the European ones (`ARMOR_B.Param5 =
/// [9,10,11,0]`, `ACCESSARY_B.Param5 = [12,0,0,0]`). A row with only one list
/// leaves the other at `-1`/`0`.
const TYPE_GATE_PARAMS: [usize; 2] = [1, 5];

/// What the data says about one (equipment, elixir, powder) combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReinforceChance {
    /// The elixir's own success chance for the *next* enhancement step, in
    /// percent, straight out of its ladder.
    pub percent: u32,
    /// The matched Lucky Powder's bonus for the same step, in percentage
    /// points. Kept as a second figure rather than folded into `percent`
    /// because it is unknown whether a server adds or multiplies: no itemdata
    /// column and nothing in the original states the combination rule.
    pub powder_bonus: Option<u32>,
}

/// An ordinary elixir (TID `3.3.10.1`). The 192 *advanced* elixirs (TID `3.3.10.4`)
/// are a different TID and carry no ladder at all (`Param2..4 = -1`), which is
/// why they never reach this predicate.
pub fn is_elixir(row: &ItemDataRow) -> bool {
    row.type_ids() == Some(TID_ELIXIR)
}

/// A Lucky Powder (TID `3.3.10.2`).
pub fn is_lucky_powder(row: &ItemDataRow) -> bool {
    row.type_ids() == Some(TID_LUCKY_POWDER)
}

/// A magic stone (TID `3.3.11.1`, the `MAGICSTONE_<stat>_01..12` rows). Its
/// counterpart is the attribute stone (TID `3.3.11.2`,
/// `ATTRSTONE_<elem>_01..12`), and the two are exactly the `AlchemyType` 4 / 5
/// the `0x7151` request selects between, which is the only place this
/// predicate is used.
pub fn is_magic_stone(row: &ItemDataRow) -> bool {
    row.type_ids() == Some(TID_MAGIC_STONE)
}

/// The row's twelve ladder entries, or `None` when it carries none (an
/// advanced elixir's `-1` columns, or a row without the pairs at all).
fn ladder(row: &ItemDataRow) -> Option<[u8; REINFORCE_LADDER_LEN]> {
    let mut out = [0u8; REINFORCE_LADDER_LEN];
    for (block, param) in LADDER_PARAMS.into_iter().enumerate() {
        if param_of(row, param)? < 0 {
            return None;
        }
        let bytes = param_bytes_of(row, param)?;
        out[block * 4..block * 4 + 4].copy_from_slice(&bytes);
    }
    Some(out)
}

/// The ladder entry for the step that takes `opt_level` to `opt_level + 1`.
/// `None` past the end of the ladder, and `None` for a zero entry — a zero is
/// not "0 %", it is a row that says nothing about this step.
fn ladder_entry(row: &ItemDataRow, opt_level: u8) -> Option<u32> {
    let index = usize::from(opt_level);
    let value = *ladder(row)?.get(index)?;
    (value > 0).then_some(u32::from(value))
}

/// The `TypeID3` values an elixir accepts, from its two gate columns.
fn type_gate(row: &ItemDataRow) -> Vec<u32> {
    let mut gate = Vec::new();
    for param in TYPE_GATE_PARAMS {
        if param_of(row, param).unwrap_or(-1) < 0 {
            continue;
        }
        let Some(bytes) = param_bytes_of(row, param) else {
            continue;
        };
        gate.extend(bytes.iter().copied().filter(|b| *b > 0).map(u32::from));
    }
    gate
}

/// The success chance the data states for enhancing `equip` (currently at
/// `+opt_level`) with `elixir`, optionally helped by `powder`.
///
/// `None` whenever the data does not answer: a non-equipment target, an item
/// that is not an elixir, an elixir whose type gate excludes this equipment
/// class (vanilla's `UIIT_MSG_REINFORCERR_RECIPE_MISMATCH`,
/// `textuisystem.txt:2134`), a step past the ladder, or a row with no ladder.
/// A missing row is the original's answer, not an error, so the caller shows
/// nothing rather than a `0 %`.
pub fn reinforce_chance(
    equip: &ItemDataRow,
    opt_level: u8,
    elixir: &ItemDataRow,
    powder: Option<&ItemDataRow>,
) -> Option<ReinforceChance> {
    let (3, 1, tid3, _) = equip.type_ids()? else {
        return None;
    };
    if !is_elixir(elixir) || !type_gate(elixir).contains(&tid3) {
        return None;
    }
    let percent = ladder_entry(elixir, opt_level)?;
    Some(ReinforceChance {
        percent,
        powder_bonus: powder.and_then(|powder| powder_bonus(equip, opt_level, powder)),
    })
}

/// The bonus a Lucky Powder adds for this step, in percentage points, or
/// `None` when the powder does not apply. The powder's `Param1` is its own
/// degree (`1..12`, plain int — not a packed quad), and vanilla rejects a
/// powder whose degree differs from the equipment's
/// (`UIIT_MSG_REINFORCERR_EQUIPCLASS_MISMATCH_PROB_UP`,
/// `textuisystem.txt:2135`), so a mismatch simply contributes nothing.
fn powder_bonus(equip: &ItemDataRow, opt_level: u8, powder: &ItemDataRow) -> Option<u32> {
    if !is_lucky_powder(powder) {
        return None;
    }
    let degree = i64::from(equip.degree()?);
    if param_of(powder, 1)? != degree {
        return None;
    }
    ladder_entry(powder, opt_level)
}

#[cfg(test)]
mod test {
    use super::*;

    /// Rows out of `server_dep/silkroad/textdata/itemdata*.txt`. An archive
    /// is not available in CI, so the raw column values are pinned here and
    /// the code under test decodes them exactly as it decodes a shipped
    /// file.
    mod fixture {
        /// `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_WEAPON_A`, itemdata id 3675:
        /// `Param1 = 100663296` = `[6,0,0,0]` (tid3 6 = weapon),
        /// ladder `25,20,15,10 | 10,10,10,10 | 10,5,5,5`.
        pub const ELIXIR_WEAPON_A: (i64, [i64; 3], i64) =
            (100663296, [420744970, 168430090, 168101125], -1);
        /// `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_ARMOR_B`, id 3681:
        /// `Param1 = 16909056` = `[1,2,3,0]` (the CH armour classes),
        /// `Param5 = 151653120` = `[9,10,11,0]` (the EU ones),
        /// ladder `50,40,30,19 | 17,17,17,17 | 17,12,12,12`.
        pub const ELIXIR_ARMOR_B: (i64, [i64; 3], i64) =
            (16909056, [841489939, 286331153, 286002188], 151653120);
        /// `ITEM_ETC_ARCHEMY_REINFORCE_PROB_UP_A_01`, id 3683: `Param1 = 1`
        /// (its degree, a plain int), bonus ladder
        /// `50,30,20,8 | 8,7,6,6 | 6,6,4,2`. All 24 powder rows carry the
        /// identical ladder.
        pub const POWDER_DEGREE_1: (i64, [i64; 3], i64) =
            (1, [840832008, 134678022, 101057538], -1);
        /// `ITEM_ETC_ARCHEMY_REINFORCE_PROB_UP_A_03`, id 3685 — same ladder,
        /// degree 3.
        pub const POWDER_DEGREE_3: (i64, [i64; 3], i64) =
            (3, [840832008, 134678022, 101057538], -1);
        /// Any of the 192 advanced elixirs (TID `3.3.10.4`): the ladder columns
        /// are `-1`, i.e. that chance is server-only.
        pub const ADVANCED_ELIXIR: (i64, [i64; 3], i64) = (100663296, [-1, -1, -1], -1);
    }

    /// Build a row of the itemdata width with the type tuple and the five
    /// alchemy `Param` columns set — the same field indices the shipped file
    /// uses (`Param<n>` at `118 + 2*(n-1)`).
    fn alchemy_row(tid: (u32, u32, u32, u32), params: (i64, [i64; 3], i64)) -> ItemDataRow {
        let mut fields = vec![String::from("0"); 161];
        for (index, value) in [(9, tid.0), (10, tid.1), (11, tid.2), (12, tid.3)] {
            fields[index] = value.to_string();
        }
        fields[118] = params.0.to_string();
        for (block, value) in params.1.into_iter().enumerate() {
            fields[120 + block * 2] = value.to_string();
        }
        fields[126] = params.2.to_string();
        ItemDataRow(fields)
    }

    /// Equipment of `tid3` and `ItemClass` -> degree (col 61; `degree =
    /// ceil(class/3)`, itemdata.rs).
    fn equipment(tid3: u32, item_class: u32) -> ItemDataRow {
        let mut fields = vec![String::from("0"); 161];
        for (index, value) in [(9, 3), (10, 1), (11, tid3), (12, 2)] {
            fields[index] = value.to_string();
        }
        fields[61] = item_class.to_string();
        ItemDataRow(fields)
    }

    fn elixir(row: (i64, [i64; 3], i64)) -> ItemDataRow {
        alchemy_row((3, 3, 10, 1), row)
    }

    fn powder(row: (i64, [i64; 3], i64)) -> ItemDataRow {
        alchemy_row((3, 3, 10, 2), row)
    }

    /// The whole point: a known combination's chance is the elixir row's own
    /// ladder entry. Weapon A at +0 -> 25 %, at +3 -> 10 %, at +11 -> 5 %.
    #[test]
    fn a_known_combination_reads_its_chance_out_of_the_row() {
        let sword = equipment(6, 1);
        let elixir = elixir(fixture::ELIXIR_WEAPON_A);
        let chance = |plus| reinforce_chance(&sword, plus, &elixir, None).map(|c| c.percent);
        assert_eq!(chance(0), Some(25));
        assert_eq!(chance(1), Some(20));
        assert_eq!(chance(2), Some(15));
        assert_eq!(chance(3), Some(10));
        assert_eq!(chance(8), Some(10));
        assert_eq!(chance(9), Some(5));
        assert_eq!(chance(11), Some(5));
        // past the twelve-entry ladder the data says nothing
        assert_eq!(chance(12), None);
        assert_eq!(chance(200), None);
    }

    /// The European gate lives in `Param5`, so an EU armour piece matches the
    /// same row a CH one does.
    #[test]
    fn both_type_gate_columns_admit_their_classes() {
        let armor_b = elixir(fixture::ELIXIR_ARMOR_B);
        for tid3 in [1, 2, 3, 9, 10, 11] {
            assert_eq!(
                reinforce_chance(&equipment(tid3, 1), 0, &armor_b, None).map(|c| c.percent),
                Some(50),
                "tid3 {tid3} is in the row's gate"
            );
        }
        // a weapon is not: vanilla's RECIPE_MISMATCH case, and we show nothing
        assert_eq!(reinforce_chance(&equipment(6, 1), 0, &armor_b, None), None);
        assert_eq!(reinforce_chance(&equipment(4, 1), 0, &armor_b, None), None);
    }

    /// An unknown combination shows nothing — no `0 %`, no placeholder.
    #[test]
    fn an_unknown_combination_yields_nothing() {
        let sword = equipment(6, 1);
        // an advanced elixir carries no ladder at all (server-only chance)
        assert_eq!(
            reinforce_chance(&sword, 0, &elixir(fixture::ADVANCED_ELIXIR), None),
            None
        );
        // the "elixir" slot holding something that is not an elixir
        assert_eq!(
            reinforce_chance(&sword, 0, &powder(fixture::POWDER_DEGREE_1), None),
            None
        );
        // a non-equipment target (a stack of powder in the equipment slot)
        assert_eq!(
            reinforce_chance(
                &powder(fixture::POWDER_DEGREE_1),
                0,
                &elixir(fixture::ELIXIR_WEAPON_A),
                None
            ),
            None
        );
        // and a row that has no Param columns at all must not panic
        let short = ItemDataRow(vec![String::from("0"); 53]);
        assert_eq!(reinforce_chance(&short, 0, &short, None), None);
        assert_eq!(
            reinforce_chance(&sword, 0, &elixir(fixture::ELIXIR_WEAPON_A), Some(&short)),
            Some(ReinforceChance {
                percent: 25,
                powder_bonus: None,
            })
        );
    }

    /// A matched powder contributes its own ladder entry, and it stays a
    /// separate figure because the combination rule is unknown.
    #[test]
    fn a_matched_powder_adds_its_own_ladder_entry() {
        let sword = equipment(6, 1); // ItemClass 1 -> degree 1
        let elixir = elixir(fixture::ELIXIR_WEAPON_A);
        let powder_1 = powder(fixture::POWDER_DEGREE_1);
        assert_eq!(
            reinforce_chance(&sword, 0, &elixir, Some(&powder_1)),
            Some(ReinforceChance {
                percent: 25,
                powder_bonus: Some(50),
            })
        );
        assert_eq!(
            reinforce_chance(&sword, 3, &elixir, Some(&powder_1)),
            Some(ReinforceChance {
                percent: 10,
                powder_bonus: Some(8),
            })
        );
        // degree mismatch is vanilla's EQUIPCLASS_MISMATCH_PROB_UP: the powder
        // contributes nothing, and the elixir's own chance still shows
        assert_eq!(
            reinforce_chance(&sword, 0, &elixir, Some(&powder(fixture::POWDER_DEGREE_3))),
            Some(ReinforceChance {
                percent: 25,
                powder_bonus: None,
            })
        );
        // degree-3 equipment (ItemClass 7..9) matches the degree-3 powder
        let degree_3 = equipment(6, 7);
        assert_eq!(degree_3.degree(), Some(3));
        assert_eq!(
            reinforce_chance(
                &degree_3,
                1,
                &elixir,
                Some(&powder(fixture::POWDER_DEGREE_3))
            )
            .and_then(|c| c.powder_bonus),
            Some(30)
        );
    }

    /// The data caveat, as a test: **no probability is written in this
    /// module.** Change the row and the answer changes; the source holds
    /// column indices and type ids only, so any archive is displayed as
    /// authored.
    #[test]
    fn every_number_comes_from_the_row_and_none_from_the_code() {
        let sword = equipment(6, 1);
        // an invented archive: 99/88/77/66 | 55/44/33/22 | 11/9/8/7
        let invented = elixir((
            100663296,
            [
                i64::from(i32::from_be_bytes([99, 88, 77, 66])),
                i64::from(i32::from_be_bytes([55, 44, 33, 22])),
                i64::from(i32::from_be_bytes([11, 9, 8, 7])),
            ],
            -1,
        ));
        let chance = |plus| reinforce_chance(&sword, plus, &invented, None).map(|c| c.percent);
        assert_eq!(chance(0), Some(99));
        assert_eq!(chance(4), Some(55));
        assert_eq!(chance(11), Some(7));

        // No archive value appears in the source text of this module. Only
        // the non-test half is checked: the fixture constants below are data
        // and are supposed to carry numbers.
        let source = include_str!("probability.rs");
        let module = source
            .split_once("#[cfg(test)]")
            .expect("this file has a test module")
            .0;
        let code_only: String = module
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // No transcribed ladder anywhere in the code half of this file: every
        // numeric literal the code holds is a column index, a TID part or the
        // ladder length, and all of those are <= 12. A transcribed
        // probability would show up here as a literal above 12.
        let bytes = code_only.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            if !bytes[index].is_ascii_digit() {
                index += 1;
                continue;
            }
            let start = index;
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
            // `u8` / `i32` / `u32` are type names, not values
            let is_type_width = start > 0 && matches!(bytes[start - 1], b'u' | b'i');
            let literal: u32 = code_only[start..index].parse().unwrap_or_default();
            // `PARAM1_FIELD` is the one column index above 12 (itemdata
            // field 118); it stays here only until `ItemDataRow` owns it.
            assert!(
                is_type_width
                    || literal <= REINFORCE_LADDER_LEN as u32
                    || literal == PARAM1_FIELD as u32,
                "numeric literal {literal} in the code half is not a column index"
            );
        }
        // what the module *does* hold: column indices and type ids
        assert_eq!(LADDER_PARAMS, [2, 3, 4]);
        assert_eq!(TID_ELIXIR, (3, 3, 10, 1));
        assert_eq!(PARAM1_FIELD, 118);
    }

    /// A zero ladder entry is not "0 %" — it is a row saying nothing.
    #[test]
    fn a_zero_entry_is_absence_not_zero_percent() {
        let sword = equipment(6, 1);
        let zeroed = elixir((100663296, [0, 0, 0], -1));
        assert_eq!(reinforce_chance(&sword, 0, &zeroed, None), None);
    }
}

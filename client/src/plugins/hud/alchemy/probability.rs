//! Alchemy: the reinforce success chance, read out of the shipped `itemdata`.
//!
//! Idea: the enhancement odds are **not** server-only. Every ordinary elixir row
//! carries its own twelve-step success ladder, and every Lucky Powder row
//! carries a twelve-step bonus ladder, in the `(Param<N>, Param<N>_Desc)` pairs
//! at itemdata fields 118..157. So the client can *show* the odds instead of
//! making the player guess, and it can do it without a single number of its own:
//! this module only decodes the rows.
//!
//! The packing, and how the byte order was determined: where a `Param<N>_Desc`
//! reads `"1,2,3,4"`, the paired int is four values, most-significant byte
//! first. `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_WEAPON_A` (id 3675) has
//! `Param2 = 420744970 = 0x19140F0A` -> `25,20,15,10` beside a `Desc` of
//! `"1,2,3,4"`, `Param3 = 168430090` -> `10,10,10,10` (`"5,6,7,8"`) and
//! `Param4 = 168101125` -> `10,5,5,5` (`"9,10,11,12"`). Three independent
//! columns whose decoded values line up with the indices their own `Desc` names,
//! and all 17 elixir rows decode to plausible descending percentages under the
//! same rule.
//!
//! **Data caveat, and it is why nothing here is a constant:** only the
//! *structure* — which column carries what — is stable. The *values*
//! (25/20/15/10 ...) are one archive's authoring and may be tuned. Every number
//! this module reports therefore comes out of the row it was asked about: the
//! module holds column indices and type ids, never a probability.
//!
//! Stated deviation: the vanilla alchemy box shows no percentage. Its `resinfo`
//! pages declare no such control, and `textuisystem.txt` ships no key that
//! formats one. Showing the number is ours; the number itself stays the data's.

use crate::assets::textdata::itemdata::ItemDataRow;

/// `Param<n>` as the big-endian byte view of the signed int: the alchemy rows
/// pack four values into one column. The column itself is read by
/// [`ItemDataRow::param`], which owns the `118 + 2*(n - 1)` arithmetic; this is
/// the byte view its own doc comment points at. A `-1` column is empty, not
/// `[255,255,255,255]`, so callers check the raw value first.
fn param_bytes_of(row: &ItemDataRow, n: usize) -> Option<[u8; 4]> {
    Some(i32::try_from(row.param(n)?).ok()?.to_be_bytes())
}

/// The ladder is twelve entries long, so +12 is the last reachable step: three
/// packed columns of four, and no column states a ceiling of its own.
pub const REINFORCE_LADDER_LEN: usize = 12;

/// TID of an ordinary elixir (`3.3.10.1`, 17 rows) — the rows that carry a
/// ladder.
const TID_ELIXIR: (u32, u32, u32, u32) = (3, 3, 10, 1);
/// TID of Lucky Powder (`3.3.10.2`, 24 rows).
const TID_LUCKY_POWDER: (u32, u32, u32, u32) = (3, 3, 10, 2);
/// TID of a magic stone (`3.3.11.1`, 176 rows). Its counterpart is the attribute
/// stone (TID `3.3.11.2`, 168 rows), and the two are exactly the stone kinds the
/// fuse request selects between.
const TID_MAGIC_STONE: (u32, u32, u32, u32) = (3, 3, 11, 1);
/// The three columns that hold the twelve ladder entries, in index order
/// (`Desc` labels `"1,2,3,4"`, `"5,6,7,8"`, `"9,10,11,12"`).
const LADDER_PARAMS: [usize; 3] = [2, 3, 4];
/// `Param1` and `Param5` are both permitted-`TypeID3` lists, which is what the
/// 17 elixir rows show when read side by side: the weapon and shield recipes
/// name one class in `Param1` (`[6,0,0,0]`, `[4,0,0,0]`), while the armour and
/// accessory recipes name the Chinese classes in `Param1` (`[1,2,3,0]`,
/// `[5,0,0,0]`) and the European ones in `Param5` (`[9,10,11,0]`,
/// `[12,0,0,0]`). A row with only one list leaves the other at `-1` or `0`.
const TYPE_GATE_PARAMS: [usize; 2] = [1, 5];

/// What the data says about one (equipment, elixir, powder) combination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReinforceChance {
    /// The elixir's own success chance for the *next* enhancement step, in
    /// percent, straight out of its ladder.
    pub percent: u32,
    /// The matched Lucky Powder's bonus for the same step, in percentage
    /// points. Kept as a second figure rather than folded into `percent`
    /// because whether the server adds or multiplies is unresolved: no itemdata
    /// column states the combination rule.
    pub powder_bonus: Option<u32>,
}

/// An ordinary elixir (TID `3.3.10.1`). The 192 *advanced* elixirs (TID
/// `3.3.10.4`) are a different TID and carry no ladder at all — every one of
/// them has `Param2..4 = -1` — which is why they never reach this predicate.
pub fn is_elixir(row: &ItemDataRow) -> bool {
    row.type_ids() == Some(TID_ELIXIR)
}

/// A Lucky Powder (TID `3.3.10.2`).
pub fn is_lucky_powder(row: &ItemDataRow) -> bool {
    row.type_ids() == Some(TID_LUCKY_POWDER)
}

/// A magic stone (TID `3.3.11.1`) rather than an attribute stone — the one bit
/// the stone fuse request needs from the material's row.
pub fn is_magic_stone(row: &ItemDataRow) -> bool {
    row.type_ids() == Some(TID_MAGIC_STONE)
}

/// The row's twelve ladder entries, or `None` when it carries none (an advanced
/// elixir's `-1` columns, or a row without the pairs at all).
fn ladder(row: &ItemDataRow) -> Option<[u8; REINFORCE_LADDER_LEN]> {
    let mut out = [0u8; REINFORCE_LADDER_LEN];
    for (block, param) in LADDER_PARAMS.into_iter().enumerate() {
        if row.param(param)? < 0 {
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
        if row.param(param).unwrap_or(-1) < 0 {
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
/// class (the shipped `UIIT_MSG_REINFORCERR_RECIPE_MISMATCH` case,
/// `textuisystem.txt:2134`), a step past the ladder, or a row with no ladder.
/// A missing row is an answer, not an error, so the caller shows nothing rather
/// than a `0 %`.
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

/// The bonus a Lucky Powder adds for this step, in percentage points, or `None`
/// when the powder does not apply. The powder's `Param1` is its own degree
/// (`1..12`, a plain int — not a packed quad), and the shipped
/// `UIIT_MSG_REINFORCERR_EQUIPCLASS_MISMATCH_PROB_UP` (`textuisystem.txt:2135`)
/// is the refusal for a powder whose degree differs from the equipment's, so a
/// mismatch simply contributes nothing.
fn powder_bonus(equip: &ItemDataRow, opt_level: u8, powder: &ItemDataRow) -> Option<u32> {
    if !is_lucky_powder(powder) {
        return None;
    }
    let degree = i64::from(equip.degree()?);
    if powder.param(1)? != degree {
        return None;
    }
    ladder_entry(powder, opt_level)
}

#[cfg(test)]
mod test {
    use super::*;

    /// Raw column values read out of the shipped `itemdata*.txt`. The archive is
    /// not available to the test runner, so the ints are pinned here and the code
    /// under test decodes them exactly as it decodes the file. Each tuple is
    /// `(Param1, [Param2, Param3, Param4], Param5)`.
    mod fixture {
        /// `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_WEAPON_A`, id 3675:
        /// `Param1 = 100663296` = `[6,0,0,0]` (tid3 6 = weapon),
        /// ladder `25,20,15,10 | 10,10,10,10 | 10,5,5,5`.
        pub const ELIXIR_WEAPON_A: (i64, [i64; 3], i64) =
            (100663296, [420744970, 168430090, 168101125], -1);
        /// `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_ARMOR_B`, id 3681:
        /// `Param1 = 16909056` = `[1,2,3,0]` (the Chinese armour classes),
        /// `Param5 = 151653120` = `[9,10,11,0]` (the European ones),
        /// ladder `50,40,30,19 | 17,17,17,17 | 17,12,12,12`.
        pub const ELIXIR_ARMOR_B: (i64, [i64; 3], i64) =
            (16909056, [841489939, 286331153, 286002188], 151653120);
        /// `ITEM_ETC_ARCHEMY_REINFORCE_PROB_UP_A_01`, id 3683: `Param1 = 1`
        /// (its degree, a plain int), bonus ladder `50,30,20,8 | 8,7,6,6 |
        /// 6,6,4,2`. All 24 powder rows carry the identical ladder.
        pub const POWDER_DEGREE_1: (i64, [i64; 3], i64) =
            (1, [840832008, 134678022, 101057538], -1);
        /// `ITEM_ETC_ARCHEMY_REINFORCE_PROB_UP_A_03`, id 3685 — same ladder,
        /// degree 3.
        pub const POWDER_DEGREE_3: (i64, [i64; 3], i64) =
            (3, [840832008, 134678022, 101057538], -1);
        /// `ITEM_ETC_ARCHEMY_UPPER_REINFORCE_RECIPE_WE_A_1`, id 25873, one of the
        /// 192 advanced elixirs (TID `3.3.10.4`): the ladder columns are `-1`,
        /// i.e. that chance is the server's alone.
        pub const ADVANCED_ELIXIR: (i64, [i64; 3], i64) = (100663296, [-1, -1, -1], 0);
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
        alchemy_row(TID_ELIXIR, row)
    }

    fn powder(row: (i64, [i64; 3], i64)) -> ItemDataRow {
        alchemy_row(TID_LUCKY_POWDER, row)
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
    /// same row a Chinese one does.
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
        // a weapon and a shield are not: the shipped RECIPE_MISMATCH case
        assert_eq!(reinforce_chance(&equipment(6, 1), 0, &armor_b, None), None);
        assert_eq!(reinforce_chance(&equipment(4, 1), 0, &armor_b, None), None);
    }

    /// Nothing the data does not answer becomes a number.
    #[test]
    fn an_unanswered_combination_yields_nothing() {
        let sword = equipment(6, 1);
        // an advanced elixir: right type gate, no ladder
        assert_eq!(
            reinforce_chance(&sword, 0, &elixir(fixture::ADVANCED_ELIXIR), None),
            None
        );
        // a powder is not an elixir, even though it has a ladder
        assert_eq!(
            reinforce_chance(&sword, 0, &powder(fixture::POWDER_DEGREE_1), None),
            None
        );
        // the target must be equipment (3.1.x)
        let not_equipment = alchemy_row((3, 3, 11, 1), fixture::ELIXIR_WEAPON_A);
        assert_eq!(
            reinforce_chance(&not_equipment, 0, &elixir(fixture::ELIXIR_WEAPON_A), None),
            None
        );
    }

    /// A powder of the equipment's own degree adds its ladder entry, and one of
    /// another degree adds nothing — the shipped
    /// EQUIPCLASS_MISMATCH_PROB_UP case. The bonus stays a second figure.
    #[test]
    fn a_matched_powder_adds_its_own_ladder_entry() {
        // ItemClass 1 -> degree 1, ItemClass 7 -> degree 3
        let degree_1 = equipment(6, 1);
        let degree_3 = equipment(6, 7);
        let elixir = elixir(fixture::ELIXIR_WEAPON_A);
        let powder_1 = powder(fixture::POWDER_DEGREE_1);
        let powder_3 = powder(fixture::POWDER_DEGREE_3);

        let matched = reinforce_chance(&degree_1, 0, &elixir, Some(&powder_1)).unwrap();
        assert_eq!((matched.percent, matched.powder_bonus), (25, Some(50)));
        let matched = reinforce_chance(&degree_3, 2, &elixir, Some(&powder_3)).unwrap();
        assert_eq!((matched.percent, matched.powder_bonus), (15, Some(20)));

        // wrong degree: the elixir's own number stands alone
        let mismatched = reinforce_chance(&degree_1, 0, &elixir, Some(&powder_3)).unwrap();
        assert_eq!((mismatched.percent, mismatched.powder_bonus), (25, None));
        // and the bonus is never folded into the percentage
        assert_eq!(matched.percent + matched.powder_bonus.unwrap(), 35);
    }

    /// Every number comes out of the row, so changing a column changes the
    /// answer — this is what says the module holds no probability of its own.
    #[test]
    fn every_number_comes_from_the_row_and_none_from_the_code() {
        let sword = equipment(6, 1);
        // 0x01020304 -> 1,2,3,4 per cent on the first four steps
        let invented = elixir((100663296, [0x0102_0304, 0x0102_0304, 0x0102_0304], -1));
        let chance = |plus| reinforce_chance(&sword, plus, &invented, None).map(|c| c.percent);
        assert_eq!(
            [chance(0), chance(1), chance(2), chance(3)],
            [Some(1), Some(2), Some(3), Some(4)]
        );
        // and the block boundary is the column boundary, not an off-by-one
        assert_eq!([chance(4), chance(8)], [Some(1), Some(1)]);
    }

    /// A zero entry is absence, not "0 %": the packed quads pad with zeroes
    /// (`[10,5,5,5]` beside `[1,2,3,0]`), so a zero must not print.
    #[test]
    fn a_zero_entry_is_absence_not_zero_percent() {
        let sword = equipment(6, 1);
        // ladder 25,20,15,0 | 0,0,0,0 | 0,0,0,0
        let padded = elixir((100663296, [0x1914_0F00, 0, 0], -1));
        let chance = |plus| reinforce_chance(&sword, plus, &padded, None).map(|c| c.percent);
        assert_eq!(
            [chance(0), chance(1), chance(2)],
            [Some(25), Some(20), Some(15)]
        );
        for plus in 3..REINFORCE_LADDER_LEN as u8 {
            assert_eq!(chance(plus), None, "a zero at +{plus} is not a chance");
        }
    }
}

//! Specialty trade goods (`ITEM_ETC_TRADE_*`) — the 43 items a merchant buys
//! in one town and sells in another.
//!
//! **The idea:** there is no `specialtylist`/`tradegoods` table in the client
//! to load. The goods are ordinary `itemdata*.txt` rows, and the only thing
//! that makes them special is their TypeID tuple `(3, 3, 8, 1|2)`. This module
//! is therefore an *index over already-parsed itemdata*, not a file parser: it
//! picks those rows out, derives the trade region from the code name, and
//! deliberately carries **no price**. The authored price column is a
//! placeholder that is identical on all 43 rows (383); the price that matters
//! is the market price the server pushes with `0x30E0` UPDATE_PRICE. A consumer that reads
//! a price out of the item row would be reading a constant.
//!
//! Absence claim with a positive control, over the shipped
//! `Media/server_dep/silkroad/textdata/` (all 154 files): the
//! strings `SPECIALTYLIST` / `TRADEGOODS` appear in 0 files, while the same
//! loop finds `SPECIALTY` in 6 and `TRADE_GOODS` in 2 — the read path works,
//! the table really does not exist.

use std::collections::BTreeMap;

use crate::assets::textdata::itemdata::{ItemData, ItemDataRow};

/// itemdata column 26 (0-based), the authored buy price ("column 27" when
/// counted 1-based). Read here
/// only to *prove* it is a placeholder (see [`authored_price`]); no consumer
/// should take a specialty price from it.
const ITEMDATA_PRICE_COLUMN: usize = 26;

/// The placeholder every specialty good carries in the price column.
///
/// `[V]` census over the shipped `itemdata*.txt` (10 shards, 12 061 enabled
/// rows): all 43 `(3,3,8,1|2)` rows have price 383 and sell price
/// 191 — no variation at all, while the same column yields 348 on the
/// neighbouring `(3,3,8,0)` event food rows (positive control on the same read
/// path). The real price arrives over the wire (`0x30E0`).
pub const PLACEHOLDER_PRICE: u64 = 383;

/// The trade region a good belongs to, taken from the `ITEM_ETC_TRADE_<REGION>_<nn>`
/// code name — the only place in the row that says where the good is sourced.
///
/// `[V]` all 43 code names match that shape; the eleven tokens below are
/// exactly the ones that occur. Region → town mapping from
/// `textdata/specialnpcdata.txt` + `npcpos.txt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TradeRegion {
    /// Jangan (`NPC_CH_SPECIAL`, ref 2010, region 25000).
    Ch,
    /// Donwhang (`NPC_WC_SPECIAL`, ref 2059, region 26265).
    Wc,
    /// Hotan (`NPC_KT_SPECIAL`, ref 2077, region 23687).
    Kt,
    /// Constantinople (`NPC_EU_SPECIAL`, ref 7500, region 26959).
    Eu,
    /// Samarkand (`NPC_CA_SPECIAL`, ref 7535, region 27244).
    Ca,
    /// Taklamakan (`NPC_TK_SPECIAL`, ref 7569, region 26753).
    Tk,
    /// Roc Mountain (`NPC_RM_SPECIAL`, ref 7567, region 23411).
    Rm,
    /// Alexandria (`NPC_AM_SPECIAL`, ref 7568 — the one NPC with no
    /// `npcpos.txt` row).
    Am,
    /// Egypt / "SD" area (`NPC_SD_M_AREA_SPECIAL*`, refs 26794/26811/26812).
    Sd,
    /// Venice ("VE").
    Ve,
    /// "DE" area.
    De,
}

impl TradeRegion {
    /// Parse the region token out of a specialty good's code name
    /// (`ITEM_ETC_TRADE_CH_01` → [`TradeRegion::Ch`]).
    pub fn from_code_name(code_name: &str) -> Option<Self> {
        let rest = code_name.strip_prefix("ITEM_ETC_TRADE_")?;
        let token = rest.split('_').next()?;
        Some(match token {
            "CH" => Self::Ch,
            "WC" => Self::Wc,
            "KT" => Self::Kt,
            "EU" => Self::Eu,
            "CA" => Self::Ca,
            "TK" => Self::Tk,
            "RM" => Self::Rm,
            "AM" => Self::Am,
            "SD" => Self::Sd,
            "VE" => Self::Ve,
            "DE" => Self::De,
            _ => return None,
        })
    }
}

/// Whether an itemdata TypeID tuple is a **specialty trade good**.
///
/// `[V]` `TID (3, 3, 8, 1)` = 27 first-generation goods, `(3, 3, 8, 2)` = 16
/// second-generation goods (CH2/WC2/TK/RM/AM), 43 in total.
/// **`t4 == 0` is not a trade good**: those 12 rows are event food
/// (`ITEM_ETC_E060209_WHITE_DUMPLING` …, price 348) — the negative control
/// that keeps this predicate off `TID3 == 8` alone.
pub fn is_specialty_good(type_ids: (u32, u32, u32, u32)) -> bool {
    matches!(type_ids, (3, 3, 8, 1 | 2))
}

/// The authored (placeholder) price of an itemdata row — see
/// [`PLACEHOLDER_PRICE`]. Public so the test that proves the placeholder can
/// read it; **not** a price source for the trade UI.
pub fn authored_price(row: &ItemDataRow) -> Option<u64> {
    row.0.get(ITEMDATA_PRICE_COLUMN)?.trim().parse().ok()
}

/// One specialty good. Note what is *absent*: no price. The market price and
/// its fluctuation come from `0x30E0` / the trade-info window, never from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialtyGood {
    pub ref_id: i32,
    /// `ITEM_ETC_TRADE_<REGION>_<nn>`.
    pub code_name: String,
    /// `SN_*` key for the display name; resolve through the textdata name
    /// tables (`TextdataNames`), exactly like every other item.
    pub name_key: Option<String>,
    /// `media://icon/...` path of the inventory icon.
    pub icon_path: Option<String>,
    pub region: TradeRegion,
    /// TypeID4: 1 = first goods generation, 2 = the later one.
    pub generation: u32,
    /// Column 57; 40 on all 43 rows (`[V]`) — the sack
    /// stack size, *not* a level requirement (the level column 33 is 0).
    pub max_stack: Option<u32>,
}

/// All specialty goods of the loaded itemdata, keyed by ref id.
#[derive(Debug, Clone, Default)]
pub struct SpecialtyGoods(BTreeMap<i32, SpecialtyGood>);

impl SpecialtyGoods {
    /// Index the specialty goods out of the parsed itemdata table.
    pub fn from_item_data(item_data: &ItemData) -> Self {
        let goods = item_data
            .0
            .iter()
            .filter_map(|(&ref_id, row)| {
                let type_ids = row.type_ids()?;
                if !is_specialty_good(type_ids) {
                    return None;
                }
                let code_name = row.code_name().clone();
                let region = TradeRegion::from_code_name(&code_name)?;
                Some((
                    ref_id,
                    SpecialtyGood {
                        ref_id,
                        code_name,
                        name_key: row.name_key().map(str::to_string),
                        icon_path: row.icon_path(),
                        region,
                        generation: type_ids.3,
                        max_stack: row.max_stack(),
                    },
                ))
            })
            .collect();
        Self(goods)
    }

    pub fn get(&self, ref_id: i32) -> Option<&SpecialtyGood> {
        self.0.get(&ref_id)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Goods sourced in one region, in ref-id order.
    pub fn of_region(&self, region: TradeRegion) -> impl Iterator<Item = &SpecialtyGood> {
        self.0.values().filter(move |g| g.region == region)
    }

    pub fn iter(&self) -> impl Iterator<Item = &SpecialtyGood> {
        self.0.values()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::HashMap;

    /// Every one of the 43 specialty rows of
    /// `Media/server_dep/silkroad/textdata/itemdata_{5000,10000,25000,...}.txt`:
    /// `(ref_id, code_name, TypeID4, price col 26, sell col 31)`. No PK2 is
    /// available in a test run, so the rows live here as constants instead of
    /// whatever archive a machine happens to have.
    const SPECIALTY_ROWS: &[(i32, &str, u32, u64, u64)] = &[
        (2147, "ITEM_ETC_TRADE_CH_01", 1, 383, 191),
        (2148, "ITEM_ETC_TRADE_CH_02", 1, 383, 191),
        (2149, "ITEM_ETC_TRADE_CH_03", 1, 383, 191),
        (2150, "ITEM_ETC_TRADE_CH_04", 1, 383, 191),
        (2151, "ITEM_ETC_TRADE_WC_01", 1, 383, 191),
        (2152, "ITEM_ETC_TRADE_WC_02", 1, 383, 191),
        (2153, "ITEM_ETC_TRADE_WC_03", 1, 383, 191),
        (2154, "ITEM_ETC_TRADE_WC_04", 1, 383, 191),
        (2155, "ITEM_ETC_TRADE_KT_01", 1, 383, 191),
        (2156, "ITEM_ETC_TRADE_KT_02", 1, 383, 191),
        (2157, "ITEM_ETC_TRADE_KT_03", 1, 383, 191),
        (2158, "ITEM_ETC_TRADE_KT_04", 1, 383, 191),
        (7570, "ITEM_ETC_TRADE_CH_05", 2, 383, 191),
        (7571, "ITEM_ETC_TRADE_CH_06", 2, 383, 191),
        (7572, "ITEM_ETC_TRADE_CH_07", 2, 383, 191),
        (7573, "ITEM_ETC_TRADE_WC_05", 2, 383, 191),
        (7574, "ITEM_ETC_TRADE_WC_06", 2, 383, 191),
        (7575, "ITEM_ETC_TRADE_WC_07", 2, 383, 191),
        (7576, "ITEM_ETC_TRADE_TK_01", 2, 383, 191),
        (7577, "ITEM_ETC_TRADE_TK_02", 2, 383, 191),
        (7578, "ITEM_ETC_TRADE_TK_03", 2, 383, 191),
        (7579, "ITEM_ETC_TRADE_RM_01", 2, 383, 191),
        (7580, "ITEM_ETC_TRADE_RM_02", 2, 383, 191),
        (7581, "ITEM_ETC_TRADE_RM_03", 2, 383, 191),
        (7582, "ITEM_ETC_TRADE_AM_01", 2, 383, 191),
        (7583, "ITEM_ETC_TRADE_AM_02", 2, 383, 191),
        (7584, "ITEM_ETC_TRADE_AM_03", 2, 383, 191),
        (10394, "ITEM_ETC_TRADE_EU_01", 1, 383, 191),
        (10395, "ITEM_ETC_TRADE_EU_02", 1, 383, 191),
        (10396, "ITEM_ETC_TRADE_EU_03", 1, 383, 191),
        (10397, "ITEM_ETC_TRADE_EU_04", 1, 383, 191),
        (10398, "ITEM_ETC_TRADE_CA_01", 1, 383, 191),
        (10399, "ITEM_ETC_TRADE_CA_02", 1, 383, 191),
        (10400, "ITEM_ETC_TRADE_CA_03", 1, 383, 191),
        (10401, "ITEM_ETC_TRADE_CA_04", 1, 383, 191),
        (24671, "ITEM_ETC_TRADE_SD_01", 1, 383, 191),
        (24672, "ITEM_ETC_TRADE_SD_02", 1, 383, 191),
        (24673, "ITEM_ETC_TRADE_SD_03", 1, 383, 191),
        (24674, "ITEM_ETC_TRADE_SD_04", 1, 383, 191),
        (24675, "ITEM_ETC_TRADE_VE_01", 1, 383, 191),
        (24676, "ITEM_ETC_TRADE_VE_02", 1, 383, 191),
        (24677, "ITEM_ETC_TRADE_DE_01", 1, 383, 191),
        (24678, "ITEM_ETC_TRADE_DE_02", 1, 383, 191),
    ];

    /// Build an itemdata row with the columns this module reads.
    fn row(ref_id: i32, code_name: &str, t4: u32, price: u64, sell: u64) -> ItemDataRow {
        let mut fields = vec![String::new(); 58];
        fields[1] = ref_id.to_string();
        fields[2] = code_name.to_string();
        fields[5] = format!("SN_{code_name}");
        for (i, v) in [(9, 3), (10, 3), (11, 8), (12, t4)] {
            fields[i] = v.to_string();
        }
        fields[14] = "3".into(); // Country, 3 on all 43 rows
        fields[17] = "1".into(); // CanSell
        fields[26] = price.to_string();
        fields[31] = sell.to_string();
        fields[54] = format!(
            "item\\etc\\{}.ddj",
            code_name.trim_start_matches("ITEM_ETC_").to_lowercase()
        );
        fields[57] = "40".into(); // MaxStack, 40 on all 43 rows
        ItemDataRow(fields)
    }

    fn table() -> ItemData {
        ItemData(HashMap::from_iter(SPECIALTY_ROWS.iter().map(
            |&(id, code, t4, price, sell)| (id, row(id, code, t4, price, sell)),
        )))
    }

    #[test]
    fn indexes_all_43_goods() {
        let goods = SpecialtyGoods::from_item_data(&table());
        assert_eq!(goods.len(), 43);
        // Jangan silk, verbatim from itemdata_5000.txt.
        let silk = goods.get(2147).expect("2147 White Silk");
        assert_eq!(silk.code_name, "ITEM_ETC_TRADE_CH_01");
        assert_eq!(silk.name_key.as_deref(), Some("SN_ITEM_ETC_TRADE_CH_01"));
        assert_eq!(
            silk.icon_path.as_deref(),
            Some("media://icon/item/etc/trade_ch_01.ddj")
        );
        assert_eq!(silk.region, TradeRegion::Ch);
        assert_eq!(silk.generation, 1);
        assert_eq!(silk.max_stack, Some(40));
        // Second goods generation, same region.
        assert_eq!(goods.get(7570).map(|g| g.generation), Some(2));
        assert_eq!(goods.get(7570).map(|g| g.region), Some(TradeRegion::Ch));
    }

    #[test]
    fn region_counts_match_the_data() {
        let goods = SpecialtyGoods::from_item_data(&table());
        let count = |r| goods.of_region(r).count();
        // 4 first-generation + 3 second-generation in each of the two
        // start-town regions; the later areas carry fewer.
        assert_eq!(count(TradeRegion::Ch), 7);
        assert_eq!(count(TradeRegion::Wc), 7);
        assert_eq!(count(TradeRegion::Kt), 4);
        assert_eq!(count(TradeRegion::Eu), 4);
        assert_eq!(count(TradeRegion::Ca), 4);
        assert_eq!(count(TradeRegion::Tk), 3);
        assert_eq!(count(TradeRegion::Rm), 3);
        assert_eq!(count(TradeRegion::Am), 3);
        assert_eq!(count(TradeRegion::Sd), 4);
        assert_eq!(count(TradeRegion::Ve), 2);
        assert_eq!(count(TradeRegion::De), 2);
        assert_eq!(
            [
                TradeRegion::Ch,
                TradeRegion::Wc,
                TradeRegion::Kt,
                TradeRegion::Eu,
                TradeRegion::Ca,
                TradeRegion::Tk,
                TradeRegion::Rm,
                TradeRegion::Am,
                TradeRegion::Sd,
                TradeRegion::Ve,
                TradeRegion::De,
            ]
            .map(count)
            .iter()
            .sum::<usize>(),
            43
        );
    }

    /// The point of the whole module: the itemdata price is a constant, so a
    /// specialty price can only come from the wire (`0x30E0`).
    #[test]
    fn price_does_not_come_from_the_item_row() {
        let prices: Vec<u64> = SPECIALTY_ROWS
            .iter()
            .map(|&(id, code, t4, price, sell)| {
                authored_price(&row(id, code, t4, price, sell)).expect("price column")
            })
            .collect();
        assert_eq!(prices.len(), 43);
        assert!(
            prices.iter().all(|&p| p == PLACEHOLDER_PRICE),
            "all 43 rows carry the same authored price"
        );
        // ... and the sell column is just as constant (191 on all 43).
        assert_eq!(
            SPECIALTY_ROWS.iter().map(|r| r.4).collect::<Vec<_>>(),
            vec![191u64; 43]
        );
        // The struct we hand out therefore has no price field at all: this is
        // the compile-time half of the same statement.
        let goods = SpecialtyGoods::from_item_data(&table());
        let good = goods.get(2147).expect("2147");
        assert_eq!(
            good,
            &SpecialtyGood {
                ref_id: 2147,
                code_name: "ITEM_ETC_TRADE_CH_01".into(),
                name_key: Some("SN_ITEM_ETC_TRADE_CH_01".into()),
                icon_path: Some("media://icon/item/etc/trade_ch_01.ddj".into()),
                region: TradeRegion::Ch,
                generation: 1,
                max_stack: Some(40),
            }
        );
    }

    #[test]
    fn event_food_is_not_a_trade_good() {
        // ITEM_ETC_E060209_WHITE_DUMPLING, ref 7554, verbatim: (3,3,8,0),
        // price 348 — the row that makes "TID3 == 8" the wrong predicate.
        assert!(!is_specialty_good((3, 3, 8, 0)));
        assert!(is_specialty_good((3, 3, 8, 1)));
        assert!(is_specialty_good((3, 3, 8, 2)));
        // Neighbouring families that must not match.
        assert!(!is_specialty_good((3, 3, 9, 0)), "quest items");
        assert!(!is_specialty_good((3, 1, 7, 1)), "job suit");
        assert!(!is_specialty_good((3, 3, 3, 2)), "transport scroll");

        let dumpling = {
            let mut r = row(7554, "ITEM_ETC_E060209_WHITE_DUMPLING", 0, 348, 174);
            r.0[12] = "0".into();
            r
        };
        let table = ItemData(HashMap::from([(7554, dumpling)]));
        assert_eq!(SpecialtyGoods::from_item_data(&table).len(), 0);
    }

    #[test]
    fn region_token_parsing_rejects_other_items() {
        assert_eq!(
            TradeRegion::from_code_name("ITEM_ETC_TRADE_SD_03"),
            Some(TradeRegion::Sd)
        );
        // The quest delivery boxes (ITEM_QNO_TRADE_*) share the word TRADE but
        // are a different family — they must not parse as a region.
        assert_eq!(
            TradeRegion::from_code_name("ITEM_QNO_TRADE_CH_SPECIAL2_1_01"),
            None
        );
        assert_eq!(TradeRegion::from_code_name("ITEM_ETC_TRADE_XX_01"), None);
    }
}

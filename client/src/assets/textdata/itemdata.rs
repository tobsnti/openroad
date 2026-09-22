//!
//! `dead_code` is allowed module-wide: the module models a file format, and parts of it have no consumer in the client yet.
use bevy::asset::Asset;
use bevy::prelude::TypePath;
use std::collections::HashMap;
use std::ops::{Deref, Index};

#[derive(Debug, Clone)]
pub struct ItemDataRow(pub(crate) Vec<String>);

#[derive(Asset, TypePath, Debug, Clone)]
pub struct ItemData(pub HashMap<i32, ItemDataRow>);

#[repr(usize)]
enum ItemdataFields {
    CodeName = 2,
    NameStrId = 5,
    TypeId1 = 9,
    TypeId2 = 10,
    TypeId3 = 11,
    TypeId4 = 12,
    Country = 14,
    /// 1 = the item may be sold to NPCs (0 on quest/mall items).
    CanSell = 17,
    /// Full repair cost (authored; the confirm dialog prorates it by lost
    /// durability — an estimate, the server's deduction is authoritative).
    CostRepair = 27,
    /// NPC sell value per unit (authored, e.g. sword 890 buy → 427 sell;
    /// col 26 `Price` is the BUY price the price-policy table mirrors).
    SellPrice = 31,
    ReqLevel1 = 33,
    ResourcePath = 52,
    DropResourcePath = 53,
    IconPath = 54,
    MaxStack = 57,
    /// Required sex: 0 = woman, 1 = man, 2 = universal (weapons/shields).
    Gender = 58,
    ItemClass = 61,
    Range = 94,
    /// RefItemData Param1/Desc1_128 (first of the 20 param/desc pairs at
    /// 118..157). `Desc1_128` = the summoned character's code name
    /// (`ITEM_COS_T_HORSE1` → `COS_T_HORSE1`).
    ///
    /// **`Param1` is a rent duration in minutes on the *pet-scroll* family
    /// only, not on mount/transport scrolls.** Over `itemdata*.txt` (all 10
    /// shards, 12,061 rows):
    ///
    /// | TID | rows | `Param1` |
    /// |---|---|---|
    /// | `3/3/3/2` (ride+transport scrolls) | 44 | **0** on 43, `-1` on `ITEM_ETC_AUTOMOB` |
    /// | `3/2/1/1` (growth-pet flute) | 10 | `-1` (permanent) |
    /// | `3/2/1/2` (grab-pet scroll) | 20 | **4320** (= 3 d) on 5, **40320** (= 28 d) on 15 |
    /// | `3/3/12/1` (guild-guard scroll) | 15 | 20 — unit unknown |
    ///
    /// Positive control on the same read path: this field yields 30,000 on the
    /// `3/3/3/1` return scrolls and 3,600 on the `3/3/3/10` hour buffs, so the
    /// zeros are the authored value, not a misread column. The old comment
    /// claimed the minutes for the COS summon scrolls, and a `cos_rent_minutes()`
    /// accessor read them there — it was removed with this correction;
    /// read the column through
    /// [`ItemDataRow::param`] when a consumer for the pet-scroll rental
    /// actually exists.
    Param1 = 118,
    Desc1 = 119,
    /// Desc2_128: on laddered mount scrolls the comma-separated level-tier
    /// list — 13 of the 44 `3/3/3/2` rows carry one
    /// (`ITEM_COS_C_OSTRICH_SCROLL` → `5,10,20,30,45,60,75,90,105,120`,
    /// `ITEM_COS_T_WHITEELEPHANT_SCROLL` → `20,30,...,120`), the other 31 and
    /// all 20 grab-pet scrolls carry `xxx`.
    Desc2 = 121,
}

/// The three Seal grades of v1.188's `ItemRare.txt`, ascending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealTier {
    Star,
    Moon,
    Sun,
}

impl SealTier {
    /// The grade's display name, as the tooltip footer prints it.
    pub fn label(self) -> &'static str {
        match self {
            SealTier::Star => "Seal of Star",
            SealTier::Moon => "Seal of Moon",
            SealTier::Sun => "Seal of Sun",
        }
    }
}

/// A `(lower, upper)` white-stat column pair of RefItemData. The actual value
/// on a concrete item lies between the two bounds, selected by the item's
/// 5-bit-per-stat `variance` roll (see the inventory tooltip).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
pub enum StatRange {
    Durability, // 63/64
    Defense,    // 65/66 (PD, armor & shield physical defense)
    // Weapon-only stat columns.
    PhyAtkMin,       // 95/96
    PhyAtkMax,       // 97/98
    MagAtkMin,       // 100/101
    MagAtkMax,       // 102/103
    PhyReinforceMin, // 105/106 (weapon phys reinforce range, value / 10 = %)
    PhyReinforceMax, // 107/108
    MagReinforceMin, // 109/110 (weapon mag reinforce range)
    MagReinforceMax, // 111/112
    AttackRate,      // 113/114 (hit rate)
    Critical,        // 116/117
    // Armor / shield stat columns (weapon columns above are zero for these).
    BlockRate,         // 74/75 (shield block rate)
    ArmorPhyReinforce, // 82/83 (single lo/hi, value / 10 = %)
    ArmorMagReinforce, // 84/85 (single lo/hi, value / 10 = %)
    // Accessory stat columns (earring/necklace/ring): absorption percentages.
    PhyAbsorb, // 71/72 (single lo/hi, %)
    MagAbsorb, // 79/80 (single lo/hi, %)
}

impl StatRange {
    fn columns(self) -> (usize, usize) {
        match self {
            StatRange::Durability => (63, 64),
            StatRange::Defense => (65, 66),
            StatRange::PhyAtkMin => (95, 96),
            StatRange::PhyAtkMax => (97, 98),
            StatRange::MagAtkMin => (100, 101),
            StatRange::MagAtkMax => (102, 103),
            StatRange::PhyReinforceMin => (105, 106),
            StatRange::PhyReinforceMax => (107, 108),
            StatRange::MagReinforceMin => (109, 110),
            StatRange::MagReinforceMax => (111, 112),
            StatRange::AttackRate => (113, 114),
            StatRange::Critical => (116, 117),
            StatRange::BlockRate => (74, 75),
            StatRange::ArmorPhyReinforce => (82, 83),
            StatRange::ArmorMagReinforce => (84, 85),
            StatRange::PhyAbsorb => (71, 72),
            StatRange::MagAbsorb => (79, 80),
        }
    }
}

impl Index<ItemdataFields> for Vec<String> {
    type Output = String;

    fn index(&self, index: ItemdataFields) -> &Self::Output {
        &self[index as usize]
    }
}

impl Deref for ItemData {
    type Target = HashMap<i32, ItemDataRow>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ItemDataRow {
    /// Some items have no 3d resource (path is empty or a placeholder like
    /// "xxx"), in which case there is nothing to render.
    pub fn resource_path(&self) -> Option<String> {
        Self::bsr_path(&self.0[ItemdataFields::ResourcePath])
    }

    /// The ground-drop model (column 53): gold piles use the money models,
    /// everything else a per-category drop bag — distinct from the inventory
    /// model in [`Self::resource_path`], which is "xxx" for etc items.
    pub fn drop_resource_path(&self) -> Option<String> {
        Self::bsr_path(&self.0[ItemdataFields::DropResourcePath])
    }

    /// Some items have no 3d resource (path is empty or a placeholder like
    /// "xxx"), in which case there is nothing to render.
    fn bsr_path(path: &str) -> Option<String> {
        if path.is_empty() || path == "xxx" {
            return None;
        }
        Some(format!("data://res/{}", path.replace("\\", "/")))
    }

    pub fn code_name(&self) -> &String {
        &self.0[ItemdataFields::CodeName]
    }

    /// A column as a trimmed string, `None` when the row is too short or the
    /// value is empty / the `xxx` placeholder.
    fn field(&self, index: usize) -> Option<&str> {
        let value = self.0.get(index)?.trim();
        (!value.is_empty() && value != "xxx" && value != "0").then_some(value)
    }

    /// Like [`Self::field`] but keeps `"0"` (a valid numeric value).
    fn numeric_field(&self, index: usize) -> Option<f32> {
        let value = self.0.get(index)?.trim();
        value.parse::<f32>().ok()
    }

    /// UI icon of the item as an asset path (`media://icon/...`), or `None`
    /// when the column is empty or a placeholder.
    pub fn icon_path(&self) -> Option<String> {
        let path = self.field(ItemdataFields::IconPath as usize)?;
        Some(format!(
            "media://icon/{}",
            path.replace('\\', "/").to_lowercase()
        ))
    }

    /// Key of the item's display name in the textdata name tables (`SN_*`).
    pub fn name_key(&self) -> Option<&str> {
        self.field(ItemdataFields::NameStrId as usize)
    }

    /// Level required to use/equip the item (`None` when unrestricted).
    pub fn required_level(&self) -> Option<u32> {
        let level = self.numeric_field(ItemdataFields::ReqLevel1 as usize)? as u32;
        (level > 0).then_some(level)
    }

    /// The item's degree (1-based tier shown in tooltips), from the ItemClass
    /// column. Only meaningful for equipment.
    pub fn degree(&self) -> Option<u32> {
        let class = self.numeric_field(ItemdataFields::ItemClass as usize)? as u32;
        (class > 0).then(|| class.div_ceil(3))
    }

    /// Maximum stack count of the item (1 for unstackables).
    pub fn max_stack(&self) -> Option<u32> {
        Some(self.numeric_field(ItemdataFields::MaxStack as usize)? as u32)
    }

    /// Authored full repair cost (col 27; sword: 198 = 890 × ~0.22).
    pub fn cost_repair(&self) -> Option<u64> {
        Some(self.numeric_field(ItemdataFields::CostRepair as usize)? as u64)
    }

    /// Per-unit NPC sell value (col 31), when the item is sellable at all
    /// (col 17 `CanSell`). Verified against the live archive: 890→427 for
    /// the degree-1 sword, exactly the gold vanilla pays.
    pub fn sell_price(&self) -> Option<u64> {
        if self.numeric_field(ItemdataFields::CanSell as usize)? == 0.0 {
            return None;
        }
        Some(self.numeric_field(ItemdataFields::SellPrice as usize)? as u64)
    }

    /// Origin column: 0 = Chinese, 1 = European (equipment tooltip race line).
    pub fn country(&self) -> Option<u32> {
        Some(self.numeric_field(ItemdataFields::Country as usize)? as u32)
    }

    /// Required sex for equipment (col 58): `"Woman"` (0) or `"Man"` (1);
    /// `None` for universal items (2 — weapons, shields, accessories) which
    /// carry no sex line in the tooltip.
    pub fn gender(&self) -> Option<&'static str> {
        match self.numeric_field(ItemdataFields::Gender as usize)? as u32 {
            0 => Some("Woman"),
            1 => Some("Man"),
            _ => None,
        }
    }

    /// Whether this is a "rare"/Seal-grade item — a separate itemdata entry
    /// with a `_RARE` code-name suffix (the tooltip colors its name gold).
    pub fn is_rare(&self) -> bool {
        self.code_name().ends_with("_RARE")
    }

    /// Which Seal grade this item carries, from the code-name tier suffix.
    ///
    /// There is **no per-item rarity field** anywhere — not in itemdata and not
    /// in the 0x3013 stream (`docs/formats/textdata-itemdata.md:73-74`):
    /// Seal-of-X items are separate itemdata entries distinguished only by the
    /// suffix. This lives here rather than in the tooltip because two surfaces
    /// now need it — the tooltip's footer line and the inventory slot's glow —
    /// and a second copy of the suffix table is how they would come to disagree
    /// about what a given item is.
    pub fn seal_tier(&self) -> Option<SealTier> {
        if !self.is_rare() {
            return None;
        }
        let code = self.code_name();
        Some(if code.ends_with("_C_RARE") {
            SealTier::Sun
        } else if code.ends_with("_B_RARE") {
            SealTier::Moon
        } else {
            // A bare `_RARE` with no tier letter. All three tiers exist in the
            // v1.188 `ItemRare.txt`, and Star is the ungraded one.
            SealTier::Star
        })
    }

    /// Weapon attack reach in **world units** — col 94 (`Range`) verbatim,
    /// undivided. World units are what every position and distance in the
    /// client is expressed in, so this is the form combat consumes directly.
    ///
    /// One value per weapon class across all 10 `itemdata*.txt`: dagger 3,
    /// sword/blade/axe/rod/staff/harp 6, spear/glaive/2h sword 18, **bow and
    /// crossbow 180**. Note these are reach past the bodies, not centre to
    /// centre — see `EquippedWeapon::engagement_reach`,
    /// which floors them for that reason.
    pub fn attack_reach(&self) -> Option<f32> {
        let range = self.numeric_field(ItemdataFields::Range as usize)?;
        (range > 0.0).then_some(range)
    }

    /// The same reach in **display units**, which the item tooltip labels "m".
    ///
    /// Defined on top of [`Self::attack_reach`] so the two cannot drift apart:
    /// never multiply this back up to get world units, call `attack_reach`.
    pub fn attack_distance(&self) -> Option<f32> {
        /// A display unit is ten world units (`docs/formats/worldmap.md`:
        /// one region spans 192 du = 1920 world units).
        const WORLD_UNITS_PER_DISPLAY_UNIT: f32 = 10.0;
        Some(self.attack_reach()? / WORLD_UNITS_PER_DISPLAY_UNIT)
    }

    /// A white-stat `(lower, upper)` bound pair, `None` when the columns are
    /// missing, unparseable, or both zero (stat not present on this item).
    pub fn stat_range(&self, stat: StatRange) -> Option<(f32, f32)> {
        let (lo, hi) = stat.columns();
        let bounds = (self.numeric_field(lo)?, self.numeric_field(hi)?);
        (bounds != (0.0, 0.0)).then_some(bounds)
    }

    /// The item's SRO type tuple `(TypeID1..4)`, or `None` if the columns are
    /// unparseable. `TID1 == 3` is the item family; `TID2` splits it into
    /// equipment (1) / summon (2) / expendable (3).
    pub fn type_ids(&self) -> Option<(u32, u32, u32, u32)> {
        let field = |f: ItemdataFields| self.0[f].parse::<u32>().ok();
        Some((
            field(ItemdataFields::TypeId1)?,
            field(ItemdataFields::TypeId2)?,
            field(ItemdataFields::TypeId3)?,
            field(ItemdataFields::TypeId4)?,
        ))
    }

    /// Whether this item is equipment (`TID1/2 == 3/1`: weapons, armor,
    /// accessories, shields). Equipment carries an extra optimization-level byte
    /// in the entity-spawn stream, so the spawn parser branches on this.
    pub fn is_equipment(&self) -> bool {
        matches!(self.type_ids(), Some((3, 1, _, _)))
    }

    /// Whether this item is gold (a money pile). Matched by code name
    /// (`ITEM_ETC_GOLD_*`), which is stable across the pile-size variants.
    pub fn is_gold(&self) -> bool {
        self.code_name().starts_with("ITEM_ETC_GOLD")
    }

    /// A mount/transport summon scroll (`TID == 3/3/3/2`) — the items whose
    /// use (0x704C) summons a rideable COS. Pet summon items are a different
    /// family (`3/2/1/*`, COS-container items) and are out of this predicate.
    pub fn is_cos_summon_scroll(&self) -> bool {
        matches!(self.type_ids(), Some((3, 3, 3, 2)))
    }

    /// Items whose `0x704C` body carries a `u8 targetSlot` — they act on
    /// **another inventory item** rather than on the user or a world entity.
    ///
    /// Six TIDs share the tail: COS revive
    /// ("Grass of life", `3,3,1,6`), plus transgender, reinforce, COS
    /// extension, pet helper and nasrun extension under `3,3,13,*`. One
    /// predicate rather than a revive special-case, because the wire shape is
    /// what they have in common.
    ///
    /// The tail itself is not fully understood — see
    /// [`ItemUseRequest::WithSlot`](packets::agent::inventory::ItemUseRequest).
    pub fn needs_target_slot(&self) -> bool {
        matches!(
            self.type_ids(),
            Some((3, 3, 1, 6)) | Some((3, 3, 13, 8 | 11 | 12 | 15 | 16))
        )
    }

    /// The COS revive item specifically ("Grass of life"), the one member of
    /// [`Self::needs_target_slot`] whose target must be a **dead pet's**
    /// summon scroll.
    pub fn is_cos_revive(&self) -> bool {
        matches!(self.type_ids(), Some((3, 3, 1, 6)))
    }

    /// The summoned COS's characterdata code name (`Desc1_128`), or `None`
    /// when the column is blank/`xxx` (non-COS items).
    pub fn cos_code_name(&self) -> Option<&str> {
        self.field(ItemdataFields::Desc1 as usize)
    }

    /// `Param<n>` (1-based) as the raw authored integer. The 20
    /// `(Param<N>, Param<N>_Desc)` pairs sit at fields 118..157, so `Param<n>`
    /// is field `118 + 2*(n - 1)`; `docs/formats/textdata-itemdata.md` stops at
    /// field 94 and does not describe them. On alchemy items they carry the
    /// mechanic.
    pub fn param(&self, n: usize) -> Option<i64> {
        let index = ItemdataFields::Param1 as usize + 2 * n.checked_sub(1)?;
        self.0.get(index)?.trim().parse().ok()
    }

    /// `Param<n>` read as the **big-endian byte view** of the signed int: the
    /// alchemy rows pack four values into one column, and the paired `_Desc`
    /// names the indices, which is how the order was determined —
    /// `MAGICSTONE_STR_01.Param2 = 169090560 = 0x0A141E00` -> `10,20,30,0`
    /// beside a `Desc` reading `"10, 20, 30, 0"`. A `-1` column is empty, not
    /// `[255,255,255,255]`, so callers check the raw value first.
    pub fn param_bytes(&self, n: usize) -> Option<[u8; 4]> {
        Some(i32::try_from(self.param(n)?).ok()?.to_be_bytes())
    }

    // `cos_rent_minutes()` and `cos_level_tiers()` used to live here and were
    // removed on 2026-08-21: neither had a
    // reader, and the first one read the *wrong* family — it was documented and
    // unit-tested as "the mount scroll's rent duration" while `Param1` is 0 on
    // all 44 of those rows (census on `Param1` above). Both were plain column
    // reads: the rent column is [`Self::param`]`(1)`, the tier list is
    // `Desc2_128` split on commas. Reinstating either is trivial the day a
    // consumer exists (shop tooltip / rent plate, capture-gated by observation
    // order 237); shipping them dead is what let the wrong reading stand for a
    // week.

    /// Name of the character animation group that applies while this item
    /// is equipped, i.e. the weapon class of a weapon (`None` for
    /// non-weapon items). The names match the .bsr animation group names
    /// of the character resources.
    pub fn animation_group(&self) -> Option<&'static str> {
        let type_id = |field: ItemdataFields| self.0[field].parse::<u32>().ok();
        // weapons are TypeID1/2/3 = 3/1/6 (item / equippable / weapon)
        if (
            type_id(ItemdataFields::TypeId1)?,
            type_id(ItemdataFields::TypeId2)?,
            type_id(ItemdataFields::TypeId3)?,
        ) != (3, 1, 6)
        {
            return None;
        }
        match type_id(ItemdataFields::TypeId4)? {
            2 | 3 => Some("sword"),           // CH sword / blade
            4 | 5 => Some("spear"),           // CH spear / glaive
            6 | 12 => Some("bow"),            // CH bow / EU crossbow
            7 => Some("onehand_sword"),       // EU one-handed sword
            8 => Some("twohand_sword"),       // EU two-handed sword
            9 => Some("dual_axe"),            // EU dual axes
            10 | 15 => Some("onehand_staff"), // EU warlock rod / cleric rod
            11 => Some("twohand_staff"),      // EU staff
            13 => Some("dagger"),             // EU dagger
            14 => Some("harf"),               // EU harp ("harf" in the .bsr)
            _ => None,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn row(path: &str) -> ItemDataRow {
        let mut fields = vec![String::new(); 53];
        fields[52] = String::from(path);
        ItemDataRow(fields)
    }

    #[test]
    fn resource_path_converts_backslashes() {
        assert_eq!(
            row("item\\china\\weapon\\sword_01.bsr").resource_path(),
            Some(String::from("data://res/item/china/weapon/sword_01.bsr"))
        );
    }

    #[test]
    fn resource_path_none_for_placeholder() {
        assert_eq!(row("xxx").resource_path(), None);
        assert_eq!(row("").resource_path(), None);
    }

    fn typed_row(tid: (u32, u32, u32, u32)) -> ItemDataRow {
        let mut fields = vec![String::new(); 53];
        fields[9] = tid.0.to_string();
        fields[10] = tid.1.to_string();
        fields[11] = tid.2.to_string();
        fields[12] = tid.3.to_string();
        ItemDataRow(fields)
    }

    #[test]
    fn animation_group_of_weapons() {
        assert_eq!(typed_row((3, 1, 6, 2)).animation_group(), Some("sword"));
        assert_eq!(typed_row((3, 1, 6, 3)).animation_group(), Some("sword"));
        assert_eq!(typed_row((3, 1, 6, 4)).animation_group(), Some("spear"));
        assert_eq!(typed_row((3, 1, 6, 6)).animation_group(), Some("bow"));
        assert_eq!(typed_row((3, 1, 6, 12)).animation_group(), Some("bow"));
        assert_eq!(
            typed_row((3, 1, 6, 8)).animation_group(),
            Some("twohand_sword")
        );
        assert_eq!(typed_row((3, 1, 6, 14)).animation_group(), Some("harf"));
    }

    /// Every row in this test is copied field-for-field from `itemdata*.txt`.
    /// The previous version built a `3/3/3/2` scroll with
    /// `Param1 = 4320` — a combination that **exists in no shard** — and so
    /// "verified" a rent duration on the one family whose rent column is
    /// always 0.
    #[test]
    fn cos_scroll_columns_resolve_summon_target() {
        let row = |tid: (&str, &str, &str, &str), param1: &str, desc1: &str, desc2: &str| {
            let mut fields = vec![String::new(); 122];
            for (i, v) in [(9, tid.0), (10, tid.1), (11, tid.2), (12, tid.3)] {
                fields[i] = v.to_string();
            }
            fields[118] = param1.to_string();
            fields[119] = desc1.to_string();
            fields[121] = desc2.to_string();
            ItemDataRow(fields)
        };

        // ITEM_COS_C_OSTRICH_SCROLL, verbatim: a laddered *mount* scroll —
        // 3/3/3/2, Param1 0, ten level tiers in Desc2.
        let ostrich = row(
            ("3", "3", "3", "2"),
            "0",
            "COS_C_OSTRICH",
            "5,10,20,30,45,60,75,90,105,120",
        );
        assert!(ostrich.is_cos_summon_scroll());
        assert_eq!(ostrich.cos_code_name(), Some("COS_C_OSTRICH"));
        // The rent column is 0 on this family — all 44 rows of it.
        assert_eq!(ostrich.param(1), Some(0));

        // ITEM_COS_P_RABBIT_SCROLL, verbatim: the family that really is rented.
        // 3/2/1/2, Param1 4320 = 3 days, no tier list.
        let rabbit = row(("3", "2", "1", "2"), "4320", "COS_P_RABBIT", "xxx");
        assert!(
            !rabbit.is_cos_summon_scroll(),
            "grab-pet scrolls are a different item family (3/2/1/2)"
        );
        assert_eq!(rabbit.param(1), Some(4320));
        // ITEM_COS_P_MYOWON_SCROLL, verbatim: the 28-day period of the pair.
        let myowon = row(("3", "2", "1", "2"), "40320", "COS_P_MYOWON", "xxx");
        assert_eq!(myowon.param(1), Some(40_320));
        assert_eq!(myowon.param(1).map(|m| m / 1440), Some(28));

        // Pet flutes (3/2/1/1) and potions are not summon scrolls; short rows
        // read as absent.
        assert!(!typed_row((3, 2, 1, 1)).is_cos_summon_scroll());
        assert!(!typed_row((3, 3, 1, 1)).is_cos_summon_scroll());
        assert_eq!(typed_row((3, 3, 3, 2)).cos_code_name(), None);
        assert_eq!(typed_row((3, 3, 3, 2)).param(1), None);
    }

    /// The six classes whose 0x704C body carries a target slot. Getting this
    /// set wrong either arms an item that should just be used (the cursor
    /// sticks) or sends a 3-byte body for a class that wants four — which is
    /// the disconnect this predicate exists to prevent.
    #[test]
    fn needs_target_slot_covers_the_six_target_slot_classes() {
        // COS revive — "Grass of life".
        assert!(typed_row((3, 3, 1, 6)).needs_target_slot());
        assert!(typed_row((3, 3, 1, 6)).is_cos_revive());
        // transgender, reinforce, COS extension, pet helper, nasrun extension
        for tid4 in [8, 11, 12, 15, 16] {
            let row = typed_row((3, 3, 13, tid4));
            assert!(row.needs_target_slot(), "3,3,13,{tid4}");
            assert!(!row.is_cos_revive(), "only 3,3,1,6 is the revive item");
        }
        // Neighbours that must NOT arm: a plain potion, the COS HP/HGP potion
        // (that class takes a `u32 targetGId`, not a slot), a summon scroll,
        // and an unrelated 3,3,13 member.
        for tids in [(3, 3, 1, 1), (3, 3, 1, 9), (3, 3, 3, 2), (3, 3, 13, 1)] {
            assert!(!typed_row(tids).needs_target_slot(), "{tids:?}");
        }
    }

    #[test]
    fn is_equipment_for_equippable_family() {
        assert!(typed_row((3, 1, 6, 2)).is_equipment()); // sword
        assert!(typed_row((3, 1, 1, 1)).is_equipment()); // armor
        assert!(typed_row((3, 1, 4, 1)).is_equipment()); // shield
        assert!(!typed_row((3, 3, 1, 1)).is_equipment()); // potion (expendable)
        assert!(!row("some.bsr").is_equipment()); // unparseable types
    }

    #[test]
    fn is_gold_matches_gold_codename() {
        fn named(code: &str) -> ItemDataRow {
            let mut fields = vec![String::new(); 53];
            fields[2] = String::from(code);
            ItemDataRow(fields)
        }
        assert!(named("ITEM_ETC_GOLD_01").is_gold());
        assert!(named("ITEM_ETC_GOLD_03").is_gold());
        assert!(!named("ITEM_ETC_HP_POTION_01").is_gold());
    }

    fn stat_row() -> ItemDataRow {
        let mut fields = vec![String::new(); 161];
        fields[5] = String::from("SN_ITEM_CH_SPEAR_01_A");
        fields[14] = String::from("0");
        fields[33] = String::from("1");
        fields[54] = String::from("item\\china\\weapon\\icon_spear.ddj");
        fields[57] = String::from("1");
        fields[61] = String::from("4"); // ItemClass 4 -> degree 2
        fields[63] = String::from("42");
        fields[64] = String::from("51");
        fields[94] = String::from("18");
        fields[95] = String::from("15");
        fields[96] = String::from("16");
        fields[113] = String::from("24");
        fields[114] = String::from("30");
        ItemDataRow(fields)
    }

    #[test]
    fn icon_path_builds_media_icon_url() {
        assert_eq!(
            stat_row().icon_path(),
            Some(String::from(
                "media://icon/item/china/weapon/icon_spear.ddj"
            ))
        );
        assert_eq!(row("nope").icon_path(), None); // short row
    }

    #[test]
    fn tooltip_scalar_accessors() {
        let r = stat_row();
        assert_eq!(r.name_key(), Some("SN_ITEM_CH_SPEAR_01_A"));
        assert_eq!(r.required_level(), Some(1));
        assert_eq!(r.degree(), Some(2));
        assert_eq!(r.max_stack(), Some(1));
        assert_eq!(r.country(), Some(0));
        assert_eq!(r.attack_distance(), Some(1.8));
        assert_eq!(r.attack_reach(), Some(18.0));
    }

    /// The spear's col 94 is `18`: eighteen **world** units, which the tooltip
    /// prints as "1.8 m". This pins the ×10 relationship from both ends so the
    /// divisor cannot migrate into [`ItemDataRow::attack_reach`] — combat's
    /// approach consumes the world-unit form, and dividing it there is exactly
    /// what would walk a bow user into melee.
    #[test]
    fn attack_reach_is_world_units_and_distance_is_display_units() {
        // every CH bow / EU crossbow row in the corpus authors 180
        let mut bow = stat_row();
        bow.0[94] = String::from("180");
        assert_eq!(bow.attack_reach(), Some(180.0));
        assert_eq!(bow.attack_distance(), Some(18.0));

        // 0 = no reach authored (armor, accessories), not a reach of zero
        let mut unarmed = stat_row();
        unarmed.0[94] = String::from("0");
        assert_eq!(unarmed.attack_reach(), None);
        assert_eq!(unarmed.attack_distance(), None);

        // ...and a row too short to carry the column at all
        assert_eq!(row("nope").attack_reach(), None);
    }

    #[test]
    fn stat_range_bounds_and_absence() {
        let r = stat_row();
        assert_eq!(r.stat_range(StatRange::Durability), Some((42.0, 51.0)));
        assert_eq!(r.stat_range(StatRange::AttackRate), Some((24.0, 30.0)));
        assert_eq!(r.stat_range(StatRange::PhyAtkMin), Some((15.0, 16.0)));
        // both-zero columns mean "stat not present"
        assert_eq!(r.stat_range(StatRange::Defense), None);
        // short rows must not panic
        assert_eq!(row("x").stat_range(StatRange::Critical), None);
    }

    #[test]
    fn armor_stat_columns() {
        // armor reinforcement lives at 82-85, shield block rate at 74/75 —
        // distinct from the weapon columns (105-112).
        let mut fields = vec![String::from("0"); 161];
        fields[74] = String::from("10");
        fields[75] = String::from("20");
        fields[82] = String::from("32");
        fields[83] = String::from("39");
        fields[84] = String::from("69");
        fields[85] = String::from("84");
        let r = ItemDataRow(fields);
        assert_eq!(r.stat_range(StatRange::BlockRate), Some((10.0, 20.0)));
        assert_eq!(
            r.stat_range(StatRange::ArmorPhyReinforce),
            Some((32.0, 39.0))
        );
        assert_eq!(
            r.stat_range(StatRange::ArmorMagReinforce),
            Some((69.0, 84.0))
        );
    }

    #[test]
    fn gender_and_rarity() {
        fn sexed(code: &str, sex: &str) -> ItemDataRow {
            let mut fields = vec![String::from("0"); 60];
            fields[2] = String::from(code);
            fields[58] = String::from(sex);
            ItemDataRow(fields)
        }
        assert_eq!(sexed("ITEM_CH_M_HEAVY_01_BA_A", "1").gender(), Some("Man"));
        assert_eq!(
            sexed("ITEM_CH_W_HEAVY_01_BA_A", "0").gender(),
            Some("Woman")
        );
        assert_eq!(sexed("ITEM_CH_SWORD_01_A", "2").gender(), None); // universal
        assert!(sexed("ITEM_CH_SWORD_01_A_RARE", "2").is_rare());
        assert!(!sexed("ITEM_CH_SWORD_01_A", "2").is_rare());
    }

    /// The three Seal grades come from the code-name tier letter and nowhere
    /// else — there is no rarity column and no wire field. Pinned because two
    /// surfaces read it now (the tooltip footer and the slot glow), so a drift
    /// here would make them describe the same item differently.
    #[test]
    fn the_seal_grade_comes_from_the_code_name_tier_letter() {
        fn coded(code: &str) -> ItemDataRow {
            let mut fields = vec![String::from("0"); 60];
            fields[2] = String::from(code);
            ItemDataRow(fields)
        }
        assert_eq!(
            coded("ITEM_CH_SWORD_01_B_RARE").seal_tier(),
            Some(SealTier::Moon)
        );
        assert_eq!(
            coded("ITEM_CH_SWORD_01_C_RARE").seal_tier(),
            Some(SealTier::Sun)
        );
        assert_eq!(
            coded("ITEM_CH_SWORD_01_A_RARE").seal_tier(),
            Some(SealTier::Star)
        );
        // a bare `_RARE` with no tier letter is the ungraded one
        assert_eq!(
            coded("ITEM_CH_SWORD_01_RARE").seal_tier(),
            Some(SealTier::Star)
        );
        assert_eq!(coded("ITEM_CH_SWORD_01_A").seal_tier(), None);
        // every grade the suffix names is one `is_rare` also accepts
        for code in [
            "ITEM_CH_SWORD_01_A_RARE",
            "ITEM_CH_SWORD_01_B_RARE",
            "ITEM_CH_SWORD_01_C_RARE",
        ] {
            assert!(coded(code).is_rare(), "{code}");
            assert!(coded(code).seal_tier().is_some(), "{code}");
        }
    }

    /// Every grade prints a name; the labels are what the tooltip footer shows.
    #[test]
    fn each_seal_grade_has_a_label() {
        assert_eq!(SealTier::Star.label(), "Seal of Star");
        assert_eq!(SealTier::Moon.label(), "Seal of Moon");
        assert_eq!(SealTier::Sun.label(), "Seal of Sun");
    }

    #[test]
    fn sell_price_gated_by_can_sell() {
        // degree-1 sword row values from the live archive: CanSell 1 (col
        // 17), SellPrice 427 (col 31)
        fn priced(can_sell: &str, sell: &str) -> ItemDataRow {
            let mut fields = vec![String::from("0"); 60];
            fields[17] = String::from(can_sell);
            fields[31] = String::from(sell);
            ItemDataRow(fields)
        }
        assert_eq!(priced("1", "427").sell_price(), Some(427));
        // mall/quest items (CanSell 0) have no NPC sell value
        assert_eq!(priced("0", "200").sell_price(), None);
    }

    #[test]
    fn animation_group_none_for_non_weapons() {
        // shield (tid3 = 4) and armor (tid3 = 1)
        assert_eq!(typed_row((3, 1, 4, 1)).animation_group(), None);
        assert_eq!(typed_row((3, 1, 1, 1)).animation_group(), None);
        // unparseable type columns
        assert_eq!(row("some.bsr").animation_group(), None);
    }
}

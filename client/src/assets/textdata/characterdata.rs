use bevy::asset::Asset;
use bevy::prelude::TypePath;
use packets::agent::pet::CosKind;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::ops::{Deref, Index};

// TODO: There are other files that use tab-separated rows as well.
//  Only thing that's different are number and order of fields.
//  Should also be made immutably readable so that we can prevent cloning / copying of data.
#[derive(Debug, Clone)]
pub struct CharacterDataRow(pub(crate) Vec<String>);

#[derive(Asset, TypePath, Debug, Clone)]
pub struct CharacterData(pub HashMap<i32, CharacterDataRow>);

/// Ceiling on a knockdown's prone dwell, in seconds.
///
/// Not a gameplay value — a **sanity clamp** on a column whose unit is
/// unknown. If
/// `KO_RecoverTime` turns out to be ticks or tenths rather than milliseconds,
/// the raw number could pin a body to the floor for minutes; this bounds the
/// damage from that misreading to something a player would call "a long
/// knockdown" rather than "the game froze". Chosen as the longest dwell that
/// still reads as one knockdown rather than a bug.
pub const KNOCKDOWN_RECOVER_MAX_SECS: f32 = 5.0;

#[repr(usize)]
#[allow(dead_code)]
enum ChardataFields {
    ID = 1,
    CodeName,
    // RefObjCommon OrgObjCodeOfObjName128: the code name of the base object a
    // variant reuses the model of. `xxx` for base objects; a code name (e.g.
    // `MOB_CH_WATERGHOST`) for summon clones/variants that carry no own model.
    OrgObjCode = 4,
    // RefObjCommon NameStrID: the `SN_*` key resolved against textdataname to a
    // localized display name (shared column with itemdata, see `itemdata.rs`).
    NameStrId = 5,
    // RefObjCommon DescStrID128. On growth-pet ladder rows this holds the *next*
    // stage's code name (`COS_P_WOLF_001` → `COS_P_WOLF_002`, `xxx` at the top) —
    // the chain the 0x30C9 ModelChanged update walks.
    DescStrId128 = 6,
    // TypeID1..4 (RefObjCommon schema, shared with itemdata): the character
    // family and its subtype. TID1 == 1 is the bionic/character family.
    TypeId1 = 9,
    TypeId2,
    TypeId3,
    TypeId4,
    // RefObjCommon: ... DecayTime(13), Country(14), Rarity(15)
    Rarity = 15,
    // RefObjChar Speed1/Speed2: walk/run speed in world units per second
    // (players are 16/50; COS_C mounts run 90-150, COS_T transports 36-50).
    // Column numbers per `SR_Db2Media/Settings.cs:43-51`.
    Speed1 = 46,
    Speed2 = 47,
    // RefObjChar Scale, the column right after the two speeds: the character's
    // size as a **percentage**, 100 = normal. Across every shipped
    // `characterdata*.txt`, all 26
    // `CHAR_*` player rows are exactly 100, and the file-wide spread is
    // 25 … 400 with the `MOB_THIEF_NPC_*` families forming a clean size ladder
    // 94/96/98/100/102/104/106 — which is what a percentage looks like and a
    // flag or an id does not. The original client reads the same value and
    // converts it in exactly the shape [`CharacterDataRow::scale_factor`]
    // copies.
    Scale = 48,
    ResourcePath = 52,
    // RefObjCommon AssocFileIcon_128 — the object's own 32x32 UI icon, the
    // third of the `52-56 AssocFile{Obj,Drop,Icon,1,2}_128` run
    // (`SR_Db2Media/Settings.cs`).
    // 5,579 of the 5,587 COS rows
    // (TID 1/2/3/*) carry one, e.g. ref 6106 `COS_P_WOLF_001` ->
    // `cos\cos_p_wolf_01.ddj`. This is the only per-COS icon source there is —
    // the summon *item*'s icon does not reach the laddered growth stages,
    // which have no 1:1 item.
    AssocFileIcon = 54,
    // RefObjChar tail (after the shared RefObjCommon columns): Lvl(57),
    // CharGender(58), MaxHP(59). In the shipped characterdata:
    // MOB_CH_MANGNYANG lvl 1 / 54 HP, MOB_CH_TIGERWOMAN lvl 20 / 598720 HP,
    // NPCs 0 / 0. CharGender: CHAR_CH_MAN_* = 1, CHAR_CH_WOMAN_* = 0, 2 on
    // gender-neutral monsters — matches itemdata's Sex encoding.
    Level = 57,
    Gender = 58,
    MaxHp = 59,
    // RefObjChar: InventorySize(61) — the COS cargo-grid capacity for
    // transports (COS_T_DONKEY 32 ... COS_T_COW3 135; 0 for ride-only mounts);
    // CanBeVehicle(66)/CanControl(67) — 1 on rideable/commandable COS.
    InventorySize = 61,
    CanBeVehicle = 66,
    CanControl = 67,
    // RefObjChar Knockdown / KO_RecoverTime. These map 1:1 onto go-sro
    // `model/ref_char.go:13-36` (`… ExpToGive, Knockdown, KORecoveryTime, …`).
    //
    // Read for the knockdown animation's prone dwell: the wire says a hit
    // knocked its target down (the displacement arms of 0xB070/0xB071) but
    // nowhere says for how long, and the original's timing is code-side. This
    // is the only *data* answer to that question, and it is per-victim — which
    // is the right granularity, since lying on the ground is a property of the
    // body that fell, not of the skill that felled it.
    //
    // The **unit is unknown**: both columns are described only as "knockdown
    // flags". `ClientCharacterData::knockdown_recover_secs` therefore refuses
    // implausible values rather than trusting the column blindly — see there.
    Knockdown = 81,
    KoRecoverTime = 82,
}

impl Index<ChardataFields> for Vec<String> {
    type Output = String;

    fn index(&self, index: ChardataFields) -> &Self::Output {
        &self[index as usize]
    }
}

impl Display for CharacterDataRow {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        self.0.iter().enumerate().for_each(|(i, e)| {
            s.push_str(&format!("{} -> '{}'", i, e));
        });
        f.write_str(&s)
    }
}

impl Deref for CharacterData {
    type Target = HashMap<i32, CharacterDataRow>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl CharacterDataRow {
    /// The `.bsr` model path, or `None` when the row has no own model — the
    /// column is `xxx`/blank for summon clones and invisible helpers (e.g.
    /// `MOB_CH_WATERGHOST_CLON`, the "water ghost slave"). Loading `xxx`
    /// `MOB_CH_WATERGHOST_CLON`, the "water ghost slave"). Loading `xxx`
    /// produces spurious `Path not found: res/xxx` errors, so callers skip
    pub fn resource_path(&self) -> Option<String> {
        let path = &self.0[ChardataFields::ResourcePath];
        if path.is_empty() || path == "xxx" {
            return None;
        }
        Some(format!("data://res/{}", path.replace("\\", "/")))
    }

    /// The row's size as a plain multiplier: `Scale` is a percentage with 100 =
    /// normal, so this is `percent / 100`.
    ///
    /// The original does the same division and shortcuts the common case
    /// literally — `v == 100 ? 1.0 : v / 100.0` — before handing the factor to
    /// its model scale setter, whose product *model extent × scale × 20* is the
    /// camera's character height (`camera::character_height`).
    ///
    /// An unreadable or non-positive column falls back to 1.0 rather than
    /// collapsing a character to nothing: this feeds a transform scale, and a
    /// bad row must not make a model disappear.
    ///
    /// Do **not** confuse this with the `scale` byte on the wire (character
    /// create/list and the player spawn record): that one is two nibbles,
    /// Height and Volume of the creation sliders, 0..4 each
    /// (`packets::agent::lobby::CharacterCreate::scale`). Feeding `0x11`
    /// through this conversion would draw a 17 % character.
    pub fn scale_factor(&self) -> f32 {
        let percent = self
            .0
            .get(ChardataFields::Scale as usize)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(100.0);
        if percent == 100.0 || !percent.is_finite() || percent <= 0.0 {
            return 1.0;
        }
        percent / 100.0
    }

    pub fn code_name(&self) -> &String {
        &self.0[ChardataFields::CodeName]
    }

    /// `Service` (column 0): the data's own on/off switch for a row. `0` is off;
    /// anything else, or an unreadable/blank column, counts as in service.
    pub fn in_service(&self) -> bool {
        self.0
            .first()
            .and_then(|v| v.trim().trim_start_matches('\u{feff}').parse::<i32>().ok())
            .is_none_or(|v| v != 0)
    }

    /// The object's own UI icon as an asset path (`media://icon/...`), or
    /// `None` when the column is blank or a placeholder.
    ///
    /// Same shape as [`ItemDataRow::icon_path`](crate::assets::textdata::itemdata::ItemDataRow::icon_path)
    /// — both columns hold a backslashed, mixed-case path under `icon/`, and
    /// `bevy_pk2` is case-insensitive, so lowercasing is only for consistency.
    pub fn icon_path(&self) -> Option<String> {
        let path = self.0.get(ChardataFields::AssocFileIcon as usize)?.trim();
        if path.is_empty() || path == "xxx" {
            return None;
        }
        Some(format!(
            "media://icon/{}",
            path.replace('\\', "/").to_lowercase()
        ))
    }

    /// Code name of the base object this row reuses the model of (a summon
    /// clone / variant), or `None` for a base object (`xxx`/blank).
    pub fn org_obj_code(&self) -> Option<&str> {
        let code = &self.0[ChardataFields::OrgObjCode];
        (!code.is_empty() && code != "xxx").then_some(code.as_str())
    }

    /// The `SN_*` name key (RefObjCommon NameStrID), or `None` when the column is
    /// blank. Resolve it to a localized display name via `ClientTextNames::name`,
    /// mirroring how the inventory tooltip names items.
    pub fn name_key(&self) -> Option<&str> {
        let key = &self.0[ChardataFields::NameStrId];
        (!key.is_empty()).then_some(key.as_str())
    }

    /// The character's SRO type tuple `(TypeID1..4)`, or `None` if the columns
    /// are unparseable. `TID1 == 1` is the character family; `TID2` splits it
    /// into player (1) vs NPC/monster (2).
    pub fn type_ids(&self) -> Option<(u32, u32, u32, u32)> {
        let field = |f: ChardataFields| self.0[f].parse::<u32>().ok();
        Some((
            field(ChardataFields::TypeId1)?,
            field(ChardataFields::TypeId2)?,
            field(ChardataFields::TypeId3)?,
            field(ChardataFields::TypeId4)?,
        ))
    }

    /// A player character (`TID1/2 == 1/1`): uses the full player spawn record
    /// (inventory/avatar lists, name, guild, ...).
    pub fn is_player(&self) -> bool {
        matches!(self.type_ids(), Some((1, 1, _, _)))
    }

    /// Monster rarity class: 0 normal, 1 champion, 3 unique, 4 giant, ...
    /// (verified against Media.pk2 characterdata: MOB_CH_TIGERWOMAN = 3).
    pub fn rarity(&self) -> u8 {
        self.0[ChardataFields::Rarity].parse().unwrap_or(0)
    }

    /// The character's level (RefObjChar `Lvl`), or `None` when the column is
    /// missing/zero — NPCs carry 0 there, so "no level" and "NPC" coincide.
    pub fn level(&self) -> Option<u32> {
        let level = self.0.get(ChardataFields::Level as usize)?.parse().ok()?;
        (level > 0).then_some(level)
    }

    /// The character's maximum HP (RefObjChar `MaxHP`), or `None` when the
    /// column is missing/zero (NPCs).
    pub fn max_hp(&self) -> Option<u32> {
        let hp = self.0.get(ChardataFields::MaxHp as usize)?.parse().ok()?;
        (hp > 0).then_some(hp)
    }

    /// How long this body stays prone after a knockdown (RefObjChar
    /// `KO_RecoverTime`, col 82), in seconds — or `None` when the column is
    /// missing, zero, or reads as something other than a duration.
    ///
    /// **The column's unit is unknown.** It maps 1:1 onto go-sro's
    /// `KORecoveryTime`, but it is elsewhere described as a "knockdown flag", so
    /// it cannot simply be trusted as milliseconds. Two guards make a wrong guess
    /// harmless rather than absurd:
    ///
    /// - values of 0 or 1 are read as a **flag**, not a duration ("this body
    ///   can be knocked down"), and yield `None`;
    /// - the result is clamped to [`KNOCKDOWN_RECOVER_MAX_SECS`], so a column
    ///   that turns out to be ticks or tenths cannot pin a body to the floor.
    ///
    /// `None` sends the caller to the animation's own authored length instead
    /// (`player::play_knockdowns`), which is data-derived too. Either way no
    /// number is invented here.
    pub fn knockdown_recover_secs(&self) -> Option<f32> {
        let raw: u32 = self
            .0
            .get(ChardataFields::KoRecoverTime as usize)?
            .trim()
            .parse()
            .ok()?;
        // 0/1 is the flag reading, not a 1 ms recovery.
        if raw <= 1 {
            return None;
        }
        Some((raw as f32 / 1000.0).min(KNOCKDOWN_RECOVER_MAX_SECS))
    }

    /// The player character's sex (RefObjChar `CharGender`): `"Woman"` (0) or
    /// `"Man"` (1); `None` for gender-neutral rows (2 — monsters/NPCs). Encoded
    /// like itemdata's Sex column, so it compares directly against
    /// `ItemDataRow::gender()` for equip checks.
    pub fn gender(&self) -> Option<&'static str> {
        match self.0.get(ChardataFields::Gender as usize)?.trim() {
            "0" => Some("Woman"),
            "1" => Some("Man"),
            _ => None,
        }
    }

    /// A monster (`TID1/2/3 == 1/2/1`): NPC record plus a trailing rarity byte.
    /// Non-player, non-monster, non-COS character-family rows (plain NPCs) use
    /// the plain NPC record.
    pub fn is_monster(&self) -> bool {
        matches!(self.type_ids(), Some((1, 2, 1, _)))
    }

    /// A COS (callable object summon): `TID1/2/3 == 1/2/3`, subtype in TID4.
    /// The tid3 gate is load-bearing — `1/2/4/*` holds `COS_GUARD_*` fortress
    /// guards *and* `STRUCTURE_*`/`MOB_FW_*` rows, so tid4 alone is ambiguous.
    pub fn cos_kind(&self) -> Option<CosKind> {
        match self.type_ids() {
            Some((1, 2, 3, tid4)) => CosKind::from_type_id4(tid4 as u8),
            _ => None,
        }
    }

    /// RefObjChar `Speed1` — walk speed in world units/s, `None` when missing.
    pub fn walk_speed(&self) -> Option<f32> {
        let speed: f32 = self.0.get(ChardataFields::Speed1 as usize)?.parse().ok()?;
        (speed > 0.0).then_some(speed)
    }

    /// RefObjChar `Speed2` — run speed in world units/s, `None` when missing.
    pub fn run_speed(&self) -> Option<f32> {
        let speed: f32 = self.0.get(ChardataFields::Speed2 as usize)?.parse().ok()?;
        (speed > 0.0).then_some(speed)
    }

    /// RefObjChar `InventorySize` — transport cargo capacity, `None` when 0.
    ///
    /// **Not dead code, keep it:** its consumer is the transport cargo grid
    /// — a 7×4 = 28-cell lattice with
    /// `ceil(size / 28)` pages, which is the only way job goods can be carried
    /// (`UIIT_MSG_COS_CANNOT_BUY_SPECIALTY_TO_INVENTORY`). The column is also
    /// not COS-only: across `characterdata*.txt` (all 10 shards, 13,685 rows)
    /// the 39 transports hold 32…177 (→ 2…7 pages), the 13 grab
    /// pets a uniform 140 (→ 5 pages), the 26 `CHAR_CH_*` player rows 45 (the
    /// 5×9 player inventory) and two `MOB_SD_*` rows 255; every other row is 0.
    pub fn inventory_size(&self) -> Option<u8> {
        let size: u8 = self
            .0
            .get(ChardataFields::InventorySize as usize)?
            .parse()
            .ok()?;
        (size > 0).then_some(size)
    }

    /// RefObjChar `CanBeVehicle` — whether players can board this character.
    ///
    /// **Kept although [`CosKind::is_rideable`] looks equivalent**: in the
    /// shipped tables the flag is 1 on exactly the 63 tid4-1 and 39 tid4-2 rows
    /// and 0 on all other 13,583 rows (all 10 `characterdata*.txt` shards) — so
    /// today the two agree, but that
    /// is a property of *this* data, not of the format. The column is the
    /// client's own ride gate (`UIIT_MSG_COSERR_CANT_RIDE`); a vSRO shard that
    /// clears it on a `COS_C_*` row would still be answered correctly here,
    /// while deriving rideability from the type nibble alone could not see it.
    pub fn can_be_vehicle(&self) -> bool {
        self.0
            .get(ChardataFields::CanBeVehicle as usize)
            .is_some_and(|v| v.trim() == "1")
    }

    /// RefObjChar `CanControl` — whether the player may give this COS orders.
    ///
    /// The one column that separates the guild guard from every other summon:
    /// a census over the shipped `characterdata*.txt` (all 10 files, 13,685
    /// rows, 3,483 of them `TypeID1/2/3 = 1/2/3`) gives `CanControl = 1` on all
    /// 63 rides (tid4 1), 39 transports (2), 1,260 growth pets (3), 13 grab
    /// pets (4) and the single `NPC_CH_QT_FLAMEMASTER_COS` (8), and
    /// `CanControl = 0` on all 2,100 `COS_GUILD_{CH,EU}_SOLDIER*` rows (5) plus
    /// the seven quest rows of tid4 6/7.
    ///
    /// The original has its own refusal for the 0 case —
    /// `UIIT_MSG_COSERR_YOU_CANT_CONTROL_THIS_OBJ`, "The selected transport is
    /// not user controlled." (`textdata/textuisystem.txt`) — which is what the
    /// COS command bar shows (`hud/cos_command.rs`).
    ///
    /// A row too short to have column 67 (test fixtures, a truncated shard)
    /// reads as *not* controllable, the same fail-safe direction as
    /// [`Self::can_be_vehicle`].
    pub fn can_control(&self) -> bool {
        self.0
            .get(ChardataFields::CanControl as usize)
            .is_some_and(|v| v.trim() == "1")
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn typed_row(tid: (u32, u32, u32, u32)) -> CharacterDataRow {
        let mut fields = vec![String::new(); 53];
        fields[9] = tid.0.to_string();
        fields[10] = tid.1.to_string();
        fields[11] = tid.2.to_string();
        fields[12] = tid.3.to_string();
        CharacterDataRow(fields)
    }

    /// The shape of the column (see the enum comment): players are exactly 100,
    /// the thief-NPC ladder walks around it in steps of 2, and the file-wide
    /// spread is 25 … 400.
    #[test]
    fn the_scale_column_is_a_percentage_with_100_as_normal() {
        let row = |percent: &str| {
            let mut fields = vec![String::new(); 53];
            fields[ChardataFields::Scale as usize] = percent.to_string();
            CharacterDataRow(fields)
        };

        // Every shipped CHAR_* row: 100, and the original shortcuts this case.
        assert_eq!(row("100").scale_factor(), 1.0);

        // The MOB_THIEF_NPC_* ladder and the extremes of the corpus.
        assert_eq!(row("94").scale_factor(), 0.94);
        assert_eq!(row("106").scale_factor(), 1.06);
        assert_eq!(row("25").scale_factor(), 0.25);
        assert_eq!(row("400").scale_factor(), 4.0);

        // A broken row must not make a character vanish.
        for broken in ["", "xxx", "0", "-5"] {
            assert_eq!(
                row(broken).scale_factor(),
                1.0,
                "'{broken}' must fall back to normal size"
            );
        }

        // Short rows (the index files are one column wide) must not panic.
        assert_eq!(CharacterDataRow(vec![String::new(); 3]).scale_factor(), 1.0);
    }

    fn recover_row(raw: &str) -> CharacterDataRow {
        let mut fields = vec![String::new(); 90];
        fields[ChardataFields::KoRecoverTime as usize] = raw.to_string();
        CharacterDataRow(fields)
    }

    /// The plain reading: milliseconds into seconds.
    #[test]
    fn ko_recover_time_reads_as_milliseconds() {
        assert_eq!(recover_row("1500").knockdown_recover_secs(), Some(1.5));
    }

    /// 0 and 1 are the *flag* reading the docs describe ("knockdown flags"),
    /// not a 1 ms recovery. Treating them as durations would make every such
    /// body snap upright the instant it landed.
    #[test]
    fn a_flag_valued_column_is_not_a_duration() {
        assert_eq!(recover_row("0").knockdown_recover_secs(), None);
        assert_eq!(recover_row("1").knockdown_recover_secs(), None);
        // ...and so is a missing or unparseable column
        assert_eq!(recover_row("").knockdown_recover_secs(), None);
        assert_eq!(CharacterDataRow(vec![]).knockdown_recover_secs(), None);
    }

    /// The column's unit is unverified, so an absurd value is clamped rather
    /// than trusted — a body pinned to the floor for minutes would read as a
    /// freeze, not a knockdown.
    #[test]
    fn an_implausible_recovery_is_clamped() {
        assert_eq!(
            recover_row("600000").knockdown_recover_secs(),
            Some(KNOCKDOWN_RECOVER_MAX_SECS)
        );
    }

    #[test]
    fn classifies_character_subtypes() {
        let player = typed_row((1, 1, 1, 1));
        assert!(player.is_player() && !player.is_monster());

        let monster = typed_row((1, 2, 1, 0));
        assert!(monster.is_monster() && !monster.is_player());

        // Plain NPCs are neither player nor monster (the resolver's NPC fallback).
        let npc = typed_row((1, 2, 2, 0));
        assert!(!npc.is_player() && !npc.is_monster());
    }

    #[test]
    fn cos_kind_requires_the_tid3_gate() {
        assert_eq!(typed_row((1, 2, 3, 1)).cos_kind(), Some(CosKind::Vehicle));
        assert_eq!(typed_row((1, 2, 3, 2)).cos_kind(), Some(CosKind::Transport));
        assert_eq!(typed_row((1, 2, 3, 3)).cos_kind(), Some(CosKind::GrowthPet));
        assert_eq!(typed_row((1, 2, 3, 4)).cos_kind(), Some(CosKind::GrabPet));
        assert_eq!(
            typed_row((1, 2, 3, 5)).cos_kind(),
            Some(CosKind::GuildGuard)
        );

        // 1/2/4/* is COS_GUARD_*/STRUCTURE_* territory — tid4 alone would
        // misclassify these as Vehicle/PickPet.
        assert_eq!(typed_row((1, 2, 4, 1)).cos_kind(), None);
        assert_eq!(typed_row((1, 2, 4, 4)).cos_kind(), None);
        // tid4 6/7/8 have shipped rows and keep their raw nibble, see
        // `CosKind::Unmapped`; tid4 9+ has no row in any shard.
        assert_eq!(
            typed_row((1, 2, 3, 6)).cos_kind(),
            Some(CosKind::Unmapped(6))
        );
        assert_eq!(typed_row((1, 2, 3, 9)).cos_kind(), None);
        assert_eq!(typed_row((3, 3, 3, 2)).cos_kind(), None);
        // COS rows are neither player nor monster.
        let cos = typed_row((1, 2, 3, 2));
        assert!(!cos.is_player() && !cos.is_monster());
    }

    #[test]
    fn speeds_and_inventory_read_their_columns() {
        // COS_T_HORSE1-shaped row: walk 24 / run 48 / cargo 33 / vehicle 1.
        let mut fields = vec![String::new(); 68];
        fields[46] = "24".to_string();
        fields[47] = "48".to_string();
        fields[61] = "33".to_string();
        fields[66] = "1".to_string();
        let row = CharacterDataRow(fields);
        assert_eq!(row.walk_speed(), Some(24.0));
        assert_eq!(row.run_speed(), Some(48.0));
        assert_eq!(row.inventory_size(), Some(33));
        assert!(row.can_be_vehicle());

        // Zero/short rows read as absent, not as 0-speed.
        let short = CharacterDataRow(vec![String::new(); 53]);
        assert_eq!(short.walk_speed(), None);
        assert_eq!(short.run_speed(), None);
        assert_eq!(short.inventory_size(), None);
        assert!(!short.can_be_vehicle());
    }

    /// Three real rows from the shipped `characterdata*.txt`, copied column by
    /// column (row width 105): a ride, a transport and a guild guard.
    /// Only the columns the COS accessors read are filled — the values are the
    /// shipped ones, not a plausible-looking combination.
    fn shipped_cos_row(
        code: &str,
        tid4: u32,
        speeds: (&str, &str),
        max_hp: &str,
        inventory: &str,
        can_be_vehicle: &str,
        can_control: &str,
    ) -> CharacterDataRow {
        let mut fields = vec![String::new(); 105];
        fields[2] = code.to_string();
        fields[9] = "1".to_string();
        fields[10] = "2".to_string();
        fields[11] = "3".to_string();
        fields[12] = tid4.to_string();
        fields[46] = speeds.0.to_string();
        fields[47] = speeds.1.to_string();
        fields[59] = max_hp.to_string();
        fields[61] = inventory.to_string();
        fields[66] = can_be_vehicle.to_string();
        fields[67] = can_control.to_string();
        CharacterDataRow(fields)
    }

    /// `CanControl` (col 67) is what tells a guild guard from a summon the
    /// player may command — the only COS class with a 0 there (all 2,100
    /// `COS_GUILD_*` rows in the shipped data). The
    /// original answers the 0 case with
    /// `UIIT_MSG_COSERR_YOU_CANT_CONTROL_THIS_OBJ`.
    #[test]
    fn can_control_separates_the_guild_guard_from_commandable_cos() {
        // COS_GUILD_CH_SOLDIER1_001, id 9524: tid4 5, walk 20 / run 50,
        // MaxHP 1000, no cargo, CanBeVehicle 0, CanControl 0.
        let guard = shipped_cos_row(
            "COS_GUILD_CH_SOLDIER1_001",
            5,
            ("20", "50"),
            "1000",
            "0",
            "0",
            "0",
        );
        // COS_C_HORSE1, id 2191: tid4 1, walk 45 / run 90, MaxHP 983, no cargo,
        // CanBeVehicle 1, CanControl 1.
        let horse = shipped_cos_row("COS_C_HORSE1", 1, ("45", "90"), "983", "0", "1", "1");
        // COS_T_LIZARD, id 22691: tid4 2, walk 24 / run 48, MaxHP 70648,
        // 88 cargo cells, CanBeVehicle 1, CanControl 1.
        let transport = shipped_cos_row("COS_T_LIZARD", 2, ("24", "48"), "70648", "88", "1", "1");

        assert!(!guard.can_control(), "the guild guard takes no orders");
        assert!(horse.can_control() && transport.can_control());

        // Same read path, same rows: the neighbouring column disagrees with
        // `CanControl` exactly where the data says it should (the guard is
        // neither controllable nor rideable, the horse is both, and cargo is
        // the transport's alone).
        assert!(!guard.can_be_vehicle());
        assert!(horse.can_be_vehicle() && transport.can_be_vehicle());
        assert_eq!(horse.cos_kind(), Some(CosKind::Vehicle));
        assert_eq!(guard.inventory_size(), None);
        assert_eq!(horse.inventory_size(), None);
        assert_eq!(transport.inventory_size(), Some(88));

        // A row too short to hold column 67 is not "controllable by default".
        let short = CharacterDataRow(vec![String::new(); 53]);
        assert!(!short.can_control());
    }

    #[test]
    fn unparseable_types_classify_as_nothing() {
        let blank = CharacterDataRow(vec![String::new(); 53]);
        assert!(!blank.is_player() && !blank.is_monster());
        assert_eq!(blank.type_ids(), None);
    }

    #[test]
    fn level_and_max_hp_read_refobjchar_tail() {
        let mut fields = vec![String::new(); 61];
        fields[57] = "20".to_string();
        fields[59] = "598720".to_string();
        let row = CharacterDataRow(fields);
        assert_eq!(row.level(), Some(20));
        assert_eq!(row.max_hp(), Some(598720));

        // NPC rows carry 0/0 there; short rows (test fixtures) have no tail.
        let mut npc = vec![String::new(); 61];
        npc[57] = "0".to_string();
        npc[59] = "0".to_string();
        let npc = CharacterDataRow(npc);
        assert_eq!(npc.level(), None);
        assert_eq!(npc.max_hp(), None);
        let short = CharacterDataRow(vec![String::new(); 53]);
        assert_eq!(short.level(), None);
        assert_eq!(short.max_hp(), None);
    }

    #[test]
    fn name_key_reads_col_5_and_skips_blanks() {
        let mut fields = vec![String::new(); 53];
        fields[5] = "SN_MOB_THIEF_NPC".to_string();
        let row = CharacterDataRow(fields);
        assert_eq!(row.name_key(), Some("SN_MOB_THIEF_NPC"));

        let blank = CharacterDataRow(vec![String::new(); 53]);
        assert_eq!(blank.name_key(), None);
    }
}

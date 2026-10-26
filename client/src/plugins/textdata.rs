use bevy::app::App;
use bevy::asset::{AssetServer, Assets};
use bevy::log::{debug, info};
use bevy::prelude::{
    AssetEvent, Commands, Handle, MessageReader, OnEnter, Plugin, Res, ResMut, Resource, Update,
};

use crate::assets::resinfo::item_rare::{ItemRareTable, RareEffect};
use crate::assets::textdata::actionwnddata::{ActionCommand, ActionWndData};
use crate::assets::textdata::characterdata::{CharacterData, CharacterDataRow};
use crate::assets::textdata::collectionbook::CollectionBookTable;
use crate::assets::textdata::dungeoninfo::{DungeonEntry, DungeonInfo};
use crate::assets::textdata::effectsound::{EffectSound, EffectSoundTable, SoundAddress};
use crate::assets::textdata::gameguide::{GuideIndex, GuideNode};
use crate::assets::textdata::itemdata::{ItemData, ItemDataRow};
use crate::assets::textdata::leveldata::LevelData;
use crate::assets::textdata::magicoption::{MagicOptionData, MagicOptionInfo};
use crate::assets::textdata::magicoptionassign::MagicOptionAssign;
use crate::assets::textdata::mallmenu::{MallMenuTable, MallTab};
use crate::assets::textdata::masterydata::{MasteryData, MasteryInfo};
use crate::assets::textdata::names::TextdataNames;
use crate::assets::textdata::npcchat::{NpcChat, NpcChatEntry};
use crate::assets::textdata::npcpos::NpcPositions;
use crate::assets::textdata::optionalteleport::{OptionalTeleport, OptionalTeleportTable};
use crate::assets::textdata::quest::QuestTable;
use crate::assets::textdata::questreward::{
    QuestRewardItem, QuestRewardItems, QuestRewardModes, QuestRewardValue, QuestRewardValues,
};
use crate::assets::textdata::shops::{ShopLayout, ShopTable};
use crate::assets::textdata::skilldata::{SkillData, SkillDataRow};
use crate::assets::textdata::skilleffect::{SkillEffectTable, SkillEntry};
use crate::assets::textdata::skillgroup::SkillGroupTable;
use crate::assets::textdata::teleport::{TeleportInfo, TeleportLink, TeleportTable};
use crate::assets::textdata::uisystem::UiSystemText;
use crate::assets::textdata::worldmap::WorldMapTable;
use crate::assets::textdata::zonenames::ZoneNames;
use crate::assets::textdata::zonesound::{ZoneSound, ZoneSoundTable};
use crate::assets::textdata::Textdata;
use crate::scenes::SceneState;

#[derive(Default)]
pub struct TextdataPlugin;

#[derive(Resource, Default)]
#[allow(dead_code)]
struct TextdataHandles(Vec<Handle<Textdata>>);

#[derive(Resource, Default)]
pub struct ClientCharacterData(Option<CharacterData>);

impl ClientCharacterData {
    /// Test-only constructor: builds the resource straight from a table so
    /// data-shape regressions (e.g. #643, a race silently missing from the
    /// race board) can be pinned to real characterdata rows in a unit test.
    #[cfg(test)]
    pub(crate) fn from_table(data: CharacterData) -> Self {
        Self(Some(data))
    }

    pub fn get(&self, id: &i32) -> Option<&CharacterDataRow> {
        match &self.0 {
            Some(cd) => cd.get(id),
            None => None,
        }
    }

    /// The whole table, for consumers that scan it (monster pickers).
    pub fn data(&self) -> Option<&CharacterData> {
        self.0.as_ref()
    }

    /// The `.bsr` model path for a row, following the `OrgObjCode` reference
    /// when the row has no own model (`xxx`): summon clones/variants reuse
    /// their base object's model (e.g. the water ghost slave renders the
    /// water ghost model). Bounded chase guards against a reference cycle.
    pub fn model_path(&self, row: &CharacterDataRow) -> Option<String> {
        if let Some(path) = row.resource_path() {
            return Some(path);
        }
        let Some(cd) = &self.0 else {
            return None;
        };
        let mut base_code = row.org_obj_code()?.to_string();
        for _ in 0..4 {
            let base = cd.values().find(|r| r.code_name() == &base_code)?;
            if let Some(path) = base.resource_path() {
                return Some(path);
            }
            base_code = base.org_obj_code()?.to_string();
        }
        None
    }
}

#[derive(Resource, Default)]
pub struct ClientItemData(Option<ItemData>);

impl ClientItemData {
    /// Seed the resource directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_data(data: ItemData) -> Self {
        Self(Some(data))
    }

    pub fn get(&self, id: &i32) -> Option<&ItemDataRow> {
        match &self.0 {
            Some(id_data) => id_data.get(id),
            None => None,
        }
    }

    /// The full table (for pickers/scans), `None` until loaded.
    pub fn data(&self) -> Option<&ItemData> {
        self.0.as_ref()
    }
}

/// Lets the packets crate resolve an item's class from the client-held
/// itemdata (same mapping as `net::entity_spawn::TextdataResolver`), so
/// resolver-driven item parsing (inventory pickups) works with just the item
/// table.
impl packets::agent::character_data::ItemClassResolver for ClientItemData {
    fn item_class(&self, ref_id: u32) -> packets::agent::character_data::ItemClass {
        use packets::agent::character_data::ItemClass;
        match self.get(&(ref_id as i32)).and_then(|i| i.type_ids()) {
            Some((3, 1, _, _)) => ItemClass::Equipment,
            Some((3, 2, tid3, tid4)) => ItemClass::Container { tid3, tid4 },
            Some((3, 3, tid3, tid4)) => ItemClass::Expendable { tid3, tid4 },
            _ => ItemClass::Unknown,
        }
    }
}

#[derive(Resource, Default)]
pub struct ClientSkillData(Option<SkillData>);

impl ClientSkillData {
    pub fn get(&self, id: &i32) -> Option<&SkillDataRow> {
        match &self.0 {
            Some(sd) => sd.get(id),
            None => None,
        }
    }

    /// Whether the table has arrived (distinguishes "unknown skill id" from
    /// "table not loaded yet" for consumers that want to retry).
    pub fn is_loaded(&self) -> bool {
        self.0.is_some()
    }

    /// The whole table, for consumers that index/scan it (skill book rules).
    pub fn data(&self) -> Option<&SkillData> {
        self.0.as_ref()
    }
}

#[derive(Resource, Default)]
pub struct ClientSkillEffects(Option<SkillEffectTable>);

impl ClientSkillEffects {
    /// Animation binding + effect emissions for a skill's basic-group
    /// codename (skilldata col 5), e.g. `SKILL_CH_SWORD_SMASH_A`.
    pub fn get(&self, codename: &str) -> Option<&SkillEntry> {
        let table = self.0.as_ref()?;
        table
            .by_codename
            .get(codename)
            .map(|&idx| &table.skills[idx])
    }

    /// The unique `.bsr` MODEL paths referenced by skill emissions (bow
    /// arrows). Preloaded so a cast's short-lived projectile doesn't lose
    /// the async load race and silently render nothing.
    pub fn model_paths(&self) -> Vec<String> {
        let Some(table) = self.0.as_ref() else {
            return Vec::new();
        };
        let mut paths: Vec<String> = table
            .skills
            .iter()
            .flat_map(|s| s.emissions.iter())
            .filter(|e| e.efp_path.ends_with(".bsr"))
            .map(|e| e.efp_path.clone())
            .collect();
        paths.sort_unstable();
        paths.dedup();
        paths
    }
}

#[derive(Resource, Default)]
pub struct ClientSkillGroups(Option<SkillGroupTable>);

impl ClientSkillGroups {
    /// Display row (skillgroup.txt Row column) of a skill series, matched by
    /// the branch's icon concept occurring in `basic_group`. `None` when no
    /// branch matches or the table has not loaded yet.
    pub fn row_for(&self, mastery: u32, basic_group: &str) -> Option<u32> {
        self.0
            .as_ref()
            .and_then(|t| t.row_for(mastery, basic_group))
    }

    /// The branch's authoritative icon asset path (`media://icon/skillgroup/
    /// <race>/pack_<concept>.ddj`, skillgroup.txt col 6), matched the same way
    /// as [`Self::row_for`]. `None` falls back to the root skill's own icon.
    pub fn icon_for(&self, mastery: u32, basic_group: &str) -> Option<String> {
        self.0
            .as_ref()
            .and_then(|t| t.branch_for(mastery, basic_group))
            .map(|branch| format!("media://{}", branch.icon))
    }
}

#[derive(Resource, Default)]
pub struct ClientMasteryData(Option<MasteryData>);

impl ClientMasteryData {
    /// Seed the resource directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_data(data: MasteryData) -> Self {
        Self(Some(data))
    }

    pub fn get(&self, id: u32) -> Option<&MasteryInfo> {
        self.0.as_ref().and_then(|data| data.get(&id))
    }

    /// All masteries, unordered; callers sort by (group, tab) for the window.
    pub fn iter(&self) -> impl Iterator<Item = &MasteryInfo> {
        self.0.iter().flat_map(|data| data.values())
    }
}

#[derive(Resource, Default)]
pub struct ClientLevelData(Option<LevelData>);

impl ClientLevelData {
    /// Seed the resource directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_data(data: LevelData) -> Self {
        Self(Some(data))
    }

    /// Exp required to complete the given level, from leveldata.txt.
    pub fn max_exp(&self, level: u8) -> Option<u64> {
        self.0.as_ref().and_then(|data| data.max_exp(level))
    }

    /// SP cost of raising a mastery to `target_level` (leveldata col 2).
    pub fn mastery_sp_cost(&self, target_level: u8) -> Option<u32> {
        self.0
            .as_ref()
            .and_then(|data| data.mastery_sp_cost(target_level))
    }
}

#[derive(Resource, Default)]
pub struct ClientTextNames(Option<TextdataNames>);

impl ClientTextNames {
    /// Seed the resource directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_table(table: TextdataNames) -> Self {
        Self(Some(table))
    }

    /// Localized display string for an `SN_*` key, from the textdataname
    /// tables (item/mob/npc names).
    pub fn name(&self, key: &str) -> Option<&str> {
        self.0.as_ref().and_then(|names| names.get(key))
    }
}

/// The game's own teleport destinations (`refoptionalteleport.txt`): zone key
/// plus a server-style position per row. The label comes from the zone key
/// through [`ClientTextNames`], so the list reads in the player's own
/// language and follows the installed data rather than a transcription.
#[derive(Resource, Default)]
pub struct ClientOptionalTeleports(Option<OptionalTeleportTable>);

impl ClientOptionalTeleports {
    /// The destinations in service, in the table's own order. Empty until the
    /// table has loaded — the window then offers only its own debug spots.
    pub fn active(&self) -> Vec<&OptionalTeleport> {
        self.0
            .as_ref()
            .map(|table| table.active().collect())
            .unwrap_or_default()
    }
}

#[derive(Resource, Default)]
pub struct ClientMagicOptions(Option<MagicOptionData>);

impl ClientMagicOptions {
    /// Definition of an item magic option by its id (a `MagicParam.kind`).
    pub fn get(&self, id: u32) -> Option<&MagicOptionInfo> {
        self.0.as_ref().and_then(|data| data.get(&id))
    }
}

/// The alchemy legality table (`magicoptionassign.txt`).
#[derive(Resource, Default)]
pub struct ClientMagicOptionAssign(Option<MagicOptionAssign>);

impl ClientMagicOptionAssign {
    /// Seed the resource directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_table(table: MagicOptionAssign) -> Self {
        Self(Some(table))
    }

    /// Whether this attribute may go on this item class; `None` where the
    /// table does not decide (see [`MagicOptionAssign::permits`]).
    pub fn permits(&self, tid3: u32, tid4: u32, attr: &str) -> Option<bool> {
        self.0
            .as_ref()
            .and_then(|table| table.permits(tid3, tid4, attr))
    }

    /// The attributes an item class may carry, for a window that wants to
    /// show the whole list rather than ask about one stone.
    pub fn allowed(&self, tid3: u32, tid4: u32) -> Option<&[String]> {
        self.0.as_ref().and_then(|table| table.allowed(tid3, tid4))
    }
}

/// Whether this alchemy stone may be fused onto this item, as the player's own
/// tables decide it: the stone names its `MATTR_*`, the target names its class,
/// and `magicoptionassign.txt` holds the pairing.
///
/// `None` is "the data does not say" and keeps its three causes apart only in
/// the sense that all three are undecided: the stone is not a magic stone (an
/// elemental `NATTR_*` attribute stone, or any other item), the target class
/// has no row, or the attribute appears nowhere in the table.
pub fn alchemy_fusion_allowed(
    assign: &ClientMagicOptionAssign,
    stone: &ItemDataRow,
    target: &ItemDataRow,
) -> Option<bool> {
    let attr = stone.magic_stone_attribute()?;
    let (_, _, tid3, tid4) = target.type_ids()?;
    assign.permits(tid3, tid4, attr)
}

#[derive(Resource, Default)]
pub struct ClientRareEffects(std::collections::HashMap<String, Vec<RareEffect>>);

impl ClientRareEffects {
    /// The rare auras for an item by its code name — the `ItemRare.txt`
    /// (raretype) aura followed by the `ItemOptionEfp.txt` (enchant) one, both
    /// attached when the item is worn.
    pub fn get(&self, code: &str) -> &[RareEffect] {
        self.0.get(code).map_or(&[], Vec::as_slice)
    }
}

/// Keeps the rare-aura table handles alive (ItemRare.txt, ItemOptionEfp.txt).
#[derive(Resource, Default)]
struct RareEffectHandles(Vec<Handle<ItemRareTable>>);

/// The action window's command table (`actionwnddata.txt`). Its slots are keyed
/// on `(group, index)` — the layout's `CommandID` is not unique (see
/// `assets/textdata/actionwnddata.rs`).
#[derive(Resource, Default)]
pub struct ClientActionCommands(Option<ActionWndData>);

impl ClientActionCommands {
    /// The live command in `index` of `group`, or `None` for an unassigned slot.
    pub fn slot(&self, group: u8, index: u8) -> Option<&ActionCommand> {
        self.0.as_ref().and_then(|table| table.slot(group, index))
    }
}

/// The Item Mall's menu table (`mallitemmenulistdata.txt`): which tabs exist,
/// in which order, with which string key and icon, plus the division bits the
/// article-list captions come from.
///
/// **Loaded before its window exists, on purpose.** The mall page itself is
/// still a port branch; what lands here is the *table*, because two windows
/// need the same rows and would otherwise transcribe them twice — the tab
/// strip and the article list (whose caption keys are the division bits,
/// including the `Recommend` key no `SILKMALL` grep finds). The accessors
/// below therefore have no caller yet.
#[derive(Resource, Default)]
pub struct ClientMallMenu(Option<MallMenuTable>);

#[allow(dead_code)]
impl ClientMallMenu {
    /// Test-only: build the resource from transcribed rows, so a caller's tab
    /// logic is pinned against the table instead of against its own output.
    #[cfg(test)]
    pub(crate) fn from_table(table: MallMenuTable) -> Self {
        Self(Some(table))
    }

    /// The tabs the table puts in service, in authored order. Empty when the
    /// table has not loaded — the window then draws no tabs rather than a
    /// hand-copied list of them.
    pub fn tabs(&self) -> Vec<&MallTab> {
        self.0
            .as_ref()
            .map(|table| table.active_tabs().collect())
            .unwrap_or_default()
    }

    /// Caption key for a division bit (`8` is `Recommend`).
    pub fn division(&self, bit: u32) -> Option<&str> {
        self.0.as_ref().and_then(|table| table.division(bit))
    }
}

/// The talisman collection book (`collectionbook_theme.txt` + its item
/// sibling) — the whole book's content, entirely client-side.
#[derive(Resource, Default)]
pub struct ClientCollectionBook(Option<CollectionBookTable>);

impl ClientCollectionBook {
    pub fn table(&self) -> Option<&CollectionBookTable> {
        self.0.as_ref()
    }
}

#[derive(Resource, Default)]
pub struct ClientUiStrings(Option<UiSystemText>);

impl ClientUiStrings {
    /// Test-only table built from explicit rows. Exists so a caller's key logic
    /// can be pinned against **transcribed** keys instead of against its own
    /// output. Asserting that `explanation_key(..)` starts with
    /// `"UIO_NEWCHAR_EXPLANATION_"` only restates `str::replace`.
    #[cfg(test)]
    pub(crate) fn from_rows(rows: &[(&str, &str)]) -> Self {
        Self(Some(crate::assets::textdata::uisystem::UiSystemText(
            rows.iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        )))
    }

    /// Test-only table parsed from **real** `textuisystem.txt` rows
    /// (`enabled \t KEY \t … \t English`) through the production parser.
    ///
    /// Complements [`from_rows`]: that one pins explicit key/value pairs, this
    /// one pins the *file format* too, so a test cannot pass because it
    /// hand-fed the pairs the code already produces.
    #[cfg(test)]
    pub(crate) fn from_tsv(content: &str) -> Self {
        Self(Some(
            crate::assets::textdata::uisystem::UiSystemText::parse(content),
        ))
    }

    /// Localized UI string for a `UIIT_*`-style key, from textuisystem.txt.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.as_ref().and_then(|strings| strings.get(key))
    }

    /// Lookup with a hardcoded fallback for when the table is missing the key
    /// (or has not loaded, e.g. in offline preview scenes). Fallback hits on a
    /// loaded table are logged so stale keys surface during playtests.
    pub fn get_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        match self.get(key) {
            Some(value) => value,
            None => {
                if self.0.is_some() {
                    bevy::log::debug!("textuisystem: no entry for {key}, using \"{fallback}\"");
                }
                fallback
            }
        }
    }

    /// [`Self::get_or`] with the row's markup resolved, for the rows that carry
    /// it — see [`plain_text`]. Use this for message bodies and notices; plain
    /// labels are unaffected either way.
    pub fn get_plain_or(&self, key: &str, fallback: &str) -> String {
        plain_text(self.get_or(key, fallback))
    }

    /// [`Self::get_plain_or`] for rows that have to read as a **sentence**:
    /// an unfilled row is treated as a miss, not as the message.
    ///
    /// Idea: this data set leaves unfilled rows as a bare `0` rather than as an
    /// empty column — 339 of its 5324 rows resolve to exactly `"0"`, among them
    /// `UIIO_CLIENT_START_CONTENT_FAIL_BILLING_FAILED`, the text for login
    /// error `0x0B`. Preferring the row there hands the player the digit `0` as
    /// the explanation of a failed login. Labels keep [`Self::get_plain_or`],
    /// because a bare number is a perfectly good label.
    pub fn get_message_or(&self, key: &str, fallback: &str) -> String {
        match self.get(key) {
            Some(value) if !is_unfilled_row(value) => plain_text(value),
            Some(value) => {
                bevy::log::debug!(
                    "textuisystem: {key} is unfilled ({value:?}), using the fallback"
                );
                plain_text(fallback)
            }
            None => self.get_plain_or(key, fallback),
        }
    }
}

/// A row that carries no sentence: empty, or digits only — the shape this data
/// set uses for an unfilled entry. Deliberately narrow: anything with a letter
/// in it is the data's message, however odd it looks, and stays untouched.
fn is_unfilled_row(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || value.chars().all(|c| c.is_ascii_digit())
}

/// Flatten one textuisystem row's markup into text a Bevy `Text` node can show.
///
/// Idea: a large minority of textuisystem's rows are authored for the original's
/// `CIFPML` rich-text control, not for a plain label — they carry `<sml2>`,
/// `<br>` and `<font …>` tags (`docs/re/ui/help-tooltip-widget.md`: 93 rows use
/// the `sml2` dialect). A plain `Text` node renders those tags **literally**, so
/// the user sees the markup. We do not implement CIFPML; the honest reduction is
/// to keep the authored line breaks (`<br>`) and drop the rest of the markup,
/// which leaves markup-free rows byte-identical.
///
/// Deliberately not a general HTML parser: the dialect is a closed set of tags
/// in shipped data, and anything unterminated is passed through untouched rather
/// than swallowed.
pub fn plain_text(pml: &str) -> String {
    let mut out = String::with_capacity(pml.len());
    let mut rest = pml;
    while let Some(open) = rest.find('<') {
        let Some(len) = rest[open..].find('>') else {
            break;
        };
        out.push_str(&rest[..open]);
        let tag = rest[open + 1..open + len].trim_end_matches('/');
        let tail = &rest[open + len + 1..];
        if tag.eq_ignore_ascii_case("br") {
            out.push('\n');
        } else if tag.eq_ignore_ascii_case("/sml2") && !tail.trim().is_empty() {
            // `</sml2>` closes a size run, and the next run starts a new line.
            // Only when text actually follows: of the 91 shipped rows that carry
            // the tag, 90 END with it, so an unconditional break would append a
            // trailing newline to all ninety.
            out.push('\n');
        }
        rest = tail;
    }
    out.push_str(rest);
    out
}

#[derive(Resource, Default)]
pub struct ClientShops(Option<ShopTable>);

impl ClientShops {
    /// The denormalized inventory of a shop that has no NPC, by its store
    /// codename — the item mall's five storefronts (`MALL_CONSUME`,
    /// `MALL_AVATAR`, `MALL_PET`, `MALL_PREMIUM`, `MALL_ARCHEMY`), which is
    /// the same key `mallitemmenulistdata.txt` gives the mall's menu entries.
    pub fn shop_for_store(&self, store_codename: &str) -> Option<&ShopLayout> {
        self.0
            .as_ref()
            .and_then(|shops| shops.store(store_codename))
    }

    /// The denormalized shop inventory of an NPC, by its characterdata
    /// codename.
    pub fn shop_for_npc(&self, npc_codename: &str) -> Option<&ShopLayout> {
        self.0.as_ref().and_then(|table| table.get(npc_codename))
    }
}

/// Itemdata codename → ref id, built once when the item table arrives (the
/// shop chain references items by codename; scanning 40k rows per lookup
/// would be silly).
#[derive(Resource, Default)]
pub struct ClientItemIndex(std::collections::HashMap<String, i32>);

impl ClientItemIndex {
    /// Seed the index directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_pairs<I: IntoIterator<Item = (String, i32)>>(pairs: I) -> Self {
        Self(pairs.into_iter().collect())
    }

    pub fn id(&self, codename: &str) -> Option<i32> {
        self.0.get(codename).copied()
    }
}

/// Characterdata codename → ref id, the mirror of [`ClientItemIndex`] for the
/// character table.
///
/// Built for the same reason: the GM commands name a monster by codename
/// (`/loadmonster`, `/zoe2`), and without an index that is a linear scan of
/// ~14k rows per lookup — which is what `ClientCharacterData::model_path`
/// still does for its one-off resolve.
#[derive(Resource, Default)]
pub struct ClientCharacterIndex(std::collections::HashMap<String, i32>);

impl ClientCharacterIndex {
    /// Seed the index directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_pairs<I: IntoIterator<Item = (String, i32)>>(pairs: I) -> Self {
        Self(pairs.into_iter().collect())
    }

    pub fn id(&self, codename: &str) -> Option<i32> {
        self.0.get(codename).copied()
    }
}

#[derive(Resource, Default)]
pub struct ClientWorldMap(Option<WorldMapTable>);

impl ClientWorldMap {
    pub fn table(&self) -> Option<&WorldMapTable> {
        self.0.as_ref()
    }
}

/// `regioninfo.txt` x `effectenvsnd.txt` — which zone a region id belongs to
/// and what that zone sounds like (EP-22.1). Playback lives elsewhere.
#[derive(Resource, Default)]
pub struct ClientZoneSounds(Option<ZoneSoundTable>);

// Consumed by the playback half (#771).
#[allow(dead_code)]
impl ClientZoneSounds {
    /// Zone at a sector-local position; `RECT` claims beat the sector's `ALL`
    /// background (`assets::textdata::zonesound`).
    pub fn zone_at(&self, region_id: u16, local_x: f32, local_z: f32) -> Option<&ZoneSound> {
        self.0
            .as_ref()
            .and_then(|t| t.zone_at(region_id, local_x, local_z))
    }

    /// Zone owning a whole sector, for callers that only have a region id.
    pub fn zone_for_region(&self, region_id: u16) -> Option<&ZoneSound> {
        self.0.as_ref().and_then(|t| t.zone_for_region(region_id))
    }
}

/// `effectsound.txt` — the SFX registry (EP-22.2): an address tuple resolves
/// to a `.wav` under `Data.pk2:prim/snd/` plus the row's own volume, so a call
/// site plays what the data says instead of a hard-coded path.
#[derive(Resource, Default)]
pub struct ClientEffectSounds(Option<EffectSoundTable>);

impl ClientEffectSounds {
    /// Wrap a parsed table. The loader uses it, and so does any test that
    /// needs the registry answered without standing up the asset pipeline —
    /// the tuple field is private, so without this the only reachable state
    /// from another module was "table not loaded", which is exactly the arm a
    /// sound test must *not* take.
    pub fn from_table(table: EffectSoundTable) -> Self {
        Self(Some(table))
    }

    /// First row registered at an address, or `None` while the table is still
    /// loading / when the data does not name that sound.
    pub fn first(&self, address: &SoundAddress) -> Option<&EffectSound> {
        self.0.as_ref().and_then(|t| t.first(address))
    }

    /// Every variant at an address, in file order (the equip/skill wiring of
    /// the follow-up tickets picks among them).
    pub fn sounds(&self, address: &SoundAddress) -> &[EffectSound] {
        self.0.as_ref().map(|t| t.sounds(address)).unwrap_or(&[])
    }
}

#[derive(Resource, Default)]
pub struct ClientDungeonInfo(Option<DungeonInfo>);

impl ClientDungeonInfo {
    /// Dungeon entry for a dungeon-flagged region id (`0x8000 | id`).
    pub fn by_region(&self, region_id: u16) -> Option<&DungeonEntry> {
        self.0.as_ref().and_then(|info| info.by_region(region_id))
    }

    /// All dungeons, ordered by id (for pickers).
    pub fn entries(&self) -> impl Iterator<Item = &DungeonEntry> {
        self.0.iter().flat_map(|info| info.entries())
    }
}

#[derive(Resource, Default)]
pub struct ClientTeleport(Option<TeleportTable>);

impl ClientTeleport {
    /// The teleporter id + outgoing links of the teleporter owned by an NPC/
    /// building characterdata ref id.
    pub fn destinations(&self, owner_ref: i32) -> Option<(u32, &[TeleportLink])> {
        self.0
            .as_ref()
            .and_then(|table| table.destinations(owner_ref))
    }

    /// Display info of a teleporter id (destination naming).
    pub fn info(&self, id: u32) -> Option<&TeleportInfo> {
        self.0.as_ref().and_then(|table| table.info.get(&id))
    }

    /// Display key (`SN_NPC_*_GATE`) of a gate building's ref id — gate refs
    /// have no characterdata rows, so the spawn path names them from here.
    pub fn owner_name_key(&self, owner_ref: i32) -> Option<&str> {
        self.0
            .as_ref()
            .and_then(|table| table.buildings.get(&owner_ref))
            .map(|building| building.name_key.as_str())
    }

    /// Codename (`STORE_CH_GATE`) of a gate building's ref id — the dialog's
    /// npcchat/speech lookup key when characterdata has no row.
    pub fn owner_codename(&self, owner_ref: i32) -> Option<&str> {
        self.0
            .as_ref()
            .and_then(|table| table.buildings.get(&owner_ref))
            .map(|building| building.codename.as_str())
    }

    /// The whole table, for consumers that scan rows (the offline dungeon
    /// gates walk every owner-ref-0 circle gate).
    pub fn table(&self) -> Option<&TeleportTable> {
        self.0.as_ref()
    }

    /// Whether a spawn-record ref id is a gate building (teleportbuilding
    /// row) — the spawn resolver's structure test.
    pub fn is_gate_ref(&self, ref_id: i32) -> bool {
        self.0
            .as_ref()
            .is_some_and(|table| table.buildings.contains_key(&ref_id))
    }
}

#[derive(Resource, Default)]
pub struct ClientSpeechText(Option<UiSystemText>);

impl ClientSpeechText {
    /// NPC/quest speech string for an `SN_*` key, from
    /// `textquest_speech&name.txt`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.as_ref().and_then(|strings| strings.get(key))
    }
}

#[derive(Resource, Default)]
pub struct ClientNpcChat(Option<NpcChat>);

impl ClientNpcChat {
    /// The dialog speech string ids of an NPC, by its characterdata codename.
    pub fn get(&self, codename: &str) -> Option<&NpcChatEntry> {
        self.0.as_ref().and_then(|chat| chat.get(codename))
    }
}

/// The authored NPC spawn positions (`npcpos.txt`).
#[derive(Resource, Default)]
pub struct ClientNpcPos(Option<NpcPositions>);

impl ClientNpcPos {
    /// Seed the resource directly, bypassing the asset pipeline (tests only).
    #[cfg(test)]
    pub(crate) fn from_table(table: NpcPositions) -> Self {
        Self(Some(table))
    }

    /// The single NPC the data authors at this position — see
    /// [`NpcPositions::ref_at`] for the tolerance and for why two candidates
    /// resolve to nothing.
    pub fn ref_at(&self, region: u16, x: f32, z: f32) -> Option<i32> {
        self.0.as_ref().and_then(|table| table.ref_at(region, x, z))
    }
}

/// The display name the player's own data gives the NPC at this position, or
/// `None` when no single NPC is authored there or a link of the chain is
/// missing.
///
/// Idea: three tables of the player's own, and the only naming route that
/// needs nothing spawned — `npcpos.txt` turns a position into a `ref_obj_id`,
/// `characterdata_*.txt` turns that into the row's `SN_*` name key, and the
/// `textdataname.txt` shards turn the key into the string. A quest mark points
/// at an NPC the player has usually never been near, so the spawned-entity
/// name the rest of the HUD uses is simply not there to ask.
pub fn authored_npc_name(
    positions: &ClientNpcPos,
    characters: &ClientCharacterData,
    names: &ClientTextNames,
    region: u16,
    x: f32,
    z: f32,
) -> Option<String> {
    let ref_id = positions.ref_at(region, x, z)?;
    let key = characters.get(&ref_id)?.name_key()?;
    names.name(key).map(str::to_string)
}

/// Quest reward tables (`refqusetreward.txt` + `refquestrewarditems.txt`).
#[derive(Resource, Default)]
pub struct ClientQuestRewards(Option<(QuestRewardModes, QuestRewardValues, QuestRewardItems)>);

impl ClientQuestRewards {
    /// `true` when the quest's reward is a choose-one pick (`SelectionCnt`).
    pub fn choose_one(&self, quest: u32) -> bool {
        self.0
            .as_ref()
            .is_some_and(|(modes, _, _)| modes.choose_one(quest))
    }

    /// The quest's gold/experience reward (`refqusetreward.txt` columns
    /// 10/11) — read by the quest detail pane.
    pub fn gold_exp(&self, quest: u32) -> Option<QuestRewardValue> {
        self.0.as_ref().and_then(|(_, values, _)| values.get(quest))
    }

    /// The quest's reward rows: candidates in pick-one mode, all granted
    /// otherwise.
    pub fn items(&self, quest: u32) -> &[QuestRewardItem] {
        match &self.0 {
            Some((_, _, items)) => items.items(quest),
            None => &[],
        }
    }
}

/// The quest structure tables (`questdata.txt` + `questcontentsdata.txt`) —
/// journal decoration only: the active set itself comes off the wire, and this
/// table does not cover every id the server can send.
#[derive(Resource, Default)]
pub struct ClientQuestTable(Option<QuestTable>);

impl ClientQuestTable {
    /// The parsed table, `None` until `questdata.txt` has loaded.
    ///
    /// This is the resource's only read path and it has **no reader yet** —
    /// the journal UI that will use it is not written. It is kept rather than
    /// deleted because the loader arm above it has to hand the parsed table
    /// somewhere for the file to be loaded at all; an accessor that returns it
    /// is the smallest such landing place. If the journal is still unwritten
    /// when this file is next touched, delete the resource, the loader arm and
    /// this method together — `QuestTable` itself carries its own tests.
    pub fn table(&self) -> Option<&QuestTable> {
        self.0.as_ref()
    }
}

/// The in-game help book (#575): the `gameguidedata.txt` tree and the
/// `texthelp.txt` bodies behind it. Two files, one resource, because a guide
/// node without its body is not usable on its own.
#[derive(Resource, Default)]
pub struct ClientGameGuide {
    index: Option<GuideIndex>,
    bodies: Option<UiSystemText>,
}

impl ClientGameGuide {
    /// The guide tree in file order, empty until `gameguidedata.txt` loads.
    pub fn nodes(&self) -> &[GuideNode] {
        self.index
            .as_ref()
            .map(|i| i.nodes.as_slice())
            .unwrap_or(&[])
    }

    /// The page body for a node, with the CIFPML markup reduced the same way
    /// every other markup-carrying table is ([`plain_text`]).
    pub fn body(&self, node: &GuideNode) -> Option<String> {
        let key = node.body_key.as_deref()?;
        self.bodies.as_ref()?.get(key).map(plain_text)
    }

    /// The drawer label for a node: the `SRO_GGW_MENU_*` string when
    /// `texthelp.txt` carries one, else the row's own authored title — which
    /// is Korean, and is all the data has for a row whose menu key is missing.
    /// Falling back to the title rather than to an empty row keeps an
    /// unresolved key visible instead of silently deleting a navigation entry.
    pub fn menu_label(&self, node: &GuideNode) -> String {
        node.menu_key
            .as_deref()
            .and_then(|key| self.bodies.as_ref()?.get(key))
            .map(plain_text)
            .unwrap_or_else(|| node.title.clone())
    }
}

#[derive(Resource, Default)]
pub struct ClientZoneNames(Option<ZoneNames>);

impl ClientZoneNames {
    /// Area name for a region id, from textzonename.txt.
    pub fn name(&self, region: u16) -> Option<&str> {
        self.0.as_ref().and_then(|names| names.name(region))
    }
}

impl Plugin for TextdataPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TextdataHandles>()
            .init_resource::<ClientCharacterData>()
            .init_resource::<ClientItemData>()
            .init_resource::<ClientSkillData>()
            .init_resource::<ClientSkillEffects>()
            .init_resource::<ClientMasteryData>()
            .init_resource::<ClientSkillGroups>()
            .init_resource::<ClientLevelData>()
            .init_resource::<ClientTextNames>()
            .init_resource::<ClientOptionalTeleports>()
            .init_resource::<ClientMagicOptions>()
            .init_resource::<ClientMagicOptionAssign>()
            .init_resource::<ClientRareEffects>()
            .init_resource::<ClientZoneNames>()
            .init_resource::<ClientGameGuide>()
            .init_resource::<ClientActionCommands>()
            .init_resource::<ClientMallMenu>()
            .init_resource::<ClientCollectionBook>()
            .init_resource::<ClientQuestRewards>()
            .init_resource::<ClientQuestTable>()
            .init_resource::<ClientUiStrings>()
            .init_resource::<ClientSpeechText>()
            .init_resource::<ClientNpcChat>()
            .init_resource::<ClientNpcPos>()
            .init_resource::<ClientShops>()
            .init_resource::<ClientItemIndex>()
            .init_resource::<ClientCharacterIndex>()
            .init_resource::<ClientTeleport>()
            .init_resource::<ClientWorldMap>()
            .init_resource::<ClientDungeonInfo>()
            .init_resource::<ClientZoneSounds>()
            .init_resource::<ClientEffectSounds>()
            .add_systems(OnEnter(SceneState::Loading), load_textdata)
            .add_systems(
                Update,
                (
                    add_resource_when_textdata_loaded,
                    add_rare_effects_when_loaded,
                ),
            );
    }
}

fn load_textdata(asset_server: Res<AssetServer>, mut commands: Commands) {
    let handles = vec![
        asset_server.load("media://server_dep/silkroad/textdata/characterdata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/itemdata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/skilldata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/skilleffect.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/skillmasterydata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/skillgroup.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/leveldata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/textzonename.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/refoptionalteleport.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/refqusetreward.txt"),
        // Quest structure: questdata.txt pulls in questcontentsdata.txt.
        asset_server.load("media://server_dep/silkroad/textdata/questdata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/textdataname.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/magicoption.txt"),
        // the alchemy legality rule: which MATTR_* an item class may carry
        asset_server.load("media://server_dep/silkroad/textdata/magicoptionassign.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/textuisystem.txt"),
        // the in-game help book: the tree and its bodies (#575)
        asset_server.load("media://server_dep/silkroad/textdata/gameguidedata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/texthelp.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/actionwnddata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/mallitemmenulistdata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/collectionbook_theme.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/textquest_speech&name.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/npcchat.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/npcpos.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/refshopgroup.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/teleportdata.txt"),
        asset_server.load("media://server_dep/silkroad/textdata/worldmap_mapinfo.txt"),
        // Dungeon id → .dof path table; lives in Data.pk2, not Media.pk2.
        asset_server.load("data://dungeon/dungeoninfo.txt"),
        // Zone sound tables: regioninfo.txt pulls in effectenvsnd.txt (EP-22.1).
        // regioninfo is the one table that is NOT in Media's server_dep tree —
        // it sits at the root of Data.pk2 as `RegionInfo.txt` (the archive
        // lookup lowercases, so the request need not match its casing). Its
        // effectenvsnd sibling stays in Media, which is where `read_sibling`
        // looks either way, so the join still resolves across both archives.
        asset_server.load("data://regioninfo.txt"),
        // The SFX registry (EP-22.2).
        asset_server.load("media://server_dep/silkroad/textdata/effectsound.txt"),
    ];
    commands.insert_resource(TextdataHandles(handles));
    // Two aura layers per rare item: the raretype glow, then the enchant.
    commands.insert_resource(RareEffectHandles(vec![
        asset_server.load("media://resinfo/ItemRare.txt"),
        asset_server.load("media://resinfo/ItemOptionEfp.txt"),
    ]));
}

fn add_rare_effects_when_loaded(
    mut reader: MessageReader<AssetEvent<ItemRareTable>>,
    mut commands: Commands,
    handles: Option<Res<RareEffectHandles>>,
    tables: Res<Assets<ItemRareTable>>,
) {
    let Some(handles) = handles else { return };
    // Rebuild the merged map once any table arrives and all are ready, so each
    // item's auras stay in layer order (raretype, then enchant).
    if !reader.read().any(|e| matches!(e, AssetEvent::Added { .. })) {
        return;
    }
    let loaded: Vec<&ItemRareTable> = handles.0.iter().filter_map(|h| tables.get(h)).collect();
    if loaded.len() != handles.0.len() {
        return;
    }
    let mut merged: std::collections::HashMap<String, Vec<RareEffect>> = Default::default();
    for table in loaded {
        for (code, effects) in &table.0 {
            merged
                .entry(code.clone())
                .or_default()
                .extend(effects.iter().cloned());
        }
    }
    info!("loaded {} rare-item aura sets", merged.len());
    commands.insert_resource(ClientRareEffects(merged));
}

/// Moves each loaded table out of `Assets<Textdata>` into its `Client*`
/// resource. Taking ownership (rather than cloning) matters: itemdata,
/// skilldata and characterdata are tens of thousands of rows, and deep-cloning
/// them on the main thread was a ~0.3 s frame at login. Nothing reads
/// `Assets<Textdata>` afterwards — the resources are the only consumers.
fn add_resource_when_textdata_loaded(
    mut reader: MessageReader<AssetEvent<Textdata>>,
    mut commands: Commands,
    mut guide: ResMut<ClientGameGuide>,
    mut textdata_assets: ResMut<Assets<Textdata>>,
) {
    for event in reader.read() {
        let textdata = match event {
            AssetEvent::Added { id } => textdata_assets.remove(*id),
            _ => None,
        };

        if let Some(textdata) = textdata {
            info!("updated textdata");
            match textdata {
                Textdata::CharacterData(char_data) => {
                    let index = char_data
                        .0
                        .iter()
                        .map(|(id, row)| (row.code_name().clone(), *id))
                        .collect();
                    commands.insert_resource(ClientCharacterIndex(index));
                    commands.insert_resource(ClientCharacterData(Some(char_data)))
                }
                Textdata::ItemData(item_data) => {
                    info!("loaded {} itemdata rows", item_data.len());
                    let index = item_data
                        .0
                        .iter()
                        .map(|(id, row)| (row.code_name().clone(), *id))
                        .collect();
                    commands.insert_resource(ClientItemIndex(index));
                    commands.insert_resource(ClientItemData(Some(item_data)))
                }
                Textdata::LevelData(level_data) => {
                    commands.insert_resource(ClientLevelData(Some(level_data)))
                }
                Textdata::QuestRewards(modes, values, items) => {
                    info!(
                        "loaded {} quest reward modes, {} quests with reward rows",
                        modes.0.len(),
                        items.0.len()
                    );
                    commands.insert_resource(ClientQuestRewards(Some((modes, values, items))))
                }
                Textdata::Quests(table) => {
                    info!(
                        "loaded {} questdata rows, {} questcontentsdata rows",
                        table.by_id.len(),
                        table.contents.len()
                    );
                    commands.insert_resource(ClientQuestTable(Some(table)))
                }
                Textdata::ZoneNames(zone_names) => {
                    info!("loaded {} zone names", zone_names.0.len());
                    commands.insert_resource(ClientZoneNames(Some(zone_names)))
                }
                Textdata::Names(names) => {
                    info!("loaded {} display names", names.0.len());
                    commands.insert_resource(ClientTextNames(Some(names)))
                }
                Textdata::SkillData(skill_data) => {
                    info!("loaded {} skilldata rows", skill_data.len());
                    commands.insert_resource(ClientSkillData(Some(skill_data)))
                }
                Textdata::SkillEffects(table) => {
                    info!("loaded {} skilleffect entries", table.skills.len());
                    commands.insert_resource(ClientSkillEffects(Some(table)))
                }
                Textdata::MasteryData(masteries) => {
                    info!("loaded {} masteries", masteries.len());
                    commands.insert_resource(ClientMasteryData(Some(masteries)))
                }
                Textdata::SkillGroups(groups) => {
                    info!("loaded {} skill-group branches", groups.len());
                    commands.insert_resource(ClientSkillGroups(Some(groups)))
                }
                Textdata::MagicOptionAssign(assign) => {
                    info!("loaded {} magic-option item classes", assign.len());
                    commands.insert_resource(ClientMagicOptionAssign(Some(assign)))
                }
                Textdata::MagicOption(magic_options) => {
                    info!("loaded {} magic-option rows", magic_options.len());
                    commands.insert_resource(ClientMagicOptions(Some(magic_options)))
                }
                Textdata::CollectionBook(book) => {
                    info!(
                        "loaded {} collection themes, {} talismans",
                        book.themes.len(),
                        book.items.len()
                    );
                    commands.insert_resource(ClientCollectionBook(Some(book)))
                }
                Textdata::OptionalTeleport(table) => {
                    info!(
                        "loaded {} teleport destinations ({} in service)",
                        table.0.len(),
                        table.active().count()
                    );
                    commands.insert_resource(ClientOptionalTeleports(Some(table.clone())))
                }
                Textdata::ActionWnd(table) => {
                    info!("loaded {} action-window commands", table.0.len());
                    commands.insert_resource(ClientActionCommands(Some(table)))
                }
                Textdata::MallMenu(table) => {
                    info!(
                        "loaded {} mall tabs ({} in service), {} division bits",
                        table.tabs.len(),
                        table.active_tabs().count(),
                        table.divisions.len()
                    );
                    commands.insert_resource(ClientMallMenu(Some(table.clone())))
                }
                // The guide's two halves land independently and fill one
                // resource, so they mutate it rather than replacing it.
                Textdata::GameGuide(index) => {
                    info!("loaded {} game-guide nodes", index.nodes.len());
                    for mismatch in index.child_count_mismatches() {
                        debug!(
                            "gameguidedata: category {} states {} children, {} rows point at it",
                            mismatch.category, mismatch.stated, mismatch.actual
                        );
                    }
                    guide.index = Some(index);
                }
                Textdata::GuideText(bodies) => {
                    info!("loaded {} game-guide bodies", bodies.0.len());
                    guide.bodies = Some(bodies);
                }
                Textdata::UiSystem(strings) => {
                    info!("loaded {} UI strings", strings.0.len());
                    commands.insert_resource(ClientUiStrings(Some(strings)))
                }
                Textdata::SpeechText(strings) => {
                    info!("loaded {} speech strings", strings.0.len());
                    commands.insert_resource(ClientSpeechText(Some(strings)))
                }
                Textdata::NpcChat(chat) => {
                    info!("loaded {} npc chat entries", chat.0.len());
                    commands.insert_resource(ClientNpcChat(Some(chat)))
                }
                Textdata::NpcPos(positions) => {
                    info!("loaded {} authored npc positions", positions.len());
                    commands.insert_resource(ClientNpcPos(Some(positions)))
                }
                Textdata::Shops(shops) => {
                    info!("loaded {} npc shops", shops.by_npc.len());
                    commands.insert_resource(ClientShops(Some(shops)))
                }
                Textdata::Teleport(teleport) => {
                    info!(
                        "loaded {} teleporters ({} linked)",
                        teleport.info.len(),
                        teleport.links.len()
                    );
                    commands.insert_resource(ClientTeleport(Some(teleport)))
                }
                Textdata::WorldMap(worldmap) => {
                    info!(
                        "loaded {} world maps, {} POIs",
                        worldmap.maps.len(),
                        worldmap.pois.len()
                    );
                    commands.insert_resource(ClientWorldMap(Some(worldmap)))
                }
                Textdata::DungeonInfo(dungeons) => {
                    info!("loaded {} dungeon info rows", dungeons.0.len());
                    commands.insert_resource(ClientDungeonInfo(Some(dungeons)))
                }
                Textdata::ZoneSounds(zone_sounds) => {
                    info!(
                        "loaded {} sound zones ({} with a BGM track)",
                        zone_sounds.zones().len(),
                        zone_sounds
                            .zones()
                            .iter()
                            .filter(|z| z.bgm.is_some())
                            .count()
                    );
                    if !zone_sounds.unmatched_zones.is_empty() {
                        debug!(
                            "zone sounds: {} zone names in only one of regioninfo/effectenvsnd: {}",
                            zone_sounds.unmatched_zones.len(),
                            zone_sounds.unmatched_zones.join(", ")
                        );
                    }
                    commands.insert_resource(ClientZoneSounds(Some(zone_sounds)))
                }
                Textdata::EffectSounds(effect_sounds) => {
                    info!(
                        "loaded {} effect sound rows ({} addresses, {} mute)",
                        effect_sounds.row_count(),
                        effect_sounds.len(),
                        effect_sounds.mute_rows()
                    );
                    commands.insert_resource(ClientEffectSounds::from_table(effect_sounds))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows :3969 and :1753 verbatim from the user's
    /// `media://server_dep/silkroad/textdata/textuisystem.txt` — 10 tab-separated
    /// columns with the English text last. The first is authored for the
    /// original's `CIFPML` control and carries markup; the second does not.
    const TEXTUISYSTEM_ROWS: &str = concat!(
        "1\tUIIT_STT_GLOBAL_AUTHENTICATION_NOTICE\t\t\t\t\t\t\t\t",
        "<sml2>To prevent auto creation,<br>please enter the number/text as it appears.</sml2>\r\n",
        "1\tUIIT_MSG_LOGOUT_REMAIN_TIME\t\t\t\t\t\t\t\t",
        "It will take %d seconds to close the game.\r\n",
    );

    fn strings() -> ClientUiStrings {
        ClientUiStrings(Some(
            crate::assets::textdata::uisystem::UiSystemText::parse(TEXTUISYSTEM_ROWS),
        ))
    }

    /// A markup row must reach a `Text` node as text, not as tags — this is the
    /// only markup handling we have (`docs/re/ui/help-tooltip-widget.md` §8-5:
    /// 93 rows use the `sml2` dialect).
    #[test]
    fn get_plain_or_resolves_markup_rows() {
        assert_eq!(
            strings().get_plain_or("UIIT_STT_GLOBAL_AUTHENTICATION_NOTICE", "fallback"),
            "To prevent auto creation,\nplease enter the number/text as it appears."
        );
    }

    /// ...and a markup-free row, a `%d` template and a fallback must all come
    /// through byte-identical, so adopting `get_plain_or` at a call site can
    /// never change a plain string.
    #[test]
    fn get_plain_or_leaves_plain_rows_and_fallbacks_untouched() {
        let strings = strings();
        assert_eq!(
            strings.get_plain_or("UIIT_MSG_LOGOUT_REMAIN_TIME", "fallback"),
            "It will take %d seconds to close the game."
        );
        assert_eq!(
            strings.get_plain_or("NO_SUCH_KEY", "verbatim <fallback"),
            "verbatim <fallback"
        );
    }

    /// The message path and the label path part company on an unfilled row:
    /// the message path treats it as a miss, the label path keeps it, because
    /// a bare number is a perfectly good label (a count, a level, a price).
    #[test]
    fn only_the_message_path_steps_over_an_unfilled_row() {
        let strings = ClientUiStrings::from_rows(&[("UNFILLED", "0"), ("EMPTY", "  ")]);
        assert_eq!(
            strings.get_message_or("UNFILLED", "a sentence"),
            "a sentence"
        );
        assert_eq!(strings.get_message_or("EMPTY", "a sentence"), "a sentence");
        assert_eq!(strings.get_plain_or("UNFILLED", "a sentence"), "0");
    }

    /// The rule stops at the first letter: an odd-looking row that carries one
    /// is still the data's message and must not be replaced by our prose.
    #[test]
    fn a_row_with_letters_is_never_treated_as_unfilled() {
        let strings = ClientUiStrings::from_rows(&[("ODD", "0 of 6"), ("SHORT", "OK")]);
        assert_eq!(strings.get_message_or("ODD", "ours"), "0 of 6");
        assert_eq!(strings.get_message_or("SHORT", "ours"), "OK");
    }

    /// The dialect is a closed tag set in shipped data, so anything malformed is
    /// passed through rather than swallowed.
    #[test]
    fn plain_text_passes_through_unterminated_markup() {
        assert_eq!(plain_text("unterminated <tag"), "unterminated <tag");
        assert_eq!(plain_text("a<br/>b<BR>c"), "a\nb\nc");
    }

    /// `</sml2>` ends a size run, so a run that is followed by more text starts a
    /// new line. The row below is the shipped `UIIT_STT_ALCHEMYBOX_REINFORCE_TEXT`
    /// shortened to its two runs — the one row in the corpus where the tag is not
    /// the last thing on the line.
    #[test]
    fn plain_text_breaks_a_closing_size_run_only_when_text_follows() {
        assert_eq!(
            plain_text("<sml2>combined with + options.</sml2>Warning: all used items"),
            "combined with + options.\nWarning: all used items"
        );
        // 90 of the 91 shipped rows end with the tag: no trailing newline there
        assert_eq!(plain_text("<sml2>a single run</sml2>"), "a single run");
        assert_eq!(
            plain_text("<sml2>trailing space</sml2>  "),
            "trailing space  "
        );
        // the opening tag never breaks, and an unknown tag still just vanishes
        assert_eq!(plain_text("<sml2>a<font color=\"x\">b</font></sml2>"), "ab");
    }

    /// The two `UI / SND_BUTTON_CLICK` rows verbatim from the user's
    /// `media://server_dep/silkroad/textdata/effectsound.txt` (2026-08-16),
    /// preceded by the file's own legend line.
    const EFFECTSOUND_ROWS: &str = concat!(
        "//\tobject\thandle\tskill_ID\tevent1\tevent2\tevent3\tblank\tfolder\tfilename\tvolume\tdescription1\t\r\n",
        "\tUI\tSND_BUTTON_CLICK\t-\t-\t-\t-\t0\tui\\\tuibutton_a.wav\t80\tclick#1\t\r\n",
        "\tUI\tSND_BUTTON_CLICK\t-\t-\t-\t-\t0\tui\\\tuibutton_b.wav\t80\tclick#2\t\r\n",
    );

    /// The registry resource is what the UI click reads (#773): it must resolve
    /// the row's own asset path and volume, and answer `None` (never panic)
    /// while the table has not loaded yet.
    #[test]
    fn client_effect_sounds_resolves_the_ui_click_row() {
        let click = SoundAddress::new("UI", "SND_BUTTON_CLICK");

        let empty = ClientEffectSounds::default();
        assert!(empty.first(&click).is_none());
        assert!(empty.sounds(&click).is_empty());

        let loaded = ClientEffectSounds(Some(EffectSoundTable::parse(EFFECTSOUND_ROWS)));
        let row = loaded.first(&click).expect("UI/SND_BUTTON_CLICK");
        assert_eq!(row.asset_path(), "data://prim/snd/ui/uibutton_a.wav");
        assert_eq!(row.gain(), 0.8);
        assert_eq!(loaded.sounds(&click).len(), 2);
    }

    /// The alchemy question end to end, every link a literal row of the
    /// player's own files: the `ITEM_ETC_ARCHEMY_MAGICSTONE_STR_01` columns
    /// that name its option (`itemdata_10000.txt:1227`), two target items with
    /// the `TID3/TID4` their rows carry, and the two
    /// `magicoptionassign.txt` lines for those classes. `MATTR_STR` is legal
    /// on both; the hit-ratio stone is the one the classes separate.
    #[test]
    fn a_stone_and_a_target_decide_fusion_from_their_own_rows() {
        let assign = ClientMagicOptionAssign::from_table(MagicOptionAssign::parse(
            "1\t0\t6\t2\tMATTR_STR\tMATTR_HR\tMATTR_REPAIR\txxx\txxx\n\
             1\t0\t1\t1\tMATTR_STR\tMATTR_HP\tMATTR_MP\txxx\txxx\n",
        ));
        let item = |tid3: &str, tid4: &str, desc: &str| {
            let mut row = vec![String::new(); 120];
            row[9] = "3".to_string();
            row[10] = "1".to_string();
            row[11] = tid3.to_string();
            row[12] = tid4.to_string();
            row[119] = desc.to_string();
            ItemDataRow(row)
        };
        let stone = item("11", "1", "MATTR_STR");
        let hr_stone = item("11", "1", "MATTR_HR");
        // an elemental attribute stone, same column, different mechanic
        let attr_stone = item("11", "2", "NATTR_PA");
        let sword = item("6", "2", "");
        let armour = item("1", "1", "");

        assert_eq!(alchemy_fusion_allowed(&assign, &stone, &sword), Some(true));
        assert_eq!(alchemy_fusion_allowed(&assign, &stone, &armour), Some(true));
        assert_eq!(
            alchemy_fusion_allowed(&assign, &hr_stone, &sword),
            Some(true)
        );
        assert_eq!(
            alchemy_fusion_allowed(&assign, &hr_stone, &armour),
            Some(false)
        );
        // not a magic stone: the table is not asked at all
        assert_eq!(alchemy_fusion_allowed(&assign, &attr_stone, &sword), None);
    }

    /// The naming chain a quest mark depends on, all three links fed with
    /// literal rows from the player's own files: `npcpos.txt` row for ref 2003
    /// (the target of the captured `0x30D6` at region 25000), its
    /// `characterdata_5000.txt` codename/name-key columns, and the
    /// `textdata_object.txt` string that key resolves to. The answer is the one
    /// the data really holds, not a shape this test invented.
    #[test]
    fn a_position_resolves_to_the_npc_name_its_own_tables_hold() {
        let positions = ClientNpcPos(Some(NpcPositions::parse(
            "2003\t25000\t332.73001\t0.0\t1406.7\r\n",
        )));
        let mut row = vec![String::new(); 60];
        row[2] = "NPC_CH_SMITH".to_string();
        row[5] = "SN_NPC_CH_SMITH".to_string();
        let characters = ClientCharacterData::from_table(CharacterData(
            [(2003, CharacterDataRow(row))].into_iter().collect(),
        ));
        let names = ClientTextNames(Some(TextdataNames(
            [(
                "SN_NPC_CH_SMITH".to_string(),
                "Blacksmith Chulsan".to_string(),
            )]
            .into_iter()
            .collect(),
        )));

        // the captured mark for this target carries the truncated (332, 1406)
        assert_eq!(
            authored_npc_name(&positions, &characters, &names, 25000, 332.0, 1406.0),
            Some("Blacksmith Chulsan".to_string())
        );
        // nothing authored there: no name rather than a neighbour's
        assert_eq!(
            authored_npc_name(&positions, &characters, &names, 25000, 900.0, 900.0),
            None
        );
        // and the chain is only as long as its links — a ref whose name key
        // resolves to nothing must not fall back to the key or the codename
        assert_eq!(
            authored_npc_name(
                &positions,
                &characters,
                &ClientTextNames::default(),
                25000,
                332.0,
                1406.0
            ),
            None
        );
    }
}

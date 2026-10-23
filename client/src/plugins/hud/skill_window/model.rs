//! Skill window state: open/close toggle, tab selection, the skill-icon
//! drag carry, the practice (level-up confirmation) prompt, and quickslot
//! synchronization.

use bevy::prelude::*;

use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::hud::underbar::model::{PlayerProgress, QuickSlots, SlotAction};
use crate::plugins::settings::keymap::KEY_SKILL;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::skills::book::{SkillBook, SkillGroupIndex};
use crate::plugins::textdata::ClientSkillData;

/// Which skill tree the window shows — CH characters never see EU skills
/// and vice versa. Set by whoever spawns the local character (the Skills
/// scene's spawner, the game scene's join).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SkillTreeRace {
    #[default]
    Chinese,
    European,
}

impl SkillTreeRace {
    /// Fallback top-level tab labels, for when masterydata or textuisystem
    /// has not loaded — see [`top_tab_labels`], which is what the window
    /// shows. Kept because the offline preview scenes have neither table.
    pub fn top_tabs(self) -> &'static [&'static str] {
        match self {
            SkillTreeRace::Chinese => &["Weapon", "Force"],
            SkillTreeRace::European => &["Physical", "Magical", "Assist"],
        }
    }

    /// Whether a mastery id belongs to this race's tree (CH 257-276, EU 513+;
    /// GM 289 is neither).
    pub fn owns_mastery(self, id: u32) -> bool {
        match self {
            SkillTreeRace::Chinese => id < 289,
            SkillTreeRace::European => id >= 512,
        }
    }
}

/// Top-tab captions, from the player's own data.
///
/// Idea: `skillmasterydata.txt` col 5 names the caption of the tab its mastery
/// sits under (`UIIT_CTL_WEAPON_SKILL`, `UIIT_CTL_EU_SKILLWND_PHYSICAL`, …),
/// and the parser has carried that column as `category_key` without a consumer
/// ever since — the field LUECKEN lists as S1. The window wrote its own five
/// English words instead, and **three of the five disagree with the shipped
/// strings**: the player's `textuisystem.txt` says `Melee` (L3088), `Caster`
/// (L3089) and `Buff` (L3090) where the literals said Physical, Magical and
/// Assist. (CH's two happen to agree: L706 `Weapon`, L707 `Force`.)
///
/// All or nothing on purpose: the tab **count** drives the clamp in
/// [`SkillWindowState::top_tab`] and the art's own 60 px pitch, so a half
/// derived bar would be worse than the literals. A table that does not yield
/// exactly one caption per tab of this race falls back whole.
///
/// The placeholder masteries (`skillmasterydata` ids 1-4: `tab = 255`,
/// `name_key = xxx`, no category) drop out of both tests, which is also why
/// the window never shows them.
pub fn top_tab_labels(
    race: SkillTreeRace,
    masteries: &crate::plugins::textdata::ClientMasteryData,
    strings: &crate::plugins::textdata::ClientUiStrings,
) -> Vec<String> {
    let fallback = race.top_tabs();
    let mut by_tab: std::collections::BTreeMap<u8, &str> = std::collections::BTreeMap::new();
    for info in masteries.iter().filter(|m| race.owns_mastery(m.id)) {
        if info.category_key.is_empty() || info.category_key == "xxx" {
            continue;
        }
        if usize::from(info.tab) >= fallback.len() {
            continue;
        }
        by_tab.entry(info.tab).or_insert(&info.category_key);
    }
    if by_tab.len() != fallback.len() {
        return fallback.iter().map(|label| label.to_string()).collect();
    }
    by_tab
        .values()
        .enumerate()
        .map(|(index, key)| strings.get(key).unwrap_or(fallback[index]).to_string())
        .collect()
}

/// A pending level-up awaiting confirmation in the practice box.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PracticeAction {
    /// Learn the next rung of a skill ladder (the rung's skilldata id).
    LearnSkill { group_id: i32, skill_id: i32 },
    /// Raise a mastery by one level.
    RaiseMastery { mastery: u32 },
}

/// Open/close + tab state of the skill window (the S key).
#[derive(Resource, Default)]
pub struct SkillWindowState {
    pub open: bool,
    /// Selected top-level tab (mastery table col 6).
    pub top_tab: u8,
    /// Selected mastery within the top tab; `None` picks the first.
    pub mastery: Option<u32>,
    /// An open practice (confirmation) box, if any.
    pub prompt: Option<PracticeAction>,
    /// Preserved scroll offset of the branch-row list, so a learn/level-up
    /// rebuild keeps the view where it was (reset on tab/mastery change).
    pub scroll_y: f32,
}

/// A skill icon being carried (skilldata id). From the window it's a
/// vanilla click-carry toward the underbar (survives stray releases;
/// Escape/right-click cancels); lifted *from* an underbar slot it's a
/// hold-drag — releasing outside a slot discards it (vanilla drag-out).
/// Parallel to the inventory's `InventoryState.drag`, sharing the same
/// `DragGhost` cursor icon.
#[derive(Resource, Default)]
pub struct SkillDrag {
    pub skill: Option<u32>,
    pub from_underbar: bool,
}

/// The `KeySkill` shortcut toggles the window (unless the chat input is capturing
/// keys). Rebindable via the Key Map tab; S by default.
pub fn toggle_skill_window(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<ChatState>,
    options: Res<GameOptions>,
    mut state: ResMut<SkillWindowState>,
) {
    let Some(key) = options.key_for(KEY_SKILL) else {
        return;
    };
    if keys.just_pressed(key) && !chat.input_open {
        state.open = !state.open;
        if !state.open {
            state.prompt = None;
        }
    }
}

/// Repaint when the book/SP/tab state feeding the window changes.
pub fn skill_window_needs_refresh(
    state: Res<SkillWindowState>,
    race: Res<SkillTreeRace>,
    book: Res<SkillBook>,
    index: Res<SkillGroupIndex>,
    progress: Res<PlayerProgress>,
    fresh: Query<(), Added<super::ui::SkillWindowRoot>>,
) -> bool {
    state.is_changed()
        || race.is_changed()
        || book.is_changed()
        || index.is_changed()
        || progress.is_changed()
        || !fresh.is_empty()
}

/// Keep quickslot skill references consistent with the book: leveling a
/// skill upgrades its slots to the new rung (like vanilla), withdrawing it
/// entirely clears them.
pub fn sync_quickslots_with_book(
    book: Res<SkillBook>,
    skill_data: Res<ClientSkillData>,
    mut quickslots: ResMut<QuickSlots>,
) {
    if !book.is_changed() {
        return;
    }
    let remap = |action: &Option<SlotAction>| -> Option<Option<SlotAction>> {
        let Some(SlotAction::Skill { ref_id }) = action else {
            return None;
        };
        let group = skill_data.get(&(*ref_id as i32))?.group_id();
        if group == 0 {
            return None;
        }
        match book.learned.get(&group) {
            Some(learned) if learned.skill_id == *ref_id as i32 => None,
            Some(learned) => Some(Some(SlotAction::Skill {
                ref_id: learned.skill_id as u32,
            })),
            None => Some(None),
        }
    };
    // compute first so an unchanged frame doesn't dirty the underbar refresh
    let slot_changes: Vec<(usize, Option<SlotAction>)> = quickslots
        .slots
        .iter()
        .enumerate()
        .filter_map(|(idx, action)| remap(action).map(|new| (idx, new)))
        .collect();
    let special_change = remap(&quickslots.special);
    if slot_changes.is_empty() && special_change.is_none() {
        return;
    }
    for (idx, new_action) in slot_changes {
        quickslots.slots[idx] = new_action;
    }
    if let Some(new_special) = special_change {
        quickslots.special = new_special;
    }
}

/// The mastery's abbreviation used in the `UIIT_STT_MASTERY_GROUP_<abbr>_<n>`
/// branch-series tooltip keys (CH masteries only; EU has no such keys).
pub fn mastery_group_abbrev(id: u32) -> Option<&'static str> {
    Some(match id {
        257 => "VI",   // Bicheon
        258 => "HS",   // Heuksal
        259 => "PA",   // Pacheon
        273 => "HAN",  // Cold
        274 => "PUNG", // Lightning
        275 => "HWA",  // Fire
        276 => "KI",   // Force
        _ => return None,
    })
}

/// Display name for a mastery: its masterydata `name_key` resolved through
/// textuisystem, falling back to the well-known v1.188 names when either
/// table is missing (offline preview scenes).
pub fn mastery_name<'a>(
    id: u32,
    masteries: &'a crate::plugins::textdata::ClientMasteryData,
    strings: &'a crate::plugins::textdata::ClientUiStrings,
) -> &'a str {
    masteries
        .get(id)
        .and_then(|info| strings.get(&info.name_key))
        .unwrap_or_else(|| mastery_display_name(id))
}

/// Hardcoded fallback names for [`mastery_name`], for when textuisystem or
/// masterydata has not loaded.
pub fn mastery_display_name(id: u32) -> &'static str {
    match id {
        257 => "Bicheon",
        258 => "Heuksal",
        259 => "Pacheon",
        273 => "Cold",
        274 => "Lightning",
        275 => "Fire",
        276 => "Force",
        289 => "GM",
        513 => "Warrior",
        514 => "Wizard",
        515 => "Rogue",
        516 => "Warlock",
        517 => "Bard",
        518 => "Cleric",
        _ => "Mastery",
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::assets::textdata::masterydata::{MasteryData, MasteryInfo};
    use crate::plugins::textdata::{ClientMasteryData, ClientUiStrings};
    use std::collections::HashMap;

    /// S1: `skillmasterydata` col 5 names the tab captions and the parser has
    /// carried it as `category_key` with no consumer. Three of the five words
    /// the window wrote itself disagree with the shipped strings — the
    /// player's data says Melee/Caster/Buff where the literals said
    /// Physical/Magical/Assist.
    #[test]
    fn the_top_tab_captions_come_from_the_mastery_table() {
        let masteries = ClientMasteryData::from_data(MasteryData(HashMap::from([
            (513, mastery(513, "UIIT_CTL_EU_SKILLWND_PHYSICAL", 0)),
            (514, mastery(514, "UIIT_CTL_EU_SKILLWND_MASICAL", 1)),
            (517, mastery(517, "UIIT_CTL_EU_SKILLWND_ASSIST", 2)),
            // the placeholder rows: tab 255, no category — must drop out
            (1, mastery(1, "", 255)),
        ])));
        let strings = ClientUiStrings::from_rows(&[
            ("UIIT_CTL_EU_SKILLWND_PHYSICAL", "Melee"),
            ("UIIT_CTL_EU_SKILLWND_MASICAL", "Caster"),
            ("UIIT_CTL_EU_SKILLWND_ASSIST", "Buff"),
        ]);

        assert_eq!(
            top_tab_labels(SkillTreeRace::European, &masteries, &strings),
            vec!["Melee", "Caster", "Buff"],
        );
        assert_ne!(
            top_tab_labels(SkillTreeRace::European, &masteries, &strings),
            SkillTreeRace::European
                .top_tabs()
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>(),
            "the shipped strings differ from the literals this replaces"
        );
    }

    /// All or nothing: the tab count drives the clamp and the art's pitch, so
    /// a table that does not cover every tab of this race falls back whole
    /// rather than showing a short bar.
    #[test]
    fn an_incomplete_mastery_table_keeps_the_fallback_labels() {
        let masteries = ClientMasteryData::from_data(MasteryData(HashMap::from([(
            513,
            mastery(513, "UIIT_CTL_EU_SKILLWND_PHYSICAL", 0),
        )])));
        let strings = ClientUiStrings::from_rows(&[("UIIT_CTL_EU_SKILLWND_PHYSICAL", "Melee")]);
        assert_eq!(
            top_tab_labels(SkillTreeRace::European, &masteries, &strings),
            vec!["Physical", "Magical", "Assist"],
        );
        // and an empty table too
        assert_eq!(
            top_tab_labels(
                SkillTreeRace::Chinese,
                &ClientMasteryData::default(),
                &strings
            ),
            vec!["Weapon", "Force"],
        );
    }

    fn mastery(id: u32, category_key: &str, tab: u8) -> MasteryInfo {
        MasteryInfo {
            id,
            name_key: String::new(),
            tooltip_key: String::new(),
            category_key: category_key.to_string(),
            tab,
            group: 0,
            icon: None,
            focus_icon: None,
        }
    }
}

//! Alchemy box state: the Attribute Grant page's five item slots.
//!
//! Idea: the vanilla alchemy box does not *move* items. A page slot holds a
//! reference to an inventory slot (`CommandID` 0 = the equipment, 1..4 = the
//! stones — `resinfo/ifalchemyenchant.txt`), and the inventory only changes
//! when the server acks a fuse. So placement is pure client state and needs no
//! wire support; only the fuse action does, and that opcode map
//! (`docs/re/systems/alchemy.md`) is still `[S]`-inferred rather than
//! captured, so nothing is sent from here yet.

use bevy::prelude::*;

use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::settings::keymap::KEY_ALCHEMY;
use crate::plugins::settings::options::GameOptions;

/// `GDR_AB_ENCHANT_SLOT_01..04` / `GDR_AB_REINFORCE_SLOT_01..04` — the four
/// stone slots next to the equipment slot (`ifalchemyenchant.txt:88-107` and
/// `ifalchemyreinforce.txt:88-107`, `CommandID` 1..4).
pub const STONE_SLOTS: usize = 4;
/// Page-slot index of `GDR_AB_ENCHANT_SLOT_EQUIP` / `_REINFORCE_SLOT_EQUIP`
/// (`CommandID` 0).
pub const EQUIP_SLOT: usize = 0;

/// The two pages the OLD classic shell hosts, one host control each at the
/// same rect `0,150,376,192` (`ifalchemybox.txt:6-23` `CIFAlchemyEnchantMagic`
/// id 23, `:25-42` `CIFAlchemyReinforce` id 22) — so exactly one of them is
/// visible at a time, and which one is a client-side choice.
///
/// Default = `EquipEnhance`, and the reason is the file's own paint order:
/// `ifalchemybox.txt` lists the reinforce host *after* the enchant host, and in
/// this tree later means further front (its last three entries are `CLOSE`,
/// `DRAG`, `TITLE`, which are unambiguously on top of the pages). That is the
/// authored order rather than a confirmed fact about what the original
/// shows first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlchemyPage {
    /// `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT`, body `ifalchemyreinforce.txt`,
    /// art `alcm_window_reinforcement.ddj` (violet gems).
    #[default]
    EquipEnhance,
    /// `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM`, body `ifalchemyenchant.txt`,
    /// art `alcm_window_allowance.ddj` (green gems).
    AttGrant,
}

impl AlchemyPage {
    /// Both pages, in the order the selector row shows them (the shell's own
    /// authoring order, front page first).
    pub const ALL: [AlchemyPage; 2] = [AlchemyPage::EquipEnhance, AlchemyPage::AttGrant];

    fn index(self) -> usize {
        match self {
            AlchemyPage::EquipEnhance => 0,
            AlchemyPage::AttGrant => 1,
        }
    }
}

/// Open/closed state of the alchemy box, which page is in front, and what sits
/// in each page's slots.
///
/// The slots are **per page**: vanilla instantiates one full slot set per page
/// host (`GDR_AB_ENCHANT_SLOT_*` in one tree, `GDR_AB_REINFORCE_SLOT_*` in the
/// other), so a placement cannot follow the player across a page switch — and
/// it must not, because an elixir is a valid material on one page only
/// (`UIIT_MSG_ENCHANT_SLOT_MISMATCH_REINFORCE`, `textuisystem.txt:2168`).
#[derive(Resource, Default)]
pub struct AlchemyState {
    pub open: bool,
    pub page: AlchemyPage,
    /// A fuse request is out and its ack has not arrived. The button stays
    /// pressable-looking but refuses, because the original's own answer to a
    /// second press is to do nothing: the request builders are called from a
    /// button whose window is modal-ish for the duration, and a double fuse
    /// would send the *same* slot references twice.
    pub pending: bool,
    /// Inventory wire slots per page, indexed like the vanilla `CommandID`s:
    /// 0 = equipment, 1..=4 = the stone slots.
    slots: [[Option<u8>; STONE_SLOTS + 1]; AlchemyPage::ALL.len()],
}

impl AlchemyState {
    /// The slot set of the page that is in front.
    fn current(&self) -> &[Option<u8>; STONE_SLOTS + 1] {
        &self.slots[self.page.index()]
    }

    /// The inventory wire slot shown in page slot `index` (0 = equipment) of
    /// the page that is in front.
    pub fn slot(&self, index: usize) -> Option<u8> {
        self.current().get(index).copied().flatten()
    }

    /// Put an inventory item into page slot `index` of the front page. An item
    /// can only sit in one slot, so placing it again moves it rather than
    /// duplicating it — the slots are references, and two references to one
    /// item would let the player "fuse" a stone with itself.
    pub fn place(&mut self, index: usize, inventory_slot: u8) {
        let page = self.page.index();
        if index >= self.slots[page].len() {
            return;
        }
        for slot in self.slots[page].iter_mut() {
            if *slot == Some(inventory_slot) {
                *slot = None;
            }
        }
        self.slots[page][index] = Some(inventory_slot);
    }

    /// Empty page slot `index` of the front page, returning what was in it.
    pub fn take(&mut self, index: usize) -> Option<u8> {
        let page = self.page.index();
        self.slots[page].get_mut(index).and_then(Option::take)
    }

    /// Drop every placement on *every* page (closing the window releases the
    /// references, and it closes both pages at once).
    pub fn clear(&mut self) {
        self.slots = [[None; STONE_SLOTS + 1]; AlchemyPage::ALL.len()];
        self.pending = false;
    }

    /// The slots a fuse request carries, in `CommandID` order: the equipment
    /// first, then the stones as the row reads. `None` when the page cannot
    /// make a request at all — the box needs the equipment plus at least one
    /// material, which is also exactly the count below which the wire body
    /// would collide with the cancel form
    /// (`packets::agent::alchemy::ALCHEMY_MIN_FUSE_SLOTS`).
    ///
    /// The *order* of the stones follows the original's own slot vector, in
    /// which the equipment slot is `CommandID` 0, so equip first is the file's
    /// order. Whether a server cares about the order of the stones among
    /// themselves is unknown.
    pub fn fuse_slots(&self) -> Option<Vec<u8>> {
        let equipment = self.slot(EQUIP_SLOT)?;
        let mut slots = vec![equipment];
        slots.extend((1..=STONE_SLOTS).filter_map(|index| self.slot(index)));
        (slots.len() >= 2).then_some(slots)
    }
}

/// Toggle with the `KeyAlchemy` shortcut (unless the chat input is capturing
/// keys). Its vanilla default **is** known: `textuisystem.txt` L2273
/// `UIIT_STT_TOGGLE_ENCHANT` reads `Alchemy ( Y )`, so `keymap.rs` binds `Y`
/// (#657 — this comment previously claimed vanilla ships no default, which is
/// what kept the action unbound). A stored `SROptionSet.dat` binding and the
/// options window's Key Map tab both still override it.
pub fn toggle_alchemy_window(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<ChatState>,
    options: Res<GameOptions>,
    mut state: ResMut<AlchemyState>,
) {
    let Some(key) = options.key_for(KEY_ALCHEMY) else {
        return;
    };
    if keys.just_pressed(key) && !chat.input_open {
        state.open = !state.open;
        if !state.open {
            state.clear();
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn alchemy_slots_hold_inventory_references() {
        let mut state = AlchemyState::default();
        state.place(EQUIP_SLOT, 13);
        state.place(1, 20);
        assert_eq!(state.slot(EQUIP_SLOT), Some(13));
        assert_eq!(state.slot(1), Some(20));
        assert_eq!(state.take(1), Some(20));
        assert_eq!(state.slot(1), None);
        assert_eq!(state.take(1), None);
    }

    /// One inventory item cannot occupy two page slots at once.
    #[test]
    fn alchemy_placing_the_same_item_twice_moves_it() {
        let mut state = AlchemyState::default();
        state.place(1, 20);
        state.place(3, 20);
        assert_eq!(state.slot(1), None);
        assert_eq!(state.slot(3), Some(20));
    }

    #[test]
    fn alchemy_clear_releases_every_slot() {
        let mut state = AlchemyState::default();
        state.place(EQUIP_SLOT, 13);
        state.place(4, 21);
        state.clear();
        assert!((0..=STONE_SLOTS).all(|i| state.slot(i).is_none()));
    }

    /// Each page host owns its own slot set in vanilla, so a placement does
    /// not follow the player across a page switch.
    #[test]
    fn alchemy_pages_keep_their_own_slots() {
        let mut state = AlchemyState::default();
        assert_eq!(state.page, AlchemyPage::EquipEnhance);
        state.place(EQUIP_SLOT, 13);
        state.page = AlchemyPage::AttGrant;
        assert_eq!(state.slot(EQUIP_SLOT), None, "the other page is empty");
        state.place(EQUIP_SLOT, 21);
        state.page = AlchemyPage::EquipEnhance;
        assert_eq!(state.slot(EQUIP_SLOT), Some(13));
        // closing releases both pages
        state.clear();
        state.page = AlchemyPage::AttGrant;
        assert!((0..=STONE_SLOTS).all(|i| state.slot(i).is_none()));
    }

    /// Out-of-range indices are ignored rather than panicking (the UI feeds
    /// these from cell components).
    #[test]
    fn alchemy_out_of_range_slot_is_ignored() {
        let mut state = AlchemyState::default();
        state.place(STONE_SLOTS + 1, 7);
        assert_eq!(state.slot(STONE_SLOTS + 1), None);
        assert_eq!(state.take(STONE_SLOTS + 1), None);
    }
}

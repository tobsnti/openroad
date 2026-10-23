//! Alchemy box state: which page is in front, and the five item slots each
//! page holds.
//!
//! Idea: the vanilla alchemy box does not *move* items. A page slot holds a
//! reference to an inventory slot (`CommandID` 0 = the equipment, 1..4 = the
//! stones — `resinfo/ifalchemyenchant.txt`), and the inventory only changes
//! when the server acks a fuse. So placement is pure client state and needs no
//! wire support; only the fuse action does, and that opcode map
//! (`docs/re/systems/alchemy.md`) is still `[S]`-inferred rather than
//! captured, so nothing is sent from here yet.
//!
//! The shell hosts two pages at the same rect, so exactly one is visible at a
//! time and each keeps its own placements.

use bevy::prelude::*;

use packets::agent::alchemy::ALCHEMY_MIN_FUSE_SLOTS;

use crate::plugins::hud::chat::model::ChatState;
use crate::plugins::settings::keymap::KEY_ALCHEMY;
use crate::plugins::settings::options::GameOptions;

/// `GDR_AB_ENCHANT_SLOT_01..04` / `GDR_AB_REINFORCE_SLOT_01..04` — the four
/// stone slots next to the equipment slot (`ifalchemyenchant.txt` ids 38..41 at
/// `164/212/260/308,56,32,32`, `ifalchemyreinforce.txt` identical,
/// `CommandID` 1..4).
pub const STONE_SLOTS: usize = 4;
/// Page-slot index of `GDR_AB_ENCHANT_SLOT_EQUIP` / `_REINFORCE_SLOT_EQUIP`
/// (id 30 at `59,56,32,32`, `CommandID` 0).
pub const EQUIP_SLOT: usize = 0;

/// The three pages the shell hosts, one control each at `0,150,376,h`, in
/// `resinfo/ifnewalchemybox.txt` — the description this client loads:
/// `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM` (`CIFAlchemyEnchantMagic`, id 23),
/// `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT` (`CIFAlchemyReinforce`, id 22) and
/// `GDR_ALCHEMYBOX_ELEMENT_MANUFACTURING` (`CIFAlchemyProcess`, id 21). They
/// share an origin, which is what makes this a page *selection* rather than
/// three windows.
///
/// Default = `EquipEnhance`, from the file's own order: the reinforce host is
/// listed *after* the enchant host, and later in that file means further front —
/// its last three entries are `CLOSE`, `DRAG` and `TITLE`, which are
/// unambiguously on top of the pages. `[S]`: this is the file's own order, not a
/// statement about what the original draws first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AlchemyPage {
    /// `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT` — equipment plus elixir.
    #[default]
    EquipEnhance,
    /// `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM` — equipment plus stone.
    AttGrant,
    /// `GDR_ALCHEMYBOX_ELEMENT_MANUFACTURING` — the shell's third page.
    ///
    /// It is **shown but cannot act**: its request would be the manufacture
    /// opcode, and nothing has ever been seen to carry or answer it, so this page
    /// draws its art and its own description and offers no button. A page that
    /// pretends to act would be worse than one that plainly does not.
    Elementation,
}

/// The tallest page host, which is what the window is composed around so that
/// switching pages cannot resize it.
pub const TALLEST_HOST_H: f32 = 228.0;

impl AlchemyPage {
    /// Selector order, left to right: the file's own order of the three hosts.
    pub const ALL: [AlchemyPage; 3] = [
        AlchemyPage::EquipEnhance,
        AlchemyPage::AttGrant,
        AlchemyPage::Elementation,
    ];

    /// Height of the page host, which is also its art's own DDS height:
    /// 228 for the reinforce and element pages, 192 for the enchant page.
    pub fn host_height(self) -> f32 {
        match self {
            AlchemyPage::AttGrant => 192.0,
            AlchemyPage::EquipEnhance | AlchemyPage::Elementation => 228.0,
        }
    }

    /// Whether the page can form a request at all. The element page cannot:
    /// no opcode carries its action.
    pub fn can_act(self) -> bool {
        !matches!(self, AlchemyPage::Elementation)
    }

    /// Index into the per-page slot array.
    fn index(self) -> usize {
        match self {
            AlchemyPage::EquipEnhance => 0,
            AlchemyPage::AttGrant => 1,
            AlchemyPage::Elementation => 2,
        }
    }
}

/// Open/closed state of the alchemy box, which page is in front, and what sits
/// in each page's slots.
#[derive(Resource, Default)]
pub struct AlchemyState {
    pub open: bool,
    /// The page host that is in front.
    pub page: AlchemyPage,
    /// A fuse request is out. The button keeps its enabled face but refuses,
    /// because sending the *same* slot references twice is what a double press
    /// would do.
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

    /// The slots a fuse request carries: the **equipment first**, then the
    /// stones in `CommandID` order as the row reads. `None` when the page cannot
    /// make a request at all — the box needs the equipment plus at least one
    /// material, which is also the count below which the stone body would sit
    /// one byte from the cancel form
    /// (`packets::agent::alchemy::ALCHEMY_MIN_FUSE_SLOTS`).
    pub fn fuse_slots(&self) -> Option<Vec<u8>> {
        let equip = self.slot(EQUIP_SLOT)?;
        let mut slots = vec![equip];
        slots.extend((1..=STONE_SLOTS).filter_map(|index| self.slot(index)));
        (slots.len() >= ALCHEMY_MIN_FUSE_SLOTS).then_some(slots)
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

    /// Each page host owns its own slot set, so a placement does not follow
    /// the player across a page switch.
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
        // closing releases both pages at once
        state.clear();
        state.page = AlchemyPage::AttGrant;
        assert_eq!(state.slot(EQUIP_SLOT), None);
    }

    /// The wire order is the window order: equipment first, then the stones as
    /// the row reads, and a page that cannot make a request yields nothing.
    #[test]
    fn the_fuse_list_leads_with_the_equipment() {
        let mut state = AlchemyState::default();
        assert_eq!(state.fuse_slots(), None, "an empty page sends nothing");
        state.place(EQUIP_SLOT, 19);
        assert_eq!(
            state.fuse_slots(),
            None,
            "the equipment alone is not a fuse"
        );
        state.place(1, 15);
        assert_eq!(state.fuse_slots(), Some(vec![19, 15]));
        // a gap in the stone row does not reorder what is left
        state.place(3, 16);
        assert_eq!(state.fuse_slots(), Some(vec![19, 15, 16]));
        // and without the equipment there is no request, however many stones
        state.take(EQUIP_SLOT);
        assert_eq!(state.fuse_slots(), None);
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

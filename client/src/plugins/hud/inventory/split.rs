//! Stack split — vanilla's `MsgBoxDivideCount` prompt.
//!
//! Idea: the original client raises a message box of kind 5 when you
//! **Shift-click** a bag item, asks how many pieces to split off, and on
//! Confirm sends the ordinary `0x7034` op-0 move into the **first free bag
//! slot** with that amount. Nothing here is invented: the gesture, the two
//! preconditions and the target rule are the original's own behaviour.
//!
//! What the original client does:
//! * its item-click dispatcher reacts to Shift held down **and** a clicked
//!   container id of `0x46` (inventory — the same container enumeration the
//!   storage builders use, `packets/src/agent/inventory.rs`), which enters the
//!   split precondition check;
//! * that check refuses a slot that holds nothing and a stack of `<= 1`, then
//!   opens msgbox kind 5 — kind 5 is `"MsgBoxDivideCount"` in the dispatcher's
//!   own string switch — and remembers the source slot;
//! * its confirm handler walks the container's slot vector and takes the index
//!   of the **first empty entry**, or `-1` when there is none. On `-1` it drops
//!   the whole operation; otherwise it builds a move `0x46 -> 0x46` with the
//!   entered amount and forgets the source slot again.
//!
//! This module is the rule half: the state the prompt carries and the three
//! questions the original asks before it opens. The box that asks them lives
//! next to it (`sync_split_modal`).
//!
//! What it deliberately does **not** do is mirror the split locally: the bag
//! is applied from the `0xB034` ack only, where `Inventory::apply_move`
//! honours the echoed amount for a partial move into an empty slot
//! (`net/inventory.rs`).

use bevy::prelude::*;

use packets::agent::character_data::ItemTypeData;

use crate::plugins::net::inventory::{Inventory, BAG_FIRST_SLOT};
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientTextNames};

/// The open split prompt, or `None`. One at a time, like every other msgbox.
#[derive(Resource, Default)]
pub struct SplitPrompt {
    pub prompt: Option<SplitRequest>,
}

/// What the box was opened on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitRequest {
    /// Wire slot of the stack being divided.
    pub source: u8,
    /// Its stack count at open time — the box can split off `1..=stack-1`.
    pub stack: u16,
    pub ref_id: u32,
    pub name: String,
}

/// The "distributed number" currently entered.
#[derive(Resource)]
pub struct SplitAmount(pub u16);

impl Default for SplitAmount {
    fn default() -> Self {
        Self(1)
    }
}

/// The stack count of `slot`, or 0 for an empty or non-stacking slot.
///
/// Non-expendables carry no `stack_count` on the wire and never stack, so they
/// can never open this box.
pub fn stack_of(inventory: &Inventory, slot: u8) -> u16 {
    match inventory.get(slot).map(|item| &item.data) {
        Some(ItemTypeData::Expendable { stack_count, .. }) => *stack_count,
        _ => 0,
    }
}

/// The original's first-free-slot lookup: the first empty slot of the
/// container, `None` when it is full.
///
/// Ours starts at [`BAG_FIRST_SLOT`] because our slot numbers are the server's
/// absolute ones while the original's are bag-relative — the same bias the
/// storage builders carry (`packets/src/agent/inventory.rs`).
pub fn first_free_bag_slot(inventory: &Inventory) -> Option<u8> {
    (BAG_FIRST_SLOT..inventory.size()).find(|slot| inventory.get(*slot).is_none())
}

/// Can this slot be divided at all? The original refuses twice: the slot has
/// to hold something, and its stack has to be `> 1`.
pub fn splittable(inventory: &Inventory, slot: u8) -> bool {
    slot >= BAG_FIRST_SLOT && stack_of(inventory, slot) > 1
}

/// Open the box on `slot` if it may be divided, and report whether it opened.
///
/// An exclusive closure rather than system parameters: the only caller is the
/// slot-press observer in `ui.rs`, which is already at Bevy's 16-parameter
/// ceiling — a 17th makes the whole tuple fail the trait with an error that
/// names no type. The keyboard read and this resource write therefore happen
/// in a `commands.queue` closure. The caller undoes the carry it started when
/// this returns `true`.
pub fn open_split_prompt(world: &mut World, slot: u8) -> bool {
    let Some(inventory) = world
        .query_filtered::<&Inventory, With<Player>>()
        .iter(world)
        .next()
        .cloned()
    else {
        return false;
    };
    if !splittable(&inventory, slot) {
        return false;
    }
    let stack = stack_of(&inventory, slot);
    let Some(item) = inventory.get(slot) else {
        return false;
    };
    let ref_id = item.ref_id;
    let name = world
        .get_resource::<ClientItemData>()
        .and_then(|data| data.get(&(ref_id as i32)))
        .and_then(|row| row.name_key())
        .and_then(|key| {
            world
                .get_resource::<ClientTextNames>()
                .and_then(|names| names.name(key))
        })
        .map(str::to_string)
        .unwrap_or_default();
    let Some(mut prompt) = world.get_resource_mut::<SplitPrompt>() else {
        return false;
    };
    prompt.prompt = Some(SplitRequest {
        source: slot,
        stack,
        ref_id,
        name,
    });
    true
}

/// The amount a split may produce: `1..=stack-1`. Never the whole stack, which
/// would be a plain move, and never 0.
pub fn clamp_amount(entered: u16, stack: u16) -> u16 {
    entered.clamp(1, stack.saturating_sub(1).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use packets::agent::character_data::{InventoryItem, RentInfo};

    fn stack_item(slot: u8, count: u16) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id: 4,
            data: ItemTypeData::Expendable {
                stack_count: count,
                assimilation_prob: None,
                mag_params: vec![],
            },
        }
    }

    /// Anything that is not an expendable carries no stack count at all —
    /// `ItemTypeData::Unknown` stands in for that class here.
    fn non_stacking(slot: u8) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id: 100,
            data: ItemTypeData::Unknown,
        }
    }

    fn inventory(size: u8, items: Vec<InventoryItem>) -> Inventory {
        let mut slots = vec![None; size as usize];
        for item in items {
            let index = item.slot as usize;
            if index >= slots.len() {
                slots.resize(index + 1, None);
            }
            slots[index] = Some(item);
        }
        Inventory {
            slots,
            avatar_slots: vec![],
            gold: 0,
        }
    }

    /// The original's two refusals, plus the container check that keeps the
    /// paper doll out of it.
    #[test]
    fn only_a_bag_stack_of_more_than_one_can_be_split() {
        let inv = inventory(
            45,
            vec![
                non_stacking(6),
                stack_item(BAG_FIRST_SLOT, 20),
                stack_item(BAG_FIRST_SLOT + 1, 1),
                non_stacking(BAG_FIRST_SLOT + 2),
            ],
        );
        assert!(splittable(&inv, BAG_FIRST_SLOT), "a 20-stack splits");
        assert!(
            !splittable(&inv, BAG_FIRST_SLOT + 1),
            "a single piece is not a stack: the original refuses a stack of <= 1"
        );
        assert!(
            !splittable(&inv, BAG_FIRST_SLOT + 2),
            "equipment never stacks, so it carries no stack count"
        );
        assert!(
            !splittable(&inv, 6),
            "an equipped item is not container 0x46"
        );
        assert!(
            !splittable(&inv, BAG_FIRST_SLOT + 3),
            "an empty slot has nothing to divide"
        );
    }

    /// The original's first-free-slot lookup: first empty slot, `None` when
    /// the bag is full.
    #[test]
    fn the_split_target_is_the_first_free_bag_slot() {
        let mut items: Vec<InventoryItem> = (BAG_FIRST_SLOT..BAG_FIRST_SLOT + 3)
            .map(|slot| stack_item(slot, 5))
            .collect();
        items.push(stack_item(BAG_FIRST_SLOT + 4, 5));
        let inv = inventory(BAG_FIRST_SLOT + 5, items);
        assert_eq!(first_free_bag_slot(&inv), Some(BAG_FIRST_SLOT + 3));

        let full = inventory(
            BAG_FIRST_SLOT + 2,
            (BAG_FIRST_SLOT..BAG_FIRST_SLOT + 2)
                .map(|slot| stack_item(slot, 5))
                .collect(),
        );
        assert_eq!(first_free_bag_slot(&full), None, "a full bag sends nothing");
    }

    /// The amount the box may produce: `1..=stack-1` — never the whole stack
    /// (that is a plain move) and never 0. The box clamps with this rule, so
    /// it is pinned here before the box exists.
    #[test]
    fn the_distributed_number_stays_between_one_and_stack_minus_one() {
        assert_eq!(clamp_amount(0, 20), 1);
        assert_eq!(clamp_amount(19, 20), 19);
        assert_eq!(clamp_amount(20, 20), 19);
        assert_eq!(clamp_amount(9999, 20), 19);
        // a 2-stack can only ever give one piece away
        assert_eq!(clamp_amount(5, 2), 1);
    }
}

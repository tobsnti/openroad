//! The avatar container (0x7034 ops 35/36) — putting a costume item on and
//! taking it off.
//!
//! Idea: the five avatar slots are a **second container**, not five more
//! equipment slots. `Inventory::avatar_slots` has its own slot numbering, so a
//! move between it and the bag is not the ordinary op-0 move the paper doll
//! uses — the client has two dedicated ops for it, which is why the avatar
//! panel had no gesture at all until now (it was drawn from 0x3013 and read by
//! nothing that could change it).
//!
//! How the wire looks (the two request variants in
//! `packets/src/agent/inventory.rs` carry the detail): the original's
//! item-move builder switches on the source container id and knows `0x4E` as
//! the avatar container — `0x4E -> 0x46` is op `0x23` (35), `0x46 -> 0x4E` is
//! op `0x24` (36) — and its inventory-op serializer writes each request as two
//! bare slot bytes.
//!
//! Gestures, both taken from what this window already does rather than
//! invented:
//! * **Left-click while carrying a bag item** drops it on the avatar slot —
//!   the second click of the vanilla click-carry, exactly like dropping on an
//!   equipment slot (`ui.rs` `on_slot_press`).
//! * **Right-click on an occupied avatar slot** takes the item off into the
//!   first free bag slot — the rule `right_click_action` already applies to
//!   equipment, and the same "first empty slot" target the split box uses
//!   (`split::first_free_bag_slot`, the original's first-free-slot lookup).
//!
//! Nothing is mirrored locally: the containers change only when the 0xB034 ack
//! arrives (`model::on_inventory_operation_response` ->
//! `Inventory::apply_avatar_move`), like every other move in this window.
//!
//! This module registers **no systems** — it is one entity-scoped observer,
//! attached where the avatar cells are spawned, so the scene gate of `hud/mod.rs`
//! is not involved (the cells only exist while the window does).

use bevy::picking::pointer::PointerButton;
use bevy::prelude::*;

use packets::agent::prelude::InventoryOperationRequest;
use packets::Packet;

use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::split::first_free_bag_slot;
use crate::plugins::hud::inventory::ui::{AvatarSlotCell, DragGhost};
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;

/// What a press on an avatar slot resolves to, so the decision is testable
/// without a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AvatarClick {
    /// Put the carried bag item on: op 36, `bag -> avatar`.
    PutOn {
        bag: u8,
        avatar: u8,
    },
    /// Take this avatar slot off into a free bag slot: op 35.
    TakeOff {
        avatar: u8,
        bag: u8,
    },
    /// Cancel the carry (right-click while carrying), like `on_slot_press`.
    CancelCarry,
    Nothing,
}

/// The decision itself. `carried` is `InventoryState::drag`.
pub(crate) fn avatar_click_action(
    avatar: u8,
    primary: bool,
    carried: Option<u8>,
    inventory: &Inventory,
) -> AvatarClick {
    if !primary {
        if carried.is_some() {
            return AvatarClick::CancelCarry;
        }
        // taking off needs something to take off and somewhere to put it
        if inventory.get_avatar(avatar).is_none() {
            return AvatarClick::Nothing;
        }
        return first_free_bag_slot(inventory).map_or(AvatarClick::Nothing, |bag| {
            AvatarClick::TakeOff { avatar, bag }
        });
    }
    match carried {
        Some(bag) => AvatarClick::PutOn { bag, avatar },
        // A carry *out of* the avatar container would need a second drag kind
        // in `InventoryState`; the right-click above is the take-off gesture,
        // the same one the equipment slots use.
        None => AvatarClick::Nothing,
    }
}

/// Press on one of the five avatar cells.
pub fn on_avatar_press(
    mut press: On<Pointer<Press>>,
    cells: Query<&AvatarSlotCell>,
    mut state: ResMut<InventoryState>,
    inventories: Query<&Inventory, With<Player>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut commands: Commands,
) {
    press.propagate(false);
    let Ok(cell) = cells.get(press.entity) else {
        return;
    };
    let Ok(inventory) = inventories.single() else {
        return;
    };
    let primary = press.event.button == PointerButton::Primary;
    let action = avatar_click_action(cell.slot, primary, state.drag, inventory);
    let request = match action {
        AvatarClick::PutOn { bag, avatar } => InventoryOperationRequest::InventoryToAvatar {
            source: bag,
            target: avatar,
        },
        AvatarClick::TakeOff { avatar, bag } => InventoryOperationRequest::AvatarToInventory {
            source: avatar,
            target: bag,
        },
        AvatarClick::CancelCarry => {
            clear_carry(&mut state, &ghosts, &mut commands);
            return;
        }
        AvatarClick::Nothing => return,
    };
    // the carry ends either way, like the bag/equipment drop does — the
    // containers themselves wait for the 0xB034 ack
    clear_carry(&mut state, &ghosts, &mut commands);
    let Ok(conn) = conn.single() else {
        warn!("inventory: not sending avatar move, no agent connection");
        return;
    };
    debug!("inventory: avatar op {request:?}");
    if let Err(e) = conn.get_sender().send(Packet::from(request).into()) {
        error!("network: failed to send avatar move: {}", e.0);
    }
}

/// Same two lines `ui::end_carry` runs; kept here so this module does not have
/// to widen that function's visibility.
fn clear_carry(
    state: &mut InventoryState,
    ghosts: &Query<Entity, With<DragGhost>>,
    commands: &mut Commands,
) {
    state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use packets::agent::character_data::{InventoryItem, ItemTypeData, RentInfo};

    use crate::plugins::net::inventory::BAG_FIRST_SLOT;

    fn item(slot: u8) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id: 7,
            data: ItemTypeData::Unknown,
        }
    }

    fn inventory(bag: Vec<u8>, avatar: Vec<u8>) -> Inventory {
        let mut slots = vec![None; (BAG_FIRST_SLOT + 4) as usize];
        for slot in bag {
            slots[slot as usize] = Some(item(slot));
        }
        let mut avatar_slots = vec![None; 5];
        for slot in avatar {
            avatar_slots[slot as usize] = Some(item(slot));
        }
        Inventory {
            slots,
            avatar_slots,
            gold: 0,
        }
    }

    #[test]
    fn a_carried_bag_item_dropped_on_an_avatar_slot_puts_it_on() {
        let inv = inventory(vec![BAG_FIRST_SLOT], vec![]);
        assert_eq!(
            avatar_click_action(2, true, Some(BAG_FIRST_SLOT), &inv),
            AvatarClick::PutOn {
                bag: BAG_FIRST_SLOT,
                avatar: 2
            }
        );
        // an occupied avatar slot is a swap, not a refusal — the server
        // decides, and `apply_avatar_move` mirrors what it confirms
        let inv = inventory(vec![BAG_FIRST_SLOT], vec![2]);
        assert_eq!(
            avatar_click_action(2, true, Some(BAG_FIRST_SLOT), &inv),
            AvatarClick::PutOn {
                bag: BAG_FIRST_SLOT,
                avatar: 2
            }
        );
        // idle left-click does nothing: there is no avatar-side carry
        assert_eq!(
            avatar_click_action(2, true, None, &inv),
            AvatarClick::Nothing
        );
    }

    #[test]
    fn right_click_takes_the_avatar_item_off_into_the_first_free_bag_slot() {
        let inv = inventory(vec![BAG_FIRST_SLOT, BAG_FIRST_SLOT + 1], vec![0]);
        assert_eq!(
            avatar_click_action(0, false, None, &inv),
            AvatarClick::TakeOff {
                avatar: 0,
                bag: BAG_FIRST_SLOT + 2
            }
        );
        // empty avatar slot: nothing to take off
        assert_eq!(
            avatar_click_action(1, false, None, &inv),
            AvatarClick::Nothing
        );
        // right-click while carrying cancels, exactly like `on_slot_press`
        assert_eq!(
            avatar_click_action(0, false, Some(BAG_FIRST_SLOT), &inv),
            AvatarClick::CancelCarry
        );
    }

    #[test]
    fn a_full_bag_has_nowhere_to_put_the_avatar_item() {
        let full: Vec<u8> = (BAG_FIRST_SLOT..BAG_FIRST_SLOT + 4).collect();
        let inv = inventory(full, vec![0]);
        assert_eq!(
            avatar_click_action(0, false, None, &inv),
            AvatarClick::Nothing
        );
    }
}

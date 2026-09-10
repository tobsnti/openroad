//! One answer to "what item is under the cursor", for every window that draws
//! item cells.
//!
//! Idea: the HUD has eleven modules that paint a grid of items (inventory,
//! storage, guild storage, shop, exchange, stall, quickslots, paperdoll…), and
//! before this resource each one answered that question for itself — or not at
//! all. The inventory published a *slot index* through `InventoryState`, the
//! storage window published the *item* through a resource of its own, the shop
//! polled its cells straight into a text field, and exchange collected
//! `Hovered` and threw it away. The visible result was the one the player
//! reports first: tooltips in the inventory, nothing in the shop, and the
//! impression that the window is dead (2026-08-17).
//!
//! So the *question* gets one owner. A window publishes what it is hovering
//! and stops there; the tooltip renders it. Adding the next window is one
//! observer, not another tooltip implementation.

use bevy::prelude::*;
use packets::agent::character_data::InventoryItem;

/// What the cursor is currently over, across all item grids.
///
/// Two shapes, because two genuinely different things are hovered: an item the
/// player *owns* (with its stack count, durability and magic options) and an
/// entry in a *catalog* (a shop's stock — a ref id and a price, no instance
/// data). The catalog case must not be faked into an `InventoryItem`: a
/// tooltip that invents durability for something nobody owns yet is exactly
/// the class of made-up value ADR-0009 rules out.
#[derive(Resource, Default)]
pub struct HoveredItem(pub Option<HoveredItemKind>);

#[derive(Clone, Debug)]
pub enum HoveredItemKind {
    /// An item the player holds (inventory, storage, exchange, stall…).
    Owned(InventoryItem),
    /// A shop entry: itemdata ref id plus the priced line the shop renders.
    Catalog { ref_id: i32, price: String },
}

impl HoveredItem {
    pub fn owned(&self) -> Option<&InventoryItem> {
        match &self.0 {
            Some(HoveredItemKind::Owned(item)) => Some(item),
            _ => None,
        }
    }

    pub fn catalog(&self) -> Option<(i32, &str)> {
        match &self.0 {
            Some(HoveredItemKind::Catalog { ref_id, price }) => Some((*ref_id, price.as_str())),
            _ => None,
        }
    }

    pub fn clear(&mut self) {
        if self.0.is_some() {
            self.0 = None;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The two shapes must stay distinguishable: a shop entry is not an owned
    /// item, and reading it as one is how invented stats would creep in.
    #[test]
    fn a_catalog_entry_is_not_an_owned_item() {
        let mut hovered = HoveredItem(Some(HoveredItemKind::Catalog {
            ref_id: 3626,
            price: "1,000 Gold".into(),
        }));
        assert!(hovered.owned().is_none());
        assert_eq!(hovered.catalog(), Some((3626, "1,000 Gold")));
        hovered.clear();
        assert!(hovered.catalog().is_none());
    }
}

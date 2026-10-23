//! Idea: an offline preview of the guild warehouse. Run with
//! `SCENE=ui_testing`. The window is built by its own `Update` system
//! (`sync_guild_storage_window`, which runs in `SceneState::UiTesting`), so
//! this only fills the model the **network** layer would otherwise write: an
//! open session, mirrored slots and the guild account's gold. Without a
//! server, `GuildStorageState` is empty and the window never appears at all —
//! which is exactly what the gallery run showed.
//!
//! Everything here is **mock**: the session's npc is a placeholder entity (no
//! npc is spawned in this scene), and the ref ids are real itemdata rows so
//! the cells resolve icons and names from the player's own data rather than
//! from invented strings.

use bevy::prelude::*;

use packets::agent::character_data::{EquipmentData, InventoryItem, ItemTypeData, RentInfo};

use crate::plugins::hud::guild_storage::model::{GuildStorageSession, GuildStorageState};
use crate::plugins::net::inventory::Inventory;
use crate::scenes::SceneState;

pub struct GuildStorageUiPreviewPlugin;

impl Plugin for GuildStorageUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(SceneState::UiTesting), seed_mock_guild_storage);
    }
}

/// Real itemdata rows, so the cells draw the player's own icons.
const CH_SPEAR: u32 = 3632; // ITEM_CH_SPEAR_01_A
const CH_SHIELD: u32 = 3681; // ITEM_CH_SHIELD_01_A
const HP_POTION: u32 = 4; // ITEM_ETC_HP_POTION_01

fn seed_mock_guild_storage(mut state: ResMut<GuildStorageState>) {
    let equipment = |slot: u8, ref_id: u32, opt_level: u8| InventoryItem {
        slot,
        rent: RentInfo::default(),
        ref_id,
        data: ItemTypeData::Equipment(EquipmentData {
            opt_level,
            variance: 0,
            durability: 40,
            mag_params: vec![],
            socket_tag: 1,
            sockets: vec![],
            elixir_tag: 2,
            adv_elixirs: vec![],
        }),
    };
    let stack = |slot: u8, ref_id: u32, count: u16| InventoryItem {
        slot,
        rent: RentInfo::default(),
        ref_id,
        data: ItemTypeData::Expendable {
            stack_count: count,
            assimilation_prob: None,
            mag_params: vec![],
        },
    };

    // Two pages' worth of capacity with holes, so the preview shows filled
    // cells, empty cells and a page strip that has somewhere to go.
    let mut slots: Vec<Option<InventoryItem>> = vec![None; 48];
    for item in [
        equipment(0, CH_SPEAR, 4),
        equipment(1, CH_SHIELD, 0),
        stack(3, HP_POTION, 250),
        stack(11, HP_POTION, 7),
        equipment(26, CH_SPEAR, 0),
    ] {
        // the index is bound first: in `slots[..] = Some(item)` the right side
        // is evaluated before the place expression, so reading `item.slot`
        // inside the brackets reads a moved value (E0382)
        let slot = item.slot as usize;
        slots[slot] = Some(item);
    }

    state.session = Some(GuildStorageSession {
        // no npc exists in this scene; the field only matters for the 0x7251
        // close, which a preview never sends
        npc: Entity::PLACEHOLDER,
        npc_id: 0,
    });
    state.items = Inventory {
        slots,
        avatar_slots: vec![],
        gold: 98_765_432,
    };
    state.synced = true;
    state.active_page = 0;
}

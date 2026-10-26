//! The mall's article page (`resinfo/ifitemmall.txt` + `ifitemmallshop.txt`
//! + `ifitemmallshopslot.txt`): the storefront strip and the goods of the
//! selected store.
//!
//! Idea: both halves are already in the tree and meet on one key. The tab rows
//! come from `mallitemmenulistdata.txt` through [`ClientMallMenu`], and the
//! goods from the ref-shop chain through `ClientShops::shop_for_store` — and
//! the table's `code` column (`MALL_CONSUME`, `MALL_AVATAR`, …) *is* the store
//! codename that chain is keyed by. Nothing here transcribes a list.
//!
//! **One translation, straight from the data.** The original draws this as a
//! full screen: the menu column `GDR_ITEM_MALL_MENU_FRAME_1` `9,37,122,290`
//! and the article pane `GDR_ITEM_MALL_SHOPLIST` `120,37,409,555`
//! (`ifitemmall.txt:158`, `:510`). This window is their union with the origin
//! `(9, 37)` subtracted, so every authored distance survives; the pane's own
//! blocks keep their `ifitemmallshop.txt` coordinates plus the pane offset.
//! What the screen also holds and this window does not: the right column
//! (`GDR_ITEM_MALL_MYINFO` — its own pane, already built in `super::ui`), the
//! trunk, and the five screen buttons.
//!
//! **A row is the authored slot.** `GDR_ITEM_MALL_SHOP_SLOT_1..6`
//! (`ifitemmallshop.txt:1163` down to `:1068`) sit at x=19, y=81, 154, 227,
//! 300, 373, 446 — pitch **73** — and the slot's own tree
//! (`ifitemmallshopslot.txt`) gives the plate `mall_bar01.ddj` (measured
//! 380x76 in the player's archive), the 64x64 icon at `4,4`, the name at
//! `82,16,188,15`, the price at `305,16,33,15`, the silk mark
//! `mall_moneybutton.ddj` (20x20) at `284,12` and the unit label at
//! `342,16,28,15` with the key `UIIT_STT_ROLL_OF_CLOTH`.
//!
//! **Version note (the mandate to replace *any* client).** This archive ships
//! a second generation of the same screen: `ifnewitemmall.txt` with
//! `ifnewitemmallshopitemslot.txt`, `..._shopmainslot`, `..._shopsubslot` and
//! `ifnewitemmallmyinfo.txt`. The page built here is the `ifitemmall*` one,
//! because that is the generation whose rows our loaded tables describe
//! (`mallitemmenulistdata.txt` names `interface\mall\mall_item_tab.ddj`, the
//! 116x32 art of *this* menu). A client that ships only the `new` tree needs
//! its own page, not a patched one — the slot trees differ in block count and
//! in which control carries the price.
//!
//! **Deliberately not here, each with its reason:** the slot's five buttons
//! (`_BUTTON_1..5`: put on, buy, present, basket, bookmark) — there is no
//! purchase opcode anywhere in this repo, which is the same reason
//! `super::ui` ships without the buy-silk button; the `CIFPageManager`
//! (`:1251`), which needs a page state the first six goods do not; and the
//! header's `CIFPML` description (`:147`), which would need an SML renderer
//! we do not have.

use bevy::prelude::*;
use bevy::text::LineHeight;
use bevy::ui::widget::NodeImageMode;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::textdata::shops::{ShopCurrency, ShopGood};
use crate::assets::FontAssets;
use crate::plugins::hud::art::art_path;
use crate::plugins::hud::game_window::{self, abs_node, GameWindowStyle};
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::textdata::{
    ClientItemData, ClientItemIndex, ClientMallMenu, ClientShops, ClientTextNames, ClientUiStrings,
};

/// Authored screen origin of this window's content: the menu column's corner
/// (`ifitemmall.txt:158`). Every rect below is an authored screen rect minus
/// this, so the two halves keep their real distance.
const ORIGIN: (f32, f32) = (9.0, 37.0);
/// `GDR_ITEM_MALL_SHOPLIST` `120,37,409,555` (`:510`) — the article pane's
/// authored screen rect. Everything below is derived from it and [`ORIGIN`]
/// rather than pre-subtracted by hand.
const PANE: (f32, f32, f32, f32) = (120.0, 37.0, 409.0, 555.0);
/// The union of the menu column (`9,37,122,290`) and the pane, in content
/// space.
const CONTENT_W: f32 = PANE.0 + PANE.2 - ORIGIN.0;
const CONTENT_H: f32 = PANE.1 + PANE.3 - ORIGIN.1;
/// The pane's origin inside the content box; shop-local rects are offset by it.
const PANE_X: f32 = PANE.0 - ORIGIN.0;
/// Ours: the window sits where the other mall pane does.
const WINDOW_RIGHT: f32 = 700.0;
const WINDOW_TOP: f32 = 40.0;
/// `mall_item_tab.ddj`, measured 116x32 — the art `mallitemmenulistdata.txt`
/// names for a tab row. Nine rows at 32 fill the authored column's 290 of
/// height exactly once (288), and the 3 is `(122 - 116) / 2`.
const TAB_X: f32 = 3.0;
const TAB_W: f32 = 116.0;
const TAB_H: f32 = 32.0;
/// The icon the table names per row, measured 20x20 (`mall_consum_icon.ddj`).
const TAB_ICON: f32 = 20.0;
/// Six slots at x=19, from y=81, pitch 73 (`ifitemmallshop.txt:1068-1163`).
const SLOT_X: f32 = 19.0;
const SLOT_FIRST_Y: f32 = 81.0;
const SLOT_PITCH: f32 = 73.0;
pub const SLOTS_PER_PAGE: usize = 6;
/// `mall_bar01.ddj`, measured 380x76 — the row plate, drawn at its native
/// size because the descriptor's `Rect` is `0,0,0,0`.
const PLATE: (f32, f32) = (380.0, 76.0);
/// Slot-local rects of `ifitemmallshopslot.txt`.
const ICON_RECT: (f32, f32, f32, f32) = (4.0, 4.0, 64.0, 64.0);
const NAME_RECT: (f32, f32, f32, f32) = (82.0, 16.0, 188.0, 15.0);
const PRICE_RECT: (f32, f32, f32, f32) = (305.0, 16.0, 33.0, 15.0);
const UNIT_RECT: (f32, f32, f32, f32) = (342.0, 16.0, 28.0, 15.0);
const MARK_RECT: (f32, f32, f32, f32) = (284.0, 12.0, 20.0, 20.0);
/// `GDR_ITEM_MALL_SHOP_BG_1` `16,16,377,12` (`:82`) — the caption band, one
/// 12-unit line tall, so its text box is a one-line box.
const CAPTION_RECT: (f32, f32, f32, f32) = (16.0, 16.0, 377.0, 12.0);
/// Every text block in both trees carries `FontIndex=0`.
const FONT_INDEX: usize = 0;
/// Art the two trees name. `TAB_ART` is the row art
/// `mallitemmenulistdata.txt` column 5 gives every tab; the icon per row is
/// column 6 and comes from the table itself.
const TAB_ART: &str = "interface\\mall\\mall_item_tab.ddj";
const PLATE_ART: &str = "interface\\mall\\mall_bar01.ddj";
const MARK_ART: &str = "interface\\mall\\mall_moneybutton.ddj";
/// Ours: the icon sits inset in the 116x32 row, and the label starts after it.
const TAB_ICON_INSET: f32 = 6.0;
const TAB_LABEL_X: f32 = 30.0;
const TAB_LABEL_Y: f32 = 10.0;
/// `GDR_ITEM_MALL_SHOP_MAIN_BG` `26,81,364,440` (`:1045`), the list's own
/// backing tile letter.
const LIST_BG_RECT: (f32, f32, f32, f32) = (26.0, 81.0, 364.0, 440.0);
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_d.ddj";

/// Which storefront the page shows, by its `mallitemmenulistdata` order.
#[derive(Resource, Default, Debug)]
pub struct MallShopState {
    pub open: bool,
    pub tab: usize,
}

#[derive(Component)]
pub struct MallShopRoot;

/// A tab row, by its index in [`ClientMallMenu::tabs`].
#[derive(Component, Clone, Copy)]
pub struct MallShopTab(pub usize);

fn shop(rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (rect.0 + PANE_X, rect.1, rect.2, rect.3)
}

/// The goods of one storefront, flattened across its authored tabs.
///
/// The chain gives a store pages → tabs → slotted goods; the page shows the
/// first [`SLOTS_PER_PAGE`] of them in that order, because the authored pager
/// is not built (see the module note). Measured in this archive: the five
/// storefronts hold 101 goods over 21 tabs, 17 of which carry any.
fn goods_of<'a>(shops: &'a ClientShops, store: &str) -> Vec<&'a ShopGood> {
    shops
        .shop_for_store(store)
        .map(|layout| {
            layout
                .pages
                .iter()
                .flat_map(|page| page.tabs.iter())
                .flat_map(|tab| tab.goods.iter())
                .collect()
        })
        .unwrap_or_default()
}

/// An amount with the unit its currency names, or bare while the flag is
/// UNKNOWN. The mall's own 101 goods all carry silk, whose unit the page
/// already prints next to every row from the slot's own `UIIT_STT_ROLL_OF_CLOTH`
/// label, so silk prints bare here.
fn priced(good: &ShopGood) -> String {
    match good.currency {
        ShopCurrency::Silk => good.price.to_string(),
        other => match other.label() {
            Some(unit) => format!("{} {}", good.price, unit),
            None => good.price.to_string(),
        },
    }
}

/// Rebuild the page whenever the selection or the open flag changes — the
/// shape `sync_store_window` uses, and the reason is the same: a row's icon,
/// name and price all change together, so respawning the six rows is simpler
/// than mutating three cells each and cannot leave one of them stale.
#[allow(clippy::too_many_arguments)]
pub fn sync_mall_shop_window(
    state: Res<MallShopState>,
    existing: Query<Entity, With<MallShopRoot>>,
    menu: Res<ClientMallMenu>,
    shops: Res<ClientShops>,
    item_index: Res<ClientItemIndex>,
    item_data: Res<ClientItemData>,
    names: Res<ClientTextNames>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !state.is_changed() {
        return;
    }
    for root in existing.iter() {
        commands.entity(root).despawn();
    }
    if !state.open {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("mall shop: no 2d camera to attach to");
        return;
    };
    let tabs = menu.tabs();
    let s = hud_scale();
    let window = game_window::spawn_game_window_styled(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_STT_SILKMALL_ITEM_MALL", "Item Mall"),
        (CONTENT_W, CONTENT_H),
        (WINDOW_RIGHT, WINDOW_TOP),
        s,
        GameWindowStyle {
            bg_tile: BG_TILE,
            ..GameWindowStyle::default()
        },
    );
    commands
        .entity(window.root)
        .insert((MallShopRoot, GlobalZIndex(56)));
    commands
        .entity(window.expect_close_button())
        .observe(|_: On<Activate>, mut state: ResMut<MallShopState>| state.open = false);

    let size = text_px(font_index_px(FONT_INDEX), s);
    let font = fonts.two.clone();
    let text = move |rect, justify, value: String| {
        (
            Text::new(value),
            TextFont {
                font: font.clone().into(),
                font_size: FontSize::Px(size),
                ..default()
            },
            LineHeight::Px(size),
            TextColor(Color::WHITE),
            TextLayout::justify(justify),
            abs_node(rect, s),
            Pickable::IGNORE,
        )
    };
    let image = |rect, path: String| {
        (
            abs_node(rect, s),
            ImageNode {
                image: asset_server.load(path),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        )
    };

    let selected = tabs.get(state.tab);
    let goods = selected
        .map(|tab| goods_of(&shops, &tab.code))
        .unwrap_or_default();
    let caption = selected
        .map(|tab| ui_strings.get_or(&tab.text_key, &tab.code).to_string())
        .unwrap_or_default();

    commands.entity(window.content).with_children(|content| {
        content.spawn(image(shop(LIST_BG_RECT), BG_TILE.to_string()));
        content.spawn(text(shop(CAPTION_RECT), Justify::Left, caption));

        for (index, tab) in tabs.iter().enumerate() {
            let y = index as f32 * TAB_H;
            content
                .spawn((
                    MallShopTab(index),
                    Button,
                    abs_node((TAB_X, y, TAB_W, TAB_H), s),
                    ImageNode {
                        image: asset_server.load(art_path(TAB_ART)),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                ))
                .observe(on_tab_activate);
            content.spawn(image(
                (
                    TAB_X + TAB_ICON_INSET,
                    y + TAB_ICON_INSET,
                    TAB_ICON,
                    TAB_ICON,
                ),
                art_path(&tab.icon),
            ));
            content.spawn(text(
                (
                    TAB_X + TAB_LABEL_X,
                    y + TAB_LABEL_Y,
                    TAB_W - TAB_LABEL_X - TAB_ICON_INSET,
                    12.0,
                ),
                Justify::Left,
                ui_strings.get_or(&tab.text_key, &tab.code).to_string(),
            ));
        }

        for slot in 0..SLOTS_PER_PAGE {
            let top = SLOT_FIRST_Y + slot as f32 * SLOT_PITCH;
            let at =
                |rect: (f32, f32, f32, f32)| shop((SLOT_X + rect.0, top + rect.1, rect.2, rect.3));
            content.spawn(image(
                shop((SLOT_X, top, PLATE.0, PLATE.1)),
                art_path(PLATE_ART),
            ));
            let Some(good) = goods.get(slot) else {
                continue;
            };
            let row = item_index
                .id(&good.item_codename)
                .and_then(|id| item_data.get(&(id as i32)));
            if let Some(icon) = row.and_then(|row| row.icon_path()) {
                content.spawn(image(at(ICON_RECT), icon));
            }
            let name = row
                .and_then(|row| row.name_key())
                .and_then(|key| names.name(key))
                .map(str::to_string)
                .unwrap_or_else(|| good.item_codename.clone());
            content.spawn(text(at(NAME_RECT), Justify::Left, name));
            content.spawn(text(at(PRICE_RECT), Justify::Right, priced(good)));
            content.spawn(image(at(MARK_RECT), art_path(MARK_ART)));
            content.spawn(text(
                at(UNIT_RECT),
                Justify::Left,
                ui_strings
                    .get_or("UIIT_STT_ROLL_OF_CLOTH", "Silk")
                    .to_string(),
            ));
        }
    });
}

/// Scene exit: the page is rebuilt from the state, so the state has to go
/// back with the window — otherwise the next entry finds `open` true and no
/// window, and `is_changed` has nothing to react to.
pub fn cleanup_mall_shop_window(
    mut commands: Commands,
    roots: Query<Entity, With<MallShopRoot>>,
    mut state: ResMut<MallShopState>,
) {
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    *state = MallShopState::default();
}

fn on_tab_activate(
    activate: On<Activate>,
    tabs: Query<&MallShopTab>,
    mut state: ResMut<MallShopState>,
) {
    if let Ok(tab) = tabs.get(activate.entity) {
        if state.tab != tab.0 {
            state.tab = tab.0;
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::assets::textdata::shops::ShopTable;

    /// The join this page rests on: the menu table's `code` column is the
    /// store codename the ref-shop chain is keyed by. Rows are literal — the
    /// `GROUP_MALL` row of `refshopgroup.txt` (id 1051) with its `xxx` where an
    /// NPC codename would be, and the astral stone at 2 silk.
    #[test]
    fn the_menu_code_is_the_store_the_chain_is_keyed_by() {
        let table = ShopTable::assemble(
            "1\t15\t1051\tGROUP_MALL\txxx\t-1\txxx\n",
            "1\t15\tGROUP_MALL\tMALL_ARCHEMY\n",
            "1\t15\tMALL_ARCHEMY\tMALL_ARCHEMY_GROUP1\n",
            "1\t15\t2430\tMALL_ARCHEMY_ASTRAL\tMALL_ARCHEMY_GROUP1\tUIIT_STT_SILKMALL_ARCHEMY_ASTRAL\n",
            "1\t15\tMALL_ARCHEMY_ASTRAL\tPACKAGE_ITEM_ETC_ARCHEMY_MAGICSTONE_ASTRAL_01\t0\n",
            "1\t15\tPACKAGE_ITEM_ETC_ARCHEMY_MAGICSTONE_ASTRAL_01\tITEM_ETC_ARCHEMY_MAGICSTONE_ASTRAL\t0\t0\t1\n",
            "1\t15\tPACKAGE_ITEM_ETC_ARCHEMY_MAGICSTONE_ASTRAL_01\t2\t0\t2\n",
        );
        let shops = ClientShops::from_table(table);
        let goods = goods_of(&shops, "MALL_ARCHEMY");
        assert_eq!(goods.len(), 1);
        assert_eq!(goods[0].item_codename, "ITEM_ETC_ARCHEMY_MAGICSTONE_ASTRAL");
        assert_eq!(priced(goods[0]), "2");
        // a storefront the chain does not carry shows nothing rather than a
        // row of placeholders
        assert!(goods_of(&shops, "MALL_CONSUME").is_empty());
    }

    /// Six authored slots at pitch 73 fit the list plate they sit on
    /// (`GDR_ITEM_MALL_SHOP_MAIN_BG` `26,81,364,440`), and a seventh would not.
    #[test]
    fn the_six_authored_slots_fit_the_list_plate() {
        let bottom = SLOT_FIRST_Y + (SLOTS_PER_PAGE - 1) as f32 * SLOT_PITCH + PLATE.1;
        let plate_bottom = LIST_BG_RECT.1 + LIST_BG_RECT.3;
        assert!(bottom <= plate_bottom);
        let seventh = SLOT_FIRST_Y + SLOTS_PER_PAGE as f32 * SLOT_PITCH + PLATE.1;
        assert!(seventh > plate_bottom);
        // the plate is as wide as the authored slot art, inside the pane
        assert!(SLOT_X + PLATE.0 <= CONTENT_W - PANE_X);
    }
}

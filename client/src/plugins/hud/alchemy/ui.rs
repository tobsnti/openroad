//! Alchemy box window — the classic shell and its two pages, Equip Enhance and
//! Attribute Grant.
//!
//! Idea: the vanilla alchemy box is a 376-wide window whose shell
//! (`ginterface.txt:842-863`, `GDR_ALCHEMYBOX` id 44, `Rect="595,262,376,152"`)
//! hosts its pages at `y=150`. The shell declares exactly two of them, both at
//! the same rect `0,150,376,192`: `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM`
//! (`ifalchemybox.txt`, id 23, art `alcm_window_allowance.ddj`) and
//! `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT` (id 22, art
//! `alcm_window_reinforcement.ddj`). Composed extent is therefore `376x342`;
//! our content space is that minus the vanilla 42px title strip
//! (`GDR_ALCHEMYBOX_DRAG` `10,0,355,42`), which the shared `game_window`
//! chrome's caption band replaces — leaving the page art at its native 376x192
//! with no rescale (both DDJs measure 376x192 in their DDS header).
//!
//! **The two page bodies are the same layout, and that is a data fact.**
//! `ifalchemyenchant.txt` and `ifalchemyreinforce.txt` are 149 lines each and
//! are identical once the `GDR_AB_ENCHANT_` / `GDR_AB_REINFORCE_` name prefix
//! is substituted: same seven controls, same ids 30/38..42/50, same rects, same
//! button text. So the layout constants below are shared by both pages.
//!
//! Slots hold *references* to inventory slots (model.rs); nothing is sent to
//! the server, because the fuse opcode map in `docs/re/systems/alchemy.md` is
//! `[S]`-inferred rather than captured. The Fuse button is therefore drawn in
//! its vanilla disabled state.
//!
//! **The page selector is ours in position and the data's in substance:** no
//! resinfo tree declares a tab or a button for the pages — `ifalchemybox.txt`
//! carries six controls and none of them selects a page (positive control on
//! the same read: that file *does* declare `GDR_ALCHEMYBOX_CLOSE` and
//! `GDR_ALCHEMYBOX_DRAG`). What the archive ships instead is one 20x24 gem lamp
//! per page in an on/off pair (`interface/alchemy/alcm_lamp_enchant_{on,off}.ddj`,
//! `alcm_lamp_reinforcement_{on,off}.ddj`) plus one caption string per page host
//! (`textuisystem.txt:771` `UIIT_STT_ALCHEMYBOX_REINFORCE_ITEM` = *"Equip
//! Enhance"*, `:773` `..._ENCHANT_MAGIC_PARAM` = *"Att.Grant"*). So the lamps
//! and the words are the original's; only the row's rect is our choice, taken
//! in the shell's one empty band — below the caption band, above the
//! description block at `39,89,300,48`.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::assets::FontAssets;
use crate::plugins::hud::alchemy::model::{AlchemyPage, AlchemyState, EQUIP_SLOT, STONE_SLOTS};
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::DragGhost;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

/// `GDR_ALCHEMYBOX_DRAG` (`ifalchemybox.txt`) is `10,0,355,42`: the top 42
/// units of the vanilla window are its title strip.
const TITLE_STRIP: f32 = 42.0;
/// Shell `Rect="595,262,376,152"` + the page host at `0,150,376,192`.
const WINDOW_W: f32 = 376.0;
const COMPOSED_H: f32 = 150.0 + PAGE_RECT.3;
/// Content space = the composed window minus the vanilla title strip.
const CONTENT_W: f32 = WINDOW_W;
const CONTENT_H: f32 = COMPOSED_H - TITLE_STRIP;

/// Vanilla control rects rebased into content space (`y - TITLE_STRIP`; the
/// page host sits at `x=0`, so page-local `x` needs no shift).
/// `GDR_ALCHEMYBOX_PML_TEXT` `39,89,300,48`.
const PML_RECT: (f32, f32, f32, f32) = (39.0, 89.0 - TITLE_STRIP, 300.0, 48.0);
/// The page host both pages share: `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM`
/// `0,150,376,192` and `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT` `0,150,376,192`
/// (`ifalchemybox.txt`).
const PAGE_RECT: (f32, f32, f32, f32) = (0.0, 150.0 - TITLE_STRIP, 376.0, 192.0);
/// `GDR_AB_ENCHANT_SLOT_EQUIP` / `GDR_AB_REINFORCE_SLOT_EQUIP` `59,56,32,32`,
/// page-local (the two page files are identical, module doc).
const EQUIP_RECT: (f32, f32, f32, f32) = (59.0, PAGE_RECT.1 + 56.0, SLOT, SLOT);
/// `_SLOT_01..04` `164/212/260/308,56,32,32`, page-local, in both page files.
const STONE_XS: [f32; STONE_SLOTS] = [164.0, 212.0, 260.0, 308.0];
const STONE_Y: f32 = PAGE_RECT.1 + 56.0;
/// `_BUTTON_PROCESS` `132,143,112,28`, page-local, in both page files — the
/// art (`alcm_button.ddj`) is exactly 112x28.
const BUTTON_RECT: (f32, f32, f32, f32) = (132.0, PAGE_RECT.1 + 143.0, 112.0, 28.0);
const SLOT: f32 = 32.0;

/// Page-selector row — **our rect, the data's art and words** (module doc).
/// The lamp extent is the DDJ's own: all six `alcm_lamp_*_{on,off}.ddj` measure
/// 20x24 in their DDS header. `x` is the description block's `39` so the two
/// align; `y` sits in the band the shell leaves between the caption strip
/// (ends at the vanilla `y=42`, i.e. content `0`) and that block (vanilla
/// `y=89`, content `47`).
const LAMP: (f32, f32) = (20.0, 24.0);
const SELECTOR_ORIGIN: (f32, f32) = (39.0, 10.0);
/// The entries share the band equally, so the pitch is the width divided by
/// their own count rather than a hand-picked number: the row is inset by its
/// origin on both sides.
const SELECTOR_PITCH: f32 = (CONTENT_W - 2.0 * SELECTOR_ORIGIN.0) / AlchemyPage::ALL.len() as f32;
/// Gap between a lamp and its caption, and the caption box.
const SELECTOR_LABEL: (f32, f32, f32) = (4.0, SELECTOR_PITCH - LAMP.0 - 4.0 - 6.0, 12.0);
/// 32x32 in its DDS header — the exact slot extent.
const SLOT_DDJ: &str = "media://interface/alchemy/alcm_slot_closed.ddj";
const BUTTON_DISABLED_DDJ: &str = "media://interface/alchemy/alcm_button_disable.ddj";

/// The page's own 376x192 backdrop, named by its host control's `DDJ` field in
/// `ifalchemybox.txt`.
fn page_art(page: AlchemyPage) -> &'static str {
    match page {
        AlchemyPage::EquipEnhance => "media://interface/alchemy/alcm_window_reinforcement.ddj",
        AlchemyPage::AttGrant => "media://interface/alchemy/alcm_window_allowance.ddj",
    }
}

/// The selector lamp for a page. The `_on`/`_off` pair is the archive's own
/// active/inactive art, and the stem names the page it belongs to.
fn page_lamp(page: AlchemyPage, active: bool) -> String {
    let stem = match page {
        AlchemyPage::EquipEnhance => "alcm_lamp_reinforcement",
        AlchemyPage::AttGrant => "alcm_lamp_enchant",
    };
    let state = if active { "on" } else { "off" };
    format!("media://interface/alchemy/{stem}_{state}.ddj")
}

/// The caption string the archive ships for the page's host control, keyed by
/// that control's own name (`textuisystem.txt:771` / `:773`).
fn page_caption(page: AlchemyPage) -> (&'static str, &'static str) {
    match page {
        AlchemyPage::EquipEnhance => ("UIIT_STT_ALCHEMYBOX_REINFORCE_ITEM", "Equip Enhance"),
        AlchemyPage::AttGrant => ("UIIT_STT_ALCHEMYBOX_ENCHANT_MAGIC_PARAM", "Att.Grant"),
    }
}

/// The description the shell's PML control carries for the page. Both keys are
/// shipped and named after their page (`textuisystem.txt:779` / `:781`); which
/// one the EXE feeds the single `GDR_ALCHEMYBOX_PML_TEXT` control is `[S]` from
/// those names. The fallbacks are the shipped English text with its markup
/// dropped, because our PML renderer draws plain text.
fn page_description(page: AlchemyPage) -> (&'static str, &'static str) {
    match page {
        AlchemyPage::EquipEnhance => (
            "UIIT_STT_ALCHEMYBOX_REINFORCE_TEXT",
            "Equipment Enhance: when equipment and elixir are combined, the equipment is \
             usually strengthened with + options. Warning: all used items will disappear.",
        ),
        AlchemyPage::AttGrant => (
            "UIIT_STT_ALCHEMYBOX_REINFORCE_ATTR_TEXT",
            "Att.Grant: using specific alchemy items will grant attributes or change basic \
             stats of your equipment.",
        ),
    }
}

/// Content-space rect — `(x, y, w, h)`, the shape [`abs_node`] takes.
type LayoutRect = (f32, f32, f32, f32);

/// Selector entry rect for the `nth` page, and its caption rect beside it.
fn selector_rects(nth: usize) -> (LayoutRect, LayoutRect) {
    let x = SELECTOR_ORIGIN.0 + SELECTOR_PITCH * nth as f32;
    let (gap, label_w, label_h) = SELECTOR_LABEL;
    (
        (x, SELECTOR_ORIGIN.1, LAMP.0, LAMP.1),
        (
            x + LAMP.0 + gap,
            SELECTOR_ORIGIN.1 + (LAMP.1 - label_h) / 2.0,
            label_w,
            label_h,
        ),
    )
}

/// `GDR_AB_ENCHANT_BUTTON_PROCESS` `FontColor="255,255,245,218"` (resinfo
/// COLOR is A,R,G,B), dimmed to 55% because the button is disabled.
const BUTTON_TEXT_COLOR: Color = Color::srgb_u8(140, 135, 120);
/// The selected page's caption is drawn in the window title's own colour
/// (`GDR_ALCHEMYBOX_TITLE` `FontColor="255,239,218,164"`, `ifalchemybox.txt`);
/// the unselected one reuses the dimmed [`BUTTON_TEXT_COLOR`], so "dim = not
/// active" reads the same way twice.
const CAPTION_ACTIVE_COLOR: Color = Color::srgb_u8(239, 218, 164);
/// `GDR_ALCHEMYBOX_PML_TEXT` has no `FontColor` worth reading (`255,0,0,0`,
/// i.e. black, is the resinfo default for a PML control whose runs carry their
/// own colours); the body text is drawn in the HUD's off-white.
const PML_TEXT_COLOR: Color = Color::srgb_u8(230, 226, 214);

/// Default position: the registry's `595,262` on the vanilla 1024x768 screen,
/// expressed as our right/top anchor — `1024 - (595 + 376) = 53`.
const WINDOW_RIGHT: f32 = 53.0;
const WINDOW_TOP: f32 = 262.0;

#[derive(Component)]
pub struct AlchemyWindowRoot;

/// Deferred despawn marker (the storage/store precedent: a rebuild must not
/// despawn an entity the same frame its observers may still run).
#[derive(Component)]
pub struct AlchemyClosing;

/// A page slot cell, indexed like the vanilla `CommandID` (0 = equipment).
#[derive(Component)]
pub struct AlchemySlotCell {
    pub index: usize,
}

/// A page-selector lamp.
#[derive(Component)]
pub struct AlchemyPageTab {
    pub page: AlchemyPage,
}

/// Rebuild the alchemy window whenever its state changes.
#[allow(clippy::too_many_arguments)]
pub fn sync_alchemy_window(
    state: Res<AlchemyState>,
    existing: Query<(Entity, &Node), With<AlchemyWindowRoot>>,
    inventories: Query<&Inventory, With<Player>>,
    item_data: Res<ClientItemData>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cam_query: Query<Entity, With<Camera2d>>,
    mut commands: Commands,
) {
    if !state.is_changed() {
        return;
    }
    // a placement rebuilds the window — keep a dragged position
    let mut anchor = (WINDOW_RIGHT, WINDOW_TOP);
    for (entity, node) in existing.iter() {
        if let (Val::Px(right), Val::Px(top)) = (node.right, node.top) {
            anchor = (right, top);
        }
        commands.entity(entity).insert(AlchemyClosing);
    }
    if !state.open {
        return;
    }
    let Ok(camera) = cam_query.single() else {
        warn!("alchemy: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_CTL_ALCHEMYBOX", "Alchemy"),
        (CONTENT_W, CONTENT_H),
        anchor,
        s,
    );
    commands.entity(window.root).insert((
        AlchemyWindowRoot,
        GlobalZIndex(57),
        // Hovered so the drop detector can tell "released on the alchemy
        // window" from "released on nothing" — this window's own catcher
        // hovers its *cells*, so without this the gaps between them would
        // read as empty screen and offer to destroy the item.
        Hovered::default(),
    ));
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let inventory = inventories.single().ok();
    let icon_of = |page_slot: usize| -> Option<String> {
        let wire = state.slot(page_slot)?;
        let item = inventory?.get(wire)?;
        item_data
            .get(&(item.ref_id as i32))
            .and_then(|row| row.icon_path())
    };

    commands.entity(window.content).with_children(|content| {
        // the page's own 376x192 backdrop, at its native extent
        content.spawn((
            abs_node(PAGE_RECT, s),
            ImageNode {
                image: asset_server.load(page_art(state.page)),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        ));

        // the page selector: one gem lamp plus the page's shipped caption
        for (nth, page) in AlchemyPage::ALL.into_iter().enumerate() {
            let (lamp_rect, label_rect) = selector_rects(nth);
            let active = page == state.page;
            let mut lamp = content.spawn((
                AlchemyPageTab { page },
                Hovered::default(),
                abs_node(lamp_rect, s),
                ImageNode {
                    image: asset_server.load(page_lamp(page, active)),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
            ));
            lamp.observe(on_page_tab_press);
            let (key, fallback) = page_caption(page);
            content.spawn((
                Text::new(ui_strings.get_or(key, fallback).to_string()),
                TextFont {
                    font: fonts.two.clone().into(),
                    font_size: FontSize::Px(9.0 * s),
                    ..default()
                },
                TextColor(if active {
                    CAPTION_ACTIVE_COLOR
                } else {
                    BUTTON_TEXT_COLOR
                }),
                TextLayout::justify(Justify::Left),
                abs_node(label_rect, s),
                Pickable::IGNORE,
            ));
        }

        // the page description the vanilla PML control carries — one shipped
        // key per page, [S] by their names (see `page_description`).
        content.spawn((
            Text::new({
                let (key, fallback) = page_description(state.page);
                ui_strings.get_plain_or(key, fallback)
            }),
            TextFont {
                font: fonts.two.clone().into(),
                font_size: FontSize::Px(8.0 * s),
                ..default()
            },
            TextColor(PML_TEXT_COLOR),
            TextLayout::justify(Justify::Left),
            abs_node(PML_RECT, s),
            Pickable::IGNORE,
        ));

        // the five item slots (equipment + four stones)
        for index in 0..=STONE_SLOTS {
            let rect = slot_rect(index);
            let mut cell = content.spawn((
                AlchemySlotCell { index },
                Hovered::default(),
                abs_node(rect, s),
                ImageNode {
                    image: asset_server.load(SLOT_DDJ),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
            ));
            cell.observe(on_alchemy_slot_press);
            let Some(icon) = icon_of(index) else {
                continue;
            };
            cell.with_children(|slot| {
                slot.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(SLOT * s),
                        height: Val::Px(SLOT * s),
                        ..default()
                    },
                    ImageNode {
                        image: asset_server.load(icon),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            });
        }

        // Fuse — drawn in the vanilla disabled state: the action needs the
        // 0x7150 request, and that opcode map is inferred, not captured
        // (docs/re/systems/alchemy.md), so this client does not send it.
        content
            .spawn((
                abs_node(BUTTON_RECT, s),
                ImageNode {
                    image: asset_server.load(BUTTON_DISABLED_DDJ),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new(
                        ui_strings
                            .get_or("UIIT_STT_ALCHEMYBOX_COMPOUND", "Fuse")
                            .to_string(),
                    ),
                    TextFont {
                        font: fonts.two.clone().into(),
                        font_size: FontSize::Px(8.5 * s),
                        ..default()
                    },
                    TextColor(BUTTON_TEXT_COLOR),
                    TextLayout::justify(Justify::Center),
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Px(9.0 * s),
                        width: Val::Percent(100.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            });
    });
}

/// Page-slot rect in content space: index 0 is the equipment slot, 1..=4 the
/// stones.
fn slot_rect(index: usize) -> (f32, f32, f32, f32) {
    if index == EQUIP_SLOT {
        EQUIP_RECT
    } else {
        (STONE_XS[index - 1], STONE_Y, SLOT, SLOT)
    }
}

/// A lamp press raises its page. Nothing is sent: which page a client shows is
/// not something the server is told about — the whole placement path is
/// client-only for the same reason (model.rs).
fn on_page_tab_press(
    press: On<Pointer<Press>>,
    tabs: Query<&AlchemyPageTab>,
    mut state: ResMut<AlchemyState>,
) {
    if press.event.button != PointerButton::Primary {
        return;
    }
    let Ok(tab) = tabs.get(press.entity) else {
        return;
    };
    if state.page != tab.page {
        state.page = tab.page;
    }
}

fn on_close_button(_: On<Activate>, mut state: ResMut<AlchemyState>) {
    state.open = false;
    state.clear();
}

/// Press on a filled slot takes the item back out (the placement is only a
/// reference, so nothing is sent). A press while carrying is left to
/// [`place_drop_on_alchemy`] so a drop is not double-handled.
fn on_alchemy_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&AlchemySlotCell>,
    inv_state: Res<InventoryState>,
    mut state: ResMut<AlchemyState>,
) {
    if press.event.button != PointerButton::Primary || inv_state.drag.is_some() {
        return;
    }
    let Ok(cell) = cells.get(press.entity) else {
        return;
    };
    state.take(cell.index);
}

/// Dropping a carried inventory item on a page slot places it there. The
/// inventory carry and its ghost are consumed here, so no 0x7034 move goes out
/// for this drop (the storage-deposit precedent).
pub fn place_drop_on_alchemy(
    buttons: Res<ButtonInput<MouseButton>>,
    cells: Query<(&AlchemySlotCell, &Hovered)>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
    mut state: ResMut<AlchemyState>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) && !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(source) = inv_state.drag else {
        return;
    };
    let Some(index) = cells
        .iter()
        .find(|(_, hovered)| hovered.get())
        .map(|(cell, _)| cell.index)
    else {
        return;
    };
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
    state.place(index, source);
}

pub fn despawn_closing_alchemy(
    closing: Query<Entity, With<AlchemyClosing>>,
    mut commands: Commands,
) {
    for entity in closing.iter() {
        commands.entity(entity).despawn();
    }
}

pub fn cleanup_alchemy(
    windows: Query<Entity, With<AlchemyWindowRoot>>,
    mut state: ResMut<AlchemyState>,
    mut commands: Commands,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
    state.open = false;
    state.clear();
}

#[cfg(test)]
mod test {
    use super::*;

    /// The shared shell's content origin — the layout constants are rebased on
    /// it (the #310 lesson: derive it from `game_window`, never hand-tune).
    const ORIGIN_Y: f32 = game_window::CONTENT_TOP;

    /// The composed classic window is the shell's 152-tall registry rect with
    /// the page host at `y=150`: `150 + 192 = 342`. Our content is that minus
    /// the 42-unit title strip the chrome's caption band replaces.
    #[test]
    fn alchemy_content_is_the_vanilla_window_minus_its_title_strip() {
        assert_eq!((CONTENT_W, CONTENT_H), (376.0, 300.0));
        assert_eq!(COMPOSED_H - TITLE_STRIP, CONTENT_H);
        // the page fills the content space exactly, bottom-aligned
        assert_eq!(PAGE_RECT.1 + PAGE_RECT.3, CONTENT_H);
    }

    /// Vanilla rects, rebased. Window-space controls lose the title strip;
    /// page-local controls gain the page host's `y=150` and then lose it too.
    #[test]
    fn alchemy_rects_are_the_vanilla_rects_minus_the_title_strip() {
        // (vanilla y in window space, ours) — ifalchemybox.txt
        // GDR_ALCHEMYBOX_PML_TEXT (39,89), GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM
        // (0,150); ifalchemyenchant.txt page-local GDR_AB_ENCHANT_SLOT_EQUIP
        // (59,56), _SLOT_01 (164,56), _SLOT_04 (308,56),
        // _BUTTON_PROCESS (132,143).
        let cases = [
            (89.0, PML_RECT.1),
            (150.0, PAGE_RECT.1),
            (150.0 + 56.0, slot_rect(EQUIP_SLOT).1),
            (150.0 + 56.0, slot_rect(1).1),
            (150.0 + 143.0, BUTTON_RECT.1),
        ];
        for (vanilla_y, ours) in cases {
            assert_eq!(ours, vanilla_y - TITLE_STRIP, "y of vanilla {vanilla_y}");
        }
        assert_eq!(slot_rect(EQUIP_SLOT).0, 59.0);
        assert_eq!(slot_rect(1).0, 164.0);
        assert_eq!(slot_rect(STONE_SLOTS).0, 308.0);
        // every slot is the vanilla 32x32
        for index in 0..=STONE_SLOTS {
            assert_eq!((slot_rect(index).2, slot_rect(index).3), (SLOT, SLOT));
        }
    }

    /// The selector row sits in the band the shell leaves free above the
    /// description block, one entry per declared page, and the entries divide
    /// that band instead of landing on hand-picked x values.
    #[test]
    fn the_page_selector_has_one_entry_per_page_above_the_description() {
        assert_eq!(
            AlchemyPage::ALL.len(),
            2,
            "ifalchemybox.txt hosts two pages"
        );
        let mut previous_right = 0.0_f32;
        for (nth, _) in AlchemyPage::ALL.into_iter().enumerate() {
            let (lamp, label) = selector_rects(nth);
            // the lamp is the DDJ's own 20x24, never rescaled
            assert_eq!((lamp.2, lamp.3), LAMP);
            // the row clears the caption band and ends above the PML block
            assert!(lamp.1 >= 0.0, "entry {nth} starts below the caption band");
            assert!(
                lamp.1 + lamp.3 <= PML_RECT.1,
                "entry {nth} overlaps the description block"
            );
            // left-aligned with that block, and inset by the same amount on the right
            assert!(lamp.0 >= SELECTOR_ORIGIN.0);
            assert!(label.0 + label.2 <= CONTENT_W - SELECTOR_ORIGIN.0);
            // no entry overlaps the one before it
            assert!(
                lamp.0 >= previous_right,
                "entry {nth} overlaps its left neighbour"
            );
            previous_right = label.0 + label.2;
        }
        assert_eq!(
            selector_rects(0).0 .0,
            PML_RECT.0,
            "aligned with the PML block"
        );
    }

    /// Each page names its own backdrop, its own lamp pair and its own two
    /// shipped strings — all four differ, none is shared by accident.
    #[test]
    fn each_page_names_its_own_art_and_strings() {
        let mut arts = Vec::new();
        let mut keys = Vec::new();
        for page in AlchemyPage::ALL {
            arts.push(page_art(page).to_string());
            arts.push(page_lamp(page, true));
            arts.push(page_lamp(page, false));
            keys.push(page_caption(page).0);
            keys.push(page_description(page).0);
            // on and off are a pair of the same stem
            assert_eq!(
                page_lamp(page, true).replace("_on.ddj", ""),
                page_lamp(page, false).replace("_off.ddj", "")
            );
        }
        let unique: std::collections::BTreeSet<_> = arts.iter().collect();
        assert_eq!(unique.len(), arts.len(), "two pages share art: {arts:?}");
        let unique: std::collections::BTreeSet<_> = keys.iter().collect();
        assert_eq!(
            unique.len(),
            keys.len(),
            "two pages share a string: {keys:?}"
        );
        // the host control's own DDJ field, per page
        assert!(page_art(AlchemyPage::EquipEnhance).ends_with("alcm_window_reinforcement.ddj"));
        assert!(page_art(AlchemyPage::AttGrant).ends_with("alcm_window_allowance.ddj"));
    }

    /// The shared chrome adds its own ring around the vanilla interior, so
    /// the outer window is wider/taller than the original's 376x342 — the
    /// interior itself stays pixel-exact. Pinned so a chrome change surfaces
    /// here instead of silently rescaling the page art.
    #[test]
    fn alchemy_chrome_wraps_the_vanilla_interior() {
        assert_eq!(
            game_window::outer_size((CONTENT_W, CONTENT_H)),
            (400.0, 352.0)
        );
        assert_eq!(ORIGIN_Y, 36.0);
    }
}

//! Alchemy box window — the shell the client loads, and its three pages.
//!
//! **Which description this is built from, and how that was decided.** The
//! client loads `resinfo/ifnewalchemybox.txt`; it does **not** load
//! `ifalchemybox.txt`, which an earlier version of this file used. The test
//! below pins the numbers that differ, so the two cannot be confused again.
//!
//! The shell is 376 wide and hosts its pages at `y=150`. It declares **three**
//! of them, each with its own art and height:
//! `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT` (id 22, `0,150,376,228`,
//! `alcm_window_quick mastery.ddj`), `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM`
//! (id 23, `0,150,376,192`, `alcm_window_allowance.ddj`) and
//! `GDR_ALCHEMYBOX_ELEMENT_MANUFACTURING` (id 21, `0,150,376,228`,
//! `alcm_window_experiment.ddj`). Every one of the three rect heights equals its
//! art's own DDS height. Composed extent is `150 + 228 = 378`; our content space
//! is that minus the 42-unit title strip (`GDR_ALCHEMYBOX_DRAG` `10,0,355,42`),
//! which the shared `game_window` chrome's caption band replaces.
//!
//! **The window plate carries a parchment tablet, and that is why the shipped
//! font colour is black.** The shell's image set is `interface/frame/mframe_alc_`,
//! whose eight parts are seven 4x4 stubs and one real image: `mframe_alc_right_up.ddj`
//! at 376x376, i.e. the whole plate in one picture. Decoding it shows a bright
//! tablet at rows **93..139** — where the shell puts `GDR_ALCHEMYBOX_PML_TEXT`
//! (`39,97,300,48`, `FontColor="255,0,0,0"`). The description is dark text on
//! parchment, not light text on a fill tile. Measured on the same decode: rows
//! **296..375 are fully transparent** (all 1880 blocks in that band are the
//! DXT1 alpha form with both colours zero), so the drawn plate is 376x296 and
//! the page art covers its lower half.
//!
//! Slots hold *references* to inventory slots (model.rs); nothing is sent to
//! the server, because the fuse opcode map in `docs/re/systems/alchemy.md` is
//! `[S]`-inferred rather than captured. The Fuse button is therefore drawn in
//! its vanilla disabled state.
//!
//! **The page selector is ours in position and the data's in substance:** the
//! shell carries seven controls — three page hosts, the text block, the close
//! button, the drag area and the title — and **none of them selects a page**
//! (positive control on the same read: the same file *does* declare
//! `GDR_ALCHEMYBOX_CLOSE` and `GDR_ALCHEMYBOX_DRAG`). What the archive ships
//! instead is one 20x24 gem lamp per page in an on/off pair — `alcm_lamp_enchant`,
//! `alcm_lamp_reinforcement` and `alcm_lamp_element`, which are exactly these
//! three pages — plus one caption string per page host (`textuisystem.txt:771`
//! = *"Equip Enhance"*, `:773` = *"Att.Grant"*, `:772` = *"Elementation"*). So the
//! lamps and the words are the original's; only the row's rect is our choice,
//! taken in the shell's one empty band — below the caption band, above the
//! description block at `39,97,300,48`.
//!
//! **One measured difference left open on purpose:** the parchment tablet in the
//! plate spans rows 93..139, while the text block is declared at 97..145 — four
//! units apart. The offset is not rounded away here, because an unexplained
//! difference that is written down is a finding, and one that is smoothed over is
//! a later bug.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use packets::agent::alchemy::{
    AlchemyReinforceRequest, AlchemyStoneRequest, ALCHEMY_TYPE_ATTRIBUTE_STONE,
    ALCHEMY_TYPE_MAGIC_STONE,
};
use packets::agent::character_data::ItemTypeData;
use packets::Packet;

use crate::assets::textdata::itemdata::ItemDataRow;
use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::alchemy::model::{
    AlchemyPage, AlchemyState, EQUIP_SLOT, STONE_SLOTS, TALLEST_HOST_H,
};
use crate::plugins::hud::alchemy::probability::{
    is_elixir, is_lucky_powder, is_magic_stone, reinforce_chance, ReinforceChance,
};
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::game_window::{self, abs_node};
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::DragGhost;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

/// `GDR_ALCHEMYBOX_DRAG` is `10,0,355,42`: the top 42 units of the window are
/// its title strip.
const TITLE_STRIP: f32 = 42.0;
/// Shell width, and the page host's own `y`.
const WINDOW_W: f32 = 376.0;
const PAGE_TOP: f32 = 150.0;
/// The window is composed around the **tallest** page, so switching pages does
/// not resize it. 228 is the reinforce and element host height, which is also
/// those arts' own DDS height.
const COMPOSED_H: f32 = PAGE_TOP + TALLEST_HOST_H;
/// Content space = the composed window minus the title strip.
const CONTENT_W: f32 = WINDOW_W;
const CONTENT_H: f32 = COMPOSED_H - TITLE_STRIP;

/// Control rects rebased into content space (`y - TITLE_STRIP`; the page host
/// sits at `x=0`, so page-local `x` needs no shift).
/// `GDR_ALCHEMYBOX_PML_TEXT` `39,97,300,48`.
const PML_RECT: (f32, f32, f32, f32) = (39.0, 97.0 - TITLE_STRIP, 300.0, 48.0);
/// Origin of the page host all three pages share. Its **height** is the page's
/// own ([`AlchemyPage::host_height`]), so this carries only the origin and the
/// tallest extent.
const PAGE_RECT: (f32, f32, f32, f32) = (0.0, PAGE_TOP - TITLE_STRIP, 376.0, 228.0);

/// The window plate: `interface/frame/mframe_alc_` has seven 4x4 stubs and one
/// real image, `mframe_alc_right_up.ddj` at 376x376 — the whole plate in one
/// picture. Its drawn content is only **376x296**; rows 296..375 are fully
/// transparent, so the plate is placed by that height and not by the file's.
const PLATE_DDJ: &str = "media://interface/frame/mframe_alc_right_up.ddj";
const PLATE_DRAWN_H: f32 = 296.0;
/// The plate includes the title strip, which the chrome's caption band replaces,
/// so the strip is cropped off and the rest is placed at the content origin.
fn plate_crop() -> Rect {
    Rect::new(0.0, TITLE_STRIP, WINDOW_W, PLATE_DRAWN_H)
}
const PLATE_RECT: (f32, f32, f32, f32) = (0.0, 0.0, WINDOW_W, PLATE_DRAWN_H - TITLE_STRIP);
/// `GDR_AB_REINFORCE_SLOT_EQUIP` `58,74,32,32`, page-local
/// (`ifnewalchemyreinforce.txt`, the body the loaded shell's reinforce host uses).
const EQUIP_RECT: (f32, f32, f32, f32) = (58.0, PAGE_RECT.1 + 74.0, SLOT, SLOT);
/// `_SLOT_01..04` `164/212/260/308,74,32,32`, page-local, pitch 48.
const STONE_XS: [f32; STONE_SLOTS] = [164.0, 212.0, 260.0, 308.0];
const STONE_Y: f32 = PAGE_RECT.1 + 74.0;
/// `_BUTTON_PROCESS` `136,178,0,0` page-local, on `alcm_button_01.ddj`. The rect's
/// `0,0` extent means "take the size from the art", and that art measures
/// **104x28** — eight units narrower than the one this file used before.
const BUTTON_RECT: (f32, f32, f32, f32) = (136.0, PAGE_RECT.1 + 178.0, 104.0, 28.0);
const SLOT: f32 = 32.0;
/// **Ours, not vanilla's** — the success-chance line (see `probability.rs`), and
/// it belongs to the **Equip Enhance** page, the one whose ladder it reads.
/// Neither page body declares a text control for it: their seven controls are
/// the deco, five slots and the button (positive control on the same read: the
/// file *does* declare a `CIFDecoratedStatic`, so a text-capable control is a
/// thing it could have carried). The rect is therefore a reasoned choice: the
/// page's only empty band, between the slot row (`74 + 32 = 106`) and the Fuse
/// button (`y = 178`), left-aligned with the first stone slot (`x = 164`) so it
/// reads as belonging to the material row it is computed from.
///
/// **It is an addition, not a restoration.** The original shows no success chance
/// anywhere in this window — not with equipment and a matching elixir in the
/// slots, and not after the button is pressed. The number is the data's; showing
/// it is ours.
const CHANCE_RECT: (f32, f32, f32, f32) = (164.0, PAGE_RECT.1 + 120.0, 200.0, 14.0);

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
/// The face of `_BUTTON_PROCESS`, named by that control's own `DDJ` field, and
/// **the only face this button ever wears**. The archive does ship an
/// `alcm_button_01_disable.ddj`, but the original does not use it for an empty
/// page: with every slot empty the caption is drawn in full brightness and the
/// button still answers a press. A greyed-out face for "nothing placed yet" was
/// this file's own invention and is gone.
const BUTTON_DDJ: &str = "media://interface/alchemy/alcm_button_01.ddj";

/// `UIIT_MSG_REINFORCERR_NO_ITEM_LOADED` (`textuisystem.txt:2143`) — the
/// archive's own answer to a fuse press with an incomplete page. It is also the
/// guard that keeps a one-slot request off the wire (see `on_fuse_button`).
const FUSE_INCOMPLETE: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_NO_ITEM_LOADED",
    "Cannot reinforce without the specified item.",
);
/// `UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY` (`:2149`) — the archive's "not right
/// now", used when there is no agent connection to send on.
const FUSE_UNAVAILABLE: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY",
    "You are not under the state to use alchemy.",
);

/// The page's own backdrop, named by its host control's `DDJ` field. Each one's
/// DDS height equals its host rect's height (228 / 192 / 228).
fn page_art(page: AlchemyPage) -> &'static str {
    match page {
        AlchemyPage::EquipEnhance => "media://interface/alchemy/alcm_window_quick mastery.ddj",
        AlchemyPage::AttGrant => "media://interface/alchemy/alcm_window_allowance.ddj",
        AlchemyPage::Elementation => "media://interface/alchemy/alcm_window_experiment.ddj",
    }
}

/// The selector lamp for a page. The `_on`/`_off` pair is the archive's own
/// active/inactive art, and the stem names the page it belongs to. The archive
/// ships four such pairs; three of them are these three pages, which is how they
/// were identified as the page selector.
fn page_lamp(page: AlchemyPage, active: bool) -> String {
    let stem = match page {
        AlchemyPage::EquipEnhance => "alcm_lamp_reinforcement",
        AlchemyPage::AttGrant => "alcm_lamp_enchant",
        AlchemyPage::Elementation => "alcm_lamp_element",
    };
    let state = if active { "on" } else { "off" };
    format!("media://interface/alchemy/{stem}_{state}.ddj")
}

/// The caption string the archive ships for the page's host control, keyed by
/// that control's own name (`textuisystem.txt:771` / `:773` / `:772`). Two of the
/// three keys carry their control's name word for word; the reinforce host is
/// `_EQUIPMENT` while its key is `_ITEM`, which is the one break and stays `[S]`.
fn page_caption(page: AlchemyPage) -> (&'static str, &'static str) {
    match page {
        AlchemyPage::EquipEnhance => ("UIIT_STT_ALCHEMYBOX_REINFORCE_ITEM", "Equip Enhance"),
        AlchemyPage::AttGrant => ("UIIT_STT_ALCHEMYBOX_ENCHANT_MAGIC_PARAM", "Att.Grant"),
        AlchemyPage::Elementation => ("UIIT_STT_ALCHEMYBOX_ELEMENT_MANUFACTURING", "Elementation"),
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
        AlchemyPage::Elementation => (
            "UIIT_STT_ALCHEMYBOX_MATERIAL_PROCESSING_TEXT",
            "Material Processing: this is alchemy that will disjoint or fuse an item.",
        ),
    }
}

/// The chance the itemdata states for the equipment in the equip slot together
/// with whatever elixir (and Lucky Powder) sits in the four stone slots.
fn slot_chance(
    state: &AlchemyState,
    inventory: &Inventory,
    item_data: &ClientItemData,
) -> Option<ReinforceChance> {
    let row_of = |page_slot: usize| {
        let item = inventory.get(state.slot(page_slot)?)?;
        item_data.get(&(item.ref_id as i32))
    };
    let equip_item = inventory.get(state.slot(EQUIP_SLOT)?)?;
    let equip = item_data.get(&(equip_item.ref_id as i32))?;
    // the current enhancement level is the equipment body's own opt_level;
    // anything that is not equipment on the wire cannot be reinforced
    let ItemTypeData::Equipment(equipment) = &equip_item.data else {
        return None;
    };
    let stones: Vec<_> = (1..=STONE_SLOTS).filter_map(row_of).collect();
    page_chance(state.page, equip, equipment.opt_level, &stones)
}

/// The success ladder is the **Equip Enhance** page's own subject — it is the
/// page whose materials are equipment + elixir (+ Lucky Powder) and whose
/// outcome is a `+n` step (`UIIT_STT_ALCHEMYBOX_REINFORCE_TEXT`,
/// `textuisystem.txt:779`). The Att.Grant page fuses attribute stones into magic
/// options instead (`:781`, and `UIIT_MSG_ALCHEMY_APPEND_ATTR` `:786` is its
/// result line), which the elixir ladder says nothing about — so that page shows
/// no chance line at all rather than a number that means nothing there.
///
/// Within the page the slots are not typed in vanilla either — both page bodies
/// give all four stone slots the same `CIFSlotWithHelp` prototype — so the elixir
/// is found by its type ids, not by position, exactly as the shipped
/// slot-mismatch line implies (`UIIT_MSG_REINFORCERR_IS_NOT_REINFORCE_STUFF`,
/// `textuisystem.txt:2141`).
fn page_chance(
    page: AlchemyPage,
    equip: &ItemDataRow,
    opt_level: u8,
    stones: &[&ItemDataRow],
) -> Option<ReinforceChance> {
    if page != AlchemyPage::EquipEnhance {
        return None;
    }
    let elixir = stones.iter().copied().find(|row| is_elixir(row))?;
    let powder = stones.iter().copied().find(|row| is_lucky_powder(row));
    reinforce_chance(equip, opt_level, elixir, powder)
}

/// `Probability: 25 %`, or `Probability: 25 % + 50 %` with a matched Lucky
/// Powder. The label is the shipped `UIIT_STT_PROBABILITY`
/// (`textuisystem.txt:1932` = *"Probability"*); the archive ships **no** key that
/// formats a success percentage, so the composition is ours. The powder's points
/// stay a second figure because whether the server adds or multiplies them is
/// unresolved.
fn chance_line(ui_strings: &ClientUiStrings, chance: ReinforceChance) -> String {
    let label = ui_strings.get_or("UIIT_STT_PROBABILITY", "Probability");
    match chance.powder_bonus {
        Some(bonus) => format!("{label}: {} % + {bonus} %", chance.percent),
        None => format!("{label}: {} %", chance.percent),
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

/// `_BUTTON_PROCESS` `FontColor="255,255,245,218"` (resinfo COLOR is A,R,G,B) —
/// the button's caption, undimmed, because the button is never disabled.
const BUTTON_TEXT_COLOR: Color = Color::srgb_u8(255, 245, 218);
/// An unselected page's caption, dimmed from [`CAPTION_ACTIVE_COLOR`]. This dimming
/// is **ours** — the selector row itself is ours — and it is the only place in this
/// window where a dimmed text colour means anything.
const CAPTION_IDLE_COLOR: Color = Color::srgb_u8(140, 135, 120);
/// The selected page's caption is drawn in the window title's own colour
/// (`GDR_ALCHEMYBOX_TITLE` `FontColor="255,239,218,164"`, the same in both shells);
/// the unselected one uses the dimmed [`CAPTION_IDLE_COLOR`].
const CAPTION_ACTIVE_COLOR: Color = Color::srgb_u8(239, 218, 164);
/// `GDR_ALCHEMYBOX_PML_TEXT` carries `FontColor="255,0,0,0"` — **black**, and that
/// is not a resinfo default to be ignored: the window plate puts a bright
/// parchment tablet exactly under this control, so the description is dark text
/// on parchment. Drawing it off-white on a fill tile was the reason it looked
/// wrong.
const PML_TEXT_COLOR: Color = Color::srgb_u8(24, 20, 14);
/// The chance line does **not** sit on the parchment — it sits on the page art,
/// which is dark. So it keeps the HUD's off-white and must not follow
/// [`PML_TEXT_COLOR`]: the two differ because their backgrounds differ.
const PAGE_TEXT_COLOR: Color = Color::srgb_u8(230, 226, 214);

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

/// The Fuse button.
#[derive(Component)]
pub struct AlchemyFuseButton;

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
    let chance = inventory.and_then(|inventory| slot_chance(&state, inventory, &item_data));

    commands.entity(window.content).with_children(|content| {
        // The window plate, with its title strip cropped off. It carries the
        // parchment tablet the description sits on, which is why that text is
        // dark. Drawn first, so the page art covers its lower half the way the
        // shell stacks them.
        content.spawn((
            abs_node(PLATE_RECT, s),
            ImageNode {
                image: asset_server.load(PLATE_DDJ),
                image_mode: NodeImageMode::Stretch,
                rect: Some(plate_crop()),
                ..default()
            },
            Pickable::IGNORE,
        ));

        // the page's own backdrop, at its native extent — 228 or 192 tall
        let page_rect = (
            PAGE_RECT.0,
            PAGE_RECT.1,
            PAGE_RECT.2,
            state.page.host_height(),
        );
        content.spawn((
            abs_node(page_rect, s),
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
                    CAPTION_IDLE_COLOR
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

        // The element page has no slots and no button of its own here: its
        // action has no opcode, so it shows its art and its description and
        // nothing that pretends to work.
        if !state.page.can_act() {
            return;
        }

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

        // The success chance the itemdata states for what is in the slots. No
        // line at all when the data does not answer — never a `0 %`.
        if let Some(chance) = chance {
            content.spawn((
                Text::new(chance_line(&ui_strings, chance)),
                TextFont {
                    font: fonts.two.clone().into(),
                    font_size: FontSize::Px(8.5 * s),
                    ..default()
                },
                TextColor(PAGE_TEXT_COLOR),
                TextLayout::justify(Justify::Left),
                abs_node(CHANCE_RECT, s),
                Pickable::IGNORE,
            ));
        }

        // Fuse — one face, always pressable. The original does not grey this
        // button out for an empty page, so neither does this: a press with
        // nothing placed answers with the archive's own refusal line instead.
        content
            .spawn((
                AlchemyFuseButton,
                Hovered::default(),
                abs_node(BUTTON_RECT, s),
                ImageNode {
                    image: asset_server.load(BUTTON_DDJ),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
            ))
            .observe(on_fuse_button)
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

/// A Fuse press: send the page's own opcode — `0x7150` on Equip Enhance,
/// `0x7151` on Att.Grant — and then wait. Nothing is predicted: the slots stay
/// where the player put them and the bag is untouched, because no answer to
/// either opcode is wired yet.
///
/// Two refusals, both with the archive's own line:
/// - fewer than two slots filled — `UIIT_MSG_REINFORCERR_NO_ITEM_LOADED`. This is
///   not politeness, it is the wire guard: the page cannot express such a fuse,
///   and on the stone opcode the body would sit one byte from a cancel
///   (`packets::agent::alchemy::ALCHEMY_MIN_FUSE_SLOTS`).
/// - no agent connection — `UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY`.
fn on_fuse_button(
    press: On<Pointer<Press>>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    inventories: Query<&Inventory, With<Player>>,
    item_data: Res<ClientItemData>,
    ui_strings: Res<ClientUiStrings>,
    mut state: ResMut<AlchemyState>,
    mut history: ResMut<ChatHistory>,
) {
    if press.event.button != PointerButton::Primary || state.pending {
        return;
    }
    let Some(slots) = state.fuse_slots() else {
        let (key, fallback) = FUSE_INCOMPLETE;
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        return;
    };
    let packet = match state.page {
        AlchemyPage::EquipEnhance => AlchemyReinforceRequest::fuse(slots).map(Packet::from),
        AlchemyPage::AttGrant => {
            let stone_type = stone_type_in_page(&state, inventories.single().ok(), &item_data);
            AlchemyStoneRequest::fuse(stone_type, slots).map(Packet::from)
        }
        // No opcode carries this page's action, so it draws no button at all and
        // a press cannot reach here. Leaving the arm as a silent return keeps the
        // decision in one place — the page's own `can_act` — instead of a second
        // copy of it, and says nothing to the player about slots.
        AlchemyPage::Elementation => return,
    };
    // unreachable while fuse_slots() enforces the same minimum, and it stays
    // checked because the two limits must not drift apart
    let Some(packet) = packet else {
        let (key, fallback) = FUSE_INCOMPLETE;
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        return;
    };
    let Ok(connection) = conn.single() else {
        let (key, fallback) = FUSE_UNAVAILABLE;
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        return;
    };
    if let Err(e) = connection.get_sender().send(packet.into()) {
        error!("alchemy: failed to send the fuse request: {}", e.0);
        return;
    }
    state.pending = true;
}

/// Which stone kind the Att.Grant page is holding, read off the material's own
/// itemdata row rather than offered as a choice: a magic stone row gives the
/// magic kind, anything else the attribute kind. The attribute kind is the
/// fallback because it is the page's own subject — the page grants attributes,
/// and the magic stone is the narrower, positively identified case.
fn stone_type_in_page(
    state: &AlchemyState,
    inventory: Option<&Inventory>,
    item_data: &ClientItemData,
) -> u8 {
    stone_type_of((1..=STONE_SLOTS).filter_map(|index| {
        let item = inventory?.get(state.slot(index)?)?;
        item_data.get(&(item.ref_id as i32))
    }))
}

/// The kind byte for a set of material rows — the decidable half of
/// [`stone_type_in_page`], kept separate so it can be checked without a world.
fn stone_type_of<'a>(rows: impl Iterator<Item = &'a ItemDataRow>) -> u8 {
    if rows.into_iter().any(is_magic_stone) {
        ALCHEMY_TYPE_MAGIC_STONE
    } else {
        ALCHEMY_TYPE_ATTRIBUTE_STONE
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

    /// The composed window is the page host's `y=150` plus the tallest page:
    /// `150 + 228 = 378`. Our content is that minus the 42-unit title strip the
    /// chrome's caption band replaces.
    #[test]
    fn alchemy_content_is_the_window_minus_its_title_strip() {
        assert_eq!((CONTENT_W, CONTENT_H), (376.0, 336.0));
        assert_eq!(COMPOSED_H - TITLE_STRIP, CONTENT_H);
        // the tallest page fills the content space exactly, bottom-aligned
        assert_eq!(PAGE_RECT.1 + PAGE_RECT.3, CONTENT_H);
        // and no page is taller than the window is built for
        for page in AlchemyPage::ALL {
            assert!(page.host_height() <= PAGE_RECT.3);
            assert!(PAGE_RECT.1 + page.host_height() <= CONTENT_H);
        }
        assert_eq!(TALLEST_HOST_H, PAGE_RECT.3);
    }

    /// The numbers that separate the loaded description from the one this file
    /// used to read. Pinned as a pair so a future edit cannot drift back: the
    /// shell we build from puts the text block at `y=97` and gives the reinforce
    /// and element pages `228`, where the other file had `89` and `192`.
    #[test]
    fn the_shell_numbers_are_the_loaded_descriptions_not_the_other_files() {
        assert_eq!(PML_RECT.1, 97.0 - TITLE_STRIP, "text block at y=97, not 89");
        assert_eq!(AlchemyPage::EquipEnhance.host_height(), 228.0);
        assert_eq!(AlchemyPage::Elementation.host_height(), 228.0);
        assert_eq!(
            AlchemyPage::AttGrant.host_height(),
            192.0,
            "the enchant page alone is 192"
        );
        // three pages, not two
        assert_eq!(AlchemyPage::ALL.len(), 3);
        // the reinforce page's art is the one the loaded shell names
        assert!(page_art(AlchemyPage::EquipEnhance).ends_with("alcm_window_quick mastery.ddj"));
    }

    /// The plate is placed by its **drawn** height, not by the file's: the bottom
    /// 80 rows of the 376x376 image are fully transparent. The title strip is
    /// cropped off because the chrome's caption band replaces it, so the crop and
    /// the placement must account for exactly that strip and nothing else.
    #[test]
    fn the_window_plate_is_cropped_by_the_title_strip_only() {
        let crop = plate_crop();
        assert_eq!(crop.min.y, TITLE_STRIP);
        assert_eq!(crop.max.y, PLATE_DRAWN_H);
        assert_eq!((crop.min.x, crop.max.x), (0.0, WINDOW_W));
        // what is drawn is exactly what was cropped
        assert_eq!(PLATE_RECT.3, crop.height());
        assert_eq!(PLATE_RECT.2, crop.width());
        // the parchment tablet sits inside the drawn band, where the text goes
        assert!(PML_RECT.1 >= 0.0 && PML_RECT.1 + PML_RECT.3 <= PLATE_RECT.3);
    }

    /// The button wears **one** face and is always pressable: the original does not
    /// grey it out for an empty page, and its caption is the undimmed shipped
    /// colour. The only dimmed text in this window is an unselected page caption,
    /// and that row is ours to begin with.
    #[test]
    fn the_fuse_button_has_no_disabled_face() {
        assert!(BUTTON_DDJ.ends_with("alcm_button_01.ddj"), "{BUTTON_DDJ}");
        assert!(
            !BUTTON_DDJ.contains("disable"),
            "no disabled face is referenced at all"
        );
        // the caption is the shipped FontColor, not a dimmed derivation of it
        assert_eq!(BUTTON_TEXT_COLOR, Color::srgb_u8(255, 245, 218));
        assert_ne!(BUTTON_TEXT_COLOR, CAPTION_IDLE_COLOR);
        let lum = |c: Color| {
            let s = c.to_srgba();
            s.red + s.green + s.blue
        };
        assert!(lum(BUTTON_TEXT_COLOR) > lum(CAPTION_IDLE_COLOR));
    }

    /// The element page is shown and cannot act, and the two text colours differ
    /// because their backgrounds do: the description is dark on parchment, the
    /// chance line light on the page art.
    #[test]
    fn the_element_page_is_shown_but_cannot_act() {
        assert!(!AlchemyPage::Elementation.can_act());
        assert!(AlchemyPage::EquipEnhance.can_act() && AlchemyPage::AttGrant.can_act());
        assert_ne!(PML_TEXT_COLOR, PAGE_TEXT_COLOR);
        let lum = |c: Color| {
            let s = c.to_srgba();
            s.red + s.green + s.blue
        };
        assert!(
            lum(PML_TEXT_COLOR) < lum(PAGE_TEXT_COLOR),
            "dark on parchment"
        );
    }

    /// Declared rects, rebased. Window-space controls lose the title strip;
    /// page-local controls gain the page host's `y=150` and then lose it too.
    #[test]
    fn the_rects_are_the_declared_rects_minus_the_title_strip() {
        // (y in window space, ours). Shell: `GDR_ALCHEMYBOX_PML_TEXT` (39,97),
        // page host (0,150). Page body, from the reinforce description the
        // loaded shell uses: `_SLOT_EQUIP` (58,74), `_SLOT_01` (164,74),
        // `_SLOT_04` (308,74), `_BUTTON_PROCESS` (136,178).
        let cases = [
            (97.0, PML_RECT.1),
            (150.0, PAGE_RECT.1),
            (150.0 + 74.0, slot_rect(EQUIP_SLOT).1),
            (150.0 + 74.0, slot_rect(1).1),
            (150.0 + 178.0, BUTTON_RECT.1),
        ];
        for (declared_y, ours) in cases {
            assert_eq!(ours, declared_y - TITLE_STRIP, "y of declared {declared_y}");
        }
        assert_eq!(slot_rect(EQUIP_SLOT).0, 58.0);
        assert_eq!(slot_rect(1).0, 164.0);
        assert_eq!(slot_rect(STONE_SLOTS).0, 308.0);
        // every slot is 32x32, and the stone row's pitch is 48
        for index in 0..=STONE_SLOTS {
            assert_eq!((slot_rect(index).2, slot_rect(index).3), (SLOT, SLOT));
        }
        assert_eq!(slot_rect(2).0 - slot_rect(1).0, 48.0);
        // the button is the art's own 104x28, not the other family's 112x28
        assert_eq!((BUTTON_RECT.2, BUTTON_RECT.3), (104.0, 28.0));
        // and the chance line still fits between the slot row and the button
        assert!(CHANCE_RECT.1 >= slot_rect(1).1 + SLOT);
        assert!(CHANCE_RECT.1 + CHANCE_RECT.3 <= BUTTON_RECT.1);
    }

    /// Rows mirroring `probability.rs`'s own fixtures, so the tests below prove
    /// the page gate and the line's composition, not the arithmetic.
    mod fixture {
        use super::*;

        fn row(tid: (u32, u32, u32, u32), params: (i64, [i64; 3])) -> ItemDataRow {
            let mut fields = vec![String::from("0"); 161];
            for (index, value) in [(9, tid.0), (10, tid.1), (11, tid.2), (12, tid.3)] {
                fields[index] = value.to_string();
            }
            fields[118] = params.0.to_string();
            for (block, value) in params.1.into_iter().enumerate() {
                fields[120 + block * 2] = value.to_string();
            }
            ItemDataRow(fields)
        }

        /// `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_WEAPON_A`, id 3675 — weapon gate,
        /// ladder `25,20,15,10 | 10,10,10,10 | 10,5,5,5`.
        pub fn elixir_weapon_a() -> ItemDataRow {
            row(
                (3, 3, 10, 1),
                (100663296, [420744970, 168430090, 168101125]),
            )
        }

        /// `ITEM_ETC_ARCHEMY_REINFORCE_PROB_UP_A_01`, id 3683 — degree 1.
        pub fn powder_degree_1() -> ItemDataRow {
            row((3, 3, 10, 2), (1, [840832008, 134678022, 101057538]))
        }

        /// `ITEM_ETC_ARCHEMY_MAGICSTONE_STR_01`, id 6679 — TID `3.3.11.1`.
        pub fn magic_stone() -> ItemDataRow {
            row((3, 3, 11, 1), (0, [0, 0, 0]))
        }

        /// Equipment of `tid3`, with `ItemClass` -> degree (col 61).
        pub fn equipment(tid3: u32, item_class: u32) -> ItemDataRow {
            let mut fields = vec![String::from("0"); 161];
            for (index, value) in [(9, 3), (10, 1), (11, tid3), (12, 2)] {
                fields[index] = value.to_string();
            }
            fields[61] = item_class.to_string();
            ItemDataRow(fields)
        }
    }

    /// The chance line is the shipped label plus the data's own number, and
    /// nothing else — no invented sentence. With the table absent (as here)
    /// `get_or` falls back to the English of `textuisystem.txt:1932`.
    #[test]
    fn the_chance_line_is_a_shipped_label_plus_the_data_value() {
        let strings = ClientUiStrings::default();
        assert_eq!(
            chance_line(
                &strings,
                ReinforceChance {
                    percent: 25,
                    powder_bonus: None,
                }
            ),
            "Probability: 25 %"
        );
        // a matched Lucky Powder's points stay their own figure
        assert_eq!(
            chance_line(
                &strings,
                ReinforceChance {
                    percent: 25,
                    powder_bonus: Some(50),
                }
            ),
            "Probability: 25 % + 50 %"
        );
    }

    /// Our own control: the chance line lives in the page's empty band between
    /// the slot row and the Fuse button, and overlaps neither.
    #[test]
    fn the_chance_line_sits_between_the_slots_and_the_button() {
        let slots_bottom = slot_rect(1).1 + slot_rect(1).3;
        assert!(CHANCE_RECT.1 >= slots_bottom, "below the slot row");
        assert!(
            CHANCE_RECT.1 + CHANCE_RECT.3 <= BUTTON_RECT.1,
            "above the Fuse button"
        );
        // and inside the page, aligned with the first stone slot
        assert!(CHANCE_RECT.0 + CHANCE_RECT.2 <= PAGE_RECT.2);
        assert_eq!(CHANCE_RECT.0, STONE_XS[0]);
    }

    /// The success chance belongs to the Equip Enhance page: identical slot
    /// contents produce a line there and no line at all on Att.Grant.
    #[test]
    fn the_chance_line_belongs_to_the_equip_enhance_page() {
        let sword = fixture::equipment(6, 1);
        let elixir = fixture::elixir_weapon_a();
        let stones = [&elixir];
        assert_eq!(
            page_chance(AlchemyPage::EquipEnhance, &sword, 0, &stones),
            Some(ReinforceChance {
                percent: 25,
                powder_bonus: None,
            })
        );
        // the Att.Grant page's outcome is not a +n step, so no line
        assert_eq!(page_chance(AlchemyPage::AttGrant, &sword, 0, &stones), None);
    }

    /// No matching material means **no line** — not a "0 %". The equipment
    /// alone, or equipment plus a non-elixir stone, says nothing.
    #[test]
    fn a_page_without_an_elixir_shows_no_line_at_all() {
        let sword = fixture::equipment(6, 1);
        assert_eq!(page_chance(AlchemyPage::EquipEnhance, &sword, 0, &[]), None);
        let powder = fixture::powder_degree_1();
        assert_eq!(
            page_chance(AlchemyPage::EquipEnhance, &sword, 0, &[&powder]),
            None,
            "Lucky Powder on its own is not a chance"
        );
        // and an elixir whose ladder is exhausted stays silent as well
        let elixir = fixture::elixir_weapon_a();
        assert_eq!(
            page_chance(AlchemyPage::EquipEnhance, &sword, 12, &[&elixir]),
            None
        );
    }

    /// Both fuse refusals are the archive's own lines, not invented sentences.
    #[test]
    fn both_fuse_refusals_are_shipped_lines() {
        let strings = ClientUiStrings::default();
        for (key, fallback) in [FUSE_INCOMPLETE, FUSE_UNAVAILABLE] {
            assert!(
                key.starts_with("UIIT_MSG_REINFORCERR_"),
                "{key} is a shipped key"
            );
            // with the table absent the fallback is the shipped English
            assert_eq!(strings.get_or(key, fallback), fallback);
            assert!(fallback.ends_with('.'), "{key} reads as a sentence");
        }
        assert_ne!(FUSE_INCOMPLETE.0, FUSE_UNAVAILABLE.0);
    }

    /// The page's slots reach the wire unchanged and in the window's own order:
    /// equipment first, then the stones as the row reads.
    #[test]
    fn the_pages_slots_reach_the_wire_unchanged() {
        let mut state = AlchemyState::default();
        state.place(EQUIP_SLOT, 19);
        state.place(1, 15);
        let slots = state.fuse_slots().unwrap();
        assert_eq!(slots, vec![19, 15]);

        let elixir: bytes::Bytes = AlchemyReinforceRequest::fuse(slots.clone()).unwrap().into();
        assert_eq!(&elixir[3..], &[19, 15], "the slots go out unchanged");
        let stone: bytes::Bytes = AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, slots)
            .unwrap()
            .into();
        assert_eq!(&stone[3..], &[19, 15]);
        assert_eq!(stone[1], ALCHEMY_TYPE_MAGIC_STONE);
    }

    /// A page with only one item never becomes a request — the limit lives in the
    /// model and in the constructor, and the two must agree.
    #[test]
    fn a_page_with_one_item_never_becomes_a_request() {
        let mut state = AlchemyState::default();
        state.place(EQUIP_SLOT, 19);
        assert_eq!(state.fuse_slots(), None);
        assert!(AlchemyReinforceRequest::fuse(vec![19]).is_none());
        assert!(AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, vec![19]).is_none());
    }

    /// The stone kind comes from the material's row, not from a player choice,
    /// and the magic kind is the positively identified case.
    #[test]
    fn the_stone_kind_is_read_off_the_material_row() {
        let magic = fixture::magic_stone();
        let elixir = fixture::elixir_weapon_a();
        assert_eq!(
            stone_type_of([&magic].into_iter()),
            ALCHEMY_TYPE_MAGIC_STONE
        );
        assert_eq!(
            stone_type_of([&elixir, &magic].into_iter()),
            ALCHEMY_TYPE_MAGIC_STONE,
            "one magic stone anywhere in the row decides it"
        );
        assert_eq!(
            stone_type_of([&elixir].into_iter()),
            ALCHEMY_TYPE_ATTRIBUTE_STONE
        );
        assert_eq!(
            stone_type_of(std::iter::empty()),
            ALCHEMY_TYPE_ATTRIBUTE_STONE,
            "an empty row falls to the page's own subject"
        );
    }

    /// Vanilla gives all four stone slots the same prototype, so the elixir is
    /// found by its type ids and not by its position: a powder in the first
    /// slot must not shadow an elixir in the second.
    #[test]
    fn the_elixir_is_found_by_its_type_not_by_its_slot() {
        let sword = fixture::equipment(6, 1);
        let elixir = fixture::elixir_weapon_a();
        let powder = fixture::powder_degree_1();
        let expected = Some(ReinforceChance {
            percent: 25,
            powder_bonus: Some(50),
        });
        for stones in [
            vec![&elixir, &powder],
            vec![&powder, &elixir],
            vec![&powder, &powder, &elixir],
        ] {
            assert_eq!(
                page_chance(AlchemyPage::EquipEnhance, &sword, 0, &stones),
                expected,
                "the elixir is the row, not the slot index"
            );
        }
    }

    /// The selector row sits in the band the shell leaves free above the
    /// description block, one entry per declared page, and the entries divide
    /// that band instead of landing on hand-picked x values.
    #[test]
    fn the_page_selector_has_one_entry_per_page_above_the_description() {
        assert_eq!(
            AlchemyPage::ALL.len(),
            3,
            "the loaded shell hosts three pages"
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
        assert!(page_art(AlchemyPage::EquipEnhance).ends_with("alcm_window_quick mastery.ddj"));
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
            (400.0, 388.0)
        );
        assert_eq!(ORIGIN_Y, 36.0);
    }
}

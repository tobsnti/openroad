//! Alchemy box window — the classic shell and its two pages, Equip Enhance
//! and Attribute Grant.
//!
//! Idea: the vanilla alchemy box is a 376-wide window whose shell
//! (`ginterface.txt:842-863`, `GDR_ALCHEMYBOX` id 44, `Rect="595,262,376,152"`)
//! hosts its pages at `y=150`. The OLD classic shell declares exactly two of
//! them, both at the same rect `0,150,376,192`:
//! `GDR_ALCHEMYBOX_ENCHANT_MAGIC_PARAM` (`ifalchemybox.txt:6-23`, art
//! `alcm_window_allowance.ddj`) and `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT`
//! (`:25-42`, art `alcm_window_reinforcement.ddj`). Composed extent is
//! therefore `376x342`; our content space is that minus the vanilla 42px title
//! strip (`GDR_ALCHEMYBOX_DRAG` `10,0,355,42`), which the shared `game_window`
//! chrome's caption band replaces — leaving the page art at its native
//! 376x192 with no rescale (both DDJs measure 376x192 in their DDS header).
//!
//! **The two page bodies are the same layout, and that is a data fact, not an
//! assumption.** `ifalchemyenchant.txt` and `ifalchemyreinforce.txt` are 149
//! lines each and differ *only* in the `GDR_AB_ENCHANT_` / `GDR_AB_REINFORCE_`
//! name prefix: same seven controls, same ids 30/38..42/50, same rects, same
//! button text. So the layout constants below are shared by both pages and
//! cite both files. (The NEW shell's `ifnewalchemyreinforce.txt` is *not* the
//! same — slots at `y=74`, button `136,178` on `alcm_button_01.ddj` — which is
//! exactly why this builds the OLD body: it is the one that fits the 192-tall
//! host this window is already composed around, so nothing here decides which
//! of the two shells the original opens.)
//!
//! Slots hold *references* to inventory slots (model.rs), and the Fuse button
//! sends: `0x7150` on the Equip Enhance page, `0x7151` on Att.Grant
//! (`fuse_button_sends`). The slot numbers go on the wire unchanged — they are
//! inventory slots on both sides (`packets::agent::alchemy`). The button is
//! drawn in its vanilla disabled state only while the page cannot make a
//! request (no equipment, no material) or while an ack is outstanding.
//!
//! **The page selector is ours in position and the data's in substance**
//! (ADR-0009 deviation, stated): no resinfo tree declares a tab or button for
//! the pages — positive control on the same read path, the same two files do
//! declare `GDR_ALCHEMYBOX_CLOSE` and `GDR_ALCHEMYBOX_DRAG`. What the archive
//! *does* ship is one 20x24 gem lamp per page in on/off pairs
//! (`interface/alchemy/alcm_lamp_enchant_{on,off}.ddj` green,
//! `alcm_lamp_reinforcement_{on,off}.ddj` violet, plus element/decomposition
//! for the NEW shell's third page), and the page arts carry those very colours
//! — the allowance page's gems are green, the reinforcement page's violet,
//! straight out of the art. Plus one caption string per page host
//! (`textuisystem.txt:771` `UIIT_STT_ALCHEMYBOX_REINFORCE_ITEM` = *"Equip
//! Enhance"*, `:773` `..._ENCHANT_MAGIC_PARAM` = *"Att.Grant"*). So the lamps
//! and the words are the original's; only the row's rect is our choice, taken
//! in the shell's one empty band (below the caption band, above the
//! description block at `39,89,300,48`). Where the original puts that row is
//! still open.

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
use crate::plugins::hud::alchemy::enchant::{AlchemyVerb, EnchantState};
use crate::plugins::hud::alchemy::model::{AlchemyPage, AlchemyState, EQUIP_SLOT, STONE_SLOTS};
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
/// `0,150,376,192` (`ifalchemybox.txt:15`) and
/// `GDR_ALCHEMYBOX_REINFORCE_EQUIPMENT` `0,150,376,192` (`:34`).
const PAGE_RECT: (f32, f32, f32, f32) = (0.0, 150.0 - TITLE_STRIP, 376.0, 192.0);
/// `GDR_AB_ENCHANT_SLOT_EQUIP` / `GDR_AB_REINFORCE_SLOT_EQUIP` `59,56,32,32`,
/// page-local (`ifalchemyenchant.txt:120` = `ifalchemyreinforce.txt:120`).
const EQUIP_RECT: (f32, f32, f32, f32) = (59.0, PAGE_RECT.1 + 56.0, SLOT, SLOT);
/// `_SLOT_01..04` `164/212/260/308,56,32,32`, page-local
/// (`ifalchemyenchant.txt:99,78,57,36` = `ifalchemyreinforce.txt:99,78,57,36`).
const STONE_XS: [f32; STONE_SLOTS] = [164.0, 212.0, 260.0, 308.0];
const STONE_Y: f32 = PAGE_RECT.1 + 56.0;
/// `_BUTTON_PROCESS` `132,143,112,28`, page-local
/// (`ifalchemyenchant.txt:139` = `ifalchemyreinforce.txt:139`) — the art
/// (`alcm_button.ddj`) is exactly 112x28.
const BUTTON_RECT: (f32, f32, f32, f32) = (132.0, PAGE_RECT.1 + 143.0, 112.0, 28.0);
const SLOT: f32 = 32.0;
/// **Ours, not vanilla's** — the success-chance line (ADR-0009 deviation, see
/// `probability.rs`), and it belongs to the **Equip Enhance** page, the one
/// whose ladder it reads. Neither page body declares a text control for it:
/// `ifalchemyreinforce.txt`'s seven controls are the deco, five slots and the
/// button (positive control on the same read: the file *does* declare a
/// `CIFDecoratedStatic`, `:6-24`, so a text-capable control is a thing this
/// file could have carried). The rect is therefore a reasoned choice: the
/// page's only empty band, between the slot row (`56 + 32 = 88`) and the Fuse
/// button (`y = 143`), left-aligned with the first stone slot (`x = 164`) so it
/// reads as belonging to the material row it is computed from.
const CHANCE_RECT: (f32, f32, f32, f32) = (164.0, PAGE_RECT.1 + 104.0, 200.0, 14.0);

/// Page-selector row — **our rect, the data's art and words** (module doc).
/// The lamp extent is the DDJ's own: `alcm_lamp_*.ddj` are 20x24 in their DDS
/// header. `x` is the description block's `39` so the two align; `y` sits in
/// the band the shell leaves between the caption strip (ends at the vanilla
/// `y=42`, i.e. content `0`) and that block (vanilla `y=89`, content `47`).
const LAMP: (f32, f32) = (20.0, 24.0);
const SELECTOR_ORIGIN: (f32, f32) = (39.0, 10.0);
/// Three entries share the band the description block leaves free, so the
/// pitch is the width divided by them rather than a hand-picked number:
/// `(376 - 2*39) / 3`, i.e. the row is inset by its own origin on both sides.
const SELECTOR_PITCH: f32 = (CONTENT_W - 2.0 * SELECTOR_ORIGIN.0) / 3.0;
/// Gap between a lamp and its caption, and the caption box.
const SELECTOR_LABEL: (f32, f32, f32) = (4.0, SELECTOR_PITCH - LAMP.0 - 4.0 - 6.0, 12.0);
/// 32x32 in its DDS header — the exact slot extent.
const SLOT_DDJ: &str = "media://interface/alchemy/alcm_slot_closed.ddj";

/// `UIIT_MSG_REINFORCERR_NO_ITEM_LOADED` (`textuisystem.txt:2143`) — the
/// archive's own answer to a fuse press with an incomplete page. It replaces
/// the sentence this file used to invent, and it is also the guard that keeps
/// a one-slot request off the wire (see `on_fuse_button`).
const FUSE_INCOMPLETE: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_NO_ITEM_LOADED",
    "Cannot reinforce without the specified item.",
);
/// `UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY` (`:2149`) — the archive's "not
/// right now", used when there is no agent connection to send on.
const FUSE_UNAVAILABLE: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY",
    "You are not under the state to use alchemy.",
);
const BUTTON_DISABLED_DDJ: &str = "media://interface/alchemy/alcm_button_disable.ddj";
/// `alcm_button.ddj` is the enabled face of `_BUTTON_PROCESS`; the descriptor
/// gives the control exactly 112x28, which is the art's own size
/// (`ifalchemyreinforce.txt:139`).
const BUTTON_DDJ: &str = "media://interface/alchemy/alcm_button.ddj";

/// The page's own 376x192 backdrop, named by its host control in
/// `ifalchemybox.txt` (`:29` reinforce, `:10` enchant).
fn page_art(page: AlchemyPage) -> &'static str {
    match page {
        AlchemyPage::EquipEnhance => "media://interface/alchemy/alcm_window_reinforcement.ddj",
        AlchemyPage::AttGrant => "media://interface/alchemy/alcm_window_allowance.ddj",
    }
}

/// The selector lamp for a page. The `_on`/`_off` pair is the archive's own
/// active/inactive art, and the gem colour matches the page art's gems
/// (violet/green) — which is how these lamps were identified as the page
/// selector at all.
fn page_lamp(entry: SelectorEntry, active: bool) -> String {
    let stem = match entry {
        SelectorEntry::Page(AlchemyPage::EquipEnhance) => "alcm_lamp_reinforcement",
        SelectorEntry::Page(AlchemyPage::AttGrant) => "alcm_lamp_enchant",
        // the orphan pair: shipped art with no page body to belong to
        SelectorEntry::Dismantle => "alcm_lamp_decomposition",
    };
    let state = if active { "on" } else { "off" };
    format!("media://interface/alchemy/{stem}_{state}.ddj")
}

/// The caption string the archive ships for the page's host control, keyed by
/// that control's own name (`textuisystem.txt:771` / `:773`).
fn page_caption(entry: SelectorEntry) -> (&'static str, &'static str) {
    match entry {
        SelectorEntry::Page(AlchemyPage::EquipEnhance) => {
            ("UIIT_STT_ALCHEMYBOX_REINFORCE_ITEM", "Equip Enhance")
        }
        SelectorEntry::Page(AlchemyPage::AttGrant) => {
            ("UIIT_STT_ALCHEMYBOX_ENCHANT_MAGIC_PARAM", "Att.Grant")
        }
        // the 4th-gen window's own tab caption, `textuisystem.txt:4226`
        SelectorEntry::Dismantle => ("UIIT_CTL_ALCHEMYBOX_TAP_DISMANTLING", "Dismantle"),
    }
}

/// The description the shell's PML control carries for the page. Both keys are
/// shipped and named after their page (`textuisystem.txt:779` / `:781`); which
/// one the original feeds into the single `GDR_ALCHEMYBOX_PML_TEXT` control
/// follows from those names, and the fallbacks are their English text
/// abbreviated to one line (this PML renderer draws plain text, not the rows'
/// markup).
fn page_description(page: AlchemyPage) -> (&'static str, &'static str) {
    match page {
        AlchemyPage::EquipEnhance => (
            "UIIT_STT_ALCHEMYBOX_REINFORCE_TEXT",
            "Equipment Enhance: when equipment and elixir are combined, the equipment is \
             usually strengthened with + options. Warning: all used items will disappear.",
        ),
        AlchemyPage::AttGrant => (
            "UIIT_STT_ALCHEMYBOX_REINFORCE_ATTR_TEXT",
            "Att.Grant: using specific alchemy items will grant attributes.",
        ),
    }
}

/// Selector entry rect for the `nth` page, and its caption rect beside it.
fn selector_rects(nth: usize) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
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

/// A selector-row entry: one of the shell's two page hosts, or the archive's
/// orphan **decomposition** lamp.
///
/// The third entry is a stated ADR-0009 deviation, and it is the archive that
/// suggested it: `interface/alchemy/` ships `alcm_lamp_decomposition_{on,off}`
/// and `alcm_window_dismantle.ddj` while **no** `resinfo` tree declares a
/// dismantle page body for the classic shell (`ifalchemybox.txt` hosts exactly
/// two pages; the six classic files declare enchant, reinforce and
/// manufacturing bodies and nothing else). The only declared dismantle body in
/// the whole archive is the 4th-generation window's pane 28
/// (`res_ui/nifenchantwnd.2dt`, `hud/alchemy/enchant.rs`) — which has no
/// declared opener either. So the shipped lamp is used as that window's
/// opener: shipped art, shipped caption
/// (`UIIT_CTL_ALCHEMYBOX_TAP_DISMANTLING` = *"Dismantle"*), our placement, and
/// the whole selector row was already ours (module doc).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectorEntry {
    Page(AlchemyPage),
    /// Raises the 4th-gen window on its Dismantle tab.
    Dismantle,
}

impl SelectorEntry {
    pub const ALL: [SelectorEntry; 3] = [
        SelectorEntry::Page(AlchemyPage::EquipEnhance),
        SelectorEntry::Page(AlchemyPage::AttGrant),
        SelectorEntry::Dismantle,
    ];
}

/// A page-selector lamp.
#[derive(Component)]
pub struct AlchemyPageTab {
    pub entry: SelectorEntry,
}

/// `GDR_AB_ENCHANT_BUTTON_PROCESS` `FontColor="255,255,245,218"` (resinfo
/// COLOR is A,R,G,B), dimmed to 55% because the button is disabled.
const BUTTON_TEXT_COLOR: Color = Color::srgb_u8(140, 135, 120);
/// The selected page's caption is drawn in the window title's own colour
/// (`GDR_ALCHEMYBOX_TITLE` `FontColor="255,239,218,164"`,
/// `ifalchemybox.txt:106`); the unselected one reuses the dimmed
/// [`BUTTON_TEXT_COLOR`], so "dim = not active" reads the same way twice.
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

/// The Fuse button.
#[derive(Component)]
pub struct AlchemyFuseButton;

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
    let chance = inventory.and_then(|inventory| slot_chance(&state, inventory, &item_data));
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
        for (nth, entry) in SelectorEntry::ALL.into_iter().enumerate() {
            let (lamp_rect, label_rect) = selector_rects(nth);
            let active = entry == SelectorEntry::Page(state.page);
            let mut lamp = content.spawn((
                AlchemyPageTab { entry },
                Hovered::default(),
                abs_node(lamp_rect, s),
                ImageNode {
                    image: asset_server.load(page_lamp(entry, active)),
                    image_mode: NodeImageMode::Stretch,
                    ..default()
                },
            ));
            lamp.observe(on_page_tab_press);
            let (key, fallback) = page_caption(entry);
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
        // key per page, matched by name (see `page_description`).
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

        // The success chance the itemdata states for what is in the slots.
        // Nothing is drawn when the data says nothing (no elixir, wrong
        // equipment class, ladder exhausted) — a missing row is the original's
        // answer, not an error case, so there is no "0 %" and no placeholder.
        if let Some(chance) = chance {
            content.spawn((
                Text::new(chance_line(&ui_strings, chance)),
                TextFont {
                    font: fonts.two.clone().into(),
                    font_size: FontSize::Px(9.0 * s),
                    ..default()
                },
                TextColor(PML_TEXT_COLOR),
                TextLayout::justify(Justify::Left),
                abs_node(CHANCE_RECT, s),
                Pickable::IGNORE,
            ));
        }

        // Fuse — `GDR_AB_REINFORCE_BUTTON_PROCESS`
        // (`ifalchemyreinforce.txt:130-147`), and it now sends.
        //
        // The request does have a body: the original writes
        // `{u8 n, n × slot+13}` for the classic box's fuse and a bare `1` for
        // a cancel, whose only caller sits in the four-tab window's code, and
        // its ack handler reads the answer field for field. So the button is
        // enabled, and
        // `packets::agent::alchemy::ALCHEMY_MIN_FUSE_SLOTS` (not a comment)
        // keeps the one genuinely ambiguous body off the wire.
        //
        // The art follows the state: `alcm_button.ddj` when the page can be
        // fused, `alcm_button_disable.ddj` while it cannot — the archive ships
        // both faces, so "disabled" stays the original's picture of disabled.
        let ready = state.fuse_slots().is_some() && !state.pending;
        content
            .spawn((
                AlchemyFuseButton,
                Hovered::default(),
                abs_node(BUTTON_RECT, s),
                ImageNode {
                    image: asset_server.load(if ready {
                        BUTTON_DDJ
                    } else {
                        BUTTON_DISABLED_DDJ
                    }),
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
/// `textuisystem.txt:779`). The Att.Grant page fuses attribute stones into
/// magic options instead (`:781`, `UIIT_MSG_ALCHEMY_APPEND_ATTR` `:786`), a
/// result the elixir ladder in `probability.rs` says nothing about — so that
/// page shows no chance line at all rather than a number that means nothing
/// there.
///
/// Within the page the slots are not typed in vanilla either — both page
/// bodies give all four stone slots the same `CIFSlotWithHelp` prototype — so
/// the elixir is found by TID, not by position, exactly as the original's own
/// slot-mismatch messages imply
/// (`UIIT_MSG_REINFORCERR_IS_NOT_REINFORCE_STUFF`, `textuisystem.txt:2141`).
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
/// (`textuisystem.txt:1932` = *"Probability"*); the archive ships **no** key
/// that formats a success percentage, so the composition is ours (ADR-0009:
/// stated deviation rather than an invented vanilla-sounding sentence). The
/// powder's points stay a second figure because it is unknown whether a
/// server adds or multiplies them.
fn chance_line(ui_strings: &ClientUiStrings, chance: ReinforceChance) -> String {
    let label = ui_strings.get_or("UIIT_STT_PROBABILITY", "Probability");
    match chance.powder_bonus {
        Some(bonus) => format!("{label}: {} % + {bonus} %", chance.percent),
        None => format!("{label}: {} %", chance.percent),
    }
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
/// not something a server is told about — the whole placement path is
/// client-only for the same reason (model.rs).
fn on_page_tab_press(
    press: On<Pointer<Press>>,
    tabs: Query<&AlchemyPageTab>,
    mut state: ResMut<AlchemyState>,
    mut enchant: ResMut<EnchantState>,
) {
    if press.event.button != PointerButton::Primary {
        return;
    }
    let Ok(tab) = tabs.get(press.entity) else {
        return;
    };
    match tab.entry {
        SelectorEntry::Page(page) => {
            if state.page != page {
                state.page = page;
            }
        }
        // The orphan lamp opens the only declared dismantle body there is —
        // the 4th-gen window's pane 28 — on its Dismantle tab. It toggles, so
        // the same lamp closes it again: that window's descriptor declares no
        // close button of its own (72 entries, four `CNIFButton`s, all four
        // named — none of them a close).
        SelectorEntry::Dismantle => {
            enchant.verb = AlchemyVerb::Dismantle;
            enchant.open = !enchant.open;
        }
    }
}

/// A Fuse press: send the page's own verb — `0x7150` on Equip Enhance,
/// `0x7151` on Att.Grant — and then wait. Nothing is predicted: the slots stay
/// where the player put them and the bag is untouched until the ack arrives
/// (`outcome.rs`), which is the same rule the dismantle path follows and the
/// reason the item-use path once desynced.
///
/// Two refusals, both with the archive's own line:
/// - fewer than two slots filled — `UIIT_MSG_REINFORCERR_NO_ITEM_LOADED`. This
///   is not politeness, it is the wire guard: a one-slot `0x7151` body is
///   byte-identical to the *cancel* request, and the page cannot express such
///   a fuse anyway (`packets::agent::alchemy::ALCHEMY_MIN_FUSE_SLOTS`).
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
    let (key, fallback) = FUSE_INCOMPLETE;
    let Some(slots) = state.fuse_slots() else {
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        return;
    };
    let packet = match state.page {
        AlchemyPage::EquipEnhance => AlchemyReinforceRequest::fuse(slots.clone()).map(Packet::from),
        // The stone's kind is the client's classification of the item in the
        // slot, not a player choice: the builder maps its argument 1 -> 4
        // (magic stone) and 2 -> 5 (attribute stone), and which one is in the
        // page is decided by the itemdata row.
        AlchemyPage::AttGrant => {
            let stone_type = stone_type_in_page(&state, inventories.single().ok(), &item_data);
            AlchemyStoneRequest::fuse(stone_type, slots.clone()).map(Packet::from)
        }
    };
    let Some(packet) = packet else {
        // unreachable while fuse_slots() enforces the same minimum, and it
        // stays checked because the two constants must not drift apart
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        return;
    };
    let Ok(conn) = conn.single() else {
        let (key, fallback) = FUSE_UNAVAILABLE;
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        warn!("alchemy: no agent connection, fuse not sent");
        return;
    };
    if let Err(e) = conn.get_sender().send(packet.into()) {
        warn!("alchemy: failed to send fuse: {e}");
        return;
    }
    info!(
        "alchemy: fusing slots {slots:?} on the {:?} page",
        state.page
    );
    state.pending = true;
}

/// Which `AlchemyType` the Att.Grant page is holding. `is_attribute_stone`
/// lives in `probability.rs` next to the other itemdata classifications; a
/// page with no recognisable stone still sends the attribute type, because
/// that is the page's own subject (`UIIT_STT_ALCHEMYBOX_ENCHANT_MAGIC_PARAM` =
/// *"Att.Grant"*) and the server refuses what it does not accept.
fn stone_type_in_page(
    state: &AlchemyState,
    inventory: Option<&Inventory>,
    item_data: &ClientItemData,
) -> u8 {
    let row_of = |page_slot: usize| -> Option<&ItemDataRow> {
        let wire = state.slot(page_slot)?;
        item_data.get(&(inventory?.get(wire)?.ref_id as i32))
    };
    let has_magic_stone = (1..=STONE_SLOTS).filter_map(row_of).any(is_magic_stone);
    if has_magic_stone {
        ALCHEMY_TYPE_MAGIC_STONE
    } else {
        ALCHEMY_TYPE_ATTRIBUTE_STONE
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
    ///
    /// The page-local values are quoted from **`ifalchemyreinforce.txt`** here,
    /// which is the same as quoting `ifalchemyenchant.txt`: the two files carry
    /// identical rects (module doc), which is exactly why one set of constants
    /// serves both pages.
    #[test]
    fn alchemy_rects_are_the_vanilla_rects_minus_the_title_strip() {
        // (vanilla y in window space, ours) — ifalchemybox.txt
        // GDR_ALCHEMYBOX_PML_TEXT (39,89) :53, the page hosts (0,150) :15/:34;
        // ifalchemyreinforce.txt page-local GDR_AB_REINFORCE_SLOT_EQUIP
        // (59,56) :120, _SLOT_01 (164,56) :99, _SLOT_04 (308,56) :36,
        // _BUTTON_PROCESS (132,143) :139.
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
        // and inside the page
        assert!(CHANCE_RECT.0 + CHANCE_RECT.2 <= PAGE_RECT.2);
    }

    /// The success chance belongs to the Equip Enhance page: identical slot
    /// contents produce a line there and no line at all on Att.Grant. The rows
    /// mirror `probability.rs`'s own fixtures (`ITEM_ETC_ARCHEMY_REINFORCE_
    /// RECIPE_WEAPON_A`, id 3675, ladder `25,20,15,10|...`) so this test proves
    /// the page gate, not the arithmetic.
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

    /// All three selector entries are reachable, each sits in the shell band
    /// above the description block, they do not overlap, and the row still
    /// fits the 376-wide window now that a third entry shares it.
    #[test]
    fn the_page_selector_has_one_entry_per_page_above_the_description() {
        assert_eq!(AlchemyPage::ALL.len(), 2);
        assert_eq!(SelectorEntry::ALL.len(), 3, "two pages plus the opener");
        let mut previous_end = 0.0;
        for (nth, _) in SelectorEntry::ALL.into_iter().enumerate() {
            let (lamp, label) = selector_rects(nth);
            // the lamp is the DDJ's own 20x24
            assert_eq!((lamp.2, lamp.3), LAMP);
            assert!(lamp.0 >= previous_end, "entry {nth} starts after the last");
            assert!(
                lamp.1 >= 0.0 && lamp.1 + lamp.3 <= PML_RECT.1,
                "entry {nth} sits above the description block"
            );
            assert_eq!(label.1 + label.3 / 2.0, lamp.1 + lamp.3 / 2.0);
            assert!(
                label.0 + label.2 <= CONTENT_W,
                "entry {nth} fits the window"
            );
            previous_end = label.0 + label.2;
        }
    }

    /// The third entry is the archive's **orphan** lamp: `interface/alchemy/`
    /// ships `alcm_lamp_decomposition_{on,off}` while no classic page body
    /// exists for it, so it opens the one declared dismantle body there is —
    /// the 4th-gen pane (`enchant.rs`). Its caption is that window's own
    /// shipped tab string, not a sentence of ours.
    #[test]
    fn the_third_selector_entry_is_the_shipped_decomposition_lamp() {
        assert_eq!(SelectorEntry::ALL[2], SelectorEntry::Dismantle);
        assert_eq!(
            page_lamp(SelectorEntry::Dismantle, true),
            "media://interface/alchemy/alcm_lamp_decomposition_on.ddj"
        );
        assert_eq!(
            page_lamp(SelectorEntry::Dismantle, false),
            "media://interface/alchemy/alcm_lamp_decomposition_off.ddj"
        );
        assert_eq!(
            page_caption(SelectorEntry::Dismantle),
            ("UIIT_CTL_ALCHEMYBOX_TAP_DISMANTLING", "Dismantle")
        );
        // every entry owns its own lamp art
        let mut lamps: Vec<String> = SelectorEntry::ALL
            .into_iter()
            .map(|entry| page_lamp(entry, true))
            .collect();
        lamps.sort();
        lamps.dedup();
        assert_eq!(lamps.len(), 3);
    }

    /// Both refusal lines come out of the shipped text — the invented sentence
    /// this file used to print ("Fuse is not wired: …") is gone, together with
    /// the wrong reason it stated.
    #[test]
    fn both_fuse_refusals_are_shipped_lines() {
        let strings = ClientUiStrings::default();
        assert_eq!(FUSE_INCOMPLETE.0, "UIIT_MSG_REINFORCERR_NO_ITEM_LOADED");
        assert_eq!(
            strings.get_or(FUSE_INCOMPLETE.0, FUSE_INCOMPLETE.1),
            "Cannot reinforce without the specified item."
        );
        assert_eq!(
            FUSE_UNAVAILABLE.0,
            "UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY"
        );
        assert_eq!(
            strings.get_or(FUSE_UNAVAILABLE.0, FUSE_UNAVAILABLE.1),
            "You are not under the state to use alchemy."
        );
    }

    /// The send-side lock, at the level a caller cannot skip: a page holding
    /// only the equipment builds no request at all, because that body would go
    /// out as the *cancel* form (`ALCHEMY_MIN_FUSE_SLOTS`).
    #[test]
    fn a_page_with_one_item_never_becomes_a_request() {
        let mut state = AlchemyState::default();
        state.place(EQUIP_SLOT, 13);
        assert_eq!(state.fuse_slots(), None);
        assert!(AlchemyReinforceRequest::fuse(vec![13]).is_none());
        state.place(1, 20);
        assert_eq!(state.fuse_slots(), Some(vec![13, 20]), "equipment first");
        assert!(AlchemyReinforceRequest::fuse(state.fuse_slots().unwrap()).is_some());
        // and the Att.Grant page sends the stone verb with an AlchemyType
        assert_ne!(ALCHEMY_TYPE_MAGIC_STONE, ALCHEMY_TYPE_ATTRIBUTE_STONE);
        assert!(AlchemyStoneRequest::fuse(ALCHEMY_TYPE_ATTRIBUTE_STONE, vec![13, 20]).is_some());
    }

    /// The page's slot numbers are inventory slots and reach the wire that
    /// way. Biasing them again in `packets` sent the fuse 13 slots deeper into
    /// the bag than the player pointed at.
    #[test]
    fn the_pages_slots_reach_the_wire_unchanged() {
        let mut state = AlchemyState::default();
        state.place(EQUIP_SLOT, 13);
        state.place(1, 20);
        let slots = state.fuse_slots().unwrap();
        let body: bytes::Bytes = AlchemyReinforceRequest::fuse(slots).unwrap().into();
        assert!(
            body.ends_with(&[13, 20]),
            "the wire named {body:?}, not the slots the player filled"
        );
        let stone: bytes::Bytes = AlchemyStoneRequest::fuse(ALCHEMY_TYPE_MAGIC_STONE, vec![13, 20])
            .unwrap()
            .into();
        assert!(stone.ends_with(&[13, 20]), "0x7151 named {stone:?}");
    }

    /// Every page names art and strings that exist in the archive, and the two
    /// pages never share them (a copy-paste in `page_art` would show up here).
    #[test]
    fn each_page_names_its_own_art_and_strings() {
        let arts: Vec<_> = AlchemyPage::ALL.into_iter().map(page_art).collect();
        assert_eq!(
            arts,
            [
                "media://interface/alchemy/alcm_window_reinforcement.ddj",
                "media://interface/alchemy/alcm_window_allowance.ddj",
            ]
        );
        assert_eq!(
            page_lamp(SelectorEntry::Page(AlchemyPage::EquipEnhance), true),
            "media://interface/alchemy/alcm_lamp_reinforcement_on.ddj"
        );
        assert_eq!(
            page_lamp(SelectorEntry::Page(AlchemyPage::AttGrant), false),
            "media://interface/alchemy/alcm_lamp_enchant_off.ddj"
        );
        // the captions are the shipped keys, one per page host
        assert_eq!(
            page_caption(SelectorEntry::Page(AlchemyPage::EquipEnhance)).0,
            "UIIT_STT_ALCHEMYBOX_REINFORCE_ITEM"
        );
        assert_eq!(
            page_caption(SelectorEntry::Page(AlchemyPage::AttGrant)).0,
            "UIIT_STT_ALCHEMYBOX_ENCHANT_MAGIC_PARAM"
        );
        assert_ne!(
            page_description(AlchemyPage::EquipEnhance).0,
            page_description(AlchemyPage::AttGrant).0
        );
    }

    /// Rows built exactly like `probability.rs`'s fixtures — the same column
    /// indices the shipped `itemdata` uses (`Param<n>` at `118 + 2*(n-1)`), so
    /// this file needs no PK2 to test the page gate.
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
            fields[126] = String::from("-1");
            ItemDataRow(fields)
        }

        /// `ITEM_ETC_ARCHEMY_REINFORCE_RECIPE_WEAPON_A`, id 3675: gate
        /// `[6,0,0,0]` (tid3 6 = weapon), ladder `25,20,15,10|10,10,10,10|
        /// 10,5,5,5`.
        pub fn elixir_weapon_a() -> ItemDataRow {
            row(
                (3, 3, 10, 1),
                (100663296, [420744970, 168430090, 168101125]),
            )
        }

        /// `ITEM_ETC_ARCHEMY_REINFORCE_PROB_UP_A_01`, id 3683 (degree 1).
        pub fn powder_degree_1() -> ItemDataRow {
            row((3, 3, 10, 2), (1, [840832008, 134678022, 101057538]))
        }

        /// Equipment of `tid3` and `ItemClass` (col 61).
        pub fn equipment(tid3: u32, item_class: u32) -> ItemDataRow {
            let mut fields = vec![String::from("0"); 161];
            for (index, value) in [(9, 3), (10, 1), (11, tid3), (12, 2)] {
                fields[index] = value.to_string();
            }
            fields[61] = item_class.to_string();
            ItemDataRow(fields)
        }
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

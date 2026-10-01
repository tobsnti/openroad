//! Alchemy 4th-generation enchant window — `res_ui/nifenchantwnd.2dt`, root
//! `CNIFEnchantWnd` id 168 `(413,33,372,371)`.
//! Doc: `docs/re/ui/hud-enchant-window.md`. Issue #476; the classic half is #333
//! and stays where it is (`hud/alchemy/ui.rs`).
//!
//! Idea: the window's central fact is in its bytes, not in its art — **four tabs
//! over two panes**. Each `CNIFTabButton`'s `ContentId` is the `Id` of the pane
//! it raises, and the four tabs carry only two distinct values: Disjoint and
//! Dismantle both point at `CNIFAlchemySubWndType2` (id 28), Manufacture and
//! Strengthen both at `CNIFAlchemySubWndType1` (id 12). So this module renders
//! the descriptor's own tab->pane mapping rather than a hand-written table, and
//! the two panes' byte-identical rect (`409,107,376,300`) is the client's `if`,
//! not two overlapping windows.
//!
//! Like `hud/free_pvp.rs` this is a descriptor-driven window: the layout is
//! loaded from the `.2dt`, never transcribed. What is written down here is only
//! what the descriptor cannot say.
//!
//! Three traps this file exists to not fall into:
//!
//! * **The `_on`/`_off` art suffix is not state.** Three of the four tabs ship
//!   the `_on` art and only the fourth ships `_off`, which is authoring residue —
//!   exactly one tab can be active at runtime. The active tab comes from
//!   [`EnchantState::verb`], never from a filename.
//! * **`mframe_alc_` is not a working 9-slice.** Seven of its eight pieces are
//!   4x4 DXT1 stubs and the whole frame is one 376x376 bitmap packed into the
//!   `right_up` slot, so the plate is drawn as a single full-size image. A
//!   generic "every `mframe_*` is a 9-slice" helper would mangle this window.
//! * **`res_ui/nifenchantwnd.ddj` must stay ignored.** It is the same window one
//!   revision earlier (a 2DT with a texture extension), and its slot grid is the
//!   *broken* one the authors then fixed. `assets/ddj.rs` already logs-and-skips
//!   it with a regression test; we load the `.2dt` and only the `.2dt`.
//!
//! Deliberately not built yet, and why: the progress gauge (a `Style=0` gauge
//! crops along X instead of stretching, and four of our seven existing fill
//! sites already get that wrong — it deserves the shared crop helper, not a
//! fifth wrong one), the item slots' contents (no wire), and the shared
//! `CNIFMiniConfirm` dialog (`res_ui/nifenchantalchemymsgbox.2dt`, root id 172),
//! which the doc reads as shared and therefore not alchemy-local.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::twodt::{Jmxv2dtEntry, Jmxv2dtType, JMXV2DT};
use crate::assets::FontAssets;
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::DragGhost;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};
use crate::scenes::SceneState;

const DESCRIPTOR: &str = "media://res_ui/nifenchantwnd.2dt";
const ART_ROOT: &str = "media://interface/";
/// The whole window frame is this one 376x376 DXT1 bitmap; the other seven
/// `mframe_alc_` pieces are 4x4 stubs (doc §3).
const PLATE_ART: &str = "media://interface/frame/mframe_alc_right_up.ddj";
const CAPTION_FONT: f32 = 9.0;

/// `CNIFAlchemySubWndType1`'s `Id` — the pane Manufacture and Strengthen raise.
const PANE_TYPE1_ID: i32 = 12;
/// `CNIFAlchemySubWndType2`'s `Id` — the pane Disjoint and Dismantle raise.
const PANE_TYPE2_ID: i32 = 28;

/// The descriptor class name of each pane. **The pane lookup needs the class name
/// as well as the id**, because ids are unique only among siblings: record 16 of
/// this file is `id 28, parent 12, CIFSlotWithHelpEx` and record 26 is
/// `id 28, parent 5, CNIFAlchemySubWndType2`. A search by id alone finds the slot,
/// because it comes first.
///
/// Each pane is matched against **its own** class, not against the set of both:
/// "one of the two pane classes" would still accept a Type1 record carrying
/// Type2's id. That the two ids differ in this file makes the weaker rule work by
/// luck, and this window has already cost us one record that was right by
/// coincidence.
const PANE_CLASS_TYPE1: &str = "CNIFAlchemySubWndType1";
const PANE_CLASS_TYPE2: &str = "CNIFAlchemySubWndType2";

/// The class name every live slot in both panes carries. The 48x48
/// `alcm_slot_open` pictures are **not** slots: in both panes they form a
/// different grid from the live controls (pitch 78 against 48, and rows that do
/// not line up), while in the socket window of the same file each live 32x32 slot
/// sits centred in its 48x48 picture. So the pictures are stale authoring and are
/// not drawn; the live controls are the truth.
const SLOT_CONTROL: &str = "CIFSlotWithHelpEx";
/// The closed-slot picture, 32x32 in its DDS header — the exact slot extent.
const SLOT_ART: &str = "media://interface/alchemy/alcm_slot_closed.ddj";
/// The most slots any pane of this window declares as drawable: the one-to-many
/// pane's 4x2 grid. The ninth declared slot is outside the window and not drawn.
pub const PANE_SLOTS: usize = 8;

/// The two pane layouts the four verbs share.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlchemyPane {
    /// `CNIFAlchemySubWndType1` (id 12): one item plus its stones.
    ItemPlusStones,
    /// `CNIFAlchemySubWndType2` (id 28): one item in, many out.
    OneToMany,
}

impl AlchemyPane {
    /// The pane a tab's `ContentId` names. Anything else is a descriptor we do
    /// not understand, and is left unrendered rather than guessed at.
    pub fn from_content_id(content_id: i32) -> Option<Self> {
        match content_id {
            PANE_TYPE1_ID => Some(Self::ItemPlusStones),
            PANE_TYPE2_ID => Some(Self::OneToMany),
            _ => None,
        }
    }

    pub fn content_id(self) -> i32 {
        match self {
            Self::ItemPlusStones => PANE_TYPE1_ID,
            Self::OneToMany => PANE_TYPE2_ID,
        }
    }

    /// The descriptor class this pane is declared with.
    pub fn class_name(self) -> &'static str {
        match self {
            Self::ItemPlusStones => PANE_CLASS_TYPE1,
            Self::OneToMany => PANE_CLASS_TYPE2,
        }
    }
}

/// The four alchemy verbs, in the descriptor's left-to-right tab order. Local x
/// (the space this file lays out in) is **17 / 102 / 187 / 272**, each 80x24 at
/// local y=47 — pitch 85, a uniform 5-unit gap. The descriptor's own values are
/// absolute, 430/515/600/685; they are deliberately not quoted as the layout
/// numbers, because they are relative to a different origin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlchemyVerb {
    Disjoint,
    Dismantle,
    Manufacture,
    Strengthen,
}

impl AlchemyVerb {
    pub const ALL: [AlchemyVerb; 4] = [
        AlchemyVerb::Disjoint,
        AlchemyVerb::Dismantle,
        AlchemyVerb::Manufacture,
        AlchemyVerb::Strengthen,
    ];

    /// The tab caption key the descriptor carries in its `Text` field.
    pub fn string_key(self) -> &'static str {
        match self {
            Self::Disjoint => "UIIT_CTL_ALCHEMYBOX_TAP_DISJOINTING",
            Self::Dismantle => "UIIT_CTL_ALCHEMYBOX_TAP_DISMANTLING",
            Self::Manufacture => "UIIT_CTL_ALCHEMYBOX_TAP_MANUFACTURING",
            Self::Strengthen => "UIIT_CTL_ALCHEMYBOX_TAP_STRENGTHENING",
        }
    }

    pub fn from_string_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|verb| verb.string_key() == key)
    }

    /// The pane this verb works in, which is why four verbs need only two pane
    /// layouts.
    ///
    /// **The descriptor does not state which tab raises which pane** — it carries
    /// no link between the two. What settles it is the cell count each verb shows:
    /// Disjoint and Dismantle have **eight** cells in a 4x2 grid, Manufacture and
    /// Strengthening **five** (one set apart plus four in a row), and those are
    /// exactly the two panes' own counts. The mapping is therefore not a reading of
    /// `ContentId`, and saying so matters: a correct mapping with a wrong reason
    /// beside it would invite the next reader to trust the wrong one.
    pub fn pane(self) -> AlchemyPane {
        match self {
            Self::Disjoint | Self::Dismantle => AlchemyPane::OneToMany,
            Self::Manufacture | Self::Strengthen => AlchemyPane::ItemPlusStones,
        }
    }
}

#[derive(Resource)]
pub struct EnchantState {
    pub open: bool,
    /// The active tab. The `_on`/`_off` art suffixes say nothing about this.
    pub verb: AlchemyVerb,
    /// Inventory wire slots placed in the pane's cells, indexed in the pane's own
    /// reading order. A cell holds a *reference*: the window does not move items,
    /// and the bag only changes when the server says so.
    slots: [Option<u8>; PANE_SLOTS],
}

impl Default for EnchantState {
    fn default() -> Self {
        Self {
            open: false,
            // Leftmost tab; the descriptor authors no initial selection.
            verb: AlchemyVerb::Disjoint,
            slots: [None; PANE_SLOTS],
        }
    }
}

impl EnchantState {
    /// The inventory wire slot shown in cell `index`.
    pub fn slot(&self, index: usize) -> Option<u8> {
        self.slots.get(index).copied().flatten()
    }

    /// Put an inventory item into cell `index`. One item cannot sit in two cells,
    /// so placing it again moves it rather than duplicating it — the cells are
    /// references, and two references to one item would let the player act on it
    /// twice.
    pub fn place(&mut self, index: usize, inventory_slot: u8) {
        if index >= self.slots.len() {
            return;
        }
        for slot in self.slots.iter_mut() {
            if *slot == Some(inventory_slot) {
                *slot = None;
            }
        }
        self.slots[index] = Some(inventory_slot);
    }

    /// Empty cell `index`, returning what was in it.
    pub fn take(&mut self, index: usize) -> Option<u8> {
        self.slots.get_mut(index).and_then(Option::take)
    }

    /// Drop every placement. A tab switch does this: the two panes have different
    /// cell counts, so a reference to cell 7 means nothing on a five-cell pane.
    pub fn clear_slots(&mut self) {
        self.slots = [None; PANE_SLOTS];
    }
}

#[derive(Resource)]
struct EnchantDescriptor(Handle<JMXV2DT>);

#[derive(Component)]
pub struct EnchantWindow;

#[derive(Component, Clone, Copy)]
struct EnchantTab(AlchemyVerb);

/// A pane slot, indexed in the pane's own reading order (see [`pane_slots`]).
#[derive(Component)]
pub struct EnchantSlotCell {
    pub index: usize,
}

/// `frame\mframe_alc_right_up.ddj` -> a media-relative asset path.
fn art_path(background: &str) -> String {
    format!(
        "{ART_ROOT}{}",
        background.to_ascii_lowercase().replace('\\', "/")
    )
}

/// Anything laid out left to right is read **in coordinate order**, never in
/// record order — the descriptor corpus reorders records freely
/// (`docs/re/ui/frpvp-window.md` §3a, `targetmenu.2dt`).
fn order_by_x<T>(mut items: Vec<(f32, T)>) -> Vec<T> {
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    items.into_iter().map(|(_, item)| item).collect()
}

/// The pane entry for `pane`, found by **id and class name**. See
/// [`PANE_CLASS_TYPE1`] for why the id alone is not enough.
fn pane_entry(descriptor: &JMXV2DT, pane: AlchemyPane) -> Option<&Jmxv2dtEntry> {
    descriptor
        .entries()
        .iter()
        .find(|entry| pane_matches(entry.id() as i32, entry.name(), pane))
}

/// Whether a descriptor record **is** the pane: its id and its class must both
/// agree. Separated out so the trap can be pinned by a test without building a
/// descriptor.
fn pane_matches(entry_id: i32, entry_name: &str, pane: AlchemyPane) -> bool {
    entry_id == pane.content_id() && entry_name == pane.class_name()
}

/// The pane's live slot rects, in the order a player reads them.
///
/// Two rules, both of which change the result:
/// - **Reading order (y, then x), never record order.** This file stores the
///   upper row of the 4x2 grid as ids 66,67,68,**65** and the lower as
///   62,63,64,**69**; trusting the file order builds the cells transposed, and it
///   does not show while they are empty.
/// - **A slot whose local rect falls outside the window's content is not drawn.**
///   The one-to-many pane declares nine slots and the ninth sits at local
///   `(-26,145)`, left of the window; the original draws nothing there. The rule
///   is stated by position rather than by id, so it also holds if another
///   descriptor repeats the pattern.
fn pane_slots(descriptor: &JMXV2DT, pane_id: u32, content: Vec2) -> Vec<Rect> {
    drawable_in_reading_order(
        descriptor
            .children_of(pane_id)
            .filter(|entry| entry.name() == SLOT_CONTROL)
            .map(|entry| descriptor.local_rect(entry))
            .collect(),
        content,
    )
}

/// The two rules as one pure step, so both can be checked without building a
/// descriptor: drop what lies outside the content, then sort by y and then x.
fn drawable_in_reading_order(slots: Vec<Rect>, content: Vec2) -> Vec<Rect> {
    let mut slots: Vec<Rect> = slots
        .into_iter()
        .filter(|rect| {
            rect.min.x >= 0.0
                && rect.min.y >= 0.0
                && rect.max.x <= content.x
                && rect.max.y <= content.y
        })
        .collect();
    slots.sort_by(|a, b| {
        a.min
            .y
            .total_cmp(&b.min.y)
            .then(a.min.x.total_cmp(&b.min.x))
    });
    slots
}

fn load_descriptor(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(EnchantDescriptor(asset_server.load(DESCRIPTOR)));
}

/// The tabs the descriptor declares, left to right, with the pane each raises.
fn tabs(descriptor: &JMXV2DT) -> Vec<(AlchemyVerb, AlchemyPane, Rect, String)> {
    let found: Vec<(f32, (AlchemyVerb, AlchemyPane, Rect, String))> = descriptor
        .entries()
        .iter()
        .filter(|entry| entry.ni_type() == Some(Jmxv2dtType::CNIFTabButton))
        .filter_map(|entry| {
            let verb = AlchemyVerb::from_string_key(entry.text())?;
            let pane = AlchemyPane::from_content_id(entry.content_id())?;
            let rect = descriptor.local_rect(entry);
            Some((
                entry.rect().min.x,
                (verb, pane, rect, art_path(entry.background())),
            ))
        })
        .collect();
    order_by_x(found)
}

/// Rebuild the window from the descriptor whenever the state or the asset
/// changes. Without the descriptor there is no window: the layout is the data's
/// and we carry no transcribed fallback of it.
#[allow(clippy::too_many_arguments)]
fn rebuild_enchant_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    item_data: Res<ClientItemData>,
    descriptors: Res<Assets<JMXV2DT>>,
    handle: Option<Res<EnchantDescriptor>>,
    state: Res<EnchantState>,
    inventories: Query<&Inventory, With<Player>>,
    windows: Query<Entity, With<EnchantWindow>>,
    cameras: Query<Entity, With<Camera2d>>,
) {
    if !state.is_changed() && !descriptors.is_changed() {
        return;
    }
    for window in windows.iter() {
        commands.entity(window).despawn();
    }
    if !state.open {
        return;
    }
    let (Some(handle), Ok(camera)) = (handle, cameras.single()) else {
        return;
    };
    let Some(descriptor) = descriptors.get(&handle.0) else {
        return;
    };
    let Some(root) = descriptor.root() else {
        return;
    };
    let s = hud_scale();
    let size = root.rect().size();

    let node = |rect: Rect| Node {
        position_type: PositionType::Absolute,
        left: Val::Px(rect.min.x * s),
        top: Val::Px(rect.min.y * s),
        width: Val::Px(rect.width() * s),
        height: Val::Px(rect.height() * s),
        ..default()
    };
    let image = |path: &str| ImageNode {
        image: asset_server.load(path.to_string()),
        image_mode: NodeImageMode::Stretch,
        ..default()
    };

    let window = commands
        .spawn((
            EnchantWindow,
            Name::from("Enchant Window"),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(root.rect().min.x * s),
                top: Val::Px(root.rect().min.y * s),
                width: Val::Px(size.x * s),
                height: Val::Px(size.y * s),
                ..default()
            },
            GlobalZIndex(25),
            UiTargetCamera(camera),
        ))
        .id();

    // The frame: one plate, because seven of the eight `mframe_alc_` pieces are
    // 4x4 stubs and the eighth is the whole 376x376 bitmap.
    commands.entity(window).with_child((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        image(PLATE_ART),
        Pickable::IGNORE,
    ));

    for (verb, _pane, rect, art) in tabs(descriptor) {
        let caption = ui_strings.get_or(verb.string_key(), "").to_string();
        let tab = commands
            .spawn((EnchantTab(verb), Button, node(rect), image(&art)))
            .observe(
                |activate: On<Activate>,
                 tabs: Query<&EnchantTab>,
                 mut state: ResMut<EnchantState>| {
                    if let Ok(tab) = tabs.get(activate.entity) {
                        state.verb = tab.0;
                    }
                },
            )
            .id();
        commands.entity(tab).with_child((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Pickable::IGNORE,
            children![(
                Text::new(caption),
                TextFont {
                    font: fonts.nine.clone().into(),
                    font_size: FontSize::Px(CAPTION_FONT * s),
                    ..default()
                },
                TextColor(Color::WHITE),
            )],
        ));
        commands.entity(window).add_child(tab);
    }

    // The active pane, raised by the active tab's `ContentId`. Its siblings stay
    // unspawned rather than hidden — the two panes share a rect, so drawing both
    // would draw one on top of the other.
    let Some(pane) = pane_entry(descriptor, state.verb.pane()) else {
        return;
    };
    for entry in descriptor.children_of(pane.id()) {
        if entry.background().is_empty() || entry.name() == SLOT_CONTROL {
            continue;
        }
        commands.entity(window).with_child((
            node(descriptor.local_rect(entry)),
            image(&art_path(entry.background())),
            Pickable::IGNORE,
        ));
    }

    // The pane's live slots, drawn last so an item icon sits above the backdrop.
    // A filled cell shows the icon of the item it refers to: a cell that takes an
    // item but displays nothing leaves the player guessing what they placed.
    let inventory = inventories.single().ok();
    let icon_of = |cell: usize| -> Option<String> {
        let wire = state.slot(cell)?;
        let item = inventory?.get(wire)?;
        item_data
            .get(&(item.ref_id as i32))
            .and_then(|row| row.icon_path())
    };
    for (index, rect) in pane_slots(descriptor, pane.id(), size)
        .into_iter()
        .enumerate()
    {
        let cell = commands
            .spawn((
                EnchantSlotCell { index },
                Hovered::default(),
                node(rect),
                image(SLOT_ART),
            ))
            .observe(on_slot_press)
            .id();
        if let Some(icon) = icon_of(index) {
            commands.entity(cell).with_child((
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                image(&icon),
                Pickable::IGNORE,
            ));
        }
        commands.entity(window).add_child(cell);
    }
}

/// Press on a filled cell takes the item back out; the placement is only a
/// reference, so nothing is sent. A press while carrying belongs to
/// [`place_drop_on_enchant`], so a drop is not double-handled.
fn on_slot_press(
    press: On<Pointer<Press>>,
    cells: Query<&EnchantSlotCell>,
    inv_state: Res<InventoryState>,
    mut state: ResMut<EnchantState>,
) {
    if press.event.button != PointerButton::Primary || inv_state.drag.is_some() {
        return;
    }
    let Ok(cell) = cells.get(press.entity) else {
        return;
    };
    state.take(cell.index);
}

/// Dropping a carried inventory item on a cell places it there. The carry and its
/// ghost are consumed here, so no inventory move goes out for this drop.
pub fn place_drop_on_enchant(
    buttons: Res<ButtonInput<MouseButton>>,
    cells: Query<(&EnchantSlotCell, &Hovered)>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
    mut state: ResMut<EnchantState>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let Some(drag) = inv_state.drag else {
        return;
    };
    let Some((cell, _)) = cells.iter().find(|(_, hovered)| hovered.get()) else {
        return;
    };
    state.place(cell.index, drag);
    inv_state.drag = None;
    for ghost in ghosts.iter() {
        commands.entity(ghost).despawn();
    }
}

/// Whether a tab switch just happened, i.e. whether the cells must be released.
///
/// Its own function so the rule can be checked without a world. **The first
/// observation is not a switch**: on the frame the watcher first runs there is no
/// previous verb, and releasing there would be a release nobody asked for.
fn tab_switched(last: Option<AlchemyVerb>, now: AlchemyVerb) -> bool {
    matches!(last, Some(previous) if previous != now)
}

/// A tab switch empties the cells: the two panes have different cell counts, so a
/// reference to the eighth cell means nothing on a five-cell pane.
fn clear_slots_on_tab_change(
    mut state: ResMut<EnchantState>,
    mut last: Local<Option<AlchemyVerb>>,
) {
    if tab_switched(*last, state.verb) {
        state.clear_slots();
    }
    *last = Some(state.verb);
}

fn cleanup_enchant_window(
    mut commands: Commands,
    windows: Query<Entity, With<EnchantWindow>>,
    mut state: ResMut<EnchantState>,
) {
    for window in windows.iter() {
        commands.entity(window).despawn();
    }
    *state = EnchantState::default();
}

pub struct EnchantPlugin;

impl Plugin for EnchantPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EnchantState>()
            .add_systems(OnEnter(SceneState::GameWorld), load_descriptor)
            .add_systems(OnExit(SceneState::GameWorld), cleanup_enchant_window)
            .add_systems(
                Update,
                (
                    // The tab watcher runs before the rebuild, so a switch clears
                    // the cells in the same frame the new pane is drawn.
                    clear_slots_on_tab_change,
                    rebuild_enchant_window,
                    place_drop_on_enchant,
                )
                    .chain()
                    .run_if(in_state(SceneState::GameWorld)),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Four verbs, two panes. The split does **not** come out of the descriptor:
    /// Disjoint and Dismantle show eight cells, Manufacture and Strengthening
    /// five, and those are the two panes' own counts. `from_content_id` only names
    /// a pane; it does not decide which tab raises it.
    #[test]
    fn four_verbs_run_over_two_panes() {
        let panes: Vec<AlchemyPane> = AlchemyVerb::ALL.iter().map(|verb| verb.pane()).collect();
        assert_eq!(
            panes,
            vec![
                AlchemyPane::OneToMany,
                AlchemyPane::OneToMany,
                AlchemyPane::ItemPlusStones,
                AlchemyPane::ItemPlusStones,
            ]
        );
        assert_eq!(AlchemyVerb::Disjoint.pane().content_id(), 28);
        assert_eq!(AlchemyVerb::Manufacture.pane().content_id(), 12);
        // a pane can be named from its id, which is a different statement
        assert_eq!(
            AlchemyPane::from_content_id(12),
            Some(AlchemyPane::ItemPlusStones)
        );
        assert_eq!(
            AlchemyPane::from_content_id(28),
            Some(AlchemyPane::OneToMany)
        );
        // an unknown ContentId renders nothing rather than a guessed pane
        assert_eq!(AlchemyPane::from_content_id(0), None);
        assert_eq!(AlchemyPane::from_content_id(-1), None);
    }

    /// **The trap, pinned.** Ids in this descriptor are unique only among
    /// siblings: record 16 is `id 28, parent 12, CIFSlotWithHelpEx` and record 26
    /// is `id 28, parent 5, CNIFAlchemySubWndType2`. Record 16 comes first, so a
    /// search by id alone finds the **slot**. This test fails if the class check
    /// is ever dropped as a simplification.
    #[test]
    fn a_pane_lookup_by_id_alone_would_find_a_slot() {
        // the id both records share
        assert_eq!(AlchemyPane::OneToMany.content_id(), 28);
        // the slot is refused, the pane is accepted — same id, different class
        assert!(!pane_matches(28, SLOT_CONTROL, AlchemyPane::OneToMany));
        assert!(pane_matches(
            28,
            "CNIFAlchemySubWndType2",
            AlchemyPane::OneToMany
        ));
        // and the other pane is not accepted under the wrong id
        assert!(!pane_matches(
            28,
            "CNIFAlchemySubWndType1",
            AlchemyPane::OneToMany
        ));
        assert!(pane_matches(
            12,
            "CNIFAlchemySubWndType1",
            AlchemyPane::ItemPlusStones
        ));
        // each pane carries its own class, and neither of them is a slot
        assert_ne!(
            AlchemyPane::ItemPlusStones.class_name(),
            AlchemyPane::OneToMany.class_name()
        );
        for pane in [AlchemyPane::ItemPlusStones, AlchemyPane::OneToMany] {
            assert_ne!(pane.class_name(), SLOT_CONTROL);
        }
    }

    /// Reading order is y then x, never record order: the one-to-many pane stores
    /// its upper row as ids 66,67,68,**65** and its lower as 62,63,64,**69**.
    /// Trusting the file order builds the cells transposed, and that does not show
    /// while they are empty.
    #[test]
    fn the_cells_are_read_row_by_row_not_in_record_order() {
        let cell = |x: f32, y: f32| Rect::new(x, y, x + 32.0, y + 32.0);
        // the pane's eight drawable slots, in the descriptor's own record order
        let record_order = vec![
            cell(96.0, 247.0),  // 62
            cell(144.0, 247.0), // 63
            cell(192.0, 247.0), // 64
            cell(240.0, 199.0), // 65  <- upper row, stored last of its row
            cell(96.0, 199.0),  // 66
            cell(144.0, 199.0), // 67
            cell(192.0, 199.0), // 68
            cell(240.0, 247.0), // 69
        ];
        let ordered = drawable_in_reading_order(record_order, Vec2::new(372.0, 371.0));
        let xy: Vec<(f32, f32)> = ordered.iter().map(|r| (r.min.x, r.min.y)).collect();
        assert_eq!(
            xy,
            vec![
                (96.0, 199.0),
                (144.0, 199.0),
                (192.0, 199.0),
                (240.0, 199.0),
                (96.0, 247.0),
                (144.0, 247.0),
                (192.0, 247.0),
                (240.0, 247.0),
            ]
        );
        assert_eq!(ordered.len(), PANE_SLOTS);
    }

    /// A slot outside the window's content is not drawn. The one-to-many pane
    /// declares **nine**, and the ninth sits at local `(-26,145)` — left of the
    /// window, where the original draws nothing. The rule is by position, not by
    /// id, so it also holds if another descriptor repeats the pattern.
    #[test]
    fn a_slot_outside_the_content_is_not_drawn() {
        let cell = |x: f32, y: f32| Rect::new(x, y, x + 32.0, y + 32.0);
        let content = Vec2::new(372.0, 371.0);
        let declared = vec![cell(-26.0, 145.0), cell(96.0, 199.0), cell(144.0, 199.0)];
        let drawn = drawable_in_reading_order(declared, content);
        assert_eq!(drawn.len(), 2, "the negative-x slot is dropped");
        assert!(drawn.iter().all(|r| r.min.x >= 0.0));
        // every side is checked, not just the left one
        for outside in [
            cell(-1.0, 100.0),
            cell(100.0, -1.0),
            cell(content.x - 10.0, 100.0),
            cell(100.0, content.y - 10.0),
        ] {
            assert!(
                drawable_in_reading_order(vec![outside], content).is_empty(),
                "{outside:?} should not be drawn"
            );
        }
        // and the five-cell pane's own slots all sit inside
        let five = vec![
            cell(54.0, 222.0),
            cell(160.0, 223.0),
            cell(208.0, 223.0),
            cell(256.0, 223.0),
            cell(304.0, 223.0),
        ];
        assert_eq!(drawable_in_reading_order(five, content).len(), 5);
    }

    /// The tab-switch rule itself, which the system above only applies: a switch
    /// releases the cells, the **first** observation does not (there is no previous
    /// tab then, and clearing would be a release nobody asked for), and staying on
    /// the same tab does not either.
    #[test]
    fn only_a_real_tab_switch_releases_the_cells() {
        assert!(!tab_switched(None, AlchemyVerb::Disjoint), "first sight");
        assert!(
            !tab_switched(Some(AlchemyVerb::Disjoint), AlchemyVerb::Disjoint),
            "same tab"
        );
        assert!(tab_switched(
            Some(AlchemyVerb::Disjoint),
            AlchemyVerb::Dismantle
        ));
        // every pair of different verbs counts, not just neighbouring ones
        for from in AlchemyVerb::ALL {
            for to in AlchemyVerb::ALL {
                assert_eq!(
                    tab_switched(Some(from), to),
                    from != to,
                    "{from:?} -> {to:?}"
                );
            }
        }
    }

    /// Cells hold inventory references, one item in at most one cell, and a tab
    /// switch empties them — a reference to the eighth cell means nothing on a
    /// five-cell pane.
    #[test]
    fn the_cells_hold_references_and_a_tab_switch_clears_them() {
        let mut state = EnchantState::default();
        state.place(0, 13);
        state.place(3, 20);
        assert_eq!((state.slot(0), state.slot(3)), (Some(13), Some(20)));
        // the same bag slot cannot sit in two cells
        state.place(5, 13);
        assert_eq!((state.slot(0), state.slot(5)), (None, Some(13)));
        assert_eq!(state.take(5), Some(13));
        assert_eq!(state.slot(5), None);
        // out of range is ignored rather than panicking
        state.place(PANE_SLOTS, 7);
        assert_eq!(state.slot(PANE_SLOTS), None);
        // a tab switch releases every cell
        state.clear_slots();
        assert!((0..PANE_SLOTS).all(|i| state.slot(i).is_none()));
    }

    /// Tabs are identified by their caption key, and every verb has its own.
    #[test]
    fn each_verb_owns_one_caption_key() {
        let mut keys: Vec<&str> = AlchemyVerb::ALL.iter().map(|v| v.string_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 4);
        for verb in AlchemyVerb::ALL {
            assert_eq!(AlchemyVerb::from_string_key(verb.string_key()), Some(verb));
        }
        assert_eq!(
            AlchemyVerb::from_string_key("UIIT_CTL_ALCHEMYBOX_TAP"),
            None
        );
        assert_eq!(AlchemyVerb::from_string_key(""), None);
    }

    /// The tab strip is ordered by x (430/515/600/685), never by record order.
    #[test]
    fn the_tab_strip_is_ordered_by_x() {
        let shuffled = vec![
            (600.0, AlchemyVerb::Manufacture),
            (430.0, AlchemyVerb::Disjoint),
            (685.0, AlchemyVerb::Strengthen),
            (515.0, AlchemyVerb::Dismantle),
        ];
        assert_eq!(order_by_x(shuffled), AlchemyVerb::ALL.to_vec());
        // 80 wide at pitch 85 is a uniform 5 px gap
        let xs = [430.0_f32, 515.0, 600.0, 685.0];
        for pair in xs.windows(2) {
            assert_eq!(pair[1] - pair[0], 85.0);
            assert_eq!(pair[1] - (pair[0] + 80.0), 5.0);
        }
    }

    /// The active tab is state, never the `_on`/`_off` art suffix: three tabs
    /// ship `_on` and only the fourth ships `_off`, which is authoring residue.
    #[test]
    fn the_active_tab_comes_from_state() {
        let mut state = EnchantState::default();
        assert_eq!(state.verb, AlchemyVerb::Disjoint);
        assert_eq!(state.verb.pane(), AlchemyPane::OneToMany);
        state.verb = AlchemyVerb::Strengthen;
        assert_eq!(state.verb.pane(), AlchemyPane::ItemPlusStones);
    }

    #[test]
    fn art_paths_are_media_relative_and_forward_slashed() {
        assert_eq!(
            art_path("frame\\mframe_alc_right_up.ddj"),
            "media://interface/frame/mframe_alc_right_up.ddj"
        );
    }
}

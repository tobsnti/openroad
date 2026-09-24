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
//! fifth wrong one) and the shared `CNIFMiniConfirm` dialog
//! (`res_ui/nifenchantalchemymsgbox.2dt`, root id 172), which the doc reads as
//! shared and therefore not alchemy-local.
//!
//! **What the Dismantle tab now does, and why only that one.** Of the twenty
//! opcodes in the alchemy family this window can reach, only
//! `0x7157`/`0xB157` dismantle has a body we can build for *this* window
//! (`packets/src/agent/alchemy.rs`) — `0x7150` is modelled, but for the
//! classic box's builder, not this one ([`AlchemyVerb::unpublished_reason`]) —
//! so exactly one verb of the four can send. Which slot is which is the archive's own
//! statement, not a guess: `textuisystem.txt:4237`
//! `UIIT_STT_ALCHEMYBOX_TIP_DISMANTLING_MAIN_SLOT` reads *"Place Destroyer
//! Rondo"* and `:4238` `…_EQUIPMENT_SLOT` *"Place an equipment in slot"*, and
//! `:2171` `UIIT_MSG_ENCHANT_LOAD_RONDO2_FIRST` (*"Destroyer rondo must be
//! placed first."*) says which of the two comes first. That matches the
//! original's own builder byte for byte: it writes three bytes,
//! `{count, slot, slot}`, i.e. the count-prefixed slot list of the published
//! body with the Rondo leading. The
//! other three verbs press the same button and get a system line instead of a
//! packet (`on_action_button`).
//!
//! **Reachability — the opener is ours, and stated (ADR-0009).** No resinfo
//! tree raises this window: `CNIFEnchantWnd` is absent from the interface
//! registry entirely, while `CIFAlchemyBox` is in it, so which click shows the
//! four-tab window instead of the classic box in v1.188 is unknown. What the
//! shipped data *does* carry is a decomposition lamp pair
//! (`alcm_lamp_decomposition_{on,off}`) with no page body anywhere in
//! `resinfo/` to go with it — and the only declared dismantle body in the
//! whole archive is this window's pane 28. So the classic box's selector row
//! carries that shipped lamp as the third entry and it raises this window on
//! its Dismantle tab (`hud/alchemy/ui.rs`); the row's placement is ours, and
//! which generation the original shows is still open.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::alchemy::{AlchemyDismantleRequest, AlchemyDismantleResponse};
use packets::Packet;

use crate::assets::twodt::{Jmxv2dtEntry, Jmxv2dtType, JMXV2DT};
use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::inventory::model::InventoryState;
use crate::plugins::hud::inventory::ui::DragGhost;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::agent::AgentConnection;
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
/// The descriptor name every live slot in this file carries. The pane's slots
/// declare no art of their own, so they are drawn on the archive's own closed
/// plate (32x32 in its DDS header, the extent the descriptor gives them too).
const SLOT_CONTROL: &str = "CIFSlotWithHelpEx";
/// The two pane classes, by name — see [`pane_entry`] for why the id alone is
/// not enough.
const PANE_CLASSES: [&str; 2] = ["CNIFAlchemySubWndType1", "CNIFAlchemySubWndType2"];
const SLOT_ART: &str = "media://interface/alchemy/alcm_slot_closed.ddj";

/// `UIIT_MSG_ENCHANT_LOAD_RONDO2_FIRST` (`textuisystem.txt:2171`) — the
/// shipped line for "the main slot is empty", and what puts the Rondo in the
/// *first* slot of the request body.
const RONDO_FIRST: (&str, &str) = (
    "UIIT_MSG_ENCHANT_LOAD_RONDO2_FIRST",
    "Destroyer rondo must be placed first.",
);
/// `UIIT_MSG_ENCHANTERR_HAVE_NO_DISSOLVABLE_ITEM` (`:2179`) — nothing in the
/// target slots.
const NOTHING_TO_DISMANTLE: (&str, &str) = (
    "UIIT_MSG_ENCHANTERR_HAVE_NO_DISSOLVABLE_ITEM",
    "No item to disjoint.",
);
/// `UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY` (`:2149`) — the archive's own
/// "this cannot be done right now" for the alchemy family. It is the line the
/// three verbs without a published request body get; the sentence after it is
/// ours (see [`unpublished_reason`]).
const CANNOT_USE_ALCHEMY: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_CANNOT_USE_ALCHEMY",
    "You are not under the state to use alchemy.",
);
/// **Ours, stated deviation (ADR-0009).** `0xB157`'s refusal carries a `u16`
/// whose table (`AlchemyErrorCode`) is a dead page, and no `UIIT_*` key in the
/// user's `textuisystem.txt` enumerates a single value of it — so the code is
/// printed raw with a sentence of ours rather than bound to an invented
/// string. Same handling as the exchange refusals (`hud/exchange/model.rs`).
const DISMANTLE_REFUSED: &str = "The dismantle was refused.";

/// `CNIFAlchemySubWndType1`'s `Id` — the pane Manufacture and Strengthen raise.
const PANE_TYPE1_ID: i32 = 12;
/// `CNIFAlchemySubWndType2`'s `Id` — the pane Disjoint and Dismantle raise.
const PANE_TYPE2_ID: i32 = 28;

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
}

/// The four alchemy verbs, in the descriptor's left-to-right tab order
/// (x = 430, 515, 600, 685 — 80 wide at pitch 85, a uniform 5 px gap).
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

    /// The pane this verb works in — **read from the descriptor's `ContentId`**,
    /// which is why four verbs need only two pane layouts.
    pub fn pane(self) -> AlchemyPane {
        match self {
            Self::Disjoint | Self::Dismantle => AlchemyPane::OneToMany,
            Self::Manufacture | Self::Strengthen => AlchemyPane::ItemPlusStones,
        }
    }

    /// The action button's caption for this verb — one shipped key each
    /// (`textuisystem.txt:4247-4251`). Disjoint has *two* buttons in the
    /// original ("Disjoint selected slot" / "Disjoint all slots", `:4247-4248`)
    /// while the descriptor declares one, so the per-slot form is used; it is
    /// the one the single button can mean.
    pub fn action_key(self) -> &'static str {
        match self {
            Self::Disjoint => "UIIT_CTL_ALCHEMYBOX_BUTTON_DISJOINTING_SLOT",
            Self::Dismantle => "UIIT_CTL_ALCHEMYBOX_BUTTON_DISMANTLING",
            Self::Manufacture => "UIIT_CTL_ALCHEMYBOX_BUTTON_MANUFACTURING",
            Self::Strengthen => "UIIT_CTL_ALCHEMYBOX_BUTTON_STRENGTHENING",
        }
    }

    /// The English the archive ships for [`Self::action_key`], used when no
    /// string table is loaded.
    pub fn action_fallback(self) -> &'static str {
        match self {
            Self::Disjoint => "Disjoint selected slot",
            Self::Dismantle => "Dismantle",
            Self::Manufacture => "Manufacture",
            Self::Strengthen => "Strengthen",
        }
    }

    /// The request opcode this verb would send, and the field of its body that
    /// is not published — `None` for the one verb that can send.
    ///
    /// This is the whole reason three of four tabs refuse: `0x7155`
    /// (disjoin/manufacture) has **two** builders that disagree by four bytes,
    /// so its body has no fixed size; and `0x7150` (strengthen) *is* modelled
    /// (`packets::agent::alchemy::AlchemyReinforceRequest`) — but for the
    /// classic box. This window's builder is the other one, `@00825c50`, whose
    /// second byte is a sub-selector the classic body has no use for, and which
    /// value this tab writes is unmeasured.
    /// Guessing either is exactly the defect ADR-0009 exists to prevent.
    pub fn unpublished_reason(self) -> Option<&'static str> {
        match self {
            Self::Dismantle => None,
            Self::Disjoint | Self::Manufacture => Some(
                "0x7155 has two builders whose bodies differ by four bytes (an unnamed u32), \
                 so the request has no known size",
            ),
            Self::Strengthen => Some(
                "0x7150 is modelled for the classic box; this window's builder writes a \
                 sub-selector byte whose value per tab is unmeasured",
            ),
        }
    }
}

/// The `CIFSlotWithHelpEx` count of pane 28: the lone main slot plus the 4x2
/// grid.
pub const ONE_TO_MANY_SLOTS: usize = 9;
/// Index of the main slot in [`EnchantState::slots`] — the Rondo, and the one
/// the wire body writes first (module doc).
pub const MAIN_SLOT: usize = 0;

#[derive(Resource)]
pub struct EnchantState {
    pub open: bool,
    /// The active tab. The `_on`/`_off` art suffixes say nothing about this.
    pub verb: AlchemyVerb,
    /// Inventory wire slots placed in the pane's slots, in the descriptor's
    /// visual order (0 = the main/Rondo slot). Like the classic box these are
    /// *references* into the bag: nothing moves until the server says so.
    slots: [Option<u8>; ONE_TO_MANY_SLOTS],
}

impl Default for EnchantState {
    fn default() -> Self {
        Self {
            open: false,
            // Leftmost tab; the descriptor authors no initial selection.
            verb: AlchemyVerb::Disjoint,
            slots: [None; ONE_TO_MANY_SLOTS],
        }
    }
}

impl EnchantState {
    pub fn slot(&self, index: usize) -> Option<u8> {
        self.slots.get(index).copied().flatten()
    }

    /// Place an inventory slot in pane slot `index`. One bag slot can only sit
    /// in one pane slot, so placing it again moves it — two references to one
    /// item would ask the server to dismantle it twice in one request.
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

    pub fn take(&mut self, index: usize) -> Option<u8> {
        self.slots.get_mut(index).and_then(Option::take)
    }

    pub fn clear_slots(&mut self) {
        self.slots = [None; ONE_TO_MANY_SLOTS];
    }

    /// The dismantle body's slot list: the Rondo first, then every filled
    /// target in visual order. `None` when the pane cannot form a request —
    /// the caller turns that into the archive's own refusal line.
    pub fn dismantle_slots(&self) -> Option<Vec<u8>> {
        let rondo = self.slot(MAIN_SLOT)?;
        let targets: Vec<u8> = self.slots[MAIN_SLOT + 1..]
            .iter()
            .flatten()
            .copied()
            .collect();
        if targets.is_empty() {
            return None;
        }
        Some(std::iter::once(rondo).chain(targets).collect())
    }
}

#[derive(Resource)]
struct EnchantDescriptor(Handle<JMXV2DT>);

#[derive(Component)]
pub struct EnchantWindow;

#[derive(Component, Clone, Copy)]
struct EnchantTab(AlchemyVerb);

/// A pane slot, indexed in the descriptor's visual order (0 = the main slot).
#[derive(Component)]
pub struct EnchantSlotCell {
    index: usize,
}

/// The window's single action button (descriptor id 22).
#[derive(Component)]
struct EnchantActionButton;

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
    descriptors: Res<Assets<JMXV2DT>>,
    handle: Option<Res<EnchantDescriptor>>,
    state: Res<EnchantState>,
    windows: Query<Entity, With<EnchantWindow>>,
    cameras: Query<Entity, With<Camera2d>>,
    inventories: Query<&Inventory, With<Player>>,
    item_data: Res<ClientItemData>,
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
        if entry.background().is_empty() {
            continue;
        }
        commands.entity(window).with_child((
            node(descriptor.local_rect(entry)),
            image(&art_path(entry.background())),
            Pickable::IGNORE,
        ));
    }

    // The pane's own slots. The descriptor gives them no art (their
    // `Background` is empty), so they are drawn on the archive's closed-slot
    // plate — the same one the socket pane and the classic box use.
    if state.verb.pane() == AlchemyPane::OneToMany {
        let inventory = inventories.single().ok();
        for (index, rect) in pane_slots(descriptor, pane.id()).into_iter().enumerate() {
            let mut cell = commands.spawn((
                EnchantSlotCell { index },
                Hovered::default(),
                node(rect),
                image(SLOT_ART),
            ));
            cell.observe(on_slot_press);
            let icon = state
                .slot(index)
                .and_then(|wire| inventory?.get(wire))
                .and_then(|item| item_data.get(&(item.ref_id as i32)))
                .and_then(|row| row.icon_path());
            if let Some(icon) = icon {
                cell.with_children(|slot| {
                    slot.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        image(&icon),
                        Pickable::IGNORE,
                    ));
                });
            }
            let cell = cell.id();
            commands.entity(window).add_child(cell);
        }
    }

    // The action button. It belongs to the tab host, not to a pane — one
    // button serves all four verbs — and the descriptor declares exactly one
    // `CNIFButton` there (id 22, `545,359,104,28` on `alcm_button_01.ddj`,
    // whose art measures 104x28). Its caption is per-verb and shipped:
    // `UIIT_CTL_ALCHEMYBOX_BUTTON_*`, `textuisystem.txt:4247-4251`.
    if let Some(button) = action_button(descriptor, pane.parent_id()) {
        let caption = ui_strings
            .get_or(state.verb.action_key(), state.verb.action_fallback())
            .to_string();
        let entity = commands
            .spawn((
                EnchantActionButton,
                Button,
                node(descriptor.local_rect(button)),
                image(&art_path(button.background())),
            ))
            .observe(on_action_button)
            .id();
        commands.entity(entity).with_child((
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
        commands.entity(window).add_child(entity);
    }
}

/// The pane a `ContentId` names — matched on **id *and* class name**.
///
/// `Id` is not unique in this file: id 28 is both `CNIFAlchemySubWndType2` and
/// a `CIFSlotWithHelpEx` under the *other* pane, and ids 13/18/20/21/70 repeat
/// as well. A plain id lookup therefore returns the slot for one of the two
/// panes — with a wrong `parent_id`, which is how the window would lose its
/// action button. Both pane classes are named, so the name is the tiebreaker.
fn pane_entry(descriptor: &JMXV2DT, pane: AlchemyPane) -> Option<&Jmxv2dtEntry> {
    let id = pane.content_id();
    descriptor
        .entries()
        .iter()
        .find(|entry| entry.id() as i32 == id && PANE_CLASSES.contains(&entry.name()))
}

/// The pane's `CIFSlotWithHelpEx` entries **in coordinate order** (y, then x),
/// never in record order: the descriptor stores this very grid out of visual
/// order (ids 66,67,68,65 on the top row), and the lone main slot
/// at `(387,178)` sorts first because it is above both rows. That ordering is
/// what makes index 0 the Rondo slot without a hardcoded id table.
fn pane_slots(descriptor: &JMXV2DT, pane_id: u32) -> Vec<Rect> {
    in_reading_order(
        descriptor
            .children_of(pane_id)
            .filter(|entry| entry.name() == SLOT_CONTROL)
            .map(|entry| descriptor.local_rect(entry))
            .collect(),
    )
}

/// Row by row, left to right — the order a player reads the pane in, and the
/// order the request body's slot list is built in.
fn in_reading_order(mut slots: Vec<Rect>) -> Vec<Rect> {
    slots.sort_by(|a, b| {
        a.min
            .y
            .total_cmp(&b.min.y)
            .then(a.min.x.total_cmp(&b.min.x))
    });
    slots
}

/// The one `CNIFButton` the tab host owns — the window's action button. The
/// side-menu buttons (`alcm_menu_button1/2`) hang off the *root*, not off the
/// host, which is what keeps this predicate to a single hit.
fn action_button(descriptor: &JMXV2DT, host_id: u32) -> Option<&Jmxv2dtEntry> {
    descriptor
        .children_of(host_id)
        .find(|entry| entry.ni_type() == Some(Jmxv2dtType::CNIFButton))
}

/// Press on a filled slot takes the item back out; the placement is only a
/// reference, so nothing is sent. A press while carrying belongs to
/// [`place_drop_on_enchant`], so a drop is not double-handled (the classic
/// box's precedent, `hud/alchemy/ui.rs`).
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

/// Dropping a carried inventory item on a pane slot places it there. The carry
/// and its ghost are consumed here, so no 0x7034 move goes out for this drop.
pub fn place_drop_on_enchant(
    buttons: Res<ButtonInput<MouseButton>>,
    cells: Query<(&EnchantSlotCell, &Hovered)>,
    ghosts: Query<Entity, With<DragGhost>>,
    mut inv_state: ResMut<InventoryState>,
    mut state: ResMut<EnchantState>,
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

/// The action button. Exactly one of the four verbs has a published request
/// body, so exactly one of them sends; the other three say *why* nothing
/// happens instead of doing nothing (the silent-refusal defect this window was
/// audited for). The refusal lines are the archive's own; the reason sentence
/// after them is ours and marked as such.
fn on_action_button(
    _: On<Activate>,
    state: Res<EnchantState>,
    ui_strings: Res<ClientUiStrings>,
    mut history: ResMut<ChatHistory>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
) {
    let verb = state.verb;
    if let Some(reason) = verb.unpublished_reason() {
        let (key, fallback) = CANNOT_USE_ALCHEMY;
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        history.push(ChatLine::system(format!(
            "{}: {reason}.",
            ui_strings.get_or(verb.action_key(), verb.action_fallback())
        )));
        info!("alchemy: {verb:?} has no published request body — {reason}");
        return;
    }
    let Some(slots) = state.dismantle_slots() else {
        let (key, fallback) = if state.slot(MAIN_SLOT).is_none() {
            RONDO_FIRST
        } else {
            NOTHING_TO_DISMANTLE
        };
        history.push(ChatLine::system(ui_strings.get_or(key, fallback)));
        return;
    };
    info!("alchemy: dismantling slots {slots:?} (0x7157)");
    // The placements stay until the ack: a refusal must leave the pane as the
    // player left it, and only 0xB157 says which of the two happened.
    send(&conn, Packet::from(AlchemyDismantleRequest::new(slots)));
}

fn send(conn: &Query<&SilkroadConnection, With<AgentConnection>>, packet: Packet) {
    let Ok(conn) = conn.single() else {
        warn!("alchemy: no agent connection, dismantle not sent");
        return;
    };
    if let Err(e) = conn.get_sender().send(packet.into()) {
        error!("alchemy: failed to send dismantle: {}", e.0);
    }
}

/// 0xB157 — the ack. Success empties the pane (the items it referenced are the
/// server's now); a refusal leaves it alone and prints the raw code, because
/// the code table is a dead page (see [`DISMANTLE_REFUSED`]).
///
/// The inventory itself is **not** touched here: the server pushes the new
/// slot state on its own opcodes, and predicting it is how the item-use path
/// once desynced (`hud/underbar/cast.rs`).
pub fn apply_dismantle_response(
    mut reader: MessageReader<AlchemyDismantleResponse>,
    mut state: ResMut<EnchantState>,
    mut history: ResMut<ChatHistory>,
) {
    for response in reader.read() {
        if response.is_success() {
            info!(
                "alchemy: dismantle accepted (0xB157 result {})",
                response.result
            );
            state.clear_slots();
            continue;
        }
        let code = response.error_code.unwrap_or_default();
        warn!("alchemy: dismantle refused (0xB157 error {code:#06x})");
        history.push(ChatLine::system(format!(
            "{DISMANTLE_REFUSED} (code {code:#06x})"
        )));
    }
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
                    rebuild_enchant_window,
                    place_drop_on_enchant,
                    apply_dismantle_response,
                )
                    .run_if(in_state(SceneState::GameWorld)),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The window's central finding: four verbs, two panes, and the split is the
    /// descriptor's `ContentId` — 28 for Disjoint/Dismantle, 12 for
    /// Manufacture/Strengthen.
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
        // and the mapping is read back out of a ContentId, not hardcoded per tab
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

    /// The pane's slots are ordered by coordinate, not by record order — with
    /// the descriptor's **own** numbers: `nifenchantwnd.2dt` stores the grid as
    /// ids 66,67,68,65 / 62,63,64,69, and the lone main slot sits at
    /// `(387,178)`, above both rows. Reading order therefore puts the main slot
    /// first, which is what makes [`MAIN_SLOT`] the Rondo without an id table.
    #[test]
    fn the_main_slot_sorts_before_the_grid() {
        let rect =
            |x: f32, y: f32| Rect::from_corners(Vec2::new(x, y), Vec2::new(x + 32.0, y + 32.0));
        // in the descriptor's record order, which is deliberately not visual
        let stored = vec![
            rect(509.0, 280.0), // id 62
            rect(557.0, 280.0), // 63
            rect(605.0, 280.0), // 64
            rect(653.0, 232.0), // 65
            rect(509.0, 232.0), // 66
            rect(557.0, 232.0), // 67
            rect(605.0, 232.0), // 68
            rect(653.0, 280.0), // 69
            rect(387.0, 178.0), // 70 — the main slot, stored last
        ];
        let ordered = in_reading_order(stored);
        assert_eq!(ordered.len(), ONE_TO_MANY_SLOTS);
        assert_eq!(ordered[MAIN_SLOT].min, Vec2::new(387.0, 178.0));
        // then the top row left to right, then the bottom row
        let xs: Vec<f32> = ordered[1..5].iter().map(|r| r.min.x).collect();
        assert_eq!(xs, vec![509.0, 557.0, 605.0, 653.0]);
        assert!(ordered[1..5].iter().all(|r| r.min.y == 232.0));
        assert!(ordered[5..].iter().all(|r| r.min.y == 280.0));
    }

    /// `Id` is not unique in `nifenchantwnd.2dt`: **id 28 is both the
    /// `CNIFAlchemySubWndType2` pane and a `CIFSlotWithHelpEx` under the other
    /// pane** (ids 13/18/20/21/70 repeat as well). Looking the pane up by id
    /// alone returns that slot, whose `parent_id` is 12 rather than the tab
    /// host — and the action button hangs off the host, so the window would
    /// silently lose its only button on exactly one of the two panes. The
    /// class name is the tiebreaker, and it is not a slot's.
    #[test]
    fn the_pane_lookup_needs_more_than_the_id() {
        assert_eq!(AlchemyPane::OneToMany.content_id(), 28);
        assert!(PANE_CLASSES.contains(&"CNIFAlchemySubWndType2"));
        assert!(PANE_CLASSES.contains(&"CNIFAlchemySubWndType1"));
        assert!(!PANE_CLASSES.contains(&SLOT_CONTROL));
    }

    /// The wire body is the Rondo first, then the targets — that order is the
    /// archive's own (`UIIT_MSG_ENCHANT_LOAD_RONDO2_FIRST`, *"Destroyer rondo
    /// must be placed first."*) and it matches the three bytes the builder at
    /// the original writes.
    #[test]
    fn the_dismantle_body_leads_with_the_rondo() {
        let mut state = EnchantState::default();
        assert_eq!(state.dismantle_slots(), None, "an empty pane sends nothing");
        state.place(3, 20);
        assert_eq!(
            state.dismantle_slots(),
            None,
            "a target without a Rondo sends nothing"
        );
        state.place(MAIN_SLOT, 14);
        assert_eq!(state.dismantle_slots(), Some(vec![14, 20]));
        // the published body is count-prefixed, so more than one target is a
        // legal request and the count follows the list
        state.place(1, 15);
        let slots = state.dismantle_slots().unwrap();
        assert_eq!(slots, vec![14, 15, 20]);
        let request = AlchemyDismantleRequest::new(slots);
        assert_eq!(request.slot_count, 3);
        // a Rondo alone is not a request either
        let mut only_rondo = EnchantState::default();
        only_rondo.place(MAIN_SLOT, 14);
        assert_eq!(only_rondo.dismantle_slots(), None);
    }

    /// Placements are references into the bag, so one bag slot cannot occupy
    /// two pane slots — that would ask the server to dismantle it twice in one
    /// request.
    #[test]
    fn a_bag_slot_sits_in_at_most_one_pane_slot() {
        let mut state = EnchantState::default();
        state.place(1, 20);
        state.place(4, 20);
        assert_eq!(state.slot(1), None);
        assert_eq!(state.slot(4), Some(20));
        assert_eq!(state.take(4), Some(20));
        assert_eq!(state.take(4), None);
        // out of range is ignored rather than panicking (the cells feed this)
        state.place(ONE_TO_MANY_SLOTS + 1, 7);
        assert_eq!(state.slot(ONE_TO_MANY_SLOTS + 1), None);
    }

    /// Exactly one of the four verbs may send, and the other three name the
    /// field that stops them. This is the test that fails the day someone
    /// wires a guessed body.
    #[test]
    fn only_dismantle_has_a_published_request_body() {
        let sendable: Vec<AlchemyVerb> = AlchemyVerb::ALL
            .into_iter()
            .filter(|verb| verb.unpublished_reason().is_none())
            .collect();
        assert_eq!(sendable, vec![AlchemyVerb::Dismantle]);
        for verb in AlchemyVerb::ALL {
            if let Some(reason) = verb.unpublished_reason() {
                assert!(
                    reason.contains("0x715") || reason.contains("0x7150"),
                    "{verb:?}"
                );
            }
        }
        // one shipped caption key per verb, none shared
        let mut keys: Vec<&str> = AlchemyVerb::ALL.iter().map(|v| v.action_key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 4);
        assert_eq!(
            AlchemyVerb::Dismantle.action_key(),
            "UIIT_CTL_ALCHEMYBOX_BUTTON_DISMANTLING"
        );
    }

    /// The ack decides what the window shows. No server in reach has ever
    /// answered a dismantle, so the two acks here are built from the published
    /// byte layout (`{u8 result, if result == 2 u16 errorCode}`), exactly like
    /// `packets/src/agent/alchemy.rs`'s own tests.
    #[test]
    fn the_ack_empties_the_pane_only_when_it_accepted() {
        use bytes::Bytes;

        let app_with = |bytes: &'static [u8]| {
            let mut app = App::new();
            app.add_message::<AlchemyDismantleResponse>()
                .init_resource::<ChatHistory>()
                .add_systems(Update, apply_dismantle_response);
            let mut state = EnchantState::default();
            state.place(MAIN_SLOT, 14);
            state.place(1, 20);
            app.insert_resource(state);
            app.world_mut().write_message(
                AlchemyDismantleResponse::try_from(Bytes::from_static(bytes)).unwrap(),
            );
            app.update();
            app
        };

        // result 1 — accepted: the placements are released, nothing is said
        let accepted = app_with(&[0x01]);
        let state = accepted.world().resource::<EnchantState>();
        assert_eq!(state.slot(MAIN_SLOT), None);
        assert_eq!(state.slot(1), None);
        assert_eq!(accepted.world().resource::<ChatHistory>().iter().count(), 0);

        // result 2 — refused: the pane stays as the player left it and the
        // raw code is shown, because its table is a dead page
        let refused = app_with(&[0x02, 0x0E, 0x1C]);
        let state = refused.world().resource::<EnchantState>();
        assert_eq!(state.slot(MAIN_SLOT), Some(14));
        assert_eq!(state.slot(1), Some(20));
        let lines: Vec<String> = refused
            .world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| line.display())
            .collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("0x1c0e"), "{lines:?}");
    }
}

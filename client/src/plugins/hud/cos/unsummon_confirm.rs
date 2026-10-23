//! "Destroy the loaded transport?" — the confirm in front of an unsummon that
//! would drop cargo on the ground.
//!
//! Idea: unsummoning a transport that carries goods is destructive and
//! irreversible, and the original says so in its own words first. Our command
//! bar sent `CosCommand::Unsummon` straight from the click
//! (`hud/cos_command.rs`), so a mis-click emptied a full cart onto the floor.
//! This module is the gate: with cargo aboard the cell opens this dialog and
//! only its Yes writes the command. An **empty** COS is unaffected — vanilla
//! asks nothing there either, and a dialog in front of every dismissal would be
//! our invention rather than the original's.
//!
//! Every value's origin. The sentences are the player's own rows of
//! `textdata/textuisystem.txt`, cited with their line:
//! `UIIT_MSG_COS_CLEAN_CONFIRM1` **L531**, `UIIT_MSG_COS_CLEAN_CONFIRM2`
//! **L532**, `UIIT_CTL_YES` **L434**, `UIIT_CTL_NO` **L435**. The geometry is
//! the original's generic yes/no box `MsgBoxSimple`
//! (`resinfo/ifmessagebox.txt:27`): message `0,52,240,14`, Yes `72,99,76,24`,
//! No `152,99,76,24` on `com_button.ddj`, FontColor `255,254,251,216`, over
//! `GDR_MSGBOX_BG` `16,40,284,122`. The plate's outer size is **derived**: the
//! box is code-created, so no rect states it, and the background sits exactly
//! at the `msgbox2_window_` insets that `modal_dialog::modal_interior` inverts —
//! hence a test instead of a literal.
//!
//! Chrome, plate and scrim come from [`crate::plugins::hud::modal_dialog`]
//! rather than being laid out here: that helper **tiles** the two side pieces
//! vertically, which is measured, and a hand-rolled ring that stretches them
//! smears a motif the artist authored.
//!
//! Two stated deviations (ADR-0009): the authored box is **one** 14 px line and
//! this dialog shows **two** shipped rows, so it keeps the authored x/y/width
//! and grows downward rather than inventing a second rect; and `MsgBoxSimple`
//! states no `FontIndex`, so the size is ladder slot **0** (`font_index_px(0)`
//! = 12 px), the index 3547 of 3739 authored controls use.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::LineHeight;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::FontAssets;
use crate::plugins::cos::{ActiveCosList, CosCommand};
use crate::plugins::hud::cos::state::Cos;
use crate::plugins::hud::modal_dialog::{
    modal_plate_node, modal_scrim_node, spawn_modal_frame, MODAL_BOTTOM, MODAL_SIDE,
};
use crate::plugins::hud::scale::{hud_scale, text_px};
use crate::plugins::textdata::ClientUiStrings;

/// `GDR_MSGBOX_BG:CIFNormalTile` `16,40,284,122` (`ifmessagebox.txt` `Create`).
const BG_RECT: (f32, f32, f32, f32) = (16.0, 40.0, 284.0, 122.0);
/// Plate size derived from that background plus the `msgbox2_window_` insets —
/// see [`plate_is_the_authored_background_plus_the_measured_insets`].
const PLATE: (f32, f32) = (
    BG_RECT.2 + 2.0 * MODAL_SIDE,
    BG_RECT.1 + BG_RECT.3 + MODAL_BOTTOM,
);
/// `GDR_SIMPLE_TEXTBOX_MESSAGE` `0,52,240,14`, `HAlign=1`. The height is ours:
/// two lines instead of one (see the module note).
const MESSAGE_RECT: (f32, f32, f32, f32) = (0.0, 52.0, 240.0, 14.0);
const MESSAGE_LINES: f32 = 2.0;
/// `GDR_SIMPLE_BTN_OK` `72,99,76,24` and `GDR_SIMPLE_BTN_CANCEL` `152,99,76,24`.
const YES_RECT: (f32, f32, f32, f32) = (72.0, 99.0, 76.0, 24.0);
const NO_RECT: (f32, f32, f32, f32) = (152.0, 99.0, 76.0, 24.0);
/// `FontColor=COLOR,"255,254,251,216"` on both buttons.
const BUTTON_TEXT: Color = Color::srgb_u8(254, 251, 216);

const ART: &str = "media://interface/";

/// `(key, shipped English)`. The fallback is what the key carries in the
/// player's own `textuisystem.txt`, quoted with its line in the module doc, so
/// a wrong transcription shows against the file instead of hiding in a literal.
/// A loaded [`ClientUiStrings`] always wins.
type UiText = (&'static str, &'static str);

const CONFIRM_LINE_1: UiText = (
    "UIIT_MSG_COS_CLEAN_CONFIRM1",
    "If transport is destroyed, all goods will be dropped on the ground.",
);
const CONFIRM_LINE_2: UiText = (
    "UIIT_MSG_COS_CLEAN_CONFIRM2",
    "Will you destroy the selected transport?",
);
const YES_LABEL: UiText = ("UIIT_CTL_YES", "Yes");
const NO_LABEL: UiText = ("UIIT_CTL_NO", "No");

/// The pending destructive unsummon, or `None` when no dialog is up.
#[derive(Resource, Default)]
pub struct CosUnsummonConfirm {
    /// COS unique id the Yes button will dismiss.
    pub pending: Option<u32>,
}

#[derive(Component)]
pub struct CosUnsummonDialog;

/// Whether this COS has anything aboard — the one condition that puts a dialog
/// in front of the dismissal. The wire sends the bag as a list of slot-tagged
/// records, so this is "the list is not empty" and needs no capacity scan.
pub fn carries_cargo(cos: &Cos) -> bool {
    !cos.body.items.is_empty()
}

/// An absolutely positioned node from a plate-local rect.
fn plate_node((x, y, w, h): (f32, f32, f32, f32), s: f32) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x * s),
        top: Val::Px(y * s),
        width: Val::Px(w * s),
        height: Val::Px(h * s),
        ..default()
    }
}

/// Spawn/despawn the dialog to match [`CosUnsummonConfirm`].
pub fn sync_cos_unsummon_confirm(
    confirm: Res<CosUnsummonConfirm>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    cameras: Query<Entity, With<Camera2d>>,
    open: Query<Entity, With<CosUnsummonDialog>>,
    mut commands: Commands,
) {
    if !confirm.is_changed() {
        return;
    }
    for entity in open.iter() {
        commands.entity(entity).despawn();
    }
    if confirm.pending.is_none() {
        return;
    }
    let Some(camera) = cameras.iter().next() else {
        return;
    };

    let s = hud_scale();
    // One expression for both: a line height from a different number than the
    // em is the rounding bug one layer up.
    let px = text_px(font_index_px(0), s);
    let text_font = TextFont {
        font: fonts.nine.clone().into(),
        font_size: FontSize::Px(px),
        ..default()
    };
    // Its own component in bevy 0.20, not a `TextFont` field.
    let line_height = LineHeight::Px(px);

    commands
        .spawn((
            CosUnsummonDialog,
            Name::from("COS Unsummon Confirm"),
            UiTargetCamera(camera),
            GlobalZIndex(90),
            modal_scrim_node(),
        ))
        .with_children(|scrim| {
            scrim
                .spawn(modal_plate_node(PLATE.0, PLATE.1, s))
                .with_children(|plate| {
                    spawn_modal_frame(plate, &asset_server, PLATE.0, PLATE.1, s);
                    plate.spawn((
                        plate_node(BG_RECT, s),
                        ImageNode {
                            image: asset_server
                                .load(format!("{ART}ifcommon/bg_tile/com_bg_tile_b.ddj")),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));

                    // Both shipped rows, centred in the authored text box.
                    let message = format!(
                        "{}\n{}",
                        ui_strings.get_or(CONFIRM_LINE_1.0, CONFIRM_LINE_1.1),
                        ui_strings.get_or(CONFIRM_LINE_2.0, CONFIRM_LINE_2.1),
                    );
                    let (mx, my, mw, mh) = MESSAGE_RECT;
                    plate.spawn((
                        Text::new(message),
                        text_font.clone(),
                        line_height,
                        TextColor(Color::WHITE),
                        TextLayout::justify(Justify::Center),
                        plate_node((mx, my, mw, mh * MESSAGE_LINES), s),
                        Pickable::IGNORE,
                    ));

                    for (rect, label, yes) in
                        [(YES_RECT, YES_LABEL, true), (NO_RECT, NO_LABEL, false)]
                    {
                        let mut button = plate.spawn((
                            Button,
                            Hovered::default(),
                            plate_node(rect, s),
                            ImageNode {
                                image: asset_server.load(format!("{ART}ifcommon/com_button.ddj")),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                        ));
                        if yes {
                            button.observe(on_yes);
                        } else {
                            button.observe(on_no);
                        }
                        button.with_children(|button| {
                            button.spawn((
                                Text::new(ui_strings.get_or(label.0, label.1).to_string()),
                                text_font.clone(),
                                line_height,
                                TextColor(BUTTON_TEXT),
                                TextLayout::justify(Justify::Center),
                                Node {
                                    width: Val::Percent(100.0),
                                    align_self: AlignSelf::Center,
                                    ..default()
                                },
                                Pickable::IGNORE,
                            ));
                        });
                    }
                });
        });
}

/// Yes: the only place the destructive command is written.
fn on_yes(
    _: On<Activate>,
    mut confirm: ResMut<CosUnsummonConfirm>,
    mut commands_out: MessageWriter<CosCommand>,
) {
    if let Some(uid) = confirm.pending.take() {
        info!("cos: unsummon of loaded transport {uid} confirmed");
        commands_out.write(CosCommand::Unsummon(uid));
    }
}

fn on_no(_: On<Activate>, mut confirm: ResMut<CosUnsummonConfirm>) {
    confirm.pending = None;
}

/// A summon that vanished on its own (server unsummon, zone change) takes its
/// pending dialog with it — otherwise Yes would name a COS that is gone.
pub fn drop_stale_unsummon_confirm(
    list: Res<ActiveCosList>,
    mut confirm: ResMut<CosUnsummonConfirm>,
) {
    if confirm.pending.is_some_and(|uid| list.get(uid).is_none()) {
        confirm.pending = None;
    }
}

pub fn cleanup_cos_unsummon_confirm(
    mut commands: Commands,
    dialogs: Query<Entity, With<CosUnsummonDialog>>,
    mut confirm: ResMut<CosUnsummonConfirm>,
) {
    for dialog in dialogs.iter() {
        commands.entity(dialog).despawn();
    }
    confirm.pending = None;
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::plugins::cos::CosStatus;
    use crate::plugins::hud::modal_dialog::modal_interior;
    use packets::agent::character_data::{InventoryItem, ItemTypeData, RentInfo};
    use packets::agent::pet::{CosBody, CosKind};

    /// The plate is derived from the authored background and the measured
    /// `msgbox2_window_` insets — if either moves, this fails instead of the
    /// dialog silently drifting.
    #[test]
    fn plate_is_the_authored_background_plus_the_measured_insets() {
        assert_eq!(PLATE, (316.0, 178.0));
        assert_eq!(modal_interior(PLATE.0, PLATE.1), BG_RECT);
    }

    /// Every authored control stays inside the plate, the two buttons are the
    /// art's own 76x24, and they do not overlap.
    #[test]
    fn the_authored_controls_fit_the_plate_and_the_buttons_are_art_sized() {
        for (x, y, w, h) in [
            BG_RECT,
            (
                MESSAGE_RECT.0,
                MESSAGE_RECT.1,
                MESSAGE_RECT.2,
                MESSAGE_RECT.3 * MESSAGE_LINES,
            ),
            YES_RECT,
            NO_RECT,
        ] {
            assert!(x + w <= PLATE.0, "{x}+{w} wider than the plate");
            assert!(y + h <= PLATE.1, "{y}+{h} taller than the plate");
        }
        assert_eq!((YES_RECT.2, YES_RECT.3), (76.0, 24.0));
        assert_eq!((NO_RECT.2, NO_RECT.3), (76.0, 24.0));
        assert!(YES_RECT.0 + YES_RECT.2 <= NO_RECT.0);
        // the message sits above both buttons rather than into them
        assert!(MESSAGE_RECT.1 + MESSAGE_RECT.3 * MESSAGE_LINES <= YES_RECT.1);
    }

    /// `CosBody` has no `Default`, so every field is named — same shape as
    /// `state.rs`'s own `pick_pet` fixture.
    fn transport_with(items: Vec<InventoryItem>) -> Cos {
        Cos {
            unique_id: 7,
            ref_obj_id: 1,
            kind: CosKind::Transport,
            body: CosBody {
                hp: 1,
                unk_b: 0,
                growth: None,
                unk_f: None,
                name: None,
                inventory_size: 8,
                items,
                unk_g: None,
                unk_h: None,
            },
            hgp: None,
            exp: 0,
            level: None,
        }
    }

    fn cargo(slot: u8) -> InventoryItem {
        InventoryItem {
            slot,
            rent: RentInfo::default(),
            ref_id: 1,
            data: ItemTypeData::Expendable {
                stack_count: 1,
                assimilation_prob: None,
                mag_params: Vec::new(),
            },
        }
    }

    /// The gate condition, which is the whole point: a loaded transport asks,
    /// an empty one must not — vanilla asks nothing there, and a dialog in
    /// front of every dismissal would be ours.
    #[test]
    fn only_a_loaded_transport_asks() {
        assert!(!carries_cargo(&transport_with(Vec::new())));
        assert!(carries_cargo(&transport_with(vec![cargo(3)])));
    }

    /// A COS that vanished while the dialog was up takes it along, or Yes
    /// would destroy a transport that is already gone.
    #[test]
    fn a_vanished_summon_drops_its_pending_dialog() {
        let mut app = App::new();
        app.init_resource::<CosUnsummonConfirm>()
            .init_resource::<ActiveCosList>()
            .add_systems(Update, drop_stale_unsummon_confirm);
        app.world_mut().resource_mut::<CosUnsummonConfirm>().pending = Some(7);

        // still summoned: the dialog stays
        app.world_mut()
            .resource_mut::<ActiveCosList>()
            .0
            .push(CosStatus {
                unique_id: 7,
                ref_id: 1,
                kind: CosKind::Transport,
                name: None,
                hp: 1,
                hp_max: 1,
                local_only: true,
            });
        app.update();
        assert_eq!(
            app.world().resource::<CosUnsummonConfirm>().pending,
            Some(7)
        );

        // gone: so is the dialog
        app.world_mut().resource_mut::<ActiveCosList>().0.clear();
        app.update();
        assert_eq!(app.world().resource::<CosUnsummonConfirm>().pending, None);
    }
}

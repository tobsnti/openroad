//! "Destroy the loaded transport?" — the confirm in front of an unsummon that
//! would drop cargo on the ground.
//!
//! One stated deviation (ADR 0009): the authored message box is **one** 14px
//! line, and this dialog shows **two** shipped strings. Rather than drop one
//! or squeeze both into 14px, the box keeps its authored x/y/width and grows
//! to two lines. The alternative — inventing a second rect — would be a
//! number with no origin.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use crate::assets::FontAssets;
use crate::plugins::cos::CosCommand;
use crate::plugins::hud::modal_dialog::{MODAL_BOTTOM, MODAL_SCRIM, MODAL_SIDE, MODAL_TOP};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::textdata::ClientUiStrings;

/// `GDR_MSGBOX_BG:CIFNormalTile` `16,40,284,122` (`ifmessagebox.txt` `Create`).
const BG_RECT: (f32, f32, f32, f32) = (16.0, 40.0, 284.0, 122.0);
/// Plate size derived from that background plus the `msgbox2_window_` insets.
const PLATE: (f32, f32) = (
    BG_RECT.2 + 2.0 * MODAL_SIDE,
    BG_RECT.1 + BG_RECT.3 + MODAL_BOTTOM,
);
/// `GDR_SIMPLE_TEXTBOX_MESSAGE` `0,52,240,14`, `HAlign=1`. Height is ours (two
/// lines instead of one — see the module note).
const MESSAGE_RECT: (f32, f32, f32, f32) = (0.0, 52.0, 240.0, 14.0);
const MESSAGE_LINES: f32 = 2.0;
/// `GDR_SIMPLE_BTN_OK` `72,99,76,24` and `GDR_SIMPLE_BTN_CANCEL` `152,99,76,24`.
const YES_RECT: (f32, f32, f32, f32) = (72.0, 99.0, 76.0, 24.0);
const NO_RECT: (f32, f32, f32, f32) = (152.0, 99.0, 76.0, 24.0);
/// `FontColor=COLOR,"255,254,251,216"` on both buttons.
const BUTTON_TEXT: Color = Color::srgb_u8(254, 251, 216);

const PLATE_ART: &str = "media://interface/messagebox/msgbox2_window_";
const ART: &str = "media://interface/";

/// The pending destructive unsummon, or `None` when no dialog is up.
#[derive(Resource, Default)]
pub struct CosUnsummonConfirm {
    /// COS unique id the Yes button will dismiss.
    pub pending: Option<u32>,
}

#[derive(Component)]
pub struct CosUnsummonDialog;

/// An absolutely positioned node from a plate-local rect.
fn plate_node((x, y, w, h): (f32, f32, f32, f32)) -> Node {
    let s = hud_scale();
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
    let text_font = TextFont {
        font: fonts.nine.clone().into(),
        font_size: FontSize::Px(9.0 * s),
        ..default()
    };
    let img = |rect: (f32, f32, f32, f32), path: String| {
        (
            plate_node(rect),
            ImageNode {
                image: asset_server.load(path),
                image_mode: NodeImageMode::Stretch,
                ..default()
            },
            Pickable::IGNORE,
        )
    };

    commands
        .spawn((
            CosUnsummonDialog,
            Name::from("COS Unsummon Confirm"),
            UiTargetCamera(camera),
            GlobalZIndex(90),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(MODAL_SCRIM),
        ))
        .with_children(|scrim| {
            scrim
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(PLATE.0 * s),
                        height: Val::Px(PLATE.1 * s),
                        margin: UiRect::all(Val::Auto),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|plate| {
                    let (w, h) = PLATE;
                    for ((x, y, pw, ph), piece) in [
                        ((0.0, 0.0, MODAL_SIDE, MODAL_TOP), "left_up"),
                        ((MODAL_SIDE, 0.0, w - 2.0 * MODAL_SIDE, MODAL_TOP), "mid_up"),
                        ((w - MODAL_SIDE, 0.0, MODAL_SIDE, MODAL_TOP), "right_up"),
                        (
                            (0.0, MODAL_TOP, MODAL_SIDE, h - MODAL_TOP - MODAL_BOTTOM),
                            "left_side",
                        ),
                        (
                            (
                                w - MODAL_SIDE,
                                MODAL_TOP,
                                MODAL_SIDE,
                                h - MODAL_TOP - MODAL_BOTTOM,
                            ),
                            "right_side",
                        ),
                        (
                            (0.0, h - MODAL_BOTTOM, MODAL_SIDE, MODAL_BOTTOM),
                            "left_down",
                        ),
                        (
                            (
                                MODAL_SIDE,
                                h - MODAL_BOTTOM,
                                w - 2.0 * MODAL_SIDE,
                                MODAL_BOTTOM,
                            ),
                            "mid_down",
                        ),
                        (
                            (w - MODAL_SIDE, h - MODAL_BOTTOM, MODAL_SIDE, MODAL_BOTTOM),
                            "right_down",
                        ),
                    ] {
                        plate.spawn(img((x, y, pw, ph), format!("{PLATE_ART}{piece}.ddj")));
                    }
                    plate.spawn(img(
                        BG_RECT,
                        format!("{ART}ifcommon/bg_tile/com_bg_tile_b.ddj"),
                    ));

                    // both shipped lines, centred in the authored text box
                    let (mx, my, mw, mh) = MESSAGE_RECT;
                    let message = format!(
                        "{}\n{}",
                        ui_strings.get_or(
                            "UIIT_MSG_COS_CLEAN_CONFIRM1",
                            "If transport is destroyed, all goods will be dropped on the ground.",
                        ),
                        ui_strings.get_or(
                            "UIIT_MSG_COS_CLEAN_CONFIRM2",
                            "Will you destroy the selected transport?",
                        ),
                    );
                    plate.spawn((
                        Text::new(message),
                        text_font.clone(),
                        TextColor(Color::WHITE),
                        TextLayout::justify(Justify::Center),
                        plate_node((mx, my, mw, mh * MESSAGE_LINES)),
                        Pickable::IGNORE,
                    ));

                    for (rect, key, fallback, yes) in [
                        (YES_RECT, "UIIT_CTL_YES", "Yes", true),
                        (NO_RECT, "UIIT_CTL_NO", "No", false),
                    ] {
                        let mut button = plate.spawn((
                            Button,
                            Hovered::default(),
                            plate_node(rect),
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
                                Text::new(ui_strings.get_or(key, fallback).to_string()),
                                text_font.clone(),
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

/// Yes: this is the only place the destructive command is written.
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
    list: Res<crate::plugins::cos::ActiveCosList>,
    mut confirm: ResMut<CosUnsummonConfirm>,
) {
    if let Some(uid) = confirm.pending {
        if list.get(uid).is_none() {
            confirm.pending = None;
        }
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

    /// The plate is derived from the authored background and the measured
    /// `msgbox2_window_` insets — if either changes, this fails instead of the
    /// dialog silently drifting.
    #[test]
    fn plate_is_the_authored_background_plus_the_measured_insets() {
        assert_eq!(PLATE, (316.0, 178.0));
        let interior = crate::plugins::hud::modal_dialog::modal_interior(PLATE.0, PLATE.1);
        assert_eq!(interior, BG_RECT);
    }

    /// Every authored control stays inside the plate.
    #[test]
    fn the_authored_controls_fit_the_plate() {
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
    }

    /// The two buttons are the art's own 76x24 and do not overlap.
    #[test]
    fn the_two_buttons_are_art_sized_and_disjoint() {
        assert_eq!((YES_RECT.2, YES_RECT.3), (76.0, 24.0));
        assert_eq!((NO_RECT.2, NO_RECT.3), (76.0, 24.0));
        assert!(YES_RECT.0 + YES_RECT.2 <= NO_RECT.0);
    }
}

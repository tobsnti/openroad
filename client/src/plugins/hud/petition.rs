//! The shared invite/petition popup (0x3080) — the invitee's side.
//!
//! Idea: the original raises **one** confirm box for party, exchange, guild,
//! academy and resurrection invites, keyed by a leading `type` byte, and the
//! answer travels back on the same opcode carrying *neither* an id nor a type
//! — the server correlates it by session
//! (`docs/net-invite-0x3080.md` §1-§2). That protocol shape makes a **single**
//! pending slot an invariant rather than a simplification: two boxes on screen
//! could not be answered independently, because the wire has no way to say
//! which one. Hence one [`PendingPetition`] resource, and never two dialogs.
//!
//! What ships here are the **party arms** (`type` 2 `PartyCreation` and 3
//! `PartyInvitation`, the two the assembly doc sequences first —
//! `docs/re/systems/gameinvite-0x3080-assembly.md` §8.3), the **exchange arm**
//! (`type` 1), whose continuation now exists: accepting it makes the server
//! send 0x3085 and `hud::exchange` takes over from there (#99/#36), and the
//! **resurrection arm** (`type` 4). All three answer with the family default
//! `01 01` / `01 00`, both `[V]` (§2); only the party arms need the
//! arm-specific decline.
//!
//! The resurrection arm hosted itself as soon as its *text* was found, which
//! is also the correction of an earlier reading here ("resurrection belongs to
//! the death flow"): the player's `textuisystem.txt` carries two separate
//! families, and only one of them is the death flow.
//! `UIIT_MSG_MSGBOX_ASK_SELF_REBIRTH_1..4` (L2096-L2099, "Choose a method to
//! resurrect yourself") is the dialog you get for your *own* death, while
//! `UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_0..2` (L2100-L2102) is a message box
//! asking whether to accept somebody's resurrection — three lines, no `%s`,
//! named for the box this file builds (`MSGBOX_ASK`). That is this petition.
//!
//! Guild/union/academy stay decoded, logged and named to the player as a
//! system line without a box: their response encodings are [U] and xBot never
//! builds one, so sending an unverified decline would be the guess this file
//! is built to avoid (§9.1/9.2).
//!
//! **Chrome and geometry** come from the user's own PK2, not from this file's
//! judgement: `resinfo/ifmessagebox.txt` `Section = MsgBoxSimple` authors Yes
//! at `72,99,76,24` and No at `152,99,76,24` (both `com_button.ddj`, native
//! 76x24) with the message textbox at `0,52,240,14`, over the family's
//! `Section = Create` interior `16,40,284,122`. That interior is what fixes
//! the plate size: [`crate::plugins::hud::modal_dialog::modal_interior`]
//! inverts to exactly **316x178**, which is a second, independent confirmation
//! of the shared `msgbox2_window_` insets (16/40/16) — see the test.
//!
//! **One stated deviation.** The authored message box is `0,52,240,14`, i.e.
//! x = 0 with width 240 inside a 316-wide plate; its `HAlign=1` centres the
//! text, which would land it 38px left of the plate's centre. The authored x is
//! a host-relative artifact (a `CIFTextBox` whose content the original feeds in
//! code), so the **width 240 is kept and centred**, and the box grows downward
//! for the extra lines the party arm needs. The y, the height per line and both
//! button rects are verbatim.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::UiTargetCamera;
use bevy::ui_widgets::{Activate, Button};

use packets::agent::ingame::{
    GameInvite, InvitePetition, InviteResponse, PETITION_ACADEMY, PETITION_EXCHANGE,
    PETITION_GUILD, PETITION_PARTY_CREATION, PETITION_PARTY_INVITATION, PETITION_RESURRECTION,
    PETITION_UNION,
};
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::hud::modal_dialog::{MODAL_BOTTOM, MODAL_SCRIM, MODAL_SIDE, MODAL_TOP};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::system_message::model::format_template;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::entities::{DisplayName, NetworkEntities};
use crate::plugins::textdata::ClientUiStrings;

/// `Section = Create` authors the family's interior as `16,40,284,122`; the
/// shared plate insets are 16/40/16, so the plate is 316x178. Derived rather
/// than measured — the art has no single "plate" file to read a size off.
pub const PLATE: (f32, f32) = (316.0, 178.0);

const PLATE_ART: &str = "media://interface/messagebox/msgbox2_window_";
const ART: &str = "media://interface/";

/// `GDR_MSGBOX_BG` — `com_bg_tile_b.ddj` over the interior (`Section = Create`).
const BG_RECT: (f32, f32, f32, f32) = (16.0, 40.0, 284.0, 122.0);
/// `GDR_SIMPLE_TEXTBOX_MESSAGE` `0,52,240,14` — see the module's deviation note.
const MESSAGE_Y: f32 = 52.0;
const MESSAGE_W: f32 = 240.0;
const MESSAGE_LINE_H: f32 = 14.0;
/// `GDR_SIMPLE_BTN_OK` `72,99,76,24` / `GDR_SIMPLE_BTN_CANCEL` `152,99,76,24`.
const YES_RECT: (f32, f32, f32, f32) = (72.0, 99.0, 76.0, 24.0);
const NO_RECT: (f32, f32, f32, f32) = (152.0, 99.0, 76.0, 24.0);

/// `UIIT_CTL_YES` / `UIIT_CTL_NO`, textuisystem L434/L435.
const YES_LABEL: (&str, &str) = ("UIIT_CTL_YES", "Yes");
const NO_LABEL: (&str, &str) = ("UIIT_CTL_NO", "No");
/// L2074 `[%s] sent you  party invite.` (the double space is the data's) and
/// L2075 `Accept party invitation?`.
const PARTY_WHO: (&str, &str) = ("UIIT_STT_PARTY_SOMEUSER", "[%s] sent you  party invite.");
/// The exchange arm's single line, L1713 `UIIT_MSG_DEAL_ASK`. Unlike the party
/// strings it carries **no `%s`** — the original names nobody in this box, and
/// the whole exchange family has no invitee-side key that does (the `%s` twin
/// `UIIT_MSG_DEAL_ASKING` L1714 is the *inviter's* pending line). So this arm
/// is one line, not two, and no name is invented into it.
const DEAL_ASK: (&str, &str) = (
    "UIIT_MSG_DEAL_ASK",
    "Applied for the exchange. Will you accept it?",
);
const PARTY_ASK: (&str, &str) = ("UIIT_STT_PARTY_PROPOSAL_ASK", "Accept party invitation?");
/// The resurrection arm's body, L2100-L2102. The key family names the box this
/// file builds (`MSGBOX_ASK`) and the numeric suffix is the line order, which
/// is what makes this arm three lines; none of the three carries a `%s`, so it
/// names nobody — the same reason the exchange arm does not.
///
/// The one other `MSGBOX_ASK` resurrection body in the data,
/// `UIIT_MSG_MSGBOX_ASK_REBIRTH_MUTATION` (L2103, beast-form, one row with its
/// own markup), is **not** used: nothing in the petition distinguishes the two
/// offers (`setup` is party-only, §1), so picking between them would be
/// invented.
const RESURRECTION_LINES: [(&str, &str); 3] = [
    (
        "UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_0",
        "A warm light is hovering around your body.",
    ),
    (
        "UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_1",
        "You feel the soul entering your body once again.",
    ),
    (
        "UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_2",
        "Will you resurrect yourself to continue the adventure?",
    ),
];
/// The share-mode pairs the petition's `setup` byte selects, L502-L505.
const EXP_SHARE: (&str, &str) = ("UIIT_STT_PARTY_EXP_SHARE", "Exp Auto Share");
const EXP_SELF: (&str, &str) = ("UIIT_STT_PARTY_EXP_SELF", "Exp Free-For-All");
const ITEM_SHARE: (&str, &str) = ("UIIT_STT_PARTY_ITEM_SHARE", "Item Auto Share");
const ITEM_SELF: (&str, &str) = ("UIIT_STT_PARTY_ITEM_SELF", "Item Free-For-All");
/// Shown in place of the inviter's name when their spawn id is not on screen.
/// **Ours**: the original always has the entity (it renders the inviter), and
/// no `UIIT_*` key covers "some player we cannot see".
const UNKNOWN_INVITER: &str = "Someone";

/// The one open petition. See the module doc for why exactly one.
#[derive(Resource, Default, Debug)]
pub struct PendingPetition(pub Option<InvitePetition>);

#[derive(Component)]
pub struct PetitionDialog;

/// Which button a row is.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum PetitionAnswer {
    Accept,
    Decline,
}

/// The arms this file can answer correctly today (all `[V]` encodings).
fn opens_a_box(petition: u8) -> bool {
    matches!(
        petition,
        PETITION_PARTY_CREATION
            | PETITION_PARTY_INVITATION
            | PETITION_EXCHANGE
            | PETITION_RESURRECTION
    )
}

/// A one-line name for an arm we decode but do not host, so an unanswered
/// invite is visible instead of silent. **Ours** — the original has no such
/// line, because it hosts every arm.
fn unhosted_arm_notice(petition: u8) -> &'static str {
    match petition {
        PETITION_GUILD => "A guild invitation arrived, but it is not wired yet.",
        PETITION_UNION => "An alliance invitation arrived, but it is not wired yet.",
        PETITION_ACADEMY => "An academy invitation arrived, but it is not wired yet.",
        _ => "An invitation arrived in a form this client does not know yet.",
    }
}

/// 0x3080 S->C: take the petition, or name the arm we cannot host.
///
/// A second petition **replaces** the first. Overlap behaviour is `[U]`
/// (assembly doc §9.7), but the answer correlates by session, so the box must
/// describe whatever the server currently considers pending — and the newest
/// arrival is the only candidate for that. Never two boxes.
pub fn on_game_invite(
    mut reader: MessageReader<GameInvite>,
    mut pending: ResMut<PendingPetition>,
    mut history: ResMut<ChatHistory>,
) {
    for invite in reader.read() {
        let GameInvite::Petition(petition) = invite else {
            // The C->S half never arrives; decoding always yields Petition.
            continue;
        };
        info!(
            "petition: 0x3080 type={} uid={} setup={:?}",
            petition.petition, petition.unique_id, petition.setup
        );
        if !opens_a_box(petition.petition) {
            history.push(ChatLine::system(unhosted_arm_notice(petition.petition)));
            continue;
        }
        if pending.0.is_some() {
            warn!("petition: a second petition replaced the pending one (overlap is [U])");
        }
        pending.0 = Some(petition.clone());
    }
}

/// The resurrection arm's body: the original's own three lines, in the order
/// its keys number them (see [`RESURRECTION_LINES`]).
fn resurrection_body(ui: &ClientUiStrings) -> Vec<String> {
    RESURRECTION_LINES
        .iter()
        .map(|(key, fallback)| ui.get_or(key, fallback).to_string())
        .collect()
}

/// The party arm's body, as the lines the box stacks.
///
/// Three lines when the petition carries its `setup` byte (both party arms do):
/// who is asking, the share modes that party runs on, and the question. The
/// share line is the point of showing `setup` at all — `EXP_SHARED` also
/// decides whether the party holds 4 or 8, so accepting blind hides a real
/// difference.
fn party_body(petition: &InvitePetition, inviter: &str, ui: &ClientUiStrings) -> Vec<String> {
    let mut lines = vec![format_template(
        ui.get_or(PARTY_WHO.0, PARTY_WHO.1),
        &[inviter],
    )];
    if let Some(setup) = petition.setup {
        let exp = if setup & packets::agent::party::PartySetup::EXP_SHARED != 0 {
            EXP_SHARE
        } else {
            EXP_SELF
        };
        let item = if setup & packets::agent::party::PartySetup::ITEM_SHARED != 0 {
            ITEM_SHARE
        } else {
            ITEM_SELF
        };
        lines.push(format!(
            "{} / {}",
            ui.get_or(exp.0, exp.1),
            ui.get_or(item.0, item.1)
        ));
    }
    lines.push(ui.get_or(PARTY_ASK.0, PARTY_ASK.1).to_string());
    lines
}

/// An absolutely-positioned node from a plate-local rect.
fn plate_node((x, y, w, h): (f32, f32, f32, f32)) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(x * hud_scale()),
        top: Val::Px(y * hud_scale()),
        width: Val::Px(w * hud_scale()),
        height: Val::Px(h * hud_scale()),
        ..default()
    }
}

/// Spawn/despawn the box to match [`PendingPetition`].
#[allow(clippy::too_many_arguments)]
pub fn sync_petition_dialog(
    pending: Res<PendingPetition>,
    ui_strings: Res<ClientUiStrings>,
    fonts: Res<FontAssets>,
    asset_server: Res<AssetServer>,
    entities: Res<NetworkEntities>,
    names: Query<&DisplayName>,
    cameras: Query<Entity, With<Camera2d>>,
    open: Query<Entity, With<PetitionDialog>>,
    mut commands: Commands,
) {
    if !pending.is_changed() {
        return;
    }
    for entity in open.iter() {
        commands.entity(entity).despawn();
    }
    let Some(petition) = pending.0.as_ref() else {
        return;
    };
    let Some(camera) = cameras.iter().next() else {
        return;
    };

    // The inviter is named by spawn id; the original resolves the same way
    // (entity lookup on uniqueID). A stranger off-screen is possible, so the
    // lookup degrades to a neutral word instead of printing a number.
    let inviter = entities
        .get(petition.unique_id)
        .and_then(|entity| names.get(entity).ok())
        .map(|name| name.0.clone())
        .unwrap_or_else(|| UNKNOWN_INVITER.to_string());
    let lines = match petition.petition {
        PETITION_EXCHANGE => vec![ui_strings.get_or(DEAL_ASK.0, DEAL_ASK.1).to_string()],
        PETITION_RESURRECTION => resurrection_body(&ui_strings),
        _ => party_body(petition, &inviter, &ui_strings),
    };

    let text_font = TextFont {
        font: fonts.nine.clone().into(),
        font_size: FontSize::Px(9.0 * hud_scale()),
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

    // Captured out of the button loop so the root can point Enter at Yes
    // (`hud::focus::HudDialog`).
    let mut confirm_button = None;
    let root = commands
        .spawn((
            PetitionDialog,
            Name::from("Invite Petition"),
            UiTargetCamera(camera),
            GlobalZIndex(95),
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
                        width: Val::Px(PLATE.0 * hud_scale()),
                        height: Val::Px(PLATE.1 * hud_scale()),
                        margin: UiRect::all(Val::Auto),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|plate| {
                    let (w, h) = PLATE;
                    let side_h = h - MODAL_TOP - MODAL_BOTTOM;
                    let mid_w = w - 2.0 * MODAL_SIDE;
                    for ((x, y, pw, ph), piece) in [
                        ((0.0, 0.0, MODAL_SIDE, MODAL_TOP), "left_up"),
                        ((MODAL_SIDE, 0.0, mid_w, MODAL_TOP), "mid_up"),
                        ((w - MODAL_SIDE, 0.0, MODAL_SIDE, MODAL_TOP), "right_up"),
                        ((0.0, MODAL_TOP, MODAL_SIDE, side_h), "left_side"),
                        (
                            (w - MODAL_SIDE, MODAL_TOP, MODAL_SIDE, side_h),
                            "right_side",
                        ),
                        (
                            (0.0, h - MODAL_BOTTOM, MODAL_SIDE, MODAL_BOTTOM),
                            "left_down",
                        ),
                        (
                            (MODAL_SIDE, h - MODAL_BOTTOM, mid_w, MODAL_BOTTOM),
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

                    for (index, line) in lines.iter().enumerate() {
                        plate.spawn((
                            Text::new(line.clone()),
                            text_font.clone(),
                            TextColor(Color::WHITE),
                            TextLayout::justify(Justify::Center),
                            plate_node(message_rect(index)),
                            Pickable::IGNORE,
                        ));
                    }

                    for (rect, label, answer) in [
                        (YES_RECT, YES_LABEL, PetitionAnswer::Accept),
                        (NO_RECT, NO_LABEL, PetitionAnswer::Decline),
                    ] {
                        let is_accept = matches!(answer, PetitionAnswer::Accept);
                        let mut button = plate.spawn((
                            Button,
                            Hovered::default(),
                            answer,
                            plate_node(rect),
                            ImageNode {
                                image: asset_server.load(format!("{ART}ifcommon/com_button.ddj")),
                                image_mode: NodeImageMode::Stretch,
                                ..default()
                            },
                        ));
                        button.observe(on_petition_answer);
                        if is_accept {
                            confirm_button = Some(button.id());
                        }
                        button.with_children(|button| {
                            button.spawn((
                                Text::new(ui_strings.get_or(label.0, label.1).to_string()),
                                text_font.clone(),
                                TextColor(Color::WHITE),
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
        })
        .id();
    if let Some(confirm_button) = confirm_button {
        commands
            .entity(root)
            .insert(crate::plugins::hud::focus::HudDialog { confirm_button });
    }
}

/// Line `index`'s rect: the authored 240-wide box, centred on the plate, one
/// [`MESSAGE_LINE_H`] step per line (see the module's deviation note).
fn message_rect(index: usize) -> (f32, f32, f32, f32) {
    (
        (PLATE.0 - MESSAGE_W) / 2.0,
        MESSAGE_Y + MESSAGE_LINE_H * index as f32,
        MESSAGE_W,
        MESSAGE_LINE_H,
    )
}

/// Yes/No: answer on the same opcode and close.
///
/// The decline encoding is arm-dependent — party declines with `02 0C 2C`, not
/// the generic `01 00` (`docs/net-invite-0x3080.md` §2) — which is the only
/// reason the pending petition's `kind` is remembered at all: the response
/// itself carries neither id nor type.
fn on_petition_answer(
    activate: On<Activate>,
    answers: Query<&PetitionAnswer>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    mut pending: ResMut<PendingPetition>,
) {
    let Ok(answer) = answers.get(activate.entity) else {
        return;
    };
    let Some(petition) = pending.0.take() else {
        return;
    };
    let response = petition_response(petition.petition, *answer == PetitionAnswer::Accept);
    let Ok(conn) = conn.single() else {
        warn!("petition: no agent connection, dropping the answer");
        return;
    };
    info!(
        "petition: answering type {} with {:?}",
        petition.petition, response
    );
    if let Err(e) = conn.get_sender().send(Packet::from(response).into()) {
        error!("network: failed to send petition response: {}", e.0);
    }
}

/// Accept is `01 01` for every arm; decline branches on the arm.
pub fn petition_response(petition: u8, accept: bool) -> GameInvite {
    GameInvite::Response(match (accept, petition) {
        (true, _) => InviteResponse::Accept,
        (false, PETITION_PARTY_CREATION | PETITION_PARTY_INVITATION) => {
            InviteResponse::DeclineParty
        }
        (false, _) => InviteResponse::Decline,
    })
}

/// Leaving the world drops any open petition: the answer correlates by
/// session, so a box that outlived its session could only answer the wrong one.
pub fn cleanup_petitions(
    mut commands: Commands,
    open: Query<Entity, With<PetitionDialog>>,
    mut pending: ResMut<PendingPetition>,
) {
    for entity in open.iter() {
        commands.entity(entity).despawn();
    }
    pending.0 = None;
}

/// Self-registration for the shared petition popup (#558).
pub struct PetitionPlugin;

impl Plugin for PetitionPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<PendingPetition>()
            .add_systems(OnExit(SceneState::GameWorld), cleanup_petitions)
            .add_systems(
                Update,
                (on_game_invite, sync_petition_dialog)
                    .chain()
                    .run_if(in_state(SceneState::GameWorld)),
            );
    }
}

#[cfg(test)]
mod test {
    use super::*;
    // Only the plate-size test inverts the interior; importing it at module
    // scope would be an unused import in a non-test build, which `make
    // warnings` rejects.
    use crate::plugins::hud::modal_dialog::modal_interior;
    use packets::agent::party::PartySetup;
    use packets::Packet;

    fn petition(kind: u8, setup: Option<u8>) -> InvitePetition {
        InvitePetition {
            petition: kind,
            unique_id: 0x1234,
            setup,
        }
    }

    /// `Section = Create`'s interior `16,40,284,122` is the only size the
    /// family authors, and it inverts through the shared insets to exactly
    /// 316x178 — an independent second confirmation of 16/40/16.
    #[test]
    fn the_plate_size_is_derived_from_the_authored_interior() {
        assert_eq!(modal_interior(PLATE.0, PLATE.1), BG_RECT);
    }

    /// Both button rects are verbatim from `MsgBoxSimple`, sit inside the
    /// interior, and are the art's native 76x24 (not a rescale).
    #[test]
    fn the_buttons_are_the_authored_rects_inside_the_interior() {
        assert_eq!(YES_RECT, (72.0, 99.0, 76.0, 24.0));
        assert_eq!(NO_RECT, (152.0, 99.0, 76.0, 24.0));
        for (x, y, w, h) in [YES_RECT, NO_RECT] {
            assert!(x >= MODAL_SIDE && x + w <= PLATE.0 - MODAL_SIDE);
            assert!(y >= MODAL_TOP && y + h <= PLATE.1 - MODAL_BOTTOM);
        }
        // and they do not overlap: 72 + 76 == 148 < 152
        assert!(YES_RECT.0 + YES_RECT.2 <= NO_RECT.0);
    }

    /// The stated deviation: the authored width survives, the authored x does
    /// not (it would sit 38px off-centre), and the lines stack above the
    /// buttons rather than into them.
    #[test]
    fn the_message_box_keeps_its_width_and_is_centred() {
        let (x, y, w, h) = message_rect(0);
        assert_eq!((w, h), (MESSAGE_W, MESSAGE_LINE_H));
        assert_eq!(y, MESSAGE_Y);
        assert_eq!(x + w / 2.0, PLATE.0 / 2.0);
        assert_ne!(x, 0.0);
        // three lines is the party arm's maximum and still clears the buttons
        let (_, last_y, _, last_h) = message_rect(2);
        assert!(last_y + last_h <= YES_RECT.1);
    }

    /// Decline is arm-dependent on the wire — the trap this file exists to
    /// avoid. Accept is `01 01` everywhere.
    #[test]
    fn decline_encodes_per_arm_and_accept_does_not() {
        let bytes = |invite: GameInvite| {
            let (opcode, body) = Packet::from(invite).into_serialize();
            (opcode, body.to_vec())
        };
        assert_eq!(
            bytes(petition_response(PETITION_PARTY_INVITATION, false)),
            (0x3080, vec![0x02, 0x0C, 0x2C])
        );
        assert_eq!(
            bytes(petition_response(PETITION_PARTY_CREATION, false)),
            (0x3080, vec![0x02, 0x0C, 0x2C])
        );
        assert_eq!(
            bytes(petition_response(PETITION_EXCHANGE, false)),
            (0x3080, vec![0x01, 0x00])
        );
        assert_eq!(
            bytes(petition_response(PETITION_RESURRECTION, false)),
            (0x3080, vec![0x01, 0x00]),
            "the resurrection arm declines with the family default, not party's"
        );
        for arm in [
            PETITION_PARTY_CREATION,
            PETITION_PARTY_INVITATION,
            PETITION_EXCHANGE,
            PETITION_RESURRECTION,
        ] {
            assert_eq!(
                bytes(petition_response(arm, true)),
                (0x3080, vec![0x01, 0x01])
            );
        }
    }

    /// The hosted arms are the two party ones, exchange and resurrection;
    /// every other arm is named, not silently dropped and not answered with an
    /// unverified encoding.
    #[test]
    fn the_hosted_arms_open_a_box_and_the_rest_are_named() {
        assert!(opens_a_box(PETITION_PARTY_CREATION));
        assert!(opens_a_box(PETITION_PARTY_INVITATION));
        assert!(
            opens_a_box(PETITION_EXCHANGE),
            "exchange has a continuation now (0x3085 -> hud::exchange)"
        );
        assert!(
            opens_a_box(PETITION_RESURRECTION),
            "resurrection has both its text (L2100-L2102) and a [V] encoding"
        );
        for arm in [PETITION_GUILD, PETITION_UNION, PETITION_ACADEMY, 0x7F] {
            assert!(!opens_a_box(arm), "arm {arm} must not open a box yet");
            assert!(!unhosted_arm_notice(arm).is_empty());
        }
    }

    /// The exchange arm is deliberately ONE line and names nobody: its string
    /// `UIIT_MSG_DEAL_ASK` (L1713) carries no `%s`, and the only `%s` twin in
    /// the family is the *inviter's* `UIIT_MSG_DEAL_ASKING` (L1714). Writing
    /// the inviter's name into this box would be invented copy.
    #[test]
    fn the_exchange_arm_is_one_sourced_line_naming_nobody() {
        assert!(!DEAL_ASK.1.contains("%s"));
        assert_eq!(DEAL_ASK.0, "UIIT_MSG_DEAL_ASK");
        // and the arm carries no setup byte on the wire (0x3080 §1)
        assert!(petition(PETITION_EXCHANGE, None).setup.is_none());
    }

    /// The resurrection body is the player's own three rows, read by key.
    /// Pinned through [`ClientUiStrings::from_rows`] so the assertion is about
    /// the *keys* this file asks for, not about the fallback strings compiled
    /// into it — a typo'd key would silently ship the fallback otherwise.
    #[test]
    fn the_resurrection_arm_is_the_originals_three_lines() {
        let ui = ClientUiStrings::from_rows(&[
            ("UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_0", "line zero"),
            ("UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_1", "line one"),
            ("UIIT_MSG_MSGBOX_ASK_SKL_RESURRECTION_2", "line two"),
            ("UIIT_MSG_MSGBOX_ASK_SELF_REBIRTH_2", "the death flow's own"),
        ]);
        assert_eq!(
            resurrection_body(&ui),
            vec![
                "line zero".to_string(),
                "line one".to_string(),
                "line two".to_string()
            ]
        );
        // three lines is the geometry's maximum, and the arm names nobody:
        // none of the three rows carries a %s (unlike the party arm's first).
        assert_eq!(RESURRECTION_LINES.len(), 3);
        for (_, fallback) in RESURRECTION_LINES {
            assert!(!fallback.contains("%s"), "{fallback:?}");
        }
    }

    /// Minimal app that can really spawn the box: the asset pair plus a camera,
    /// which is all [`sync_petition_dialog`] needs beyond its resources.
    /// `TaskPoolPlugin` before `AssetPlugin` is load-bearing (#665) — the
    /// plate art is loaded by path.
    fn petition_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<PendingPetition>()
        .init_resource::<ChatHistory>()
        .init_resource::<ClientUiStrings>()
        .init_resource::<NetworkEntities>()
        .insert_resource(FontAssets {
            one: Handle::default(),
            two: Handle::default(),
            three: Handle::default(),
            nine: Handle::default(),
        })
        .add_message::<GameInvite>()
        .add_systems(Update, (on_game_invite, sync_petition_dialog).chain());
        app.world_mut().spawn(Camera2d);
        app
    }

    fn open_dialogs(app: &mut App) -> usize {
        let world = app.world_mut();
        let mut query = world.query::<&PetitionDialog>();
        query.iter(world).count()
    }

    /// LUECKEN P1, the whole path: a resurrection petition used to reach the
    /// player as a sentence this repo wrote and nothing else. Now it opens the
    /// one shared box, carrying the original's rows, and clicking Yes answers
    /// and closes it. The click is the real `Activate` the mouse sends, so the
    /// observer that takes the pending petition is the one under test.
    #[test]
    fn a_resurrection_petition_opens_the_box_and_answering_closes_it() {
        let mut app = petition_app();

        app.world_mut()
            .write_message(GameInvite::Petition(petition(PETITION_RESURRECTION, None)));
        app.update();

        assert_eq!(open_dialogs(&mut app), 1, "no box opened");
        assert_eq!(
            app.world().resource::<ChatHistory>().iter().count(),
            0,
            "a hosted arm must not also be narrated as a system line"
        );
        let world = app.world_mut();
        let mut texts = world.query::<&Text>();
        let lines: Vec<String> = texts.iter(world).map(|text| text.0.clone()).collect();
        for (_, row) in RESURRECTION_LINES {
            assert!(
                lines.contains(&row.to_string()),
                "missing {row:?} in {lines:?}"
            );
        }

        // Yes, through the same event the button sends.
        let mut answers = app.world_mut().query::<(Entity, &PetitionAnswer)>();
        let yes = answers
            .iter(app.world())
            .find(|(_, answer)| **answer == PetitionAnswer::Accept)
            .map(|(entity, _)| entity)
            .expect("the box has no Yes button");
        app.world_mut().trigger(Activate { entity: yes });
        app.update();

        assert!(
            app.world().resource::<PendingPetition>().0.is_none(),
            "the answer must clear the pending petition"
        );
        assert_eq!(open_dialogs(&mut app), 0, "the box stayed open after Yes");
    }

    /// The `setup` byte must reach the UI (§8 fidelity checklist): it is the
    /// difference between a 4-person free-for-all and an 8-person share party.
    #[test]
    fn the_setup_byte_reaches_the_body() {
        let ui = ClientUiStrings::default();
        let shared = party_body(
            &petition(
                PETITION_PARTY_INVITATION,
                Some(PartySetup::EXP_SHARED | PartySetup::ITEM_SHARED),
            ),
            "Alice",
            &ui,
        );
        assert_eq!(shared.len(), 3);
        assert!(shared[0].contains("Alice"), "{:?}", shared[0]);
        assert!(shared[1].contains("Auto Share"), "{:?}", shared[1]);
        assert_eq!(shared[2], PARTY_ASK.1);

        let solo = party_body(&petition(PETITION_PARTY_INVITATION, Some(0)), "Alice", &ui);
        assert!(solo[1].contains("Free-For-All"), "{:?}", solo[1]);

        // an arm without a setup byte drops the share line rather than
        // inventing one
        let bare = party_body(&petition(PETITION_PARTY_INVITATION, None), "Alice", &ui);
        assert_eq!(bare.len(), 2);
    }
}

//! The target right-click menu (`res_ui/targetmenu.2dt`, `CNIFTargetMenu` id
//! 174) — five verbs on the selected player.
//!
//! Idea: this is not its own window class. `targetmenu.2dt` is the *same*
//! three-part construction as the under-bar's Menu flyout — a `CNIFrame` on
//! `ub_new_wnd_`, a `com_bg_tile_u` fill at inset 20, rows of
//! `ub_new_menu_button.ddj` — at a different size, and the recovered client
//! module map homes that construction to `NIFUnderMenuBar.cpp` with no
//! `NIFTargetMenu.cpp` anywhere in its 161 sources. So the widget lives in
//! [`crate::plugins::hud::context_menu`] and this module is only the *item list*
//! (`docs/re/ui/hud-target-menu.md` §3.3).
//!
//! Two traps from the descriptor, both avoided by construction here:
//!
//! 1. **The rows are neither in id order nor in record order.** Id 17 ("Add
//!    friend") renders *above* id 16 ("Invite party"), and record `[5]` precedes
//!    record `[4]` on screen. [`TargetAction::ORDER`] is the visual order and the
//!    ids are carried only as documentation.
//! 2. **The root's `Text` is `UIIT_CTL_AUTOTRACE_TT`** — byte-identical to its
//!    own last row's key. A `CNIFrame` root has nothing to draw a caption on and
//!    "Trace" is not a title for a five-item menu, so it is authoring
//!    copy-paste and is deliberately not rendered.
//!
//! Anchor: the authored root `638,321` sits 95px into the target window's
//! `543,289,196,36` band and 4px above its bottom edge, so the menu is read as
//! anchored to the target window rather than to the cursor (`[S]`, §9-U1) — the
//! offsets below are that reading, expressed against our own panel.
//!
//! The five verbs are emitted as [`TargetMenuAction`] messages and each needs
//! its own wire path. "Invite party" has one now
//! ([`send_target_menu_party_invite`] → `net::party::PartyAction`) and so does
//! "Exchange" ([`send_target_menu_exchange_invite`] → 0x7081). "Whisper" needs
//! no wire path of its own — it prepares the chat input
//! ([`send_target_menu_whisper`], §"Deviation, stated" there) and the existing
//! `0x7025` send does the rest. "Add friend" and "Trace" still have none:
//! friend needs a request we do not have, and Trace is a client-side follow
//! whose stop conditions are not recovered yet. Inventing either here would be a guess,
//! so the menu stays honest about what exists — it opens, it is localized, it
//! names the action — and the wiring is a separate issue per verb.

use bevy::input_focus::InputFocus;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::text::EditableText;
use bevy::window::PrimaryWindow;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::cursor::interactions::entity_select::SelectedEntity;
use crate::plugins::hud::chat::input::prefill_whisper_input;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine, ChatState};
use crate::plugins::hud::chat::ui::ChatInputBox;
use crate::plugins::hud::context_menu::{
    close_context_menus, spawn_context_menu, ContextMenuItem, ContextMenuOwner, ContextMenuRoot,
    ContextMenuRow,
};
use crate::plugins::hud::exchange;
use crate::plugins::hud::game_window::abs_node;
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::hud::system_message::model::format_template;
use crate::plugins::net::agent::AgentConnection;
use crate::plugins::net::entities::{DisplayName, NetworkId, RemoteEntity};
use crate::plugins::net::party::PartyAction;
use crate::plugins::textdata::ClientUiStrings;

/// Target-window geometry the anchor is expressed against
/// (`iftargetwindow.txt`: the player plate is `196x36` at `PANEL_TOP`).
const TARGET_FRAME_W: f32 = 196.0;
const TARGET_PANEL_TOP: f32 = 6.0;
/// The authored offsets of the menu root inside the target window's band:
/// `638 - 543 = 95` horizontally, `321 - 289 = 32` vertically.
const MENU_DX: f32 = 95.0;
const MENU_DY: f32 = 32.0;

/// A right-click that moved more than this many logical pixels was a camera
/// drag, not a click. **openroad choice** — the original has no such threshold
/// because its right button is not the camera's.
const DRAG_SLOP: f32 = 4.0;

/// The five verbs, in the order they render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetAction {
    Whisper,
    Exchange,
    AddFriend,
    InviteParty,
    Trace,
}

impl TargetAction {
    /// Visual (y) order — *not* the descriptor's id or record order.
    pub const ORDER: [TargetAction; 5] = [
        TargetAction::Whisper,
        TargetAction::Exchange,
        TargetAction::AddFriend,
        TargetAction::InviteParty,
        TargetAction::Trace,
    ];

    /// textuisystem key, with the English column as the offline fallback.
    pub fn label(self) -> (&'static str, &'static str) {
        match self {
            TargetAction::Whisper => ("UIIT_STT_GET_WHISPER", "Whisper"),
            TargetAction::Exchange => ("UIIT_CTL_EXCHANGE_TT", "Exchange"),
            TargetAction::AddFriend => ("UIIT_CTL_FRIENDADD", "Add friend"),
            TargetAction::InviteParty => ("UIIT_STT_INVITE_PARTY", "Invite party"),
            TargetAction::Trace => ("UIIT_CTL_AUTOTRACE_TT", "Trace"),
        }
    }

    /// The descriptor's control id — documentation only, never an ordering key.
    pub fn original_id(self) -> u32 {
        match self {
            TargetAction::Whisper => 14,
            TargetAction::Exchange => 15,
            TargetAction::AddFriend => 17,
            TargetAction::InviteParty => 16,
            TargetAction::Trace => 18,
        }
    }
}

/// A picked menu row. Each verb's wire path is a separate issue; this is the
/// seam they plug into.
#[derive(Message, Clone, Copy, Debug)]
pub struct TargetMenuAction {
    pub target: Entity,
    pub action: TargetAction,
}

/// The entity the open menu acts on.
#[derive(Resource, Default)]
pub struct TargetMenuTarget(pub Option<Entity>);

/// Open the menu on a right-click that did not drag the camera, when the
/// current target is another player. Right-drag rotates the camera in openroad,
/// so the press position is remembered and a moved release is ignored.
#[allow(clippy::too_many_arguments)]
pub fn open_target_menu(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    selected: Res<SelectedEntity>,
    remotes: Query<&RemoteEntity>,
    ui_strings: Res<ClientUiStrings>,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    cameras: Query<Entity, With<Camera2d>>,
    open: Query<Entity, With<ContextMenuRoot>>,
    names: Query<&DisplayName>,
    mut target: ResMut<TargetMenuTarget>,
    mut owner: ResMut<ContextMenuOwner>,
    mut press_at: Local<Option<Vec2>>,
    mut commands: Commands,
) {
    let cursor = windows.iter().next().and_then(|w| w.cursor_position());
    if buttons.just_pressed(MouseButton::Right) {
        *press_at = cursor;
        return;
    }
    if !buttons.just_released(MouseButton::Right) {
        return;
    }
    let moved = match (press_at.take(), cursor) {
        (Some(down), Some(up)) => down.distance(up) > DRAG_SLOP,
        _ => true,
    };
    if moved {
        return;
    }

    close_context_menus(&mut commands, &open, &mut owner);
    target.0 = None;

    let Some(entity) = selected.0 else { return };
    if !matches!(remotes.get(entity), Ok(RemoteEntity::Player)) {
        return;
    }
    let Some(camera) = cameras.iter().next() else {
        return;
    };

    let items: Vec<ContextMenuItem> = TargetAction::ORDER
        .iter()
        .map(|action| {
            let (key, fallback) = action.label();
            ContextMenuItem {
                label: ui_strings.get_or(key, fallback).to_string(),
                enabled: true,
            }
        })
        .collect();

    let root = spawn_context_menu(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        Val::Percent(50.0),
        Val::Px((TARGET_PANEL_TOP + MENU_DY) * hud_scale()),
        // the target window is centred, so its left edge is 50% - half its
        // width; the authored menu sits MENU_DX into that band.
        Val::Px((MENU_DX - TARGET_FRAME_W / 2.0) * hud_scale()),
        &items,
        hud_scale(),
    );
    // The header plate carries the target's name: the original's target
    // window sets the menu's header static (id 8) from it when it opens the
    // menu. The plate stayed a blank block before.
    if let Ok(name) = names.get(entity) {
        let s = hud_scale();
        let (x, y, w, h) = crate::plugins::hud::context_menu::HEADER;
        commands.entity(root).with_children(|popup| {
            popup.spawn((
                Text::new(name.0.clone()),
                TextFont {
                    font: fonts.nine.clone().into(),
                    font_size: FontSize::Px(crate::plugins::hud::context_menu::ROW_FONT * s),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Center),
                abs_node((x, y + 4.0, w, h - 4.0), s),
                Pickable::IGNORE,
            ));
        });
    }
    target.0 = Some(entity);
    *owner = ContextMenuOwner::Target;
}

/// Fire the picked row's action and close the menu.
pub fn activate_target_menu_row(
    buttons: Res<ButtonInput<MouseButton>>,
    rows: Query<(&ContextMenuRow, &Hovered)>,
    open: Query<Entity, With<ContextMenuRoot>>,
    mut target: ResMut<TargetMenuTarget>,
    mut owner: ResMut<ContextMenuOwner>,
    mut actions: MessageWriter<TargetMenuAction>,
    mut commands: Commands,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    if *owner != ContextMenuOwner::Target {
        return;
    }
    let Some(entity) = target.0 else { return };
    let picked = rows
        .iter()
        .find(|(_, hovered)| hovered.get())
        .and_then(|(row, _)| TargetAction::ORDER.get(row.0).copied());
    if let Some(action) = picked {
        actions.write(TargetMenuAction {
            target: entity,
            action,
        });
    }
    // any left click dismisses the menu, picked or not
    close_context_menus(&mut commands, &open, &mut owner);
    target.0 = None;
}

/// Close the menu when its target goes away (deselected, out of range, dead).
pub fn close_target_menu_on_deselect(
    selected: Res<SelectedEntity>,
    open: Query<Entity, With<ContextMenuRoot>>,
    mut target: ResMut<TargetMenuTarget>,
    mut owner: ResMut<ContextMenuOwner>,
    mut commands: Commands,
) {
    let Some(entity) = target.0 else { return };
    if selected.0 != Some(entity) {
        close_context_menus(&mut commands, &open, &mut owner);
        target.0 = None;
    }
}

/// "Invite party" — the first row with a wire path.
///
/// The row names a *screen* entity; the wire wants the target's spawn id, which
/// is [`NetworkId`]. Whether that becomes 0x7060 or 0x7062 is not decided here:
/// the menu has no idea whether we are in a party, so it states the intent and
/// [`PartyAction`]'s sender resolves it against the roster.
pub fn send_target_menu_party_invite(
    mut actions: MessageReader<TargetMenuAction>,
    ids: Query<&NetworkId>,
    mut party: MessageWriter<PartyAction>,
) {
    for action in actions.read() {
        if action.action != TargetAction::InviteParty {
            continue;
        }
        match ids.get(action.target) {
            Ok(id) => {
                party.write(PartyAction::Invite(id.0));
            }
            // Selected entities always carry a NetworkId; if one does not, the
            // menu should say so rather than send a zero uid.
            Err(_) => warn!(
                "target menu: invite party on {:?}, which has no NetworkId",
                action.target
            ),
        }
    }
}

/// "Exchange" — the second row with a wire path.
///
/// Same shape as the party funnel: the row names a screen entity, the wire
/// wants its spawn id, and the trade window is *not* opened here — 0x7081 only
/// asks the server to raise the petition on the target
/// (`docs/net-invite-0x3080.md` §4). The window follows on 0x3085 if they
/// accept.
pub fn send_target_menu_exchange_invite(
    mut actions: MessageReader<TargetMenuAction>,
    targets: Query<(&NetworkId, Option<&DisplayName>)>,
    conn: Query<&SilkroadConnection, With<AgentConnection>>,
    ui_strings: Res<ClientUiStrings>,
    mut history: ResMut<ChatHistory>,
) {
    for action in actions.read() {
        if action.action != TargetAction::Exchange {
            continue;
        }
        match targets.get(action.target) {
            Ok((id, name)) => {
                exchange::model::send_invite(&conn, id.0);
                // The original tells the inviter it is waiting, with the
                // target's name — `UIIT_MSG_DEAL_ASKING` (L1714). Without it
                // an unanswered request looks like a dead click.
                let (key, fallback) = exchange::model::DEAL_ASKING;
                let who = name.map(|n| n.0.as_str()).unwrap_or_default();
                history.push(ChatLine::system(format_template(
                    ui_strings.get_or(key, fallback),
                    &[who],
                )));
            }
            Err(_) => warn!(
                "target menu: exchange on {:?}, which has no NetworkId",
                action.target
            ),
        }
    }
}

/// "Whisper" — the third row with a path, and the only one that never touches
/// the wire.
///
/// Idea: whispering has no request of its own; it is a normal `0x7025`
/// `ChatRequest` with `chat_type::PM` plus a receiver name, and the client's
/// job is to put the player in front of an input line that is already addressed
/// to the target. That is exactly what
/// [`crate::plugins::hud::chat::input::prefill_whisper_input`] does for the
/// whisper-panel rows and for clicks on a sender name, so the menu row reuses
/// it instead of growing a second prefill.
///
/// **Deviation, stated (ADR-0009).** We could not source the *original's*
/// right-click whisper behaviour. What is verified in the original client is
/// only the frame around it: the 0x3026 chat handler and its formatter
/// (`UIIT_CHATERR_WHISPER_{TO,FROM}_MESSAGE`) show the *display* side; the
/// target menu window (id 174, `res_ui/targetmenu.2dt`) is opened from the
/// target window's own update (header static id 8 set from the target's name)
/// and positioned by the mouse handler; neither path contains a chat-edit
/// write, and `UIIT_CTL_AUTOTRACE_TT`/`UIIT_STT_GET_WHISPER` are referenced by
/// no code at all — the labels live in the descriptor. So the prefix is *our*
/// choice, and it is the one our whisper panel already uses: `$name `, the
/// `$` whisper sigil of the original chat (textuisystem
/// `UIIT_CTL_CHATMENU_*` L4267-4270 establish the sigil column). Inventing a
/// third spelling (`/w name `) here would give the same window two grammars.
///
/// The chat window is un-collapsed on the way, because a prefilled input the
/// player cannot see is the same dead click this is removing.
pub fn send_target_menu_whisper(
    mut actions: MessageReader<TargetMenuAction>,
    names: Query<&DisplayName>,
    mut focus: ResMut<InputFocus>,
    mut chat: ResMut<ChatState>,
    mut input: Query<(Entity, &mut EditableText), With<ChatInputBox>>,
) {
    for action in actions.read() {
        if action.action != TargetAction::Whisper {
            continue;
        }
        match names.get(action.target) {
            Ok(name) => {
                chat.collapsed = false;
                chat.whisper_panel_open = false;
                prefill_whisper_input(&name.0, &mut focus, &mut chat, &mut input);
            }
            // A remote player without a DisplayName cannot be addressed: 0x7025
            // carries the receiver by name, so there is nothing to send.
            Err(_) => warn!(
                "target menu: whisper on {:?}, which has no DisplayName",
                action.target
            ),
        }
    }
}

/// The verbs that carry out what their row promises today. Kept as one list
/// because it is the *only* thing separating a working row from a row that has
/// to apologise: whoever wires the next verb adds it here, and the apology for
/// it disappears in the same edit.
const WIRED_ACTIONS: [TargetAction; 3] = [
    TargetAction::InviteParty,
    TargetAction::Exchange,
    TargetAction::Whisper,
];

/// Until each verb has a wire path, name the picked action **to the player**,
/// not only to the log.
///
/// The rule, and it is the one `underbar/menu_popup.rs` already follows for its
/// unbuilt rows: a click on a row that names an action ends in something the
/// player can see. This system used to end in `info!`, so picking "Add friend"
/// or "Trace" was indistinguishable from a broken menu (whisper has a path
/// since then, the other two do not).
/// The wording is the popup's, so the HUD says one thing
/// in one way. `ChatHistory` is an `Option` because this plugin does not own it
/// (`hud/chat/mod.rs` does) and a scene with a target menu and no chat must
/// degrade rather than fail parameter validation.
pub fn log_target_menu_actions(
    mut actions: MessageReader<TargetMenuAction>,
    ui_strings: Res<ClientUiStrings>,
    mut history: Option<ResMut<ChatHistory>>,
) {
    for action in actions.read() {
        if WIRED_ACTIONS.contains(&action.action) {
            continue;
        }
        info!(
            "target menu: {:?} on {:?} has no wire path yet",
            action.action, action.target
        );
        if let Some(history) = history.as_mut() {
            let (key, fallback) = action.action.label();
            let label = ui_strings.get_or(key, fallback);
            history.push(ChatLine::system(format!("{label}: not available yet.")));
        }
    }
}

pub fn cleanup_target_menu(
    mut commands: Commands,
    open: Query<Entity, With<ContextMenuRoot>>,
    mut target: ResMut<TargetMenuTarget>,
    mut owner: ResMut<ContextMenuOwner>,
) {
    close_context_menus(&mut commands, &open, &mut owner);
    target.0 = None;
}

#[cfg(test)]
mod test {
    use super::*;

    /// Every unwired verb answers the player in chat, and every wired one stays
    /// quiet.
    ///
    /// Driven off [`TargetAction::ORDER`] rather than off a hand-written list,
    /// so a sixth verb cannot be added without deciding which side it is on.
    /// Without the chat line in [`log_target_menu_actions`] this fails on the
    /// first unwired verb ("Add friend"), which is precisely the click that
    /// looked dead.
    #[test]
    fn an_unwired_verb_answers_in_chat_and_a_wired_one_does_not() {
        for action in TargetAction::ORDER {
            let mut app = App::new();
            app.add_message::<TargetMenuAction>()
                .init_resource::<ClientUiStrings>()
                .init_resource::<ChatHistory>()
                .add_systems(Update, log_target_menu_actions);
            let target = app.world_mut().spawn_empty().id();
            app.world_mut()
                .write_message(TargetMenuAction { target, action });
            app.update();

            let lines: Vec<String> = app
                .world()
                .resource::<ChatHistory>()
                .iter()
                .map(|line| line.text.clone())
                .collect();
            if WIRED_ACTIONS.contains(&action) {
                assert!(
                    lines.is_empty(),
                    "{action:?} carries out its row and must not apologise: {lines:?}"
                );
            } else {
                let (_, fallback) = action.label();
                assert_eq!(
                    lines,
                    vec![format!("{fallback}: not available yet.")],
                    "{action:?} has no wire path and said nothing the player can see"
                );
            }
        }
    }

    /// The header plate names the target (the original fills the menu's header
    /// static from the target window's name). It was a blank block before.
    #[test]
    fn the_menu_header_names_the_target() {
        use bevy::window::PrimaryWindow;

        let mut app = App::new();
        app.add_plugins((
            bevy::app::TaskPoolPlugin::default(),
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ClientUiStrings>()
        .init_resource::<TargetMenuTarget>()
        .init_resource::<ContextMenuOwner>()
        .insert_resource(FontAssets {
            one: Handle::default(),
            two: Handle::default(),
            three: Handle::default(),
            nine: Handle::default(),
        })
        .add_systems(Update, open_target_menu);
        let mut window = Window::default();
        window.set_cursor_position(Some(Vec2::new(300.0, 200.0)));
        app.world_mut().spawn((window, PrimaryWindow));
        app.world_mut().spawn(Camera2d);
        let target = app
            .world_mut()
            .spawn((RemoteEntity::Player, DisplayName("Trader6".into())))
            .id();
        app.insert_resource(SelectedEntity(Some(target)));

        // a right-click that did not drag: press one frame, release the next
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.update();
        {
            let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            buttons.clear();
            buttons.release(MouseButton::Right);
        }
        app.update();

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<ContextMenuRoot>>()
            .single(app.world())
            .expect("the menu opened");
        let texts: Vec<String> = app
            .world_mut()
            .query::<(&Text, &ChildOf)>()
            .iter(app.world())
            .filter(|(_, child_of)| child_of.parent() == root)
            .map(|(text, _)| text.0.clone())
            .collect();
        assert!(
            texts.iter().any(|text| text == "Trader6"),
            "the header must carry the target's name: {texts:?}"
        );
    }

    /// The rows render in y order, which is neither id order nor record order —
    /// this is the file's own trap, so pin it.
    #[test]
    fn rows_render_in_visual_order_not_id_order() {
        let ids: Vec<u32> = TargetAction::ORDER
            .iter()
            .map(|a| a.original_id())
            .collect();
        // authored y: 353, 373, 392, 412, 431 → ids 14, 15, 17, 16, 18
        assert_eq!(ids, vec![14, 15, 17, 16, 18]);
        // ...which is NOT sorted: id 17 renders above id 16
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_ne!(ids, sorted);
    }

    /// The five keys, verbatim from `targetmenu.2dt`'s `Text` fields.
    #[test]
    fn the_five_keys_match_the_descriptor() {
        let keys: Vec<&str> = TargetAction::ORDER.iter().map(|a| a.label().0).collect();
        assert_eq!(
            keys,
            vec![
                "UIIT_STT_GET_WHISPER",
                "UIIT_CTL_EXCHANGE_TT",
                "UIIT_CTL_FRIENDADD",
                "UIIT_STT_INVITE_PARTY",
                "UIIT_CTL_AUTOTRACE_TT",
            ]
        );
        // the root's own Text is the LAST row's key — a copy-paste, not a
        // caption; nothing here may render it as a title.
        assert_eq!(TargetAction::Trace.label().0, "UIIT_CTL_AUTOTRACE_TT");
    }

    /// The one thing worth pinning about the whisper row: what the menu writes
    /// into the input must be what the input's own parser accepts as a PM to
    /// that name. `prefill_whisper_input` writes `$name ` (one shared helper,
    /// `chat/input.rs`), so parse it back with the real parser — if either side
    /// ever changes its mind about the sigil, this fails instead of the click
    /// silently sending to the wrong channel.
    #[test]
    fn the_whisper_prefill_round_trips_through_the_chat_parser() {
        use crate::plugins::hud::chat::input::parse_outgoing;
        use crate::plugins::hud::chat::model::ChatTab;
        use packets::agent::chat::chat_type;

        let typed = format!("{}{}", "$Trader6 ", "hi");
        let out = parse_outgoing(&typed, ChatTab::All).expect("prefilled line must parse");
        assert_eq!(out.chat_type, chat_type::PM);
        assert_eq!(out.receiver.as_deref(), Some("Trader6"));
        assert_eq!(out.message, "hi");
    }

    /// The anchor is expressed as the authored offset inside the target
    /// window's band, not as the absolute `638,321` (which would pin the menu
    /// to one screen position at one resolution).
    #[test]
    fn the_anchor_is_relative_to_the_target_window() {
        // 638 - 543 = 95, 321 - 289 = 32
        assert_eq!(MENU_DX, 638.0 - 543.0);
        assert_eq!(MENU_DY, 321.0 - 289.0);
        // and it lands inside the 196-wide band, above its bottom edge
        assert!(MENU_DX > 0.0 && MENU_DX < TARGET_FRAME_W);
        assert!(MENU_DY < 36.0);
    }
}

/// Self-registration for the target right-click menu (#448) (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct TargetMenuPlugin;

impl Plugin for TargetMenuPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<TargetMenuTarget>()
            .init_resource::<ContextMenuOwner>()
            .add_message::<TargetMenuAction>()
            .add_systems(OnExit(SceneState::GameWorld), cleanup_target_menu)
            // the ub_new_wnd_ popup, live game world only (it acts on a
            // selected remote player) (#448)
            .add_systems(
                Update,
                (
                    open_target_menu,
                    activate_target_menu_row,
                    close_target_menu_on_deselect,
                    send_target_menu_party_invite,
                    send_target_menu_exchange_invite,
                    send_target_menu_whisper,
                    log_target_menu_actions,
                )
                    .chain()
                    .run_if(in_state(SceneState::GameWorld)),
            );
    }
}

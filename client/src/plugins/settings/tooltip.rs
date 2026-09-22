//! Hover help for the options window's controls.
//!
//! # The idea
//!
//! The original ships **75** finished tooltip strings for this one window
//! (`textuisystem.txt`: `UIIT_STT_VIDIO_TTDESC_01..17` :968-984,
//! `_GAMESET_TTDESC_01..05` :985-989, `_AUDIO_TTDESC_01..03` :990-992,
//! `_VIEW_TTDESC_01..03` :993-995, `_INPUT_TTDESC_01..31` :996-1026,
//! `_GAME_NAMEVIEW_TTDESC_01..06` :1027-1032, `_GAME_OPTION_TTDESC_01..10`
//! :1033-1042). Each pane carries its own key-to-row mapping.
//!
//! Two things this deliberately does *not* do:
//!
//! * **No second global tooltip mechanism.** The HUD's only tooltip
//!   (`hud/inventory/tooltip.rs`) is an item panel — it resolves an
//!   `InventoryItem` into stat lines and is driven by a `HoveredSlot`
//!   resource, so there is nothing generic to reuse. Rather than lift it into
//!   a widget for one caller, help text here is a **per-pane footer line**
//!   filled by `Pointer<Over>`/`Pointer<Out>` observers on the controls
//!   themselves. Observers need no plugin registration, which keeps this off
//!   `options_window.rs` entirely.
//! * **No cursor-following bubble.** The original's placement is unknown, and a
//!   bubble needs a positioning system plus a hover-delay timer. A fixed
//!   line at the pane's bottom edge shows the same text at a known place; it
//!   is a stated deviation, not a guess at the original's geometry.

use bevy::prelude::*;

/// The footer line of a pane, written by the hover observers. One per pane —
/// every observer writes *all* of them, because only the active pane's node
/// tree is displayed (`options_window`'s `apply_active_tab` sets
/// `Display::None` on the others), so a broadcast write is cheaper than
/// teaching each control which pane it sits in.
#[derive(Component)]
pub struct OptionsTooltipLine;

/// The resolved help text of one control. Carried as a component so a test can
/// read back what a pane wired to a control without simulating pointer input.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct OptionsTooltip(pub String);

/// Height of the footer line. Two lines of 11px text plus padding: the longest
/// string in the block (`VIDIO_TTDESC_04`, 232 chars) needs more than that and
/// clips — a deliberate cap, because growing the line upwards would cover the
/// control the player is pointing at.
pub const TOOLTIP_LINE_H: f32 = 30.0;
const TOOLTIP_FONT_PX: f32 = 10.0;
/// Same parchment tone the panes use for labels (`options_video::LABEL_COLOR`).
const TOOLTIP_COLOR: Color = Color::srgb(1.0, 0.965, 0.827);
const TOOLTIP_BG: Color = Color::srgba(0.0, 0.0, 0.0, 0.82);

/// Spawns a pane's footer line. `left`/`width` are the pane-local rect of the
/// content the caller wants the line to span; `bottom` keeps it flush with the
/// pane's lower edge, whose height differs per tab (`ifoption.txt`: 313 for
/// Video/Setting/Key Map, 212 View, 219 Audio).
pub fn spawn_tooltip_line(
    pane: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    left: f32,
    width: f32,
) {
    pane.spawn((
        OptionsTooltipLine,
        Name::from("Options Tooltip Line"),
        Text::new(String::new()),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(TOOLTIP_FONT_PX),
            ..default()
        },
        TextColor(TOOLTIP_COLOR),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left),
            bottom: Val::Px(2.0),
            width: Val::Px(width),
            height: Val::Px(TOOLTIP_LINE_H),
            padding: UiRect::all(Val::Px(3.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(TOOLTIP_BG),
        // Above the pane content: the line overlaps whatever sits at the
        // pane's bottom edge while it is visible.
        ZIndex(20),
        Visibility::Hidden,
        Pickable::IGNORE,
    ));
}

/// Wires one control to its help text.
///
/// Also makes the entity hoverable: rows that are only *displayed* carry
/// `Pickable::IGNORE`, and an ignored node never gets `Pointer<Over>`. Hover
/// is not a click — an inert row stays inert, it just explains itself.
pub fn attach_tooltip(entity: &mut EntityCommands, text: impl Into<String>) {
    let text = text.into();
    if text.is_empty() {
        return;
    }
    entity.insert((OptionsTooltip(text.clone()), Pickable::default()));
    entity.observe(
        move |_: On<Pointer<Over>>,
              mut lines: Query<(&mut Text, &mut Visibility), With<OptionsTooltipLine>>| {
            for (mut line, mut visibility) in lines.iter_mut() {
                if line.0 != text {
                    *line = Text::new(text.clone());
                }
                *visibility = Visibility::Inherited;
            }
        },
    );
    entity.observe(
        |_: On<Pointer<Out>>,
         mut lines: Query<(&mut Text, &mut Visibility), With<OptionsTooltipLine>>| {
            for (mut line, mut visibility) in lines.iter_mut() {
                line.0.clear();
                *visibility = Visibility::Hidden;
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The footer line starts hidden and empty: a pane that is opened without
    /// the pointer touching anything must not show a stale string.
    #[test]
    fn a_freshly_spawned_line_is_hidden_and_empty() {
        let mut app = App::new();
        let pane = app.world_mut().spawn(Node::default()).id();
        let font = Handle::<Font>::default();
        app.world_mut()
            .commands()
            .entity(pane)
            .with_children(|p| spawn_tooltip_line(p, &font, 14.0, 336.0));
        app.world_mut().flush();

        let mut query = app
            .world_mut()
            .query_filtered::<(&Text, &Visibility), With<OptionsTooltipLine>>();
        let (text, visibility) = query.single(app.world()).expect("one tooltip line");
        assert!(text.0.is_empty());
        assert_eq!(*visibility, Visibility::Hidden);
    }

    /// An empty string is not a tooltip. Four of the original's keys have no
    /// row in this client (see `PREGAME-options-b2b3.md` §2), and a caller that
    /// resolves one of them to "" must not leave a control claiming help it
    /// cannot show.
    #[test]
    fn an_empty_text_wires_nothing() {
        let mut app = App::new();
        let control = app.world_mut().spawn(Node::default()).id();
        {
            let mut commands = app.world_mut().commands();
            let mut entity = commands.entity(control);
            attach_tooltip(&mut entity, "");
        }
        app.world_mut().flush();
        assert!(app.world().get::<OptionsTooltip>(control).is_none());
    }

    /// ...and a real text is readable back off the control, which is how the
    /// pane tests assert their key mapping without simulating pointer input.
    #[test]
    fn a_wired_control_carries_its_resolved_text() {
        let mut app = App::new();
        let control = app.world_mut().spawn(Node::default()).id();
        {
            let mut commands = app.world_mut().commands();
            let mut entity = commands.entity(control);
            attach_tooltip(&mut entity, "Able to control the game's resolution.");
        }
        app.world_mut().flush();
        assert_eq!(
            app.world().get::<OptionsTooltip>(control),
            Some(&OptionsTooltip(
                "Able to control the game's resolution.".to_string()
            ))
        );
    }
}

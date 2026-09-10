//! Restore and record the positions of the windows the original persists.
//!
//! Idea: `hud/game_window.rs` gives every framed window a title-bar drag, and
//! until now the client threw the result away on despawn — reopening a window
//! always snapped it back to its spawn anchor. The original remembers ten of
//! its windows in `wndpos.dat`; we reproduce that *set* through
//! [`WndPosSlot`] and store the anchor in `user_settings.yaml` instead of the
//! original's file (see `settings::window_positions` for why).
//!
//! A window opts in by carrying [`PersistedWindow`] on its root. Everything
//! else here is two systems:
//!
//! * **restore** — on spawn, apply the saved anchor if there is one. It rides
//!   `Added<PersistedWindow>` rather than a load-time latch, so a window that
//!   opens after the settings file loads still gets its position (#647: the
//!   apply mechanism is change detection, never `Plugin::build`).
//! * **record** — on mouse-button *release*, copy each persisted window's
//!   current anchor into [`GameOptions`]. Writing during the drag would push a
//!   new value every frame, and `persistence::save_on_change` would rewrite
//!   the settings file every frame with it.
//!
//! **The restore is clamped, and that clamp is ours, not the data's.** A saved
//! position can be off-screen — the original's own sample stores `y = -8`, and
//! a resolution change can strand a window far outside the viewport. Since our
//! title bar *is* the drag handle, an off-screen title bar is an unrecoverable
//! window for a player who cannot drag it back (WCAG 2.2 AA, and
//! `docs/re/ui/wndpos-persistence.md` §8 step 5 calls for exactly this). So the
//! restore keeps the title bar reachable. No fidelity is lost: we never load
//! the original's file, so its legal negative offsets never reach this code.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::plugins::hud::game_window::{WindowAnchor, WindowDragged};
use crate::plugins::settings::options::GameOptions;
use crate::plugins::settings::window_positions::WndPosSlot;

/// Marks a window root whose position is remembered, and which slot it is.
#[derive(Component, Debug, Clone, Copy)]
pub struct PersistedWindow(pub WndPosSlot);

/// How much of a restored window must stay inside the viewport for its title
/// bar to remain grabbable. One title-bar height's worth of chrome.
const MIN_VISIBLE_PX: f32 = 48.0;

pub struct WindowPositionsPlugin;

impl Plugin for WindowPositionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SessionWindowAnchors>().add_systems(
            Update,
            (
                restore_window_positions,
                fit_windows_into_view,
                record_window_positions,
                record_session_anchors,
            ),
        );
    }
}

/// Clamp a saved `(right, top)` anchor so the title bar stays reachable in a
/// `viewport`-sized window for a window of `size`.
///
/// The root is right/top anchored, so the window spans
/// `x = viewport.0 - right - width ..= viewport.0 - right`.
pub fn clamp_anchor(anchor: (f32, f32), size: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    let (right, top) = anchor;
    // at least MIN_VISIBLE_PX of the window inside the left and right edges
    let right_min = MIN_VISIBLE_PX - size.0;
    let right_max = (viewport.0 - MIN_VISIBLE_PX).max(right_min);
    // the title bar sits at the very top of the window, so it must not go
    // above the viewport at all — unlike the side edges there is nothing left
    // to grab once it does
    let top_max = (viewport.1 - MIN_VISIBLE_PX).max(0.0);
    (right.clamp(right_min, right_max), top.clamp(0.0, top_max))
}

/// Apply the saved anchor to a window that just spawned.
fn restore_window_positions(
    options: Res<GameOptions>,
    primary: Query<&Window, With<PrimaryWindow>>,
    mut windows: Query<(&PersistedWindow, &mut Node), Added<PersistedWindow>>,
) {
    if windows.is_empty() {
        return;
    }
    let viewport = primary
        .single()
        .map(|w| (w.width(), w.height()))
        .unwrap_or((1280.0, 720.0));
    for (persisted, mut node) in windows.iter_mut() {
        let Some(anchor) = options.windows.get(persisted.0) else {
            continue;
        };
        let (Val::Px(w), Val::Px(h)) = (node.width, node.height) else {
            continue;
        };
        let (right, top) = clamp_anchor(anchor, (w, h), viewport);
        node.right = Val::Px(right);
        node.top = Val::Px(top);
    }
}

/// Where the player left windows that the original does **not** persist.
///
/// `wndpos.dat` has exactly ten slots ([`WndPosSlot`]), and the NPC dialog,
/// the teleport board and the other conversation windows are not among them —
/// inventing an eleventh slot would put made-up data in a file format we
/// transcribed. But "not saved to disk" is not the same as "forgets between
/// two clicks": the teleport board jumped back to its spawn anchor on every
/// page turn, and the NPC dialog on every reopen (2026-08-17).
///
/// So these windows keep their anchor for the session, keyed by the shell's
/// window `Name`. One rule for every non-persisted window instead of a special
/// case per window — the same reason [`WindowAnchor`] exists.
#[derive(Resource, Default)]
pub struct SessionWindowAnchors(pub std::collections::HashMap<String, (f32, f32)>);

/// A stable identity for a window whose title is not one (the NPC dialog is
/// titled with the NPC's name, so its `Name` changes per conversation).
/// Windows that need one insert it on their root; everything else is keyed by
/// the shell's `Name`, which is stable for a fixed title.
#[derive(Component, Clone, Copy)]
pub struct SessionAnchorKey(pub &'static str);

/// Remember a dragged non-persisted window's anchor for this session.
fn record_session_anchors(
    buttons: Res<ButtonInput<MouseButton>>,
    mut anchors: ResMut<SessionWindowAnchors>,
    windows: Query<
        (&Name, Option<&SessionAnchorKey>, &Node),
        (With<WindowDragged>, Without<PersistedWindow>),
    >,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    for (name, key, node) in windows.iter() {
        let (Val::Px(right), Val::Px(top)) = (node.right, node.top) else {
            continue;
        };
        anchors.0.insert(session_key(name, key), (right, top));
    }
}

/// The session map's key: an explicit [`SessionAnchorKey`] if the window has
/// one, else its shell `Name`.
fn session_key(name: &Name, key: Option<&SessionAnchorKey>) -> String {
    key.map(|key| key.0.to_string())
        .unwrap_or_else(|| name.to_string())
}

/// Pull a window's *spawn* anchor inside the viewport.
///
/// [`clamp_anchor`] answers a different question — "can the player still grab
/// the title bar" — and is right for a position the player chose. A spawn
/// anchor is not chosen by anybody: it is transcribed design data, applied to
/// a window whose size was multiplied by `hud_scale`, so it can put a
/// perfectly correct rect off the screen. Here the rule is stronger: show the
/// whole window if it fits at all, and only fall back to the reachability
/// clamp when it cannot.
pub fn fit_anchor(anchor: (f32, f32), size: (f32, f32), viewport: (f32, f32)) -> (f32, f32) {
    let (right, top) = clamp_anchor(anchor, size, viewport);
    let right_max = viewport.0 - size.0;
    let top_max = viewport.1 - size.1;
    (
        if right_max >= 0.0 {
            right.clamp(0.0, right_max)
        } else {
            right
        },
        if top_max >= 0.0 {
            top.clamp(0.0, top_max)
        } else {
            top
        },
    )
}

/// Keep every freshly spawned window inside the viewport.
///
/// Skips windows the player has placed themselves ([`WindowDragged`]) and
/// persisted ones (those go through [`restore_window_positions`], which
/// applies the reachability clamp to a *chosen* position). Runs continuously
/// so a resized window re-fits, which is the same reason the restore does.
fn fit_windows_into_view(
    windows: Query<&Window, With<PrimaryWindow>>,
    session: Res<SessionWindowAnchors>,
    mut roots: Query<
        (
            &WindowAnchor,
            &Name,
            Option<&SessionAnchorKey>,
            &ComputedNode,
            &mut Node,
        ),
        (Without<WindowDragged>, Without<PersistedWindow>),
    >,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let viewport = (window.width(), window.height());
    for (anchor, name, key, computed, mut node) in roots.iter_mut() {
        let size = computed.size();
        if size.x <= 0.0 || size.y <= 0.0 {
            continue; // not laid out yet
        }
        // A window the player moved earlier this session reopens where they
        // left it; only its *spawn* anchor is data to be fitted.
        let wanted = session
            .0
            .get(&session_key(name, key))
            .copied()
            .unwrap_or((anchor.right, anchor.top));
        let (right, top) = fit_anchor(wanted, (size.x, size.y), viewport);
        if node.right != Val::Px(right) {
            node.right = Val::Px(right);
        }
        if node.top != Val::Px(top) {
            node.top = Val::Px(top);
        }
    }
}

/// Record where the player let go of a window **they dragged**.
///
/// The [`WindowDragged`] filter is the whole point: this runs on every left
/// button release, so without it a click anywhere on screen persisted the
/// spawn anchor of every open window as if the player had chosen it. That is
/// how `user_settings.yaml` came to hold `MainPopup [24.0, 60.0]` — the
/// inventory's own default — for a window that was never moved (2026-08-17).
fn record_window_positions(
    buttons: Res<ButtonInput<MouseButton>>,
    mut options: ResMut<GameOptions>,
    windows: Query<(&PersistedWindow, &Node), With<WindowDragged>>,
) {
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let mut changed = false;
    for (persisted, node) in windows.iter() {
        let (Val::Px(right), Val::Px(top)) = (node.right, node.top) else {
            continue;
        };
        // bypass_change_detection: writing the map is what decides whether the
        // settings file is rewritten, so only a real move may mark it changed
        changed |= options
            .bypass_change_detection()
            .windows
            .set(persisted.0, (right, top));
    }
    if changed {
        options.set_changed();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Only a window the player dragged may be persisted. Without the
    /// [`WindowDragged`] filter every left-click wrote every open window's
    /// spawn anchor into the settings file as an authored position — the
    /// defect this test exists to keep out (`MainPopup [24.0, 60.0]`, the
    /// inventory's own default, recorded for a window that was never moved).
    #[test]
    fn an_undragged_window_is_never_recorded() {
        fn app_with(dragged: bool) -> App {
            let mut app = App::new();
            app.init_resource::<GameOptions>()
                .init_resource::<ButtonInput<MouseButton>>()
                .add_systems(Update, record_window_positions);
            let mut window = app.world_mut().spawn((
                PersistedWindow(WndPosSlot::MainPopup),
                Node {
                    right: Val::Px(24.0),
                    top: Val::Px(60.0),
                    ..default()
                },
            ));
            if dragged {
                window.insert(WindowDragged);
            }
            let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
            buttons.press(MouseButton::Left);
            buttons.release(MouseButton::Left);
            app.update();
            app
        }

        let untouched = app_with(false);
        assert_eq!(
            untouched
                .world()
                .resource::<GameOptions>()
                .windows
                .get(WndPosSlot::MainPopup),
            None,
            "a window nobody dragged must not be persisted"
        );

        // positive control: the same release *does* record a dragged window,
        // so the test cannot pass by recording nothing at all
        let dragged = app_with(true);
        assert_eq!(
            dragged
                .world()
                .resource::<GameOptions>()
                .windows
                .get(WndPosSlot::MainPopup),
            Some((24.0, 60.0))
        );
    }

    /// A spawn anchor is data, not a player choice: if the window fits at all,
    /// all of it must be visible. The teleport board is the case that forced
    /// this — 464x556 design units become 696x834 px at `hud_scale` 1.5, and
    /// its transcribed `top = 120` put the pager 54 px below a 900 px screen
    /// (2026-08-17). `clamp_anchor` alone accepts that, because the title bar
    /// is still reachable.
    #[test]
    fn a_spawn_anchor_shows_the_whole_window_when_it_fits() {
        let viewport = (1600.0, 900.0);
        let board = (696.0, 834.0);

        let (_, top) = fit_anchor((620.0, 120.0), board, viewport);
        assert!(
            top + board.1 <= viewport.1,
            "the whole board must be on screen, bottom was {}",
            top + board.1
        );

        // an anchor that already fits is left alone
        assert_eq!(
            fit_anchor((120.0, 80.0), (400.0, 300.0), viewport),
            (120.0, 80.0)
        );

        // taller than the screen: nothing to fit, fall back to the reachability
        // clamp rather than inventing a position
        let huge = (400.0, 1200.0);
        let (_, top) = fit_anchor((10.0, 300.0), huge, viewport);
        assert_eq!(top, clamp_anchor((10.0, 300.0), huge, viewport).1);
    }

    /// A non-persisted window reopens where the player left it *this session*
    /// — the NPC dialog and the teleport board are not `wndpos.dat` slots, but
    /// "no file" must not mean "forgets on every page turn" (2026-08-17).
    #[test]
    fn a_session_anchor_beats_the_spawn_anchor_and_is_still_fitted() {
        let viewport = (1600.0, 900.0);
        let size = (400.0, 300.0);

        // remembered position is used verbatim when it fits
        assert_eq!(fit_anchor((300.0, 200.0), size, viewport), (300.0, 200.0));
        // ...and still gets pulled on screen when it does not
        let (_, top) = fit_anchor((300.0, 880.0), size, viewport);
        assert!(top + size.1 <= viewport.1);
    }

    /// A window saved at a sane position is restored verbatim.
    #[test]
    fn a_sane_anchor_survives_the_clamp() {
        let anchor = clamp_anchor((120.0, 80.0), (400.0, 300.0), (1920.0, 1080.0));
        assert_eq!(anchor, (120.0, 80.0));
    }

    /// A position saved on a wider screen, or dragged past an edge, must not
    /// strand the title bar outside the viewport — our title bar is the only
    /// drag handle, so an unreachable one is an unrecoverable window.
    #[test]
    fn an_offscreen_anchor_keeps_the_title_bar_reachable() {
        let viewport = (800.0, 600.0);
        let size = (400.0, 300.0);
        // saved on a 1920-wide screen: `right` far beyond this viewport
        let (right, _) = clamp_anchor((1600.0, 40.0), size, viewport);
        assert!(right <= viewport.0 - MIN_VISIBLE_PX);
        // dragged off the right edge: still MIN_VISIBLE_PX of window on screen
        let (right, _) = clamp_anchor((-900.0, 40.0), size, viewport);
        assert_eq!(right, MIN_VISIBLE_PX - size.0);
        assert!(viewport.0 - right - size.0 <= viewport.0 - MIN_VISIBLE_PX);
        // above the top edge, and below the bottom edge
        assert_eq!(clamp_anchor((10.0, -50.0), size, viewport).1, 0.0);
        assert_eq!(
            clamp_anchor((10.0, 5000.0), size, viewport).1,
            viewport.1 - MIN_VISIBLE_PX
        );
    }

    /// A viewport smaller than the margin must still produce a finite, ordered
    /// clamp rather than panicking on an inverted range.
    #[test]
    fn a_tiny_viewport_does_not_invert_the_clamp() {
        let anchor = clamp_anchor((10.0, 10.0), (400.0, 300.0), (20.0, 20.0));
        assert!(anchor.0.is_finite() && anchor.1.is_finite());
        assert_eq!(anchor.1, 0.0);
    }
}

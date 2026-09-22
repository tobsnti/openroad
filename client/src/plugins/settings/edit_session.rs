//! The options window's edit session: what "Cancel" has to be able to undo.
//!
//! # The idea
//!
//! `Media.pk2/resinfo/ifoption.txt` declares **four** footer buttons, not two:
//! id 53 `UIIT_CTL_APPLY` (`:6`), 52 `UIIT_CTL_CANCEL` (`:25`), 51
//! `UIIT_CTL_CONFIRM` (`:44`) and 50 `UIIT_STT_DEFAULT_VALUE` (`:63`).
//! Four buttons can only mean four different things if the window edits a
//! *working copy*: Apply writes through and stays open, Confirm writes through
//! and closes, Cancel discards, Default loads the factory values into the copy.
//! The data only proves the buttons *exist*; the semantics above are openroad's
//! reading of them.
//!
//! openroad inverts where the copy lives, on purpose: the live [`GameOptions`]
//! resource stays the thing every pane writes to, and this session keeps the
//! **baseline** taken when the window opened. That is the smaller correct diff
//! (no pane changes hands, no second resource for five modules to agree on)
//! *and* it keeps the live preview the audio pane already gives — moving a
//! volume slider is audible while the window is open, which is the behaviour a
//! separate `PendingOptions` copy would have silently removed. Cancel is then
//! "restore the baseline" instead of "drop the copy"; Apply/Confirm re-baseline.
//!
//! Deliberate deviation, stated per ADR 0009: the original almost certainly
//! holds a pending struct (a C++ dialog would), we hold its complement. The
//! observable behaviour of the four buttons is the same; the two differ only in
//! whether a change shows before Apply, and for volume we prefer the audible
//! preview.

use bevy::prelude::*;

use super::options::GameOptions;

/// The baseline the options window was opened with — everything Cancel has to
/// put back. Absent means "no window session has begun".
#[derive(Resource, Debug, Default)]
pub struct OptionsEditSession {
    baseline: Option<GameOptions>,
}

impl OptionsEditSession {
    /// Called when the options window spawns: remember the confirmed state.
    pub fn begin(&mut self, current: &GameOptions) {
        self.baseline = Some(current.clone());
    }

    /// Apply/Confirm: the edited state *is* the confirmed state from now on, so
    /// a later Cancel (Apply keeps the window open — the player can go on
    /// editing and cancel *that*) must not reach behind it.
    pub fn commit(&mut self, current: &GameOptions) {
        self.baseline = Some(current.clone());
    }

    /// Cancel: put the five option-window groups back. Returns false when there
    /// was nothing to restore.
    ///
    /// Only the groups this window edits are restored. `GameOptions` also
    /// carries `windows` (window positions),
    /// which is written by an entirely different screen *while the options
    /// window may be open* — a whole-struct assignment would make Cancel undo a
    /// window drag, which no button in `ifoption.txt` claims to do.
    pub fn revert(&self, live: &mut GameOptions) -> bool {
        let Some(base) = self.baseline.as_ref() else {
            return false;
        };
        live.video = base.video.clone();
        live.audio = base.audio.clone();
        live.gameplay = base.gameplay.clone();
        live.keymap = base.keymap.clone();
        live.camera = base.camera;
        true
    }

    /// Whether the window has unconfirmed edits (used by tests and available to
    /// a future "discard?" prompt).
    pub fn is_dirty(&self, live: &GameOptions) -> bool {
        self.baseline.as_ref().is_some_and(|base| {
            base.video != live.video
                || base.audio != live.audio
                || base.gameplay != live.gameplay
                || base.keymap != live.keymap
                || base.camera != live.camera
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::plugins::settings::options::SightMode;

    fn edited() -> GameOptions {
        let mut o = GameOptions::default();
        o.audio.bgm_volume = 99;
        o.audio.fx_enabled = false;
        o.gameplay.toggles.insert(2001, false);
        o.keymap.bindings.insert(3002, 0x42);
        o.camera.sight = SightMode::Quarter;
        o.video.graphic1.brightness = 4;
        o
    }

    /// The core case of `ifoption.txt`'s four buttons: Cancel is a way back.
    /// Without a baseline there is no difference between Cancel and Confirm.
    #[test]
    fn cancel_restores_every_group_the_window_edits() {
        let confirmed = GameOptions::default();
        let mut session = OptionsEditSession::default();
        session.begin(&confirmed);

        let mut live = edited();
        assert!(session.is_dirty(&live));
        assert!(session.revert(&mut live));

        assert_eq!(live.audio, confirmed.audio);
        assert_eq!(live.gameplay, confirmed.gameplay);
        assert_eq!(live.keymap, confirmed.keymap);
        assert_eq!(live.camera, confirmed.camera);
        assert_eq!(live.video, confirmed.video);
        assert!(!session.is_dirty(&live));
    }

    /// Apply writes through: after it, Cancel has nothing to undo. (That the
    /// window *stays open* is the observer's job, asserted in
    /// `options_window`.)
    #[test]
    fn apply_rebaselines_so_a_later_cancel_keeps_the_applied_values() {
        let mut session = OptionsEditSession::default();
        session.begin(&GameOptions::default());

        let mut live = edited();
        session.commit(&live);
        assert!(!session.is_dirty(&live));

        live.audio.bgm_volume = 7;
        assert!(session.is_dirty(&live));
        session.revert(&mut live);
        assert_eq!(live.audio.bgm_volume, 99, "Apply's value must survive");
    }

    /// Cancel must not undo what another screen did meanwhile: window drags
    /// are not options-window state.
    #[test]
    fn cancel_leaves_window_positions_alone() {
        let mut session = OptionsEditSession::default();
        session.begin(&GameOptions::default());

        let mut live = GameOptions::default();
        let moved = live.windows.clone();
        live.audio.bgm_volume = 1;

        session.revert(&mut live);
        assert_eq!(live.windows, moved);
    }

    /// No session begun (window never opened): nothing to restore, and no panic.
    #[test]
    fn revert_without_a_session_is_a_no_op() {
        let session = OptionsEditSession::default();
        let mut live = edited();
        let before = live.clone();
        assert!(!session.revert(&mut live));
        assert_eq!(live, before);
    }
}

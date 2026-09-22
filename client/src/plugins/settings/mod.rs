//! Player-facing game options: parse the original client's `SROptionSet.dat`
//! (`sroptionset`), model them semantically ([`options::GameOptions`]), and
//! persist openroad's own copy to a writable YAML file (`persistence`). The UI
//! that edits these lives in separate plugins.
//!
//! [`live`] documents (and pins) the one mechanism by which a changed setting
//! reaches its consumer without a restart, and carries the audit of every
//! `ClientConfig` group (#647).

pub mod edit_session;
pub mod keymap;
pub mod live;
pub mod options;
pub mod persistence;
pub mod sroptionset;
pub mod tooltip;
pub mod window_positions;

use bevy::prelude::*;

use options::GameOptions;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameOptions>()
            // The options window's Cancel baseline (`ifoption.txt`'s four
            // footer buttons, see `edit_session`). Lives here rather than in
            // the window plugin so a headless settings test can drive it.
            .init_resource::<edit_session::OptionsEditSession>()
            // The reversible import (B4). Separate from the startup migration
            // below on purpose: this one lands in the live options *while* the
            // window's baseline stands, so Cancel undoes it
            // (`persistence::ImportOriginalSettings`).
            .add_message::<persistence::ImportOriginalSettings>()
            // Chained: the import is a migration *onto* the player's own file,
            // so it has to see what that file loaded and then overwrite it —
            // running the two in an unspecified order would make the result
            // depend on which system Bevy happened to schedule first.
            .add_systems(
                Startup,
                (
                    persistence::load_user_settings,
                    persistence::import_original_settings,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    persistence::apply_import_requests,
                    persistence::save_on_change,
                )
                    .chain(),
            );
    }
}

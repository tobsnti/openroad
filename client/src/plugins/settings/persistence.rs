//! Persist [`GameOptions`] to a SEPARATE, writable YAML file.
//!
//! `config.yaml` stays an author-controlled, read-only input; the player's own
//! tweaks are the machine-written record in `<cwd>/user_settings.yaml`, so the
//! two never clobber each other. Loading is best-effort and never blocks boot:
//! a missing file is the normal first run, a corrupt one is logged and ignored.
//!
//! We never write the original client's `SROptionSet.dat`; [`import_sroptionset`]
//! only reads one in (dormant until the UI wires an import action).

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use super::options::GameOptions;
use super::sroptionset::parse_sroptionset;

/// `<cwd>/user_settings.yaml` — the working-dir-relative path idiom the rest of
/// the client uses (e.g. the screenshot writer).
///
/// `USER_SETTINGS_PATH` overrides it so two clients can run at once (party,
/// exchange, trade and stalls need a second session): both would otherwise
/// write the same file, and the second one to change a setting would silently
/// stamp the first one's window positions and options over it.
pub fn user_settings_path() -> PathBuf {
    if let Ok(path) = std::env::var("USER_SETTINGS_PATH") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push("user_settings.yaml");
    p
}

/// Startup: overwrite the options resource from the saved file if present. A
/// missing file is normal (first run); a corrupt one is logged and ignored so
/// a bad edit can never block boot.
pub fn load_user_settings(mut options: ResMut<GameOptions>) {
    let path = user_settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    match serde_yaml::from_str::<GameOptions>(&text) {
        Ok(loaded) => {
            *options = loaded;
            info!("[settings] loaded user settings from {}", path.display());
        }
        Err(e) => warn!("[settings] ignoring unreadable {}: {e}", path.display()),
    }
}

/// Update: write the file whenever the options actually change. The first
/// observation only seeds the baseline (no write), so merely loading or
/// defaulting never creates / rewrites the file — only a real change does.
/// Writes are best-effort: a failure is logged, never fatal.
pub fn save_on_change(options: Res<GameOptions>, mut last: Local<Option<GameOptions>>) {
    match last.as_ref() {
        Some(prev) if *prev == *options => {}
        None => *last = Some(options.clone()),
        Some(_) => {
            write_user_settings(&options);
            *last = Some(options.clone());
        }
    }
}

/// Serialize and write the options file. Best-effort: a failure is logged,
/// never fatal — a settings file is not worth killing a session over.
fn write_user_settings(options: &GameOptions) {
    let path = user_settings_path();
    match serde_yaml::to_string(options) {
        Ok(yaml) => {
            if let Err(e) = std::fs::write(&path, yaml) {
                warn!("[settings] failed to write {}: {e}", path.display());
            } else {
                info!("[settings] saved user settings to {}", path.display());
            }
        }
        Err(e) => warn!("[settings] failed to serialize settings: {e}"),
    }
}

/// Import-only: merge an original-client `SROptionSet.dat` into `options`,
/// returning how many records were decoded. Never round-trips or overwrites
/// the `.dat` — openroad persists to YAML only, and writing a format we have
/// never observed would mean inventing the artifact.
///
/// Merging rather than replacing is deliberate; see
/// [`GameOptions::apply_records`].
pub fn import_sroptionset(
    options: &mut GameOptions,
    path: impl AsRef<Path>,
) -> std::io::Result<usize> {
    let bytes = std::fs::read(path)?;
    let decoded = parse_sroptionset(&bytes);
    options.apply_records(&decoded.records);
    Ok(decoded.records.len())
}

/// Where a runtime import reads from when nobody names a file: the same
/// `SROPTIONSET_IMPORT` variable the startup migration uses, else
/// `<cwd>/SROptionSet.dat`.
///
/// The bare filename is the original's own (`C:\SRO\<install>\setting\
/// SROptionSet.dat`); asking the player to drop that file next to the client is
/// the cheapest thing that needs no file dialog, and the variable stays the
/// escape hatch for any other location.
pub fn sroptionset_import_path() -> PathBuf {
    if let Ok(path) = std::env::var("SROPTIONSET_IMPORT") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    let mut p = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    p.push("SROptionSet.dat");
    p
}

/// "Read an original `SROptionSet.dat` into the options I am editing right now."
///
/// # Why this exists next to the startup migration
///
/// There are deliberately **two** ways in, and they are not redundant:
///
/// * [`import_original_settings`] (`Startup`, `SROPTIONSET_IMPORT`) is the
///   one-shot *migration*. It runs before any options window has opened, so
///   there is no baseline behind it and nothing to take it back with — that is
///   fine for "adopt my old client's settings once, then forget the variable",
///   and it is the only path that writes `user_settings.yaml` itself.
/// * This message is the *reversible* way. It lands in the live [`GameOptions`]
///   — the working copy every options pane writes to — and leaves
///   `OptionsEditSession`'s baseline alone, so Cancel puts the whole import
///   back (`settings::edit_session`).
///
/// Neither replaces the other; removing one removes a property (one-shot
/// before-boot migration, or undoable in-window import), so please do not
/// tidy them into one.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ImportOriginalSettings {
    /// `None` = [`sroptionset_import_path`].
    pub path: Option<PathBuf>,
}

/// Applies [`ImportOriginalSettings`] into the live options.
///
/// Does **not** write the file: the import is an edit like any other, and
/// `save_on_change` persists it on the next frame exactly as it persists a
/// slider drag — which is also what makes a following Cancel write the old
/// values straight back.
pub fn apply_import_requests(
    mut requests: MessageReader<ImportOriginalSettings>,
    mut options: ResMut<GameOptions>,
) {
    for request in requests.read() {
        let path = request.path.clone().unwrap_or_else(sroptionset_import_path);
        match import_sroptionset(&mut options, &path) {
            Ok(count) => info!(
                "[settings] imported {count} option records from {} (Cancel takes it back)",
                path.display()
            ),
            Err(e) => warn!("[settings] could not import {}: {e}", path.display()),
        }
    }
}

/// `SROPTIONSET_IMPORT=<path>`: a one-shot migration of the player's original
/// client settings, run at `Startup` **after** [`load_user_settings`].
///
/// Idea: the importer needs *a* caller, and an env var is the cheapest honest
/// one: it is the same idiom the rest of this client already uses for
/// operator-facing switches (`USER_SETTINGS_PATH` above, `RESOLUTION`,
/// `NETCHECK`, `SCREENSHOT`), it needs no UI, and unlike a cargo feature it
/// actually *runs*.
///
/// It writes `user_settings.yaml` immediately instead of relying on
/// [`save_on_change`]: that system's first observation only seeds its baseline,
/// so an import applied before it ever ran would apply this session and then
/// vanish. Writing here also makes the migration one-shot in practice — the
/// values are in the player's own file afterwards and the variable can go.
pub fn import_original_settings(mut options: ResMut<GameOptions>) {
    let Ok(path) = std::env::var("SROPTIONSET_IMPORT") else {
        return;
    };
    if path.trim().is_empty() {
        return;
    }
    match import_sroptionset(&mut options, &path) {
        Ok(count) => {
            info!("[settings] imported {count} option records from {path}");
            write_user_settings(&options);
        }
        Err(e) => warn!("[settings] could not import {path}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::settings::options::SightMode;

    /// A minimal, hand-built `SROptionSet.dat`: the 9-byte opaque header, then
    /// `id:u16 | 0u16 | value` records whose width the id decides
    /// (`docs/formats/sroptionset.md`). Byte order is little-endian
    /// throughout, which is what `sroptionset::parse_sroptionset` reads with
    /// `get_u32_le` / `get_u16_le`.
    fn synthetic_dat() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&0u32.to_le_bytes()); // unk_uint0
        v.push(0); // unk_byte0
        v.extend_from_slice(&0u32.to_le_bytes()); // unk_uint1
                                                  // 1001 BackgroundVolumeSlider = 42 (u32)
        v.extend_from_slice(&1001u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&42u32.to_le_bytes());
        // 2012 MonsterNameCheckbox = false (bool)
        v.extend_from_slice(&2012u16.to_le_bytes());
        v.extend_from_slice(&0u16.to_le_bytes());
        v.push(0);
        v
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "openroad-sroptionset-{tag}-{}.dat",
            std::process::id()
        ))
    }

    /// The import path end to end: a synthetic `.dat` reaches the live
    /// options.
    #[test]
    fn an_import_folds_the_files_records_onto_the_current_options() {
        let path = temp_path("merge");
        std::fs::write(&path, synthetic_dat()).expect("write the fixture");

        let mut options = GameOptions::default();
        let count = import_sroptionset(&mut options, &path).expect("import");
        let _ = std::fs::remove_file(&path);

        assert_eq!(count, 2, "both records decode");
        assert_eq!(options.audio.bgm_volume, 42);
        assert_eq!(options.gameplay.toggles.get(&2012), Some(&false));
    }

    /// The merge is the whole reason `apply_records` exists: `SROptionSet.dat`
    /// has no id for the camera sight mode and no concept of window positions,
    /// so importing through a default set would quietly reset both as the
    /// price of bringing the player's volumes across.
    #[test]
    fn an_import_leaves_settings_the_file_cannot_describe_alone() {
        let path = temp_path("preserve");
        std::fs::write(&path, synthetic_dat()).expect("write the fixture");

        let mut options = GameOptions::default();
        options.camera.sight = SightMode::Quarter;
        import_sroptionset(&mut options, &path).expect("import");
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            options.camera.sight,
            SightMode::Quarter,
            "the camera mode is not in the file and must survive the import"
        );
        assert_eq!(options.audio.bgm_volume, 42, "and the file still applied");
    }

    /// A path that is not there is an `Err`, never a panic — the import runs
    /// at `Startup`, so a typo in the variable must not kill the boot.
    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let mut options = GameOptions::default();
        let err = import_sroptionset(&mut options, temp_path("absent-on-purpose"));
        assert!(err.is_err());
        assert_eq!(options, GameOptions::default(), "nothing was half-applied");
    }

    /// Untrusted file input: garbage decodes to zero records and changes
    /// nothing, rather than erroring or panicking (`sroptionset.rs` stops at
    /// the first id whose width it does not know).
    #[test]
    fn a_garbage_file_imports_nothing_and_changes_nothing() {
        let path = temp_path("garbage");
        std::fs::write(&path, [0xFFu8; 32]).expect("write the fixture");

        let mut options = GameOptions::default();
        let count = import_sroptionset(&mut options, &path).expect("import");
        let _ = std::fs::remove_file(&path);

        assert_eq!(count, 0);
        assert_eq!(options, GameOptions::default());
    }

    /// An import performed while the options window is open is an edit like any
    /// other, so `OptionsEditSession`'s baseline takes it back. If this ever
    /// fails, the import has started writing somewhere the window's Cancel
    /// cannot reach.
    #[test]
    fn a_requested_import_lands_in_the_working_copy_and_cancel_undoes_it() {
        let path = temp_path("session");
        std::fs::write(&path, synthetic_dat()).expect("write the fixture");

        let mut app = App::new();
        app.init_resource::<GameOptions>();
        app.add_message::<ImportOriginalSettings>();
        app.add_systems(Update, apply_import_requests);

        // The window opened: baseline = what the player had before.
        let before = app.world().resource::<GameOptions>().clone();
        let mut session = crate::plugins::settings::edit_session::OptionsEditSession::default();
        session.begin(&before);

        app.world_mut().write_message(ImportOriginalSettings {
            path: Some(path.clone()),
        });
        app.update();
        let _ = std::fs::remove_file(&path);

        assert_eq!(
            app.world().resource::<GameOptions>().audio.bgm_volume,
            42,
            "the import reached the live options"
        );
        assert!(session.is_dirty(app.world().resource::<GameOptions>()));

        let mut live = app.world_mut().resource_mut::<GameOptions>();
        assert!(session.revert(&mut live));
        assert_eq!(*live, before, "Cancel put the whole import back");
    }

    /// A missing file must not poison the session: the options stay exactly as
    /// they were and the click is merely logged.
    #[test]
    fn a_requested_import_of_a_missing_file_changes_nothing() {
        let mut app = App::new();
        app.init_resource::<GameOptions>();
        app.add_message::<ImportOriginalSettings>();
        app.add_systems(Update, apply_import_requests);

        app.world_mut().write_message(ImportOriginalSettings {
            path: Some(temp_path("absent-request")),
        });
        app.update();

        assert_eq!(
            *app.world().resource::<GameOptions>(),
            GameOptions::default()
        );
    }

    /// Without a path the button reads the conventional location, which is the
    /// original's own file name next to the client unless `SROPTIONSET_IMPORT`
    /// says otherwise.
    #[test]
    fn the_default_import_path_is_the_originals_file_name() {
        assert!(
            std::env::var("SROPTIONSET_IMPORT").is_err(),
            "this test asserts the default environment; something set the var"
        );
        assert_eq!(
            sroptionset_import_path()
                .file_name()
                .and_then(|n| n.to_str()),
            Some("SROptionSet.dat")
        );
    }

    /// Without the variable the system is a no-op. It runs at `Startup` in
    /// every session, so "does nothing unless asked" is the load-bearing half
    /// of its behaviour.
    #[test]
    fn without_the_env_var_the_import_system_touches_nothing() {
        assert!(
            std::env::var("SROPTIONSET_IMPORT").is_err(),
            "this test asserts the default environment; something set the var"
        );
        let mut app = App::new();
        app.init_resource::<GameOptions>();
        app.add_systems(Update, import_original_settings);
        app.update();
        assert_eq!(
            *app.world().resource::<GameOptions>(),
            GameOptions::default()
        );
    }
}

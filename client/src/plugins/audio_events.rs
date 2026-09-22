//! Event sounds: the `effectsound.txt` **handle** path.
//!
//! The idea: the original client never names a `.wav` at a UI/game event. It
//! names a *handle* (`snd_window_open`, `snd_error`, `snd_levup`, `snd_repair`,
//! `snd_equip`, ...) and `effectsound.txt` turns each into a file *and* a
//! per-row volume. So the call site here builds a [`SoundAddress`] and this
//! module answers it; which `.wav` plays and how loud is data, exchangeable
//! without a code change.
//!
//! Three consequences that are deliberate, not oversights:
//!
//! * **A handle with no row plays nothing.** 12 of the 66 literals
//!   (`SND_WARP`, `SND_THROW`, `SND_DROP`, …) have zero rows in the shipped
//!   table: the original can raise them and the data stays silent. Silence is
//!   the original's answer, so [`play`] returns without a warning rather than
//!   substituting anything.
//! * **The row's `volume` column scales the FX channel, it does not replace
//!   it.** Otherwise `Elixir_Use` (50) would be as loud as `Elixir_Suc` (100).
//! * **Several rows at one address are picked at random.** That is our
//!   decision, not something the data states — how the original chooses among
//!   the 2 `SND_BUTTON_CLICK` or the 3 `SND_REPAIR` rows is not derivable from
//!   the data.
//!   Rationale for random over `first()`: the table ships `uibutton_a.wav`
//!   *and* `uibutton_b.wav`, `itRepair_a/b/c.wav` — shipping three files that
//!   differ only by suffix and then always playing `_a` would make two of them
//!   dead data, which is the less likely reading of the authoring intent.
//!   ADR-0009: a deviation with a written rationale is legitimate; the defect
//!   would be leaving it unmarked.

use bevy::audio::Volume;
use bevy::prelude::*;
use rand::Rng;

use crate::assets::textdata::effectsound::{EffectSound, SoundAddress};
use crate::plugins::hud::toast::{ShowToast, ToastKind};
use crate::plugins::net::entities::NetworkEntities;
use crate::plugins::player::Player;
use crate::plugins::settings::options::{AudioOptions, GameOptions};
use crate::plugins::textdata::{ClientEffectSounds, ClientItemData};

use packets::agent::prelude::{EntityEquip, EntityLevelUp, ItemRepairResponse};

/// The four sounds `scenes/intro_v2/assets.rs` still loads by fixed path, and
/// the table address each of them is a copy of.
///
/// Those four `#[asset(path = "data://prim/snd/ui/…")]` fields are the
/// pre-table fallback: `AssetCollection` needs a string literal at compile
/// time, so they cannot be fed from `effectsound.txt` in place. They are a
/// *second* truth about the same four rows — and a fallback that silently
/// drifts from the data is exactly the failure this module exists to prevent,
/// so the drift is pinned by [`test::intro_fallback_paths_match_the_table`]
/// instead of being trusted. The fallback also cannot carry the rows' volume
/// column (80); that is accepted for the handful of frames before the table
/// loads, not for normal playback.
///
/// The third entry's `error.wav` is **lowercase against the table on purpose**.
/// `UI / SND_ERROR` names `ui\Error.wav`, capital E — while the file the
/// archive ships is `Prim/snd/ui/error.wav`, capital P and lowercase e (both
/// spellings occur in `Media.pk2`/`Data.pk2`). The data has no single
/// spelling, so these literals are written in the one form both normalizations
/// agree on: [`EffectSound::path`] is lowercased at parse time and `bevy_pk2`
/// indexes case-insensitively (`bevy_pk2/src/pk2/archive.rs:66`). Comparing
/// against the raw column instead would make this list fail for a difference
/// that cannot reach a lookup — see
/// [`test::the_error_row_is_capitalised_in_the_data_and_lowercased_by_the_parser`].
///
/// Removing the fields outright is the right end state, but it edits seven
/// files outside the scope of this change (`login_form.rs`, `server_select.rs`,
/// `captcha.rs`, `character_create.rs`, `character_select.rs`, `net.rs`,
/// `region_select.rs`), so it is left to the owner of those files.
///
/// The original raises `snd_window_open`, `snd_button_click` and `snd_error`
/// from its pregame screens; `snd_window_close` only from in-game windows.
/// `SND_WINDOW_CLOSE` is listed anyway because *our* shard window plays it on
/// close (`server_select.rs`) — a deviation owned by that file, not a table
/// gap.
pub const INTRO_FALLBACK_SOUNDS: [(&str, &str, &str); 4] = [
    ("UI", "SND_BUTTON_CLICK", "prim/snd/ui/uibutton_a.wav"),
    ("UI", "SND_WINDOW_OPEN", "prim/snd/ui/uiwinopen.wav"),
    ("UI", "SND_WINDOW_CLOSE", "prim/snd/ui/uiwinclose.wav"),
    ("UI", "SND_ERROR", "prim/snd/ui/error.wav"),
];

/// `UI / SND_ERROR` -> `ui\Error.wav` (80).
const ERROR: (&str, &str) = ("UI", "SND_ERROR");
/// `UI / SND_LEVUP` -> `ui\itlevelup.wav` (80).
const LEVUP: (&str, &str) = ("UI", "SND_LEVUP");
/// `UI / SND_REPAIR` -> `ui\itRepair_a|b|c.wav` (80).
const REPAIR: (&str, &str) = ("UI", "SND_REPAIR");
/// `ITEM / SND_EQUIP` -> one row per equipment class (38 rows, `event1` is
/// the class).
const EQUIP: (&str, &str) = ("ITEM", "SND_EQUIP");

/// `SND_LEVUP` is keyed by **race** in `event1`, not by the bare handle:
/// the shipped table has three rows, `CHINESS` / `UROPIAN` / `ARABIAN`, all
/// three `ui\itlevelup.wav` volume 80 (`effectsound_levup_rows_agree`). We do not
/// have the local character's race as a resource, so the address is resolved
/// against the three keys in table order and the first hit wins. That is
/// sound **only while the three rows agree**, which is exactly what the test
/// pins; if a data set ever differs, the test fails and forces a real race
/// lookup rather than silently playing the Chinese row to a European.
const LEVUP_RACES: [&str; 3] = ["CHINESS", "UROPIAN", "ARABIAN"];

pub struct AudioEventsPlugin;

impl Plugin for AudioEventsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                // Every hook below reads a `Messages<T>` that only exists once
                // the plugin owning it is in the app. The headless netcheck
                // harness builds `MinimalPlugins + NetworkCorePlugin` and has
                // neither the HUD nor combat, and Bevy panics the schedule on
                // a missing system parameter instead of skipping the system
                // (AGENTS.md), so each one is gated on its own message store.
                play_error_sound.run_if(resource_exists::<Messages<ShowToast>>),
                play_level_up_sound.run_if(resource_exists::<Messages<EntityLevelUp>>),
                play_repair_sound.run_if(resource_exists::<Messages<ItemRepairResponse>>),
                play_equip_sound.run_if(resource_exists::<Messages<EntityEquip>>),
            ),
        );
    }
}

/// Anything that can answer a [`SoundAddress`] with the rows the table holds.
///
/// Exists so this module's helpers can be exercised against a parsed
/// [`EffectSoundTable`](crate::assets::textdata::effectsound::EffectSoundTable)
/// in a test without standing up the Bevy resource that wraps it in the app.
pub trait SoundLookup {
    fn rows(&self, address: &SoundAddress) -> &[EffectSound];
}

impl SoundLookup for ClientEffectSounds {
    fn rows(&self, address: &SoundAddress) -> &[EffectSound] {
        self.sounds(address)
    }
}

impl SoundLookup for crate::assets::textdata::effectsound::EffectSoundTable {
    fn rows(&self, address: &SoundAddress) -> &[EffectSound] {
        self.sounds(address)
    }
}

/// Pick one of the rows registered at an address, given a roll in `0.0..1.0`.
///
/// Split out from [`play`] so the choice is testable without an audio device
/// and without a rng. See the module doc for why this is random at all.
pub fn pick_variant(rows: &[EffectSound], roll: f32) -> Option<&EffectSound> {
    if rows.is_empty() {
        return None;
    }
    let index = ((roll.clamp(0.0, 1.0) * rows.len() as f32) as usize).min(rows.len() - 1);
    rows.get(index)
}

/// Fold the row's own `volume` column into the FX channel's playback.
///
/// The row volume is a *scale*, never a replacement: `fx_playback()` is the
/// user's setting and the table says how loud this particular sound sits
/// inside it.
pub fn playback_for(channel: PlaybackSettings, row: &EffectSound) -> PlaybackSettings {
    PlaybackSettings {
        volume: Volume::Linear(channel.volume.to_linear() * row.gain()),
        ..channel
    }
}

/// Play the sound the data registers at `address`, or nothing at all.
///
/// `None` happens for three different reasons that all mean the same thing at
/// this call site: FX is muted, the table has not loaded yet, or the handle
/// has no row (the 12 mute handles). None of them is an error.
pub fn play(
    commands: &mut Commands,
    asset_server: &AssetServer,
    sounds: &impl SoundLookup,
    audio: &AudioOptions,
    address: &SoundAddress,
) {
    let Some(channel) = audio.fx_playback() else {
        return;
    };
    let rows = sounds.rows(address);
    let Some(row) = pick_variant(rows, rand::rng().random::<f32>()) else {
        return;
    };
    let settings = playback_for(channel, row);
    // Every one of the reasons above returns *silently* on purpose, so without
    // this line "nothing played" and "the wrong thing played" look identical in
    // a live run. One `debug!` at the single point where a playback is actually
    // spawned makes the whole audio layer checkable with
    // `RUST_LOG=client::plugins::audio_events=debug`.
    debug!(
        "audio: {}/{} -> {} at {:.3}",
        address.object,
        address.handle,
        row.asset_path(),
        settings.volume.to_linear()
    );
    commands.spawn((
        AudioPlayer::new(asset_server.load(row.asset_path())),
        settings,
    ));
}

/// The `(object, handle)` pair as an address.
fn address((object, handle): (&str, &str)) -> SoundAddress {
    SoundAddress::new(object, handle)
}

/// `SND_ERROR` on the warning toast — our error/warning path
/// (`hud::toast::ShowToast` with [`ToastKind::Warning`], the `GDR_WARNING_WND`
/// instance id 35). Quest-update and notice toasts stay silent: the original's
/// `snd_error` call sites are error sites, and giving every banner a sound would
/// be an invention.
fn play_error_sound(
    mut reader: MessageReader<ShowToast>,
    options: Res<GameOptions>,
    sounds: Option<Res<ClientEffectSounds>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let Some(sounds) = sounds else {
        reader.clear();
        return;
    };
    for toast in reader.read() {
        if toast.kind == ToastKind::Warning {
            play(
                &mut commands,
                &asset_server,
                sounds.as_ref(),
                &options.audio,
                &address(ERROR),
            );
        }
    }
}

/// The address `SND_LEVUP` actually answers to (see [`LEVUP_RACES`]).
fn levup_address(sounds: &impl SoundLookup) -> Option<SoundAddress> {
    LEVUP_RACES
        .iter()
        .map(|race| address(LEVUP).with_events([Some(race), None, None]))
        .find(|addr| !sounds.rows(addr).is_empty())
}

/// `SND_LEVUP` on 0x3054, but only
/// for the local character: the packet reports *any* entity's level-up and a
/// party member gaining a level is not the player's own fanfare.
fn play_level_up_sound(
    mut reader: MessageReader<EntityLevelUp>,
    net: Res<NetworkEntities>,
    players: Query<(), With<Player>>,
    options: Res<GameOptions>,
    sounds: Option<Res<ClientEffectSounds>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let Some(sounds) = sounds else {
        reader.clear();
        return;
    };
    for msg in reader.read() {
        let is_local = net.get(msg.unique_id).is_some_and(|e| players.contains(e));
        if !is_local {
            continue;
        }
        let Some(addr) = levup_address(sounds.as_ref()) else {
            continue;
        };
        play(
            &mut commands,
            &asset_server,
            sounds.as_ref(),
            &options.audio,
            &addr,
        );
    }
}

/// `SND_REPAIR` on a *successful*
/// repair answer (0xB034 `result == 1`). A rejected repair is an error, not a
/// repair, and the original has a separate handle for that.
fn play_repair_sound(
    mut reader: MessageReader<ItemRepairResponse>,
    options: Res<GameOptions>,
    sounds: Option<Res<ClientEffectSounds>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let Some(sounds) = sounds else {
        reader.clear();
        return;
    };
    for response in reader.read() {
        if response.is_success() {
            play(
                &mut commands,
                &asset_server,
                sounds.as_ref(),
                &options.audio,
                &address(REPAIR),
            );
        }
    }
}

/// `event1` of the `ITEM / SND_EQUIP` rows for a weapon, from its `TypeID4`.
///
/// **Ours, and the reason is written down**: the table keys its 38 equip rows
/// by class *names* (`SWORD`, `TSWORD`, `DUELAXE`, …) while the item table
/// keys the same classes by `TypeID4` numbers. Nothing in the shipped data
/// joins the two — the join lives in the original's code. What *is* already
/// committed here is the `TypeID4` legend of the weapon classes
/// (`assets/textdata/itemdata.rs:318-330`, itself derived from the animation
/// group names in the `.bsr` resources), so this maps legend name to the
/// table's own spelling of the same class and goes no further: armour,
/// accessories and the non-weapon rows (`HELM`, `RING`, `POTION`, …) are
/// deliberately left unmapped, because `TypeID3` for those is not covered by
/// any legend we own. An unmapped item simply plays nothing, which is the
/// same answer the data gives for a missing row.
fn equip_event1(type_ids: (u32, u32, u32, u32)) -> Option<&'static str> {
    let (t1, t2, t3, t4) = type_ids;
    if (t1, t2, t3) != (3, 1, 6) {
        return None;
    }
    match t4 {
        2 => Some("SWORD"),     // CH sword
        3 => Some("BLADE"),     // CH blade
        4 => Some("SPEAR"),     // CH spear
        5 => Some("SPEAR"),     // CH glaive — polearm, no own row
        6 => Some("BOW"),       // CH bow
        7 => Some("SWORD"),     // EU one-handed sword
        8 => Some("TSWORD"),    // EU two-handed sword
        9 => Some("DUELAXE"),   // EU dual axes
        10 => Some("WAND"),     // EU warlock rod
        11 => Some("STAFF"),    // EU staff
        12 => Some("CROSSBOW"), // EU crossbow
        13 => Some("DAGGER"),   // EU dagger
        14 => Some("HARP"),     // EU harp
        15 => Some("WAND"),     // EU cleric rod
        _ => None,
    }
}

/// `SND_EQUIP` on 0x3038, local
/// character only — the packet is broadcast for every entity in sight.
fn play_equip_sound(
    mut reader: MessageReader<EntityEquip>,
    net: Res<NetworkEntities>,
    players: Query<(), With<Player>>,
    items: Option<Res<ClientItemData>>,
    options: Res<GameOptions>,
    sounds: Option<Res<ClientEffectSounds>>,
    asset_server: Res<AssetServer>,
    mut commands: Commands,
) {
    let (Some(sounds), Some(items)) = (sounds, items) else {
        reader.clear();
        return;
    };
    for msg in reader.read() {
        let is_local = net.get(msg.unique_id).is_some_and(|e| players.contains(e));
        if !is_local {
            continue;
        }
        let Some(event1) = items
            .get(&(msg.ref_id as i32))
            .and_then(|row| row.type_ids())
            .and_then(equip_event1)
        else {
            continue;
        };
        play(
            &mut commands,
            &asset_server,
            sounds.as_ref(),
            &options.audio,
            &address(EQUIP).with_events([Some(event1), None, None]),
        );
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::assets::textdata::effectsound::EffectSoundTable;

    /// Rows copied verbatim from
    /// `Media.pk2:server_dep/silkroad/textdata/effectsound.txt`:
    /// the click pair, the three window/error singles, the three race-keyed
    /// level-up rows, the three repair variants, one equip row, and
    /// `SND_WARP` is *absent* on purpose — it is one of the 12 handles the
    /// shipped table answers with nothing.
    const ROWS: &str = concat!(
        "\tUI\tSND_BUTTON_CLICK\t-\t-\t-\t-\t0\tui\\\tuibutton_a.wav\t80\tclick#1\t\r\n",
        "\tUI\tSND_BUTTON_CLICK\t-\t-\t-\t-\t0\tui\\\tuibutton_b.wav\t80\tclick#2\t\r\n",
        "\tUI\tSND_WINDOW_OPEN\t-\t-\t-\t-\t0\tui\\\tuiwinopen.wav\t80\topen\t\r\n",
        "\tUI\tSND_WINDOW_CLOSE\t-\t-\t-\t-\t0\tui\\\tuiwinclose.wav\t80\tclose\t\r\n",
        "\tUI\tSND_ERROR\t-\t-\t-\t-\t0\tui\\\tError.wav\t80\terror\t\r\n",
        "\tUI\tSND_LEVUP\t-\tCHINESS\t-\t-\t0\tui\\\titlevelup.wav\t80\tlevup\t\r\n",
        "\tUI\tSND_LEVUP\t-\tUROPIAN\t-\t-\t0\tui\\\titlevelup.wav\t80\tlevup\t\r\n",
        "\tUI\tSND_LEVUP\t-\tARABIAN\t-\t-\t0\tui\\\titlevelup.wav\t80\tlevup\t\r\n",
        "\tUI\tSND_REPAIR\t-\t-\t-\t-\t0\tui\\\titRepair_a.wav\t80\trepair\t\r\n",
        "\tUI\tSND_REPAIR\t-\t-\t-\t-\t0\tui\\\titRepair_b.wav\t80\trepair\t\r\n",
        "\tUI\tSND_REPAIR\t-\t-\t-\t-\t0\tui\\\titRepair_c.wav\t80\trepair\t\r\n",
        "\tUI\tSND_ELIXIR_USE\t-\t-\t-\t-\t0\tui\\\tElixir_Use.wav\t50\telixir\t\r\n",
        "\tITEM\tSND_EQUIP\t-\tTSWORD\t-\t-\t0\tui\\\titPolearm.wav\t80\tequip\t\r\n",
    );

    fn table() -> EffectSoundTable {
        EffectSoundTable::parse(ROWS)
    }

    fn channel(volume: f32) -> PlaybackSettings {
        PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume))
    }

    #[test]
    fn row_volume_scales_the_channel_and_never_replaces_it() {
        let t = table();
        let elixir = t
            .first(&SoundAddress::new("UI", "SND_ELIXIR_USE"))
            .expect("UI/SND_ELIXIR_USE");
        // row 50 inside a half-volume FX channel = 0.25, not 0.5
        let folded = playback_for(channel(0.5), elixir);
        assert!((folded.volume.to_linear() - 0.25).abs() < 1e-6);
        // and the rest of the channel's settings survive the fold
        assert!(matches!(folded.mode, bevy::audio::PlaybackMode::Despawn));
    }

    #[test]
    fn a_row_without_a_volume_column_plays_at_channel_level() {
        let row = EffectSound {
            path: String::from("prim/snd/ui/x.wav"),
            volume: None,
        };
        let folded = playback_for(channel(0.75), &row);
        assert!((folded.volume.to_linear() - 0.75).abs() < 1e-6);
    }

    /// The point of the whole module: a handle the table does not answer is
    /// silent, and that is the *original's* answer, not a failure. Positive
    /// control on the same read path: `SND_WINDOW_OPEN`, looked up exactly the
    /// same way, does resolve.
    #[test]
    fn handle_without_a_row_is_silent() {
        let t = table();
        assert!(pick_variant(t.sounds(&SoundAddress::new("UI", "SND_WARP")), 0.0).is_none());
        assert!(pick_variant(t.sounds(&SoundAddress::new("UI", "SND_WINDOW_OPEN")), 0.0).is_some());
    }

    /// The click still sounds — and now both variants can, which `first()`
    /// never allowed (`uibutton_b.wav` was unreachable).
    #[test]
    fn click_reaches_both_variants() {
        let t = table();
        let rows = t.sounds(&SoundAddress::new("UI", "SND_BUTTON_CLICK"));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            pick_variant(rows, 0.0).map(|r| r.path.as_str()),
            Some("prim/snd/ui/uibutton_a.wav")
        );
        assert_eq!(
            pick_variant(rows, 0.99).map(|r| r.path.as_str()),
            Some("prim/snd/ui/uibutton_b.wav")
        );
        // a roll of exactly 1.0 must not index past the end
        assert!(pick_variant(rows, 1.0).is_some());
    }

    #[test]
    fn repair_reaches_all_three_variants() {
        let t = table();
        let rows = t.sounds(&SoundAddress::new("UI", "SND_REPAIR"));
        let picked: Vec<&str> = [0.0, 0.5, 0.99]
            .iter()
            .filter_map(|roll| pick_variant(rows, *roll).map(|r| r.path.as_str()))
            .collect();
        assert_eq!(
            picked,
            [
                "prim/snd/ui/itrepair_a.wav",
                "prim/snd/ui/itrepair_b.wav",
                "prim/snd/ui/itrepair_c.wav"
            ]
        );
    }

    /// Guards [`LEVUP_RACES`]: resolving the level-up handle without knowing
    /// the character's race is only honest while the three race rows agree.
    #[test]
    fn effectsound_levup_rows_agree() {
        let t = table();
        let rows: Vec<&EffectSound> = LEVUP_RACES
            .iter()
            .map(|race| {
                t.first(&SoundAddress::new("UI", "SND_LEVUP").with_events([Some(race), None, None]))
                    .unwrap_or_else(|| panic!("UI/SND_LEVUP {race}"))
            })
            .collect();
        assert!(rows.windows(2).all(|w| w[0] == w[1]), "{rows:?}");
        let addr = levup_address(&t).expect("a level-up address resolves");
        assert_eq!(
            t.first(&addr).map(|r| r.path.as_str()),
            Some("prim/snd/ui/itlevelup.wav")
        );
    }

    /// The bare handle does **not** resolve — the address needs `event1`.
    #[test]
    fn levup_needs_the_race_key() {
        let t = table();
        assert!(t.first(&SoundAddress::new("UI", "SND_LEVUP")).is_none());
    }

    /// The intro scene's four fixed asset paths must stay the paths the table
    /// names for those handles; see [`INTRO_FALLBACK_SOUNDS`]. Positive
    /// control on the same read path: the addresses do resolve here, so a
    /// failure means the paths disagree, not that the lookup broke.
    #[test]
    fn intro_fallback_paths_match_the_table() {
        let t = table();
        for (object, handle, path) in INTRO_FALLBACK_SOUNDS {
            let rows = t.sounds(&SoundAddress::new(object, handle));
            assert!(!rows.is_empty(), "{object}/{handle} has no row");
            assert!(
                rows.iter().any(|r| r.path == path),
                "{object}/{handle} does not name {path}: {rows:?}"
            );
        }
    }

    /// Why the fallback list may be lowercase where the table is not.
    ///
    /// The `SND_ERROR` row in [`ROWS`] carries the shipped `Error.wav`
    /// verbatim, capital E — and the archive's own file is
    /// `Prim/snd/ui/error.wav`, capital P and lowercase e. The data has no one
    /// spelling, so the invariant the code relies on is not "the literal equals
    /// the column" but "the parser folds case". This pins that fold, so a
    /// parser that ever stopped lowercasing fails here instead of failing as a
    /// silent missing sound on the login screen.
    #[test]
    fn the_error_row_is_capitalised_in_the_data_and_lowercased_by_the_parser() {
        assert!(
            ROWS.contains("\tError.wav\t"),
            "the fixture keeps the data's case"
        );
        let t = table();
        let row = t
            .first(&SoundAddress::new("UI", "SND_ERROR"))
            .expect("UI/SND_ERROR");
        assert_eq!(row.path, "prim/snd/ui/error.wav");
        assert_eq!(row.volume, Some(80));
        // and that is exactly the literal the intro's AssetCollection pins
        assert!(INTRO_FALLBACK_SOUNDS
            .iter()
            .any(|(_, handle, path)| *handle == "SND_ERROR" && *path == row.path));
    }

    #[test]
    fn equip_maps_the_weapon_classes_it_claims() {
        assert_eq!(equip_event1((3, 1, 6, 8)), Some("TSWORD"));
        assert_eq!(equip_event1((3, 1, 6, 2)), Some("SWORD"));
        // not a weapon (3/1/6): deliberately unmapped, plays nothing
        assert_eq!(equip_event1((3, 1, 1, 1)), None);
        assert_eq!(equip_event1((3, 3, 1, 1)), None);
        // and the mapped class does resolve in the table
        let t = table();
        let addr = SoundAddress::new("ITEM", "SND_EQUIP").with_events([
            equip_event1((3, 1, 6, 8)),
            None,
            None,
        ]);
        assert_eq!(
            t.first(&addr).map(|r| r.path.as_str()),
            Some("prim/snd/ui/itpolearm.wav")
        );
    }
}

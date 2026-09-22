//! Semantic option model built on top of the raw [`OptionRecord`] TLV decode.
//!
//! [`GameOptions`] groups the settings the way the original client's option
//! tabs do (Video / Audio / Setting / KeyMap in `OptionSet.csv`). Homogeneous,
//! indexed groups — the graphic-quality sliders and the keymap — are kept as
//! id-keyed maps rather than dozens of hand-named fields (several CSV slots are
//! unnamed anyway); the distinct scalar settings get real names.
//!
//! `Default` is the shipped option set where `SROptionSet.dat` states it, and
//! openroad's own baseline only where the file does not.
//!
//! * **KeyMap** — the block is the shipped binding set, not one player's habit.
//!   [`super::keymap::KEY_ACTIONS`] cites a `.dat` id/VK per entry (the
//!   `textuisystem.txt` L2250-2274 literals agree with every one of them).
//! * **Audio** — ids 1001/1002/1003 hold 30 / 50 / 50 and 1004-1006 all hold 1,
//!   so [`AudioOptions::default`] is those values (see there).
//! * **The Setting toggles** — ids 2001..=2028 are adopted from the file as
//!   well; see [`SHIPPED_TOGGLES`] and `docs/formats/sroptionset.md`.
//! * **Video** is *not* adopted: the video block differs between client
//!   versions, so those defaults stay openroad's own.

use std::collections::BTreeMap;

use bevy::audio::Volume;
use bevy::prelude::{PlaybackSettings, Resource};
use serde::{Deserialize, Serialize};

use super::sroptionset::{OptionRecord, OptionValue};
use super::window_positions::WindowPositions;

/// One of the two graphics profiles the client stores (Graphic 1 / Graphic 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphicProfile {
    /// `Type` (ids 501 / 601) — raw enum value, meaning UNKNOWN.
    #[serde(default)]
    pub display_type: u8,
    /// `Brightness` (ids 502 / 602) — raw slider value, scale UNKNOWN.
    #[serde(default)]
    pub brightness: u8,
    /// `WindowResolutionWidth` (ids 503 / 603).
    #[serde(default = "default_width")]
    pub width: u32,
    /// `WindowResolutionHeight` (ids 504 / 604).
    #[serde(default = "default_height")]
    pub height: u32,
    /// Graphic-quality sliders keyed by id (1..=15 / 101..=115); see the CSV
    /// in `docs/formats/sroptionset.md` for each slot's meaning.
    #[serde(default)]
    pub quality: BTreeMap<u16, u16>,
}

fn default_width() -> u32 {
    1920
}
fn default_height() -> u32 {
    1080
}

impl GraphicProfile {
    /// The stored size **only when the player actually chose one**.
    ///
    /// Idea: `width`/`height` are plain `u32` with a compiled default of
    /// 1920x1080 (`default_width`/`default_height`, the shipped values), so the
    /// struct cannot distinguish "the player picked 1920x1080" from "nobody ever
    /// touched this". Restoring a saved resolution at startup therefore treats
    /// *the default pair* as "unset" and leaves the window to `config.yaml`.
    /// That is a deliberate, stated deviation, not a guess: the alternative is
    /// either ignoring the player's saved size forever or forcing 1920x1080 on
    /// every fresh install. The cost is one case — a player whose chosen size
    /// happens to equal the shipped default keeps `config.yaml`'s window.
    pub fn chosen_size(&self) -> Option<(u32, u32)> {
        (self.width != default_width() || self.height != default_height())
            .then_some((self.width, self.height))
    }
}

impl Default for GraphicProfile {
    fn default() -> Self {
        Self {
            display_type: 0,
            brightness: 0,
            width: default_width(),
            height: default_height(),
            quality: BTreeMap::new(),
        }
    }
}

/// The three camera view modes of the original's Camera options pane.
///
/// Idea: this is **not** an openroad invention. `ifoption_camera.txt` does not
/// say what each mode does geometrically, but `textuisystem.txt` does, in the two
/// description lines the pane itself renders next to each radio:
///
/// * `UIIT_STT_SIGHT_FREE_DESC1/2` — "Mouse oriented camera control" /
///   "Operates on multidirectional angle control and mouse movement"
/// * `UIIT_STT_SIGHT_THIRD_PERSON_DESC1/2` — "Keyboard oriented camera control" /
///   **"Camera angle is fixed behind the character"**
/// * `UIIT_STT_SIGHT_QUATER_VIEW_DESC1` — **"The height is fixed to this
///   perspective."**, confirmed by `UIIT_STT_CHANGED_SIGHT_QUARTER_VIEW_DESC`
///   ("Camera angle has been set to Fixed height point")
///
/// So each mode is defined by *which orbit axis it takes away*: free takes
/// none, third-person fixes yaw behind the character, quarter fixes pitch.
/// That is a transcription; only the exact fixed pitch value is ours
/// (see `camera::QUARTER_VIEW_PITCH`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SightMode {
    /// `UIIT_STT_SIGHT_FREE` — "Free Movement View". Yaw and pitch both follow
    /// the drag. Default because it is what openroad already did, so adopting
    /// the pane does not silently change anybody's camera; the original's own
    /// default is UNKNOWN (no `resinfo/` option tree carries a default value).
    #[default]
    Free,
    /// `UIIT_STT_SIGHT_THIRD_PERSON` — "Third Person View". Yaw is pinned
    /// behind the character; pitch and zoom still respond.
    ThirdPerson,
    /// `UIIT_STT_SIGHT_QUATER_VIEW` — "Quarter Angle View". Pitch is fixed;
    /// yaw and zoom still respond. (The original misspells "Quater" in the key
    /// and spells it "QUARTER" in the pane's `_DESC` key — both spellings are
    /// in the shipped string table.)
    Quarter,
}

impl SightMode {
    /// The three radios in pane order (top to bottom by `Rect` y in
    /// `ifoption_camera.txt`: 45 / 98 / 152).
    pub const ALL: [SightMode; 3] = [SightMode::Free, SightMode::ThirdPerson, SightMode::Quarter];
}

/// Camera view mode (`GDR_OPTION_WND_CAMERA`).
///
/// **Stated non-original storage.** It is unknown whether any `SROptionSet` id
/// covers the sight mode — our parser lumps `2001..=2028` (`sroptionset.rs`)
/// with none broken out — so no id is invented here. It rides the
/// `user_settings.yaml` path with the rest of [`GameOptions`] instead, which is
/// what makes the radio survive a restart.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CameraOptions {
    #[serde(default)]
    pub sight: SightMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoOptions {
    #[serde(default)]
    pub graphic1: GraphicProfile,
    #[serde(default)]
    pub graphic2: GraphicProfile,
    /// `isWindowMode` (id 2015) — windowed vs fullscreen, as a **session-only**
    /// override of `config.yaml`'s `window_settings.mode`.
    ///
    /// `None` means "follow the config", and `#[serde(skip)]` means it can
    /// never be anything else at boot: the field is neither read from nor
    /// written to `user_settings.yaml`, so every launch starts from
    /// `config.yaml` and an in-session toggle lasts only for that session.
    ///
    /// **Stated deviation from the original**, which persists id 2015 across
    /// restarts: `config.yaml` is openroad's single authority for the window,
    /// and a persisted copy here would silently override it. An older file's
    /// `window_mode:` key is deliberately *not* aliased: it is ignored as an
    /// unknown key, which retires a stale value without a migration.
    ///
    /// It still takes part in [`GameOptions`]'s `PartialEq`, so flipping it
    /// does trigger a `persistence::save_on_change` write — harmless, since
    /// the field is simply absent from the emitted YAML.
    #[serde(skip)]
    pub window_mode_override: Option<bool>,
}

impl Default for VideoOptions {
    fn default() -> Self {
        Self {
            graphic1: GraphicProfile::default(),
            graphic2: GraphicProfile::default(),
            window_mode_override: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioOptions {
    /// Volume sliders (ids 1001..=1003). Raw slider values; the *scale* is
    /// still UNKNOWN, the *shipped values* are not — see [`default_bgm_volume`].
    #[serde(default = "default_bgm_volume")]
    pub bgm_volume: u32,
    #[serde(default = "default_fx_volume")]
    pub fx_volume: u32,
    #[serde(default = "default_env_volume")]
    pub env_volume: u32,
    /// Per-channel on/off checkboxes (ids 1004..=1006). `SROptionSet.dat` stores
    /// `1` for all three.
    #[serde(default = "default_true")]
    pub bgm_enabled: bool,
    #[serde(default = "default_true")]
    pub fx_enabled: bool,
    #[serde(default = "default_true")]
    pub env_enabled: bool,
}

/// Background music, `SROptionSet.dat` id 1001 = **30**.
///
/// The scale is still UNKNOWN; 30/50/50 on a 0..=100 slider is the reading
/// [`AudioOptions::gain`] assumes, and it is self-consistent (50 sits mid-track,
/// BGM sits under the effects — the mix the original ships with).
fn default_bgm_volume() -> u32 {
    30
}
/// Sound effects, `SROptionSet.dat` id 1002 = **50**. Same two files.
fn default_fx_volume() -> u32 {
    50
}
/// Environment/ambience, `SROptionSet.dat` id 1003 = **50**. Same two files.
fn default_env_volume() -> u32 {
    50
}
fn default_true() -> bool {
    true
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            bgm_volume: default_bgm_volume(),
            fx_volume: default_fx_volume(),
            env_volume: default_env_volume(),
            bgm_enabled: true,
            fx_enabled: true,
            env_enabled: true,
        }
    }
}

impl AudioOptions {
    /// `PlaybackSettings` for looping BGM, or `None` when BGM is muted.
    pub fn bgm_playback(&self) -> Option<PlaybackSettings> {
        self.bgm_enabled
            .then(|| PlaybackSettings::LOOP.with_volume(Self::gain(self.bgm_volume)))
    }

    /// The same looping BGM settings, but **always** produced — muted BGM is
    /// expressed as a `paused` sink rather than as a missing entity.
    ///
    /// That distinction is what makes the audio group live: a track that was
    /// never spawned because BGM happened to be off at scene entry cannot start
    /// playing when the user turns BGM on, whereas a paused sink can
    /// (`apply_background_music_options`).
    pub fn bgm_playback_settings(&self) -> PlaybackSettings {
        PlaybackSettings {
            paused: !self.bgm_enabled,
            ..PlaybackSettings::LOOP.with_volume(self.bgm_gain())
        }
    }

    /// The BGM gain on its own, independent of the enable toggle: an apply
    /// system needs the intended volume even while the sink is paused.
    pub fn bgm_gain(&self) -> Volume {
        Self::gain(self.bgm_volume)
    }

    /// `PlaybackSettings` for a one-shot sound effect, or `None` when FX is muted.
    pub fn fx_playback(&self) -> Option<PlaybackSettings> {
        self.fx_enabled
            .then(|| PlaybackSettings::DESPAWN.with_volume(Self::gain(self.fx_volume)))
    }

    /// Looping settings for a zone-ambience bed, always produced: like BGM,
    /// a muted environment channel is a *paused* sink, so turning the
    /// Environment row back on starts the bed that was already there
    /// (`plugins::zone_ambience::apply_zone_ambience_options`).
    pub fn env_playback_settings(&self) -> PlaybackSettings {
        PlaybackSettings {
            paused: !self.env_enabled,
            ..PlaybackSettings::LOOP.with_volume(self.env_gain())
        }
    }

    /// The environment gain on its own, for the same reason as `bgm_gain`.
    pub fn env_gain(&self) -> Volume {
        Self::gain(self.env_volume)
    }

    /// `PlaybackSettings` for a one-shot ambient, or `None` while the
    /// environment channel is muted. A one-shot is not worth pausing — it is
    /// simply not spawned, and its schedule keeps running so unmuting picks
    /// the next one up.
    pub fn env_oneshot(&self) -> Option<PlaybackSettings> {
        self.env_enabled
            .then(|| PlaybackSettings::DESPAWN.with_volume(self.env_gain()))
    }

    /// `bgm_volume`/`fx_volume` are read as 0-100 percent — openroad's own
    /// scale, since the original client's SROptionSet slider range (ids
    /// 1001..=1003) is UNKNOWN: it stores 30/50/50, which fits 0..=100 but does
    /// not prove it (`docs/formats/sroptionset.md`). The clamp
    /// keeps a hand-edited `config.yaml` from amplifying past unity gain,
    /// which is the mastered file level; the slider itself is the user's
    /// control, so 100 is not lowered to some "safer" default.
    fn gain(volume: u32) -> Volume {
        Volume::Linear((volume as f32 / 100.0).clamp(0.0, 1.0))
    }
}

/// The `Setting` tab toggles (ids 2001..=2028, excluding 2015 which is video).
/// Kept id-keyed because many CSV slots are unnamed; see the doc table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameplayOptions {
    #[serde(default = "default_toggles")]
    pub toggles: BTreeMap<u16, bool>,
}

/// The shipped state of every `Setting`-tab checkbox, as `SROptionSet.dat`
/// stores it across the whole 2001..=2028 range.
///
/// # The idea
///
/// A blanket "on" for all 27 boxes is not what the original ships: seven of
/// them (HideAbilityPets, the HP/MP warnings, the four condition bars) and two
/// more (2026 / 2028) ship *off*. This table is the source for those values,
/// and [`default_toggle`] is the single place a missing id is answered from.
///
/// Four of these have visible HUD consequences: 2021/2022/2023 and 2024 ship
/// **off**, and nothing reads 2024 today. The shipped value stays as the file
/// states it, so a rebuild inherits it. That is the shipped state, not a
/// preference; the one place to flip it is the table below.
///
/// 2015 (`isWindowMode`, shipped `1`) is deliberately absent: openroad models
/// it as [`VideoOptions::window_mode_override`], a session-only override whose
/// `None` means "follow `config.yaml`", and adopting the file's `1` here would
/// resurrect an override of `config.yaml` at every boot.
pub const SHIPPED_TOGGLES: [(u16, bool); 27] = [
    (2001, true),  // GameGuideCheckBox
    (2002, true),  // PartyInvitationCheckbox
    (2003, true),  // ExchangeRequestCheckbox
    (2004, true),  // PersonalMsgCheckbox
    (2005, true),  // (unnamed in OptionSet.csv; meaning unknown)
    (2006, true),  // (unnamed; meaning unknown)
    (2007, true),  // (unnamed; meaning unknown)
    (2008, true),  // SystemUIAutoHideCheckbox
    (2009, true),  // QuickPartyViewBuffStatusCheckbox
    (2010, true),  // OwnNameCheckbox
    (2011, true),  // OtherNameCheckbox
    (2012, true),  // MonsterNamCheckbox
    (2013, true),  // NPCNameCheckbox
    (2014, true),  // GuildNameCheckbox
    (2016, true),  // HighSettingIntroCheckbox
    (2017, true),  // FortressWarMarkCheckbox
    (2018, false), // HideAbilityPetsCheckbox
    (2019, false), // HPWarningCheckbox
    (2020, false), // MPWarningCheckbox
    (2021, false), // SelfConditionCheckbox
    (2022, false), // COSConditionCheckbox
    (2023, false), // PartyMemberStatusCheckbox
    (2024, false), // MonsterConditionCheckbox
    (2025, true),  // OpenTheGuideCheckbox
    (2026, false), // ActivateActionShortcutCheckbox
    (2027, true),  // CameraRotationMethod1Checkbox
    (2028, false), // CameraRotationMethod2Checkbox
];

/// The shipped table as the map `GameOptions` carries.
fn default_toggles() -> BTreeMap<u16, bool> {
    SHIPPED_TOGGLES.iter().copied().collect()
}

/// The shipped value of one toggle id — what a reader uses when the map has no
/// entry (a hand-edited `user_settings.yaml`, or a partial import).
///
/// Unknown ids answer `true`: every id outside 2001..=2028 is openroad's own
/// invention, and an openroad feature that has to invent its default is better
/// on than silently off. Stated rather than assumed, per ADR 0009.
pub fn default_toggle(id: u16) -> bool {
    SHIPPED_TOGGLES
        .iter()
        .find_map(|(known, value)| (*known == id).then_some(*value))
        .unwrap_or(true)
}

impl Default for GameplayOptions {
    fn default() -> Self {
        Self {
            toggles: default_toggles(),
        }
    }
}

/// Custom-shortcut key bindings. Values are raw Win32 VK codes (u32); the
/// VK -> Bevy `KeyCode` translation is a later UI concern.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyMapOptions {
    /// id (3001..=3099) -> Win32 VK code.
    #[serde(default)]
    pub bindings: BTreeMap<u16, u32>,
    /// `isMouseShortcutSwapped` (id 3101).
    #[serde(default)]
    pub mouse_shortcut_swapped: bool,
}

/// Login-screen state the original remembers between sessions: `RECENTSERVER`
/// is a shard *name* (ids are per gateway session) and prefills the Server row
/// as display state only — it never becomes a selection by itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LoginOptions {
    /// Name of the shard the player last committed with `Select`; empty means
    /// "never selected one".
    #[serde(default)]
    pub recent_server: String,
}

/// openroad's live, persisted player options.
#[derive(Resource, Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GameOptions {
    #[serde(default)]
    pub video: VideoOptions,
    #[serde(default)]
    pub audio: AudioOptions,
    #[serde(default)]
    pub gameplay: GameplayOptions,
    #[serde(default)]
    pub keymap: KeyMapOptions,
    /// The Camera pane's sight mode. Not in `OptionSet.csv`'s tab
    /// grouping because no id for it is identified — see [`CameraOptions`].
    #[serde(default)]
    pub camera: CameraOptions,
    /// Where the player left each of the ten windows the original persists.
    /// Not an `OptionSet.csv` group at all — vanilla keeps this in its
    /// own `wndpos.dat`, which we deliberately never read or write; see
    /// [`super::window_positions`].
    #[serde(default)]
    pub windows: WindowPositions,
    /// Login-screen memory (`RECENTSERVER`); see [`LoginOptions`].
    #[serde(default)]
    pub login: LoginOptions,
}

impl GameOptions {
    /// Fold decoded [`OptionRecord`]s onto a default set. Known ids map to their
    /// semantic field; unknown ids (and value/id type mismatches) are ignored.
    pub fn from_records(records: &[OptionRecord]) -> Self {
        let mut opts = Self::default();
        opts.apply_records(records);
        opts
    }

    /// Fold decoded [`OptionRecord`]s onto **these** options, leaving every
    /// field the file does not mention untouched.
    ///
    /// This is the shape an import needs, and the reason it is not
    /// [`Self::from_records`]: `SROptionSet.dat` has no id for the camera
    /// sight mode and knows nothing about window positions, so importing the
    /// original's file through a default set would silently reset a player's
    /// camera and window layout as the price of bringing their volumes across.
    /// Merging keeps the import to what the file actually says.
    pub fn apply_records(&mut self, records: &[OptionRecord]) {
        for rec in records {
            self.apply(*rec);
        }
    }

    fn apply(&mut self, rec: OptionRecord) {
        let OptionRecord { id, value } = rec;
        match id {
            1..=15 => {
                if let OptionValue::U16(v) = value {
                    self.video.graphic1.quality.insert(id, v);
                }
            }
            101..=115 => {
                if let OptionValue::U16(v) = value {
                    self.video.graphic2.quality.insert(id, v);
                }
            }
            501 => set_u8(&mut self.video.graphic1.display_type, value),
            502 => set_u8(&mut self.video.graphic1.brightness, value),
            503 => set_u32(&mut self.video.graphic1.width, value),
            504 => set_u32(&mut self.video.graphic1.height, value),
            601 => set_u8(&mut self.video.graphic2.display_type, value),
            602 => set_u8(&mut self.video.graphic2.brightness, value),
            603 => set_u32(&mut self.video.graphic2.width, value),
            604 => set_u32(&mut self.video.graphic2.height, value),
            1001 => set_u32(&mut self.audio.bgm_volume, value),
            1002 => set_u32(&mut self.audio.fx_volume, value),
            1003 => set_u32(&mut self.audio.env_volume, value),
            1004 => set_bool(&mut self.audio.bgm_enabled, value),
            1005 => set_bool(&mut self.audio.fx_enabled, value),
            1006 => set_bool(&mut self.audio.env_enabled, value),
            // An imported `SROptionSet.dat` *is* an explicit choice, so it
            // becomes `Some` — but still only for the session it is imported
            // in, like any other flip of this field.
            2015 => set_opt_bool(&mut self.video.window_mode_override, value),
            2001..=2028 => {
                if let OptionValue::Bool(v) = value {
                    self.gameplay.toggles.insert(id, v);
                }
            }
            3101 => set_bool(&mut self.keymap.mouse_shortcut_swapped, value),
            3001..=3099 => {
                if let OptionValue::U32(v) = value {
                    self.keymap.bindings.insert(id, v);
                }
            }
            _ => {}
        }
    }
}

fn set_u8(slot: &mut u8, value: OptionValue) {
    if let OptionValue::U8(v) = value {
        *slot = v;
    }
}
fn set_u32(slot: &mut u32, value: OptionValue) {
    if let OptionValue::U32(v) = value {
        *slot = v;
    }
}
fn set_bool(slot: &mut bool, value: OptionValue) {
    if let OptionValue::Bool(v) = value {
        *slot = v;
    }
}
/// Like [`set_bool`], for a tri-state slot where `None` means "unset". A
/// record that is present in the file is an explicit choice, so it lands as
/// `Some`; a record of the wrong type leaves the slot untouched rather than
/// turning "unset" into a guessed value.
fn set_opt_bool(slot: &mut Option<bool>, value: OptionValue) {
    if let OptionValue::Bool(v) = value {
        *slot = Some(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The startup restore path (`config::window`) asks this method, so the
    /// three answers it can give are pinned here: nothing stored, a real
    /// choice, and the documented blind spot (a player who picks exactly the
    /// shipped default pair keeps `config.yaml`'s window - see the method's own
    /// rationale and `docs/formats/sroptionset.md`).
    #[test]
    fn chosen_size_reports_only_a_size_that_differs_from_the_shipped_default() {
        assert_eq!(GraphicProfile::default().chosen_size(), None);

        let mut profile = GraphicProfile::default();
        profile.width = 1280;
        profile.height = 720;
        assert_eq!(profile.chosen_size(), Some((1280, 720)));

        // One axis is enough to count as chosen.
        let mut tall = GraphicProfile::default();
        tall.height = 1200;
        assert_eq!(tall.chosen_size(), Some((default_width(), 1200)));

        // The known cost, asserted so it cannot change unnoticed.
        let mut shipped = GraphicProfile::default();
        shipped.width = default_width();
        shipped.height = default_height();
        assert_eq!(shipped.chosen_size(), None);
    }

    /// The shipped Setting-tab defaults are the ones `SROptionSet.dat` holds,
    /// not "everything on". Pinned here so a later "looks nicer with everything
    /// on" edit has to argue with the file.
    #[test]
    fn the_setting_toggle_defaults_come_from_the_dat_files() {
        let off = [2018u16, 2019, 2020, 2021, 2022, 2023, 2024, 2026, 2028];
        let options = GameOptions::default();

        for id in 2001..=2028u16 {
            if id == 2015 {
                // Not a gameplay toggle here: openroad models `isWindowMode`
                // as `video.window_mode_override: Option<bool>`.
                assert!(!options.gameplay.toggles.contains_key(&id));
                continue;
            }
            let expected = !off.contains(&id);
            assert_eq!(
                options.gameplay.toggles.get(&id),
                Some(&expected),
                "id {id} must default to the value both .dat files store"
            );
        }
        assert_eq!(options.gameplay.toggles.len(), SHIPPED_TOGGLES.len());
    }

    /// The single place a missing id is answered from (`options_game::toggle_on`,
    /// and what `hud/nameplates.rs` should use): known ids answer with the
    /// file's value, unknown ones with openroad's stated "on".
    #[test]
    fn a_missing_entry_falls_back_to_the_shipped_value_not_to_on() {
        assert!(!default_toggle(2021), "Self Condition ships off");
        assert!(!default_toggle(2024), "Monster Condition ships off");
        assert!(default_toggle(2010), "Own Name ships on");
        assert!(default_toggle(9999), "an id we invented defaults on");
    }

    /// The window mode stays "never chosen" even though the file says 1 — the
    /// deliberate deviation documented on `VideoOptions::window_mode_override`.
    #[test]
    fn the_defaults_do_not_adopt_the_files_window_mode() {
        assert_eq!(GameOptions::default().video.window_mode_override, None);
    }

    /// A `user_settings.yaml` without the toggle map must not come back as
    /// "no toggles at all": `serde(default)` on that field would produce an
    /// empty map, i.e. blanket-on.
    #[test]
    fn a_settings_file_without_a_gameplay_group_gets_the_shipped_toggles() {
        let older: GameOptions =
            serde_yaml::from_str("video: {}\naudio: {}\n").expect("an older file still loads");
        assert_eq!(older.gameplay, GameplayOptions::default());
    }

    /// The sight mode must change behaviour **and** survive a restart. The
    /// restart half is this: it has to go through the same `user_settings.yaml`
    /// round-trip `persistence.rs` performs, with a stable spelling, or the
    /// radio silently resets on every launch.
    #[test]
    fn the_sight_mode_round_trips_through_the_settings_yaml() {
        for mode in SightMode::ALL {
            let mut options = GameOptions::default();
            options.camera.sight = mode;

            let text = serde_yaml::to_string(&options).expect("options serialize");
            let back: GameOptions = serde_yaml::from_str(&text).expect("options deserialize");

            assert_eq!(back.camera.sight, mode, "{mode:?} did not survive the file");
        }
    }

    /// A `user_settings.yaml` with no `camera:` key at all must still load —
    /// and land on the documented default mode, not on whatever happens to be
    /// first.
    #[test]
    fn a_settings_file_without_a_camera_group_still_loads_as_free() {
        let older: GameOptions =
            serde_yaml::from_str("video: {}\naudio: {}\n").expect("an older file still loads");

        assert_eq!(older.camera.sight, SightMode::Free);
    }

    /// The serialized spelling is a file format, so it is pinned rather than
    /// left to whatever `Debug` happens to print.
    #[test]
    fn the_sight_mode_is_stored_under_its_snake_case_name() {
        let text = serde_yaml::to_string(&CameraOptions {
            sight: SightMode::ThirdPerson,
        })
        .expect("serialize");

        assert!(
            text.contains("third_person"),
            "unexpected on-disk spelling: {text}"
        );
    }

    #[test]
    fn from_records_maps_known_ids_and_ignores_unknown() {
        let records = vec![
            OptionRecord {
                id: 1,
                value: OptionValue::U16(42),
            },
            OptionRecord {
                id: 502,
                value: OptionValue::U8(9),
            },
            OptionRecord {
                id: 503,
                value: OptionValue::U32(2560),
            },
            OptionRecord {
                id: 1001,
                value: OptionValue::U32(70),
            },
            OptionRecord {
                id: 1004,
                value: OptionValue::Bool(false),
            },
            OptionRecord {
                id: 2015,
                value: OptionValue::Bool(true),
            },
            OptionRecord {
                id: 2002,
                value: OptionValue::Bool(false),
            },
            OptionRecord {
                id: 3001,
                value: OptionValue::U32(0x41),
            },
            OptionRecord {
                id: 3101,
                value: OptionValue::Bool(true),
            },
            OptionRecord {
                id: 9999, // unknown -> ignored
                value: OptionValue::U32(1),
            },
        ];
        let o = GameOptions::from_records(&records);
        assert_eq!(o.video.graphic1.quality.get(&1), Some(&42));
        assert_eq!(o.video.graphic1.brightness, 9);
        assert_eq!(o.video.graphic1.width, 2560);
        assert_eq!(o.audio.bgm_volume, 70);
        assert!(!o.audio.bgm_enabled);
        assert_eq!(o.video.window_mode_override, Some(true));
        assert_eq!(o.gameplay.toggles.get(&2002), Some(&false));
        assert_eq!(o.keymap.bindings.get(&3001), Some(&0x41));
        assert!(o.keymap.mouse_shortcut_swapped);
    }

    #[test]
    fn yaml_roundtrips_non_default_values() {
        let mut o = GameOptions::default();
        o.audio.bgm_volume = 33;
        o.keymap.bindings.insert(3001, 0x42);
        o.gameplay.toggles.insert(2001, false);
        let yaml = serde_yaml::to_string(&o).expect("serialize");
        let back: GameOptions = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(o, back);
    }

    /// The window mode is `config.yaml`'s to decide, so the session override
    /// must not ride the file in either direction. Both halves are asserted
    /// because either one alone would be enough to persist it: a written key
    /// would be read back next launch, and a read key would adopt a stale
    /// `window_mode:` value from an older file.
    #[test]
    fn the_window_mode_override_never_touches_the_settings_yaml() {
        let mut o = GameOptions::default();
        o.video.window_mode_override = Some(true);
        let yaml = serde_yaml::to_string(&o).expect("serialize");
        assert!(
            !yaml.contains("window_mode"),
            "a session-only override must not be written: {yaml}"
        );

        // The legacy spelling an older file on disk may carry.
        let back: GameOptions =
            serde_yaml::from_str("video:\n  window_mode: false\n").expect("legacy doc");
        assert_eq!(
            back.video.window_mode_override, None,
            "a stored window_mode must be ignored, not honoured"
        );
    }

    #[test]
    fn empty_document_is_all_defaults() {
        let o: GameOptions = serde_yaml::from_str("{}\n").expect("empty map");
        assert_eq!(o, GameOptions::default());
    }

    #[test]
    fn partial_document_falls_back_to_default() {
        let o: GameOptions =
            serde_yaml::from_str("audio:\n  bgm_volume: 10\n").expect("partial doc");
        assert_eq!(o.audio.bgm_volume, 10);
        // a field missing from the present section keeps its default
        assert!(o.audio.fx_enabled);
        // absent top-level sections default wholesale
        assert_eq!(o.video, VideoOptions::default());
        assert_eq!(o.keymap, KeyMapOptions::default());
    }

    /// The shipped mix, pinned to the bytes it came from: `SROptionSet.dat` ids
    /// 1001/1002/1003 = 30/50/50 and 1004-1006 = 1/1/1. Unity gain would be an
    /// invented default; a change to it has to edit this test and say why.
    #[test]
    fn the_default_mix_is_30_50_50() {
        let audio = AudioOptions::default();
        assert_eq!(audio.bgm_volume, 30);
        assert_eq!(audio.fx_volume, 50);
        assert_eq!(audio.env_volume, 50);
        assert!(audio.bgm_enabled && audio.fx_enabled && audio.env_enabled);

        assert_eq!(audio.bgm_playback().unwrap().volume.to_linear(), 0.3);
        assert_eq!(audio.fx_playback().unwrap().volume.to_linear(), 0.5);
    }

    #[test]
    fn zero_volume_is_silent() {
        let mut audio = AudioOptions::default();
        audio.bgm_volume = 0;
        assert_eq!(audio.bgm_playback().unwrap().volume.to_linear(), 0.0);
    }

    #[test]
    fn half_volume_is_half_gain() {
        let mut audio = AudioOptions::default();
        audio.fx_volume = 50;
        assert_eq!(audio.fx_playback().unwrap().volume.to_linear(), 0.5);
    }

    #[test]
    fn disabled_channel_plays_nothing() {
        let mut audio = AudioOptions::default();
        audio.bgm_enabled = false;
        audio.fx_enabled = false;
        assert!(audio.bgm_playback().is_none());
        assert!(audio.fx_playback().is_none());
    }

    #[test]
    fn over_100_volume_clamps_to_unity_gain() {
        let mut audio = AudioOptions::default();
        audio.bgm_volume = 150;
        assert_eq!(audio.bgm_playback().unwrap().volume.to_linear(), 1.0);
    }

    /// Muted BGM is a *paused sink*, not a missing entity: the sink has
    /// to exist, and to remember the intended volume, or turning BGM back on
    /// mid-scene would have nothing to unpause.
    #[test]
    fn muted_bgm_still_produces_paused_playback_settings_at_the_set_volume() {
        let audio = AudioOptions {
            bgm_enabled: false,
            bgm_volume: 40,
            ..AudioOptions::default()
        };
        let settings = audio.bgm_playback_settings();
        assert!(settings.paused, "muted BGM spawns paused");
        assert_eq!(settings.volume.to_linear(), 0.4);
        assert_eq!(audio.bgm_gain().to_linear(), 0.4);
        assert!(audio.bgm_playback().is_none(), "the old gate is unchanged");

        let on = AudioOptions {
            bgm_enabled: true,
            bgm_volume: 40,
            ..AudioOptions::default()
        };
        assert!(!on.bgm_playback_settings().paused);
    }
}

//! `0x3809 SERVER_AGENT_ENVIRONMENT_WEATHER_UPDATE` → the zone's weather state.
//!
//! Idea: the packet is decoded and round-trip-tested in `packets` and written
//! as a Bevy message on every receive (`packets::lib::add_network_events`), but
//! nothing in the client reads it — the repo's canonical dead wire. This module
//! is the *state* half of that wire (receive → resource). It holds what the
//! server said, applies the original client's own validation, and deliberately
//! renders nothing itself: the raw bytes stay raw here so the rendering side
//! (`precipitation.rs`) owns every visual decision and its rationale.
//!
//! What is measured, from the player's own 4722-frame capture
//! (`packet_dump/**/0x3809.log`, 18 files, 2026-10-22) and from the original's
//! handler read statically at ImageBase 0x400000 (never executed):
//!
//! * **The body is exactly 2 bytes** — kind, then intensity — in 4722 of 4722
//!   frames, with 193 distinct bodies.
//! * **The value space is `{1, 2, 3}`** and anything else is coerced to 1:
//!   `sro_client.exe` 0x8a6f9b (`cmp al,3 / cmp al,2 / cmp al,1 / mov [esi],1`).
//!   The capture only ever shows **1 (3537x) and 2 (1185x)** — kind 3 appears in
//!   no frame at all, which is why nothing here names 2 or 3.
//! * **Kind 1 is the "no precipitation" kind.** It is the coercion target, its
//!   setter zeroes the intensity slot on entry (`0xa19652`:
//!   `cmp eax,1 / mov [edi+0x2e60],0`), and only kinds 2 and 3 take the
//!   precipitation branch at `0xa1c0fe` (`cmp eax,2 / je … / cmp eax,3 / jne`).
//! * **Both fields default to the original's constructor values**, kind 1 /
//!   intensity 0 (`0xa2fa86`: `mov [esi+0x2e5c],ebp` with `ebp = 1` from
//!   `0xa2f43b`, `mov [esi+0x2e60],ebx` with `ebx = 0` from `0xa2f3b1`).
//! * **The intensity is a crossfade weight, not an amount.** One packet every
//!   10 s; on a kind change the old kind ramps down linearly, the kind flips at
//!   intensity 0 — in **264 of 264** kind changes inside a live session — and
//!   the new kind ramps back up. Measured step width is 28, 29 *or* 30 per
//!   packet (it belongs to the transition, not to the protocol), then the value
//!   drifts by ±1 around its plateau (kind 1 plateaus at 180).
//! * **The upper bound is not measured**: the largest value seen is 200 and no
//!   frame exceeds it. So this module keeps the raw byte and offers no
//!   normalized float — a `/ 255.0` or `/ 200.0` here would be an unsourced
//!   magic number, and the one place that must normalize states its reason.
//!
//! Deliberately not gated on `GameState::Game`: this system owns no scene
//! resource (only a message reader and its own resource), so a weather packet
//! that arrives on the same frame as the state flip is not silently dropped.
//! The state is reset on leaving the world so a second entry never inherits the
//! previous session's weather.

use bevy::prelude::*;
use bevy_inspector_egui::prelude::ReflectInspectorOptions;
use bevy_inspector_egui::InspectorOptions;
use packets::agent::ingame::WeatherUpdate;

/// The kind the original coerces every unknown value to, and the only kind that
/// draws no precipitation (`sro_client.exe` 0x8a6f9b / 0xa19652).
pub const WEATHER_KIND_CLEAR: u8 = 1;

/// Highest kind the original accepts (`sro_client.exe` 0x8a6f9d..0x8a6fa7).
pub const WEATHER_KIND_MAX: u8 = 3;

/// The zone weather last announced by the agent server.
///
/// `kind` is intentionally a raw `u8` and not an enum: the capture contains only
/// kinds 1 and 2, and the binary names no asset family for 2 versus 3, so
/// turning the byte into `Rain`/`Snow` here would bake an unmeasured guess into
/// the type system. The rendering side maps it, with its sources cited.
#[derive(Resource, Reflect, InspectorOptions, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Resource, InspectorOptions)]
pub struct Weather {
    /// 1..=3, coerced by [`sanitize_kind`]; 1 = no precipitation.
    pub kind: u8,
    /// Raw intensity byte — a crossfade weight (see the module doc). Values up
    /// to 200 have been observed; the upper bound is not measured.
    pub intensity: u8,
}

impl Default for Weather {
    /// The original's own constructor defaults (`sro_client.exe` 0xa2fa86).
    fn default() -> Self {
        Self {
            kind: WEATHER_KIND_CLEAR,
            intensity: 0,
        }
    }
}

impl Weather {
    /// Whether the server is announcing precipitation at all — kinds 2 and 3
    /// take the original's precipitation branch (`sro_client.exe` 0xa1c0fe),
    /// kind 1 does not, and a zero weight means the crossfade is at its
    /// crossing point.
    pub fn is_precipitating(&self) -> bool {
        self.kind != WEATHER_KIND_CLEAR && self.intensity > 0
    }
}

/// The original's validation, verbatim: 1, 2 and 3 pass, everything else
/// becomes 1 (`sro_client.exe` 0x8a6f9b-0x8a6faa).
pub fn sanitize_kind(kind: u8) -> u8 {
    if (WEATHER_KIND_CLEAR..=WEATHER_KIND_MAX).contains(&kind) {
        kind
    } else {
        WEATHER_KIND_CLEAR
    }
}

/// Applies `0x3809` to [`Weather`].
///
/// Writes only on a real change so `resource_changed::<Weather>` stays a precise
/// trigger for consumers — the server repeats this packet every 10 s and
/// `ResMut` marks the resource changed on every deref.
pub fn apply_weather_update(
    mut reader: MessageReader<WeatherUpdate>,
    mut weather: ResMut<Weather>,
) {
    for update in reader.read() {
        // Field order follows the original's handler (0x8a6fec): the kind slot
        // is written first — and writing kind 1 clears the intensity slot
        // (0xa19652) — then the intensity slot is written from the packet. Both
        // land in one assignment here, so that clear is not observable; it is
        // reproduced anyway because a consumer reacting to the kind change must
        // not see a stale intensity behind it.
        let next = Weather {
            kind: sanitize_kind(update.weather_type),
            intensity: update.intensity,
        };
        if *weather != next {
            *weather = next;
        }
    }
}

/// Leaving the world drops the zone's weather (see the module doc).
pub fn reset_weather(mut weather: ResMut<Weather>) {
    *weather = Weather::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<Weather>()
            // In the real app this comes from the `packets!` registration,
            // which a bare test App does not run.
            .add_message::<WeatherUpdate>()
            .add_systems(Update, apply_weather_update);
        app
    }

    fn send(app: &mut App, weather_type: u8, intensity: u8) -> Weather {
        app.world_mut().write_message(WeatherUpdate {
            weather_type,
            intensity,
        });
        app.update();
        *app.world().resource::<Weather>()
    }

    #[test]
    fn the_default_is_the_originals_constructor_state() {
        assert_eq!(
            Weather::default(),
            Weather {
                kind: 1,
                intensity: 0
            }
        );
        assert!(!Weather::default().is_precipitating());
    }

    #[test]
    fn only_the_three_kinds_the_original_accepts_survive() {
        for kind in [1, 2, 3] {
            assert_eq!(sanitize_kind(kind), kind);
        }
        // 0x8a6f9b: everything else is stamped back to 1.
        for kind in [0, 4, 5, 128, 255] {
            assert_eq!(sanitize_kind(kind), WEATHER_KIND_CLEAR);
        }
    }

    #[test]
    fn the_capture_baseline_lands_in_the_resource() {
        // packet_dump/0x3809.log: `01 b4`, the kind-1 plateau, 484 of 4722
        // frames and the single most frequent body in the capture.
        let mut app = app();
        let weather = send(&mut app, 1, 180);
        assert_eq!(
            weather,
            Weather {
                kind: 1,
                intensity: 180
            }
        );
        assert!(
            !weather.is_precipitating(),
            "kind 1 is the no-precipitation kind whatever its intensity says"
        );
    }

    #[test]
    fn an_out_of_range_kind_is_coerced_instead_of_stored() {
        let mut app = app();
        assert_eq!(send(&mut app, 7, 90).kind, WEATHER_KIND_CLEAR);
    }

    /// The measured crossfade, replayed frame for frame: the 55x-observed
    /// fade-out of kind 1 in steps of 30, the flip at intensity 0, and the
    /// 53x-observed fade-in to the plateau 180
    /// (`packet_dump/0x3809.log`, 2026-08-16 20:14:32 onward, one packet / 10 s).
    #[test]
    fn a_measured_crossfade_replays_through_the_resource() {
        let mut app = app();
        let capture = [
            (1u8, 150u8),
            (1, 120),
            (1, 90),
            (1, 60),
            (1, 30),
            (2, 0),
            (2, 30),
            (2, 60),
            (2, 90),
            (2, 120),
            (2, 122),
        ];
        let mut precipitating = Vec::new();
        for (weather_type, intensity) in capture {
            let weather = send(&mut app, weather_type, intensity);
            assert_eq!(weather.kind, weather_type);
            assert_eq!(weather.intensity, intensity);
            precipitating.push(weather.is_precipitating());
        }
        // The kind flips exactly at intensity 0 (264 of 264 live changes), so no
        // frame of the fade claims precipitation while the old kind fades out.
        assert_eq!(
            precipitating,
            vec![false, false, false, false, false, false, true, true, true, true, true]
        );
    }

    #[test]
    fn a_repeated_packet_does_not_mark_the_resource_changed() {
        let mut app = app();
        send(&mut app, 1, 180);
        // The server repeats `01 b4` for minutes on end; a consumer gated on
        // resource_changed must not wake for those.
        send(&mut app, 1, 180);
        assert!(!app.world().resource_ref::<Weather>().is_changed());
    }

    #[test]
    fn leaving_the_world_drops_the_zone_weather() {
        let mut app = App::new();
        app.insert_resource(Weather {
            kind: 2,
            intensity: 120,
        })
        .add_systems(Update, reset_weather);
        app.update();
        assert_eq!(*app.world().resource::<Weather>(), Weather::default());
    }
}

use bevy::prelude::*;

use crate::plugins::config::graphics::FogGraphicsSettings;
use crate::plugins::map::view_range::ViewRange;

/// Default fog color, shared with the sky gradient's horizon band (`plugins/skybox.rs`)
/// so the fully-fogged terrain horizon and the sky backdrop meet in the same color.
pub const FOG_COLOR: Srgba = Srgba::new(0.1, 0.2, 0.4, 1.0);

/// The sun's tint in the in-scattering halo, used only when
/// `FogGraphicsSettings::sun_scattering` is non-zero. Once an environment
/// profile is active `plugins/environment` replaces this with the profile's own
/// sun color every frame; this is just the pre-profile default.
const FOG_SUN_TINT: Srgba = Srgba::new(1.0, 0.95, 0.75, 1.0);

pub fn fog(settings: &FogGraphicsSettings, view: &ViewRange) -> DistanceFog {
    // Clear out to `fog_start`, then a linear fade to fully fogged at `fog_end`
    // (`graphics.view`; the environment profiles take over once one is active).
    // Regions are streamed out to at least the view distance plus
    // `UNLOAD_BUFFER` rings and despawned one hysteresis ring beyond that (see
    // `load_terrain_dynamically`), so by the time a region disappears it has
    // been fully hidden by fog for a while and the unload is imperceptible.
    let fog_start = view.fog_start;
    let fog_end = view.fog_end;

    DistanceFog {
        color: FOG_COLOR.into(),
        // Alpha *is* the scattering strength: Bevy multiplies the whole
        // in-scattering term by it, so 0 disables the sun halo outright, which
        // is the faithful 1.188 look (see `FogGraphicsSettings`).
        directional_light_color: FOG_SUN_TINT.with_alpha(settings.sun_scattering).into(),
        directional_light_exponent: settings.sun_scattering_exponent,
        falloff: FogFalloff::Linear {
            start: fog_start,
            end: fog_end,
        },
    }
}

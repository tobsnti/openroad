# ADR 0012: Graphics presets and the configurable view range

Date: 2026-10-07
Status: Accepted

## Context

The client should run on PCs about 15 years old and still look its best on
current ones. Two things stood in the way.

- **Every overworld distance was a compile-time constant.** The fog band
  (3840 to 5760), the 9x9 region load ring, the camera's far plane (7680) and
  every distance cull came from `VISIBLE_RANGE`, `FOG_RANGE` and
  `REGION_SIZE`, and nine call sites recomputed the same product. The number
  of streamed regions is the largest single driver of memory, entity count
  and per-frame work, and a weak machine could not lower it.
- **Each performance knob had to be found and set by hand.** MSAA, water
  tier, render scale, vegetation distance and the rest each carried a measured
  cost in `config.example.yaml`. A user with an old machine had to read all of
  them, and a fresh config ran the most expensive values everywhere.

The culls also measured against the static 5760 ceiling, even at night,
when the environment profiles pull the fog in to 800-1200.

## Decision

1. **`graphics.view` makes the distances configuration.** It has
   `view_distance`, `fog_start`, `fog_end`, `envi_fog`, `envi_fog_scale`,
   `fog_cull_distance` and `cull_follows_envi_fog`. They are derived and
   clamped into one `ViewRange` resource (`plugins/map/view_range.rs`) under
   the live-settings rule. Every consumer reads that resource: the streaming
   ring, the fog, the region, object, animation and effect culls, the per-part
   LOD ceiling and the camera far plane.
   - Invariants are enforced by clamping: fog ends no later than the view
     distance, and the far plane sits one region past it.
   - The defaults reproduce the old constants exactly.
2. **Two cull distances.** `static_cull` is the configured ceiling, which
   spawn-time state bakes in and `apply_object_lod` re-walks. `live_cull`
   can follow the current profile fog end (`cull_follows_envi_fog`). That
   option is off by default: fully fogged terrain still shows as a
   fog-coloured silhouette against the sky gradient, so culling it changes
   the night horizon.
3. **Presets are YAML layers under the user's file.** `graphics.preset` is one
   of `auto`, `low`, `medium`, `high` or `ultra`.
   - `ClientConfig::from_file` adds the preset's compiled-in YAML as a
     config-rs source *before* `config.yaml`, so any key the user sets wins
     and every key they leave out comes from the preset.
   - `high` is the pre-preset look. `config.example.yaml` comments out the
     preset-controlled keys, so a fresh config follows the preset.
4. **`auto` is decided by probing the GPU before the App exists.**
   - A throwaway wgpu instance asks for the adapter Bevy will pick: same
     backends, high-performance preference. No compute or storage buffers, a
     GL backend or a software device gives `low`; integrated gives `medium`;
     discrete gives `high`. `ultra` is never automatic.
   - The probe's result also gates plugin registration that needs storage
     buffers (the FPS overlay).

5. **The presets also carry the memory and load levers measured since.**
   Each is a config key of its own, and each is documented with its
   measurement in `docs/perf-remote.md`:
   - `texture_detail` drops the top mip levels of model and ground textures.
   - `anisotropy` sets the filtering clamp.
   - `streaming.residency_grace_secs` sets how long departed areas stay
     loaded.
   - `view.terrain_lod` draws distant regions on a half and a quarter grid
     with skirts.
   - `objects.nature_density` is a stable per-placement thinning of
     vegetation.
   - `gpu_light_clustering` is off by default: it measured ~0.5 ms slower
     with no point lights in view.
   - The per-frame streaming budgets (`streaming.*_per_frame`): half on
     `low`, one and a half times on `ultra`.
   - `view.character_distance`, `objects.animate`, `effect_quality`,
     `lens_flare` (from the PK2's own `sun/lens*` art) and
     `dungeon.max_lights`.
6. **The original's video options drive the same keys.** Every quality row
   but Bloom stores a step index in its SROptionSet id. That is a deliberate
   deviation, since the original's value meanings live in its code. The
   saved steps are laid over `config.yaml` in `main`, before anything reads
   them, and again on every change. That matches the original's precedence
   of its options file over defaults, and it lets restart-only rows (Texture
   Filtering, Metallic Sheen) take effect on the next launch. A row says when
   it needs a restart or the next scene.

## Consequences

- A copied-fresh `config.yaml` on an integrated GPU now runs `medium`, not
  the full look. The log names the preset and why, and `preset: high` (or any
  single key) restores it.
- An existing `config.yaml` that sets every key keeps behaving as before.
  Keys it leaves out now come from the resolved preset instead of the
  built-in defaults.
- The preset is resolved at load, so changing it needs a restart. The view
  distances and the per-part LOD apply live.
- Startup does one extra adapter enumeration (the probe).
- The GL 3.3 floor (ADR 0011's open item) is reachable in principle: under
  WebGL2-class limits a single Bevy 0.19 bug blocks it (`docs/perf-remote.md`,
  "The GL 3.3 floor").

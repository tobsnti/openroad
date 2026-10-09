# ADR 0011: GPU capability baseline

Date: 2026-10-06
Status: Accepted

## Context

The original client ran on DX9-era hardware. OpenRoad runs on Bevy and wgpu, which
request every optional GPU feature an adapter offers and the adapter's own limits, and
pick code paths from them. Nothing stopped the client from depending on capabilities
that only recent GPUs have — and it did:

- The terrain splat shader indexed a texture *binding array* per fragment (bindless
  sampling). GPUs from before ~2016 (Haswell/Broadwell-era Intel, older NVIDIA/AMD
  binding tiers) and WebGPU/WebGL2-class devices do not offer that, so the client
  failed a validation check creating the terrain bind group layout and quit at startup.
- The hand-rolled terrain pipeline used five bind groups; the baseline guarantees four.

The project wants the client to run on 10+ year old PCs. Old hardware cannot be tested
directly, but its guarantees can be emulated: WebGPU's baseline limits with BC texture
compression added describe roughly what a ~2014 D3D11/12 or Vulkan 1.0 GPU offers
(every D3D10+-class GPU supports BC, and every DDJ texture is BC).

Measurement also showed that two capabilities Bevy uses by default cost GPU time on an
integrated GPU without visible benefit here (see `docs/perf-remote.md`, "two Bevy
defaults cost ~3 ms"): bindless material slabs (~1.7 ms) and the sky environment map
(~1.6 ms).

## Decision

1. **The renderer's floor is WebGPU-baseline limits plus `TEXTURE_COMPRESSION_BC`.**
   Every rendering path the client ships must work within it: at most 4 bind groups per
   pipeline, at most 256 texture-array layers, no texture binding arrays, no reliance on
   a limit above the WebGPU default. `OPENROAD_GPU_BASELINE=1` runs the client under
   exactly that floor; a change that adds a render path is checked with it.
2. **Capabilities beyond the floor are optional, never required.** Either Bevy detects
   them and falls back on its own (GPU preprocessing, SSAO, atmosphere), or the client
   gates them behind config.
3. **Where an optional capability measured slower or invisible here, it defaults off.**
   `graphics.bindless_materials: false` withholds the bindless features;
   `graphics.sky_reflections: false` drops the environment map. Both stay available.
4. **Large texture sets are `texture_2d_array`s, not binding arrays.** The ground tiles
   are four arrays of 256 layers covering the 10-bit tile id space, each tile normalized
   to one size/format/mip count at load (`client/src/assets/tile_layers.rs`).

## Consequences

- Under the emulated floor the client starts and renders the terrain (verified on Vulkan,
  in the `SCENE=world` sandbox); the full game has not been played through under it yet.
- Both terrain draw paths now bind one identical group; the Material path lost its
  region-local tile-id remap and its material-refresh workaround.
- A future feature that wants bindless, more bind groups or larger limits must keep a
  path within the floor, or be config-gated with the floor path as the default where
  old hardware matters.
- Native OpenGL (`--features gles`, `WGPU_BACKEND=gl`) is *not* covered by this floor:
  Bevy's SSAO compute shader fails GLSL translation there, so GL support needs a Bevy
  fix or patch first. (2026-10-07: on a real GL 3.3 card SSAO is skipped, because Bevy
  only registers it with five storage textures. The blocker under GL 3.3-class limits
  is a different Bevy 0.19 bug, in the visibility-range uniform fallback; see
  `docs/perf-remote.md`, "The GL 3.3 floor", and ADR 0012.) WebGL2-class limits (no storage buffers) are below the floor
  too: Bevy's FPS overlay frame-time graph needs storage buffers.

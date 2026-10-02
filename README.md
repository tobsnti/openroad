# Evidence for ferdoran/openroad#5 — `flat_baked` terrain lighting

Six frames, one per cell of the table in the pull request. Nothing here is edited; these are the
PNGs the numbers were computed from.

## How they were taken

* Client built from `integ/all` @ `da9667f50`. `assets/shaders/terrain_splat.wgsl`,
  `client/src/plugins/config/graphics.rs` and `client/src/plugins/hud/inventory/paperdoll.rs` are
  byte-identical to `fork/main` @ `3aea54ee0` (`git diff --quiet`), so the frames describe that
  tree too.
* `SCENE=world`, Jangan, standing at the dragon fountain. The camera is the scene's own resting
  pose, read back before each frame over the Bevy Remote Protocol. Two independent runs:
  `[953.9688, 59.3050, 1166.9187]` and `[953.9687, 59.3050, 1166.9188]`, largest difference
  `6e-5` world units.
* `TimeOfDay { t: 0.5 (noon), paused: true }` — pinned, so all six frames see the same sun.
* `RenderDebugSettings.render_objects = false`, so only the terrain is in frame. At this spot a
  plaza object otherwise covers the ground the issue is about.
* `graphics.render_mode` switched through `EnvironmentSettings.mode`; the lighting model through
  `RenderDebugSettings.terrain_lighting_mode`.

## What was measured

Mean Rec.709 luminance (0..255) over the rectangle **x 480..960, y 420..620** of the 1600x900
frame — plain ground, no dev panel, no player body — and the share of pixels in that rectangle
below luminance 16 and 32.

| file | mean | median | p05 | p95 | < 16 | < 32 |
|---|---|---|---|---|---|---|
| `vanilla-0-dynamic.png` | 60.40 | 58.22 | 46.21 | 74.43 | 0.0 % | 0.0 % |
| `vanilla-1-flat_baked.png` | 63.04 | 61.00 | 48.43 | 77.64 | 0.0 % | 0.0 % |
| `vanilla-2-baked.png` | 38.44 | 35.29 | 26.86 | 48.57 | 0.0 % | 25.4 % |
| `pbr-0-dynamic.png` | 86.20 | 85.72 | 69.36 | 103.16 | 0.0 % | 0.0 % |
| `pbr-1-flat_baked.png` | 21.49 | 18.50 | 12.93 | 29.57 | 25.8 % | 95.5 % |
| `pbr-2-baked.png` | 62.84 | 61.57 | 48.72 | 77.21 | 0.0 % | 0.0 % |

Repeatability — `pbr` + `flat_baked` taken twice, camera read back before each:

| run | mean | median | < 16 | < 32 |
|---|---|---|---|---|
| 1 | 21.88 | 18.72 | 21.7 % | 95.5 % |
| 2 | 21.86 | 18.72 | 22.1 % | 95.5 % |

## Reading them

`pbr-1-flat_baked.png` is the frame the issue describes: the ground is 4.0x darker than
`pbr-0-dynamic.png` and 95.5 % of the measured rectangle sits below luminance 32.

`vanilla-1-flat_baked.png` is the one that is easy to get wrong from the source alone: there the
mode is **not** black — it is the brightest of the three, 1.64x brighter than
`vanilla-2-baked.png`. Reading `base_color * ambient * exposure` and concluding "ambient only, so
dark" would have been wrong. The difference between the two rows is one constant:
`ambient_brightness: 3000` (vanilla) against `pbr_ambient_brightness: 100` (pbr).

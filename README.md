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

---

## After the change (same client, same camera, same window)

The client in `after-*.png` is built from this PR's branch, with
`assets/shaders/terrain_splat.wgsl` taken from the same tree — so these frames test the shader
edit itself, which `cargo test` cannot: a broken WGSL branch only shows up at runtime.
No naga/shader error in the client log; the client renders.

| `render_mode` | panel value | mean before | mean after |
|---|---|---|---|
| vanilla | 0 dynamic | 60.40 | **60.41** |
| vanilla | 1 (was flat_baked) | 63.04 | **38.44** |
| vanilla | 2 baked | 38.44 | 38.50 |
| pbr | 0 dynamic | 86.20 | **86.19** |
| pbr | 1 (was flat_baked) | **21.49** | **62.82** |
| pbr | 2 baked | 62.84 | 62.88 |

Panel value 1 now reads as baked: 38.44 against 38.50 in vanilla and 62.82 against 62.88 in pbr —
0.16 % and 0.10 % apart, i.e. frame noise. The black cell is gone: `pbr` + 1 went from 21.49 with
95.5 % of the window below luminance 32 to 62.82 with 0.04 % below it.

Positive control, so the equality above is a result and not a dead measurement: `dynamic` is
unchanged (60.40 -> 60.41, 86.20 -> 86.19) and still clearly apart from baked in the same frames
(60.41 vs 38.50 = 1.57x, 86.19 vs 62.88 = 1.37x). The shader still branches, and the window still
separates the modes it is supposed to separate.

---

## The panel in the pull request

`panel-pbr-flat-baked.png` is a crop of two of the frames above — `pbr-1-flat_baked.png` over
`after-pbr-1-was_flat_baked.png`, box `x 0..1290, y 410..900` — with a label bar and one magenta
rectangle drawn in. The rectangle is the measured window `x 480..960, y 420..620`. Nothing else is
drawn, and the uncropped frames stay in this branch next to it. The crop leaves out the right part
of the frame because the FPS/diagnostics overlay is open in the before run only; comparing two
differently covered halves would not be a before/after.

**Correction to "no player body" in *What was measured* above.** The player does stand in that
window: his bounding box is `x 767..825, y 458..571` of the full frame. In the before frame 1.72 %
of the window's pixels are brighter than luminance 100 and 4.48 % are above 32 — that is him, and
it is why the before cell reads 95.5 % below 32 and not ~100 %. The ground numbers are unaffected:
the medians (18.50 before, 61.57 after) are ground pixels either way, and both runs contain the
same player in the same place, so the before/after difference is the terrain's.

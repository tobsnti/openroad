# Evidence for ferdoran/openroad#12 — the object wireframe marked the anchors, not the meshes

Eight frames and one panel. The frames are unedited screenshots; the panel is a crop of two of
them (`before-C-objects.png`, `after-C-objects.png`, box `x 660..1290, y 0..620`) with a label bar
added. Nothing else is drawn into any image.

## How they were taken

* Stage `SCENE=world`, 1600x900, own Bevy Remote Protocol port. The camera is the stage's resting
  pose and is not moved. The player position is printed at the bottom left of every frame and is
  the same in both runs down to the sixth decimal:
  `Position: [953.96875, 59.3049.., 1166.9187] - Region 168.50314 x 97.60777`.
* Two colours that do not otherwise occur in this scene, set over BRP before each run:
  `RenderDebugSettings.terrain_wireframe_color = Srgba(0,1,0,1)` (green) and
  `RenderDebugSettings.object_wireframe_color = Srgba(1,0,1,1)` (magenta).
* Four switch positions per run, 2 s settle each:
  **A** both off · **B** `terrain_wireframe` only · **C** `object_wireframe` only · **D** both.
* **before** = client built from `integ/all` @ `da9667f50`. That tree still registers
  `WireframePlugin`, and its wireframe loop in `render_debug.rs` is identical to `fork/main` @
  `3aea54ee0` except for one `use` line. Taking the before frames there is deliberate: on today's
  `main` the plugin is commented out (that is #25), so nothing at all would draw and the frames
  would not isolate the marker bug.
* **after** = `fork/main` + #25 (plugin re-registered) + this PR, built locally.

## What was counted

Every pixel of each 1600x900 frame (1 440 000):
`green = G - max(R,B) > 30`, `magenta = min(R,B) - G > 30`.

| switch | before green | before magenta | after green | after magenta |
|---|---|---|---|---|
| A both off | 12 | 1 | 11 | 8 |
| B terrain only | 19 140 | 0 | 18 936 | 9 |
| **C objects only** | 13 | **0** | 1 | **65 054** |
| D both | 18 728 | **0** | 12 671 | **63 440** |

## Reading them

* **C is the issue.** With `object_wireframe = true` the before frame has **not one** magenta
  pixel — not at threshold 30 either, so not even an antialiased trace — while ship, railing,
  statues and buildings stand in the frame (`map objects 4381` on the diagnostics panel). After
  the change the same switch in the same frame draws 65 054 magenta pixels.
* **A is the negative control**: with both switches off neither run draws anything (12 and 11
  green pixels are the scene's own green, not lines).
* **B is the positive control**: the terrain wireframe draws in both runs, so the pipeline, the
  material and the plugin were never the problem.
* **D** shows the expected interaction: green falls from 18 728 to 12 671 because the magenta
  object lines now cover part of the terrain lines.
* The player (white figure, lower left of the panel) carries **no** wireframe after the change.
  He is not a map object, and the ancestor check keeps him out.

## What these images do not show

* They do not show the entity counts the fix is really about — `Wireframe` on 263 anchor entities
  of which **0** carry a `Mesh3d` before, against 7 828 entities of which **7 828** carry one
  after. That is a BRP query, not a picture.
* They do not show #25 on their own. The before client has `WireframePlugin` registered; on
  today's `main` it is not, and then neither switch draws anything at all.
* They do not touch the terrain toggle, the object nav-mesh view or the `Q` hotkey.
* The FPS/diagnostics overlay is open in the before run only. The panel crop excludes that part of
  the frame on purpose, so both tiles show the same piece of scene. The full frames keep it.

# Evidence for the loading screen keeping its authored 4:3 rect (PR #20)

One panel and four unedited frames. The panel is the two frames `before-loading.png` and
`after-loading.png` stacked, with one magenta rectangle drawn in; nothing else is drawn.

## How they were taken

* Both frames are the client's own BRP screenshot (`brp_extras/screenshot`), 1600x900 — a **16:9**
  window, i.e. not the 4:3 the screen was authored for. Same window, same `config.yaml`
  (byte-identical file), same assets directory, same dev panels.
* The loading screen is `SceneState::Loading` at client start, so no server is involved. It stands
  for about 0.6-0.9 s; `scripts/_shot-loading.py` starts the client and shoots as fast as BRP
  answers (first frame ~0.6 s after start), which yields 2-4 usable frames per run.
* **before** = a client built from `fork/main` + #25 + #26. `git diff` of
  `client/src/scenes/loading_screen.rs`, `client/src/scenes/game_scene.rs` and `assets/shaders/`
  between `main` and that branch is **empty**, so this is `main`'s loading screen.
* **after** = a client built from this PR's branch at `6db45d008`.

## What was measured

The gold of the loading frame art, counted the same way in both states: rows `y 725..785`,
pixels with `R>120, G>90, B < G-20`, columns with more than 5 such pixels.

| | before | after |
|---|---|---|
| predicted from the source | `x 241 .. 1362`, 1121 px | `x 380.75 .. 1221.5`, 840.75 px |
| **measured gold columns** | **`246 .. 1350`, 1105 px** | **`385 .. 1213`, 829 px** |
| runs that reproduced it | 4 of 4, identical to the pixel | 3 of 3, identical to the pixel |

The prediction is arithmetic, not a guess: `DESIGN = (1600, 1200)` and
`FRAME_RECT = (241, 973, 1121, 64)`, so the frame node is `15.0625 %` from the left and `70.0625 %`
wide. Before, those percentages are of the **window** (1600 px). After,
`design_fit_rect(1600, 900, contain)` gives the centred 4:3 box `x 200 .. 1400` and the same
percentages are of **that box** (1200 px). The visible gold starts a few pixels inside the node
rect in both states because the art has transparent margins — the same few pixels on both sides.

Vertically nothing moves: the box is as tall as the window, so `973/1200` stays `y 729.75` in both.

## Reading them

* The magenta rectangle in the panel is that 4:3 box, `x 200 .. 1400`. Before, the art and the
  chrome run past both of its edges; after, both sit inside it and a blurred, dimmed copy of the
  art fills the rest of the window (mean RGB of the left margin `(68, 39, 18)` — it is a dark
  blur, not a black bar).
* The two frames show the **same** background art by luck: the art is picked at random from five
  files on both sides. `before-loading-other-art.png` and `after-loading-other-art.png` are a
  second matching pair from other runs — different art, **same two numbers**.

## What these images do not show

* Only the startup path. The world-entry overlay (`spawn_game_loading_overlay`) uses the same
  surface but needs a server, and is not in any frame here.
* They do not show a window that is taller than 4:3; at 16:9 the height is the binding axis, so
  only the horizontal changes.
* The dev panels (FPS graph, EnvironmentSettings, TimeOfDay) are in both frames. They come from
  the shared `config.yaml` and are not part of the change.

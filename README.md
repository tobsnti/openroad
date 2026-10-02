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

---

# Added 2026-10-02: the ORIGINAL client, measured — `panel-original-before-after.png`

The three new `original-*.png` files and `panel-original-before-after.png` come from the user's own
original client, so the pair above becomes **original / before / after**.

## Which client, and how the frames were taken

* `C:\SRO\Evolin\sro_client.exe`, 11 485 184 B, sha256
  `5A4A865EF1E111B4922ADCB173B96C1D268404730FE304B15564B7AD3FCDAE8F`. The PE carries **no version
  resource** (every field empty); the client prints `Ver 1.208` on its login screen and sends
  `module "SR_Client", version 208` in its patch check. We call this protocol family *1.188*; the
  **binary calls itself 208**. It is **one** client, not "the original" in general.
* Server behind it: our own local skrillax stack through a decrypting MITM proxy, i.e. the frames
  are a real world entry, not a mock.
* **Window, not fullscreen.** The client area was measured with `GetClientRect`/`ClientToScreen`
  (not derived from the outer window size): **1280x720** for the 16:9 frames, **800x600** for the
  4:3 one. Window grab, **1:1, nothing scaled**. The files here are cropped to the client area only.
* These are the **world-entry** loading screen. **This client has no startup loading screen** —
  it goes straight from launch to the intro fly-through and the login dialog.

## What the original does, counted with the same rule as above
Gold = `R>120, G>90, B < G-20`, columns with more than 5 hits, in the frame's row band.

| client area | gold bar columns | as a percentage of the window | background art | margins |
|---|---|---|---|---|
| 800x600 (4:3) | `123 .. 676` | `15.38 % .. 84.50 %` | full width `0..799` | — |
| **1280x720 (16:9)** | **`197 .. 1080`** | **`15.39 % .. 84.38 %`** | **`160 .. 1119`** = exactly `design_fit_rect(1280,720,contain)` | **pure black, mean RGB `(0.00, 0.00, 0.00)`, maximum 0** |

Two independent elements give the same answer: the caption `NOW LOADING...` sits at
`17.12 % .. 32.12 %` of the window at 4:3 and `17.03 % .. 32.19 %` at 16:9.

**So the original splits the screen into two layers with two different rules:**

1. **The background art keeps the centred 4:3 box** — the same box this PR introduces, to the pixel.
2. **The margins beside it are pure black**, not a blurred copy.
3. **The chrome (frame, gauge, caption) scales with the WINDOW**, not with the 4:3 box.

Measured in the panel, all three tiles at 1280x720 with the same rule:

| tile | gold bar columns | width |
|---|---|---|
| original | **197 .. 1080** | 884 px |
| before (`fork/main`) | **197 .. 1080** | 884 px |
| after (this PR) | `308 .. 970` | 663 px |

`before` and the original are **identical to the pixel** for the chrome.

## How to read the panel
Each tile is 1280x720, rows 320..680, with one magenta rectangle drawn in — the centred 4:3 box
`x 160..1120` — and nothing else. The original tile is native 1280x720; the before/after tiles are
the 1600x900 frames above scaled to 1280x720 (Lanczos). Both are 16:9, so every percentage of the
window is preserved by that scale; the numbers in the captions are measured **after** scaling.

## What these new images do **not** show

* **No reference for the startup path.** The original has no startup loading screen, and this PR
  touches both surfaces (`game_scene.rs`). Only the world-entry surface has an original to compare to.
* **One client, one art file.** Both original runs drew the same background art. The statement
  "art inside the 4:3 box" rests on the **geometry** (margins exactly 160 px wide = `design_fit_rect`,
  colour exactly `(0,0,0)`), not on the picture's content.
* **The dev panels** (FPS graph, EnvironmentSettings, TimeOfDay) are in the before/after tiles and
  not in the original tile — they are ours, not part of the comparison. The panel crop (rows
  320..680) leaves them out.
* The bottom 26 rows of the original client area carry the Windows taskbar in the raw grab
  (720 + title bar + taskbar do not fit on a 768 px screen). They are outside the panel crop, and
  `original-loading-1280x720.png` is otherwise untouched.
* **1024x576 was tried and dropped:** the login fields took no keyboard input at that size (the
  `LIST` button in the same dialog did fire). Cause unknown; that is why the 16:9 evidence is 1280x720.

## Added later the same day: what `fork/main` does with the ART at 16:9

The tile captions above compare the gold bar. The background art needs the same treatment, measured
the same way in all three frames: non-black columns (luminance > 24, columns where more than half of
the band's rows are non-black) and the margin colour, in the same relative row band
(44.4 % .. 77.8 % of the height — below the dev panels, above the gold bar).

| frame | non-black columns | left margin (mean RGB, max, share > 12) | right margin |
|---|---|---|---|
| **original** 1280x720 (box 160..1120) | **162 .. 1119** | **(0.00, 0.00, 0.00), max 0, 0.0000** | **(0.00, 0.00, 0.00), max 0, 0.0000** |
| **before** `fork/main` 1600x900 (box 200..1400) | **0 .. 1599** | (83.52, 41.10, 15.07), **max 255**, 0.5242 | (104.25, 72.88, 41.12), **max 255**, 0.5903 |
| after (this PR) 1600x900 | 0 .. 1599 | (67.52, 31.30, 8.72), **max 101**, 0.9584 | (72.86, 49.65, 23.51), **max 103**, 0.9952 |
| before, other art | 0 .. 1599 | (69.13, 58.29, 42.59), **max 255**, 0.6438 | (97.96, 79.18, 68.96), max 250, 0.6295 |
| after, other art | 0 .. 1599 | (44.69, 34.98, 25.60), **max 71**, 0.9740 | (61.19, 47.87, 41.66), max 79, 1.0000 |

**`fork/main` stretches the art to both window edges** (`0..1599`, sharp art at full brightness in
the margin, max 255), where the original puts **exactly nothing** (max 0). **So the art half of this
PR is a real restoration**: it takes the art out of the stretch and back into the same centred 4:3
box the original uses. What stays different from the original is the **fill** of the margin — a
dimmed blur (max 101/103) instead of black (max 0) — and the same two numbers show that too.
The second art pair gives the same picture (before max 255, after max 71/79).

Complete, per layer:

| layer | original | `fork/main` | this PR | verdict |
|---|---|---|---|---|
| art, placement | contain, 4:3 box | **stretched to the window** | contain, 4:3 box | **restoration** |
| art, margin | **pure black** (max 0) | — (no margin) | dimmed blur (max 101) | **deliberate deviation** |
| chrome (bar, caption) | **stretched to the window** | **stretched, pixel-identical to the original** | moved into the 4:3 box | **change, not a restoration** |

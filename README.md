# Evidence for ferdoran/openroad#11 — the nav-mesh layers fighting for the same pixels

`issue11-stipple-panel.png` is the still panel the pull request already embeds: four crops of the
same spot, 2.4x, before on top (shot 1 blue, shot 2 yellow — same camera, 2 s apart), after below
(green beside blue, both shots alike).

`issue11-flicker.gif` is the same evidence in time, because that is where the defect lives: two
frames, 700 ms each, looping. Left tile before, right tile after; frame 1 of the GIF is shot 1 of
both runs, frame 2 is shot 2. **It is not real-time footage** — the two screenshots are 2 s apart
and the GIF alternates them so the instability is visible without reading a table.

## How the frames were taken

* Stage `SCENE=world`, 1600x900, own BRP port. The camera is placed by a fixed sequence (14 wheel
  steps back to the 400 stop, one right-drag `(800,100) -> (800,800)`), read back twice:
  `[953.9686, 384.4764, 1083.4824]`, equal to the fourth decimal.
* Two screenshots per state, 2 s apart, nothing moving in between.
* `*-stipple-*.png` are the 5x magnified crops of the same spot; the GIF tiles are a 1:1 piece of
  those crops (`x 230..570, y 300..900`), never resized — downscaling blends a one-pixel line with
  the pavement and the colour the whole argument rests on goes pale.
* **before** = client from `integ/all` @ `da9667f50` (`navmesh_lines.rs` identical to `fork/main`).
  **after** = `fork/main` + #25 + this PR, built locally.

## What was counted

Colour classes per pixel (`blue: B-max(R,G)>60`, `green: G-max(R,B)>60`, `yellow: min(R,G)-B>60`),
then pixels whose class differs between shot 1 and shot 2 of the same run:

| | class pixels | changed between the two shots |
|---|---|---|
| before, in this crop | 10 575 | **7 800 (73.8 %)** — blue 9 400 -> 3 800, yellow 400 -> 2 575 |
| after, in this crop | 10 600 | **0 (0.00 %)** |

On the whole frame the same measurement is 884 of 13 350 before and 38 after, against a control of
36-38 with the nav-mesh view switched off — i.e. after the change the view is as steady as not
drawing it at all.

## What the GIF does not show

* It does not show a yellow global edge. This stage has no region seam in view (nearest ~950 units,
  the camera stops at 400), so that half of the legend is pinned by a test, not by a picture.
* It does not decide passable against blocked — the PR changes yellow only.
* It is not real-time: 700 ms per frame for two screenshots taken 2 s apart.
* The green in the before run is not an edge: it is the cursor-hit marker, 42 px at the mouse
  position, drawn by a system this PR's base does not register.

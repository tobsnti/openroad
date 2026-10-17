# Evidence — the intro notice line was laid out inside the footer bar

Branch for PR *"intro: derive the notice offset from the footer bar height"*
(`fix/intro-notice-clears-the-footer`). The images are here so they stay **out of
the diff**; the PR embeds them by raw URL.

## What the pair shows

`client/src/scenes/intro_v2/chrome.rs` positions the notice line by its **bottom
edge** (`NOTICE_BOTTOM_PCT`) and the footer bar is anchored to the bottom edge too
(`BAR_H_PCT = 100*172/1200 = 14.333 %`). With the old literal `NOTICE_BOTTOM_PCT =
10.0` the line was composed **below the top of the bar** — at 720p the node sat at
y 628..648 while the bar starts at y 616.8 — so the sentence was set, had its text,
and was never drawn to the player.

## Conditions (identical for both shots)

| | |
|---|---|
| Client | built from fork `main` `e7a6298f3`; the AFTER shot adds the one-line change of this PR |
| Window | 1280x720, `scale_factor 1.0` (from the client log: `physical_width: 1280, physical_height: 720`) |
| Capture | the client's own screenshot path (`bevy_brp_extras::screenshot`), no `screencapture`, no window focus |
| Scene | `intro_v2` login form, local stub peer on loopback only |
| Trigger | login refused: `LoginError { error_code: 1, wrong_attempt: 1/6 }` — **the same error in both runs** (`client-t3.log:81`, `client-fix.log:81`), so the same sentence is expected in both: *"Password entry has failed 1 out of 6 times."* |

## Numbers (recounted here, `measure.py`)

A pixel counts as notice text within Manhattan distance 120 of
`NOTICE_COLOR = srgb_u8(255, 103, 29)`. Window **x 300..960, y 575..660**.

| | BEFORE | AFTER |
|---|---|---|
| orange px in the window | **0** | **1477** |
| rows carrying them | — | y 588..604 |
| columns | — | x 435..842 |
| full-width band y 575..660 | 81 | 1477 |

The 81 px in the BEFORE band are **not text**: they all sit at x 986..1162, inside
the diagnostics overlay. That is why the counting window stops at x 960, and why
both tiles of the context panel are cropped there.

## Limits — what these images do NOT show

* **Not the original client.** No v1.188 screenshot is involved. The original gives
  no number here either: `resinfo` id 500 carries `Rect="0,0,0,0"`, so the placement
  is code-side there as well. The new value is derived from our own two sourced
  numbers, not copied from anywhere.
* **Not the two-line case.** With error code 3 the sentence wraps and the top line
  peeked ~7 px above the bar even before the fix (a different frame, not this pair).
  So "nothing was ever visible" is true for the one-line case shown here.
* **Different backdrop.** The intro camera animates; the two shots are at different
  times of the cutscene. Nothing about the fix depends on the backdrop.
* **FPS differs** (167.55 vs 158.22). Live scene, not a benchmark.
* The shots were taken in a tree that **also** carried the (uncommitted) capture
  harness, because `AutoScreenshotPlugin` is commented out on `main`
  (`client/src/plugins/dev/mod.rs:138`). That harness draws nothing and touches no
  layout; it is a separate contribution.

## Files

* `notice-band-1to1.png` — the proof: 1:1 crops x 300..960, y 568..668 of both
  frames. **Nothing is drawn on it.**
* `notice-context.png` — where that window sits in the frame. The yellow box is the
  counted window and **the only thing drawn**; both tiles cropped to x 0..960.
* `full-before-1280x720.png`, `full-after-1280x720.png` — the untouched frames.
* `measure.py` — run it next to the two full frames to reproduce the table.

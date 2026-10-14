# Evidence: a visible keyboard focus in the intro (openroad)

All frames: 1600x900, `SCENE=intro`, login form, the 3D stage FROZEN over BRP
(`world.mutate_resources` on `Time<Virtual>.context.paused = true`). With the stage frozen,
two frames of one run with no input differ in **0 of 1 440 000 pixels**, so every number
below is a count, not an estimate against a noise floor.

Tab is written as a real `KeyboardInput { logical_key: Key::Tab }` through
`world.write_message`: `brp_extras/send_keys` sends Tab as the character `\t`, which the
focused text field consumes (bevy_ui_widgets `text_input.rs`), so it cannot drive tab
navigation at all.

The three intro buttons have no `TabIndex` on either branch — that is PR ferdoran#47, a slice
of its own. For these frames their `TabIndex` was injected at runtime over BRP, so the
before/after measures the real complaint ("Tab reaches Connect and nothing shows") instead of
"Tab never arrives".

| file | what it is |
|---|---|
| `focusring-panel.png` | the three rows below, cropped to Connect+Exit (x 660..950, y 580..650) |
| `before-connect-focused.png` | before, Connect has the keyboard focus — identical to the frame where Exit has it (0 of 96 000 pixels in the login region) |
| `after-connect-focused.png` | after, same step: 432 pixels, a 1 px white ring outside the art |
| `after-id-field-focused.png` | after, the ID field focused: 322 pixels — the ring also works where there is no art at all |
| `control-mouse-over-connect.png` | the control: the MOUSE over Connect in the BEFORE build — 2 310 pixels, the original's `button_europe_focus.ddj` |

Before binary: the gallery build of `evidence-base` (b716327d3); the files this slice touches
are byte-identical between that ref and `fork/main`. After binary: `feature/intro-focus-ring`.
Each row of the panel is one run — rows are not compared against each other, because the two
runs froze the fly-through at different moments and the terrain behind the buttons differs.

The screenshots are the user's own PK2 art, rendered; they are published here as evidence for
one PR and are not part of the source tree.

# `CIFComboBox` — the open list

The client draws a combo box code-side: no `Media/resinfo/` prototype file
exists for it, all 26 classic instances carry `DDJ=""` and all 22
fourth-generation entries carry `Image=""`. The *closed* field is transcribed
in `client/src/plugins/hud/widgets/combo_box.rs`. The **open** list is not: no
window in this tree opens a combo yet, and a widget nobody spawns is a
transcription nobody can catch being wrong.

The numbers are kept here so the open list can be re-implemented from them
whenever the first consumer lands.

They describe the original's item mall with the `Set Inquiry Period` combo open
(client area 1:1 inside an 806x629 window) — `ifitemmallshop.txt:1209`
`GDR_ITEM_MALL_SILK_COMBOBOX`, `Rect="165,85,78,20"`. Every number below is a
pixel coordinate in that frame: the field is painted at x 342..423, y 185..204;
the panel at x 342..423, y 204..264 — the two share the y=204 line, so the
panel's widget-local y is `FIELD_H - 1`, not `FIELD_H`.

## Geometry

| Value | px | Where it comes from |
|---|---|---|
| row height | 13 | glyph tops at y 212 / 225 / 238 / 251 — three intervals, all 13. Markedly tighter than the 20 px field. |
| panel top inset | 5 | panel top (y 204) to first row top (y 209): 1 px frame line + 4 px opaque black. |
| panel bottom inset | 4 | last row bottom (y 260, excl.) to panel bottom (y 264): 3 px black + 1 px frame line. |
| row inset x | 4 | fill starts x 346 = panel + 4, ends x 419 = panel + 82 - 4. Symmetric. |
| row text top | 3 | row 1 spans y 209..221, its glyphs start at 212. Text-node inset, not a baseline promise. |

Derived: panel height for `rows` entries is `5 + rows*13 + 4`; four rows give
61, exactly the y 204..264 the panel occupies. The panel takes the field's painted width
(82 px), i.e. the classic authored 78 plus the 4 px the field is painted wider.

## Colours

| Value | Colour | Where it comes from |
|---|---|---|
| frame | `rgb(123,121,123)` | sampled at (346,204), (423,220), (346,264) — identical. **Top, right and bottom only**: the left edge (x 342) is dark, so the frame reads as a bevel, not a box. An `Outline` / `BorderColor::all` silently draws the fourth edge. |
| panel fill | black at 50 % | four different backgrounds shine through at half strength: header bar `rgb(123,121,123)`→`rgb(61,60,61)` at y 238, divider `rgb(73,69,63)`→`rgb(36,34,31)` at y 244, dialog `rgb(37,35,32)`→`rgb(18,17,16)` at y 209-218, `rgb(8,12,8)`→`rgb(4,6,4)` at y 219. Four ratios, 0.486-0.500. |
| inset ring | opaque black | over the bright header bar at y 238, x 343..345 and x 420..422 stay `rgb(0,0,0)` — *not* 50 %. |
| row text | `rgb(255,255,255)`, centred | "1 day" spans x 369..396 (centre 382.5) in a panel x 342..423 (centre 382.5). The site is authored `HAlign=0`, so the list does **not** inherit the field's alignment. |
| hover fill | `rgb(128,128,255)` | hovered row 3 filled over x 346..419, y 248..260 — 844 of the row's 962 px, the rest being glyphs. An exact half/full triple, i.e. set by the client, not blended. |
| hover text | `rgb(255,255,128)` | same frame, 118 px at x 359..408, y 251..259. It replaces the white wholesale: the hovered row has no white glyph pixel left. |

Fill and text are **one state**: a row that keeps white text on a filled
background is a state the original never shows.

**There is no selection marker.** With the field already set to `7 days`, i.e.
row 1 being the current value, row 1 carries nothing: the whole frame holds
exactly 844 `rgb(128,128,255)` pixels, all of them in row 3, under the pointer.
With the pointer on row 1 the same fill sits 26 px (two rows) higher, at y
222..235 instead of y 248..260. The highlight follows the pointer; it does not
stick to the selection.

One caveat, recorded rather than smoothed: the row-1 hover is 14 px tall — one
pixel into the next row — while the last row is 13. 13 is the safer
choice (14 would run into the opaque inset ring on the last row), but nothing
here says which the client intends.

**Pressed paints nothing of its own.** In the pressed state the
row holds 938 px `rgb(128,128,255)` and 98 px `rgb(255,255,128)` in y 235..248
— the same colour pair as hover, in the same one-state fill+text form. So a
press needs no third visual; only the hover state and the commit on release.
It also supports the 13/14 caveat above: its middle row is 14 px too, only the
last row is 13.

## The rule for whoever implements it

**A combo's rows are assembled per window, in code — never sliced out of the
text file.** The original's open list holds *less* than the
shipped text offers: `textuisystem.txt` L3498-L3503 ships six consecutive lines
(`1 day`, `7 days`, **`28 days`**, **`Permanence`**, `1 month`, `3 months`) and
the original lists exactly four — `1 day`, `7 days`, `1 month`, `3 months`.
`28 days` and `Permanence` are shipped, plausible, adjacent — and not in the
box. The original agrees on the order: the four shown keys are pushed onto one
control in that sequence, and the two dropped keys
(`UIIT_CTL_SILK_INQUIRY_MONTH_DAY`, `_PERMANENCE`) never appear.

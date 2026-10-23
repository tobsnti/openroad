# resinfo — the classic UI grammar (`Media/resinfo/*.txt`)

The 247 `resinfo/*.txt` files are the classic (pre-4th-gen) UI's own layout
data: every window, every control, its rect, art path, colours and string key.
This page documents **how to read them**, because the naive reading is wrong in
a way that bites exactly where it matters.

## Grammar

```
Interface Text                      <- fixed header line
Section = Create,"0","0"            <- a named section
{
    GDR_BTN_CLOSE:CIFButton         <- a block: Name:Class
    {
        ClientRect=RECT,"0,0,0,0"   <- Key=TYPE,"value" lines
        …
    }
}
```

Value types: `INTEGER`, `STRING`, `POINT` (2 numbers), `RECT` (x,y,w,h),
`COLOR` (a,r,g,b — **alpha first**).

## Read blocks by key name, never by line offset

Keys inside a block are emitted **alphabetically**, and the key *set* varies, so
a key's line position varies with it. A census over all **3740** blocks:

| `Rect=` offset from the block header | blocks |
|---|---:|
| +9 | 3403 |
| +10 | 16 |
| +11 | 305 |
| +13 | 15 |
| +17 | 1 |

There are seven key-sets (base ×3259, `CommandID`+`HelpString` ×302,
`StretchType` ×169, the animation set ×6, `ADDID` ×2, two malformed). Both
`CommandID` (sorts before `DDJ`) and `HelpString` (before `ID`) push `Rect`
down — so **the 337 exceptions are precisely the interactive controls**:
buttons, slots, anything carrying a help string. A `lines[n + 9]` reader is
wrong on every one of them, silently.

Use `client/src/assets/resinfo/interface_text::parse_sections`, which is
key-driven, and ask a block for `entries.get("Rect")`.

## Three traps the corpus actually contains

1. **`grep -a` semantics.** `ifoption_quit.txt` is plain CRLF text that grep's
   binary heuristic rejects; without `-a` its 4 blocks silently vanish from any
   census.
2. **A stray trailing period.** `ifallianceguild.txt:78` is
   `SubSection=STRING,"".` — a regex anchored on `"$` does not match it at all.
3. **Literal `\n` sequences.** 1160 occurrences across 28 files store multi-line
   strings as the two characters `\` and `n`; read verbatim, every one collapses
   into a run-on line.

Malformed lines are skipped with a warning rather than being fatal: this is
user-supplied PK2 data, and the corpus is known to contain malformed blocks.

## `FontIndex` is a slot number, not a size

A block's `FontIndex=INTEGER,"n"` picks one of five authored text slots. The
ladder is itself data, in **points**:
`Media/server_dep/silkroad/event/event_interface.txt:2` reads
`//titlefont : 0 = "9", 1 = "8", 2 = "12", 3 = "11", 4 = "15"`. At 96 dpi
points become pixels at x 4/3, so the slots are 12 / 10.67 / 16 / 14.67 / 20 px;
openroad rounds the two fractional ones up to whole pixels.

| `FontIndex` | data (pt) | openroad (px) | blocks in the corpus |
|---:|---:|---:|---:|
| 0 | 9 | 12 | 3547 |
| 1 | 8 | 11 | 30 |
| 2 | 12 | 16 | 156 |
| 3 | 11 | 15 | 2 |
| 4 | 15 | 20 | 2 |
| 7 | — | 12 (slot 0) | 3 |

Two consequences:

* **The ladder is not ordered** (9, 8, 12, 11, 15). A higher index is not a
  bigger font, so an out-of-range index cannot be clamped to the largest entry
  without reading the index as a magnitude. openroad answers it with slot 0 —
  the size the only real occurrences sit among: all three are `FontIndex=7` in
  `ifchatbubblewindow.txt:12,31,50`, and the nine chat trees carry 107
  `FontIndex` attributes in total, these three sevens and 104 zeros.
* **A pixel size written by hand is a magic number with an extra step.** The
  resolution lives in one place,
  `client/src/assets/resinfo/interface_text::font_index_px`, and a pane that
  transcribes its rects from a tree transcribes the index alongside them.

## Status in openroad

`InterfaceTextLoader` is **not registered** (`client/src/assets/mod.rs:135-136`).
That is deliberate: it declares `extensions() = ["txt", "yaml"]`, and
`TextDataLoader` (`client/src/assets/textdata/mod.rs:376-377`) already claims
`"txt"` for the `server_dep/silkroad/textdata` tables. Registering both makes
the extension mapping ambiguous, so enabling this loader needs a discriminator
(a path prefix or an explicit loader handle) first — see #477.

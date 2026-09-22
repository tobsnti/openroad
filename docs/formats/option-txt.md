# `config/option.txt` (client-wide settings)

`Media/config/option.txt` is the archive's own switchboard. OpenRoad reads two
keys from it (`IntroOption::from_option_txt` in `client/src/assets/intro_scene.rs`);
everything else in the file drives launcher/version behaviour we do not have and
is deliberately not parsed.

| Key | Meaning | OpenRoad |
|---|---|---|
| `IntroName` | cutscene camera script, a Windows path (`script\intro\<name>.txt`) | selects `media://script/intro/<name>.txt` |
| `IntroBGM` | track under the splash, bare file name | selects `music://<name>.ogg` |

## Format

* one `Key = "Value"` per line, CRLF line ends;
* `//` starts a line comment;
* encoding is per-archive — UTF-16 (with BOM) or CP949; the shared textdata
  decoder (`client/src/assets/textdata/decode.rs`) handles both.

The comment rule is not cosmetic. Every archive keeps its *unused* variants on
commented lines directly around the active one:

```text
//IntroName = "script\intro\constantinople.txt"
IntroName = "script\intro\china_wharf.txt"
//IntroName = "script\intro\roc.txt"
```

A parser that ignores `//` takes the first line and flies the wrong cutscene.

## Why it has to be read

Across four independent data sets, all four name a *different* pair,
and two of them name cutscene scripts that do not exist in the reference
archive at all. A hard-wired name is therefore wrong on three of four data sets
even though it looks right on one.

## Precedence

`assets/intros/<name>.intro` on disk > `config.yaml` `scenes.intro_location`
(empty by default) > `config/option.txt` > the scene's own fallback track.
The config key exists so a test setup can force one flight; the data are the
default. See `docs/cutscene-convert.md` for the on-disk override.

## Unknown

* If a key appears uncommented more than once, the last one wins in OpenRoad.
  No known archive does this, so the original's rule is unknown.

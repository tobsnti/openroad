# Evidence — the capture harness, run at the target state

Branch for PR *"dev: make the prescribed capture method exist at HEAD"*
(`dev/screenshot-harness-exists`). Artifacts live here so they stay **out of the
diff**. Home paths in the logs are rewritten to `~`; no key, salt or credential
appears in either log.

## The pair that matters

The plugin is now registered **unconditionally**, so a green run with the switches
set proves only half of it. The other half is the run **without** them: it has to
behave exactly as before — no capture, no early exit, no log line.

| | with `OPENROAD_SCREENSHOT` + `OPENROAD_SCREENSHOT_AT=6,8` | without either variable |
|---|---|---|
| harness log lines | **4** | **0** |
| PNGs written | **2** — `intro_0.png` 1 605 694 B, `intro_1.png` 1 639 497 B | **0** |
| end of run | exits **itself**, `exit=0` after 9.3 s | still running at 10 s, 20 s, 25 s; killed by the driver |

Quoted from `run-with-switches.log`:

```
WARN  client::plugins::dev::auto_screenshot: OPENROAD_SCREENSHOT is set — this run exits itself after the last shot
INFO  client::plugins::dev::auto_screenshot: auto-screenshot -> ~/s3-shots/intro_0.png
INFO  bevy_render::view::window::screenshot: Screenshot saved to ~/s3-shots/intro_0.png
INFO  client::plugins::dev::auto_screenshot: auto-screenshot -> ~/s3-shots/intro_1.png
INFO  bevy_render::view::window::screenshot: Screenshot saved to ~/s3-shots/intro_1.png
INFO  client::plugins::dev::auto_screenshot: auto-screenshot: 2 shot(s) taken, exiting (OPENROAD_SCREENSHOT is set)
```

Two details the timestamps settle:

* The ladder counts **app-elapsed** seconds, not wall seconds: the shots asked for
  6 s and 8 s landed 6.6 s and 8.6 s after process start, the difference being
  window and asset start-up.
* The last PNG is written at `…19.517` and the exit is sent at `…19.987`. That gap
  is `EXIT_GRACE_SECS = 0.5`, and it is the reason the last picture survives: the
  file is saved by an observer a frame later, so exiting on the last shot time
  loses it.

## What the images are

`intro_0.png` / `intro_1.png` are the **unmodified output of the harness** — 1600x900,
written by the client's own screenshot path (`bevy_render::view::window::screenshot`),
not by a compositor tool. They are evidence that the tool works, not a visual claim
about the intro: the dev panels in them are open because the local `config.yaml` has
`dev_tools` on, and the backdrop is whatever the intro camera was looking at.

## Conditions

* Binary built from this branch on top of fork `main` `e7a6298f3`
  (`cargo build -p client`, rc=0, 90.1 s), own target dir `/tmp/gate-sch`.
* `SCENE=intro_v2`, the user's own PK2 data, local `config.yaml` (gitignored, not here).
* Same binary and same config for both runs; the **only** difference is the two
  environment variables.

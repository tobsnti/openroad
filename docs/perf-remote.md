# Remote performance insights via the Bevy Remote Protocol

How to inspect and drive a *running* client for performance work, from the
shell — no MCP server, no debugger attach. The client hosts a BRP JSON-RPC
server over HTTP; the `brp_perf` CLI (`tools/src/bin/brp_perf.rs`) wraps the
useful calls, and everything is also reachable with plain `curl`.

## Enabling

1. `config.yaml`: set `diagnostics: true`. That adds `RemotePlugin` + the HTTP
   transport from `bevy_brp_extras`, `RenderDiagnosticsPlugin`, the per-phase
   draw counters and the render-asset counters — see `client/src/main.rs`.

   **Leave `dev_tools: false` while measuring.** `dev_tools` implies
   `diagnostics`, but it also adds the egui world inspector (which reflects
   every entity into egui each frame) and the navmesh debug draws, which run
   whether or not the dev windows are visible. Together they cost double-digit
   FPS, so a reading taken through `dev_tools` describes a build nobody plays.

   `make perf set` and `make perf attribute` work at this tier too: they reach
   `RenderDebugSettings` by reflection over BRP, and `RenderControlsPlugin`
   registers that resource unconditionally (three shipping culling systems read
   it as a plain `Res<_>`). Only the egui panel that edits it is dev-gated, and
   the CLI does not need the panel.
2. Run the client (`make run world`). The server listens on
   `http://127.0.0.1:15702/`; override the port with the `BRP_EXTRAS_PORT`
   env var on the client side. `brp_perf` honors `--port`, `$BRP_PORT`, and
   `$BRP_EXTRAS_PORT` (in that order).

Numbers are only meaningful once past loading (`GameState::Game`).

### Measuring a Windows build from WSL

On a WSL2 machine the client has to run on Windows — a Linux build there has no
GPU (no `/dev/dri`, only `/dev/dxg`), so wgpu falls back to a software
rasterizer and every number describes llvmpipe rather than the card. But
`make perf` cannot then be driven from the WSL shell, for two independent
reasons:

- Bevy's `RemoteHttpPlugin` binds `127.0.0.1` only (`DEFAULT_ADDR`), so the
  server is not listening on any interface WSL can route to.
- WSL2's default networking is NAT, not mirrored: `localhost` inside WSL is
  WSL's own loopback, and the Windows host is a *different* machine at the
  default gateway (`ip route show default`). Windows-to-WSL localhost
  forwarding is automatic; WSL-to-Windows is not.

So run the CLI on the Windows side too. `make build windows` cross-compiles
`brp_perf.exe` next to `client.exe` for exactly this reason — no Rust toolchain
is needed on Windows:

```powershell
# PowerShell, in the repo; client.exe already running via `make run wsl release`
.\scripts\perf-capture.ps1
```

That script is the whole workflow in one run: it takes the baseline (fps, draw
calls per phase, per-pass GPU times, world/cache counts), then toggles terrain,
terrain lighting, shadows, objects and effects off one at a time and re-measures
after each, restores every setting it touched in a `finally` block, re-checks
the baseline for drift, and writes the lot to `perf-<timestamp>.txt`.

It reads each setting's original value rather than assuming a default -- which
matters for `enable_shadows`, seeded from `graphics.shadows.enabled` -- and it
warns when `-Settle` is shorter than the 120 frames the diagnostic averages
over, the mistake that silently mixes pre-change frames into every number.

`-NoExperiments` gives a read-only capture that changes nothing; `-Settle`
raises the wait per measurement (default 8 s, enough down to 15 fps).

The individual commands, if you want one in isolation:

```powershell
$brp = "target\x86_64-pc-windows-gnu\release\brp_perf.exe"
& $brp fps --settle-secs 8
& $brp snapshot --prefix render_phase/
& $brp snapshot --prefix render/
& $brp snapshot --prefix world_counts/
```

Binding the server to `0.0.0.0` would let WSL drive it instead, but BRP can
*mutate* the running world (`world.mutate_resources` is what `perf set` uses),
so that would put a game-controlling RPC endpoint on the LAN. It is not exposed
as a config knob for that reason; ask if the convenience is worth it.

Note `make profile chrome` builds and runs a **Linux** client, so on WSL its
frame-time totals are llvmpipe's, not the GPU's. The per-span CPU ranking is
still meaningful (that is main-thread work), but a representative capture needs
a Windows profiling build:
`cargo build --release -p client --target x86_64-pc-windows-gnu --features profile-chrome`.


## Workflow (`make perf ...`)

```bash
make perf snapshot                       # dump all diagnostics as a table
make perf snapshot PREFIX=world_counts/  # only entity counters
make perf snapshot PREFIX=render_phase/  # draw calls per phase (also: cache_counts/, render/)
make perf fps SECS=3                     # settled ~120-frame avg fps / frame time
make perf sample SECS=30 INTERVAL=250 OUT=before.jsonl   # record JSONL for offline diffing
make perf get                            # print RenderDebugSettings
make perf set FIELD=render_effects VALUE=false           # toggle a subsystem remotely
make perf attribute SECS=3               # per-subsystem frame-cost table (see below)
```

`cargo run -p tools --bin brp_perf -- <cmd> --help` shows all flags.

## What the diagnostics dump contains

`openroad/diagnostics` (registered in `main.rs`, handler in
`client/src/plugins/diagnostics.rs`) returns every diagnostic in the
`DiagnosticsStore` as `{path: {value, avg, smoothed}}`:

- `fps`, `frame_time`, `frame_count`, `entity_count` — Bevy's built-ins.
- `frame_time/max_window` — the worst single frame in the same ~120-frame
  history `frame_time`'s own `avg`/`smoothed` are computed from. Both of
  those are means, so a single hitch buried in an otherwise-smooth window
  barely moves either one; read this *against* `frame_time.avg` in the same
  snapshot — close together means genuinely smooth, `max_window` far above
  `avg` means a spike happened recently and the average hid it. This is the
  frame-*pacing* signal `avg`/`smoothed` can't give you; see
  `client/src/plugins/diagnostics.rs:frame_time_max_window_system`. Also on
  the in-game corner panel as `frame max`.
- `frame_time/low_1pct`, `frame_time/low_0_1pct`, `frame_time/jitter_ms`,
  `frame_time/stutter_rate` — frame-*pacing* metrics, as distinct from the
  frame-*rate* metrics above. Each catches something `max_window` can't:
  `low_1pct`/`low_0_1pct` are the mean of the slowest 1%/0.1% of frames over a
  longer, independent ~3600-sample history (the standard "1% low"/"0.1% low"
  read from GPU review tooling) — read together with `frame_time.avg`, the gap
  between them says whether bad frames are common or a one-off, which a bare
  max can't distinguish. `jitter_ms` is the mean absolute delta between
  *consecutive* frames, so it catches an alternating fast/slow pattern that has
  an unremarkable max and percentile (percentiles only see magnitude, never
  sequence). `stutter_rate` is frames/second exceeding 2x the current smoothed
  average — a rate rather than a raw count, so it reads the same regardless of
  how full the history buffer is. See `frame_pacing_system` in the same file.
- `world_counts/*` — per-category entity counters (terrain blocks/tiles, map
  objects, mesh parts, effects, particles, bones, …) plus load/gating gauges:
  `loading_compounds`, `loading_resources` (in-flight object loads),
  `unspawned_resources` (characters/NPCs/map objects still loading or parked
  behind `RESOURCE_SPAWNS_PER_FRAME` in `dynamic_resource_loader.rs`),
  `terrain_building` (regions parked in the per-frame mesh-build budget),
  `paused_animations`, `paused_effects` (distance-gated subtrees).
- `cache_counts/*` — sizes of the dedup/registry maps (`sro_meshes`,
  `sro_bind_poses`, `sro_animation_clips`, `sro_material_variants`,
  `sro_materials`, `spawned_map_objects`, `effect_meshes`,
  `effect_materials`). The maps hold weak ids and are swept every 10s, so
  the counts track *live* cached assets: expect a climb while exploring and
  a drop shortly after leaving an area. Growth that never plateaus while
  revisiting the same area indicates a cache leak. `resident_assets` is the
  exception: *released* `.bsr`/region files held strongly for
  `RESIDENCY_GRACE` (30 s, `asset_residency.rs`) so a quick return finds them
  still decoded — it should drain to ~0 within that window of standing still.
- `render/*/elapsed_gpu`, `render/*/elapsed_cpu` and the pipeline statistics
  (`vertex_shader_invocations`, `clipper_primitives_out`, …) from
  `RenderDiagnosticsPlugin`. Bevy requests every adapter feature
  (`WgpuSettingsPriority::Functionality`), so **on Vulkan and DX12 the
  `elapsed_gpu` rows are real timestamp queries** — no opt-in needed. Metal and
  WebGPU have no timestamp queries, and there only `elapsed_cpu` (the pass's
  command-encode time) is recorded; it still ranks passes, but real GPU numbers
  there need an Xcode GPU frame capture. The on-screen panel walks whatever is
  present and prefers GPU per pass, marking each row `gpu` or `cpu`.
- `render_phase/<phase>/{batch_sets,bins,unbatchable,draws}` — **draw calls per
  render phase**, summed over every view (shadow cascades and the offscreen
  portrait/paper-doll rigs included). `draws` is the actionable total;
  `batch_sets` counts multi-draw-indirect sets, `bins` batchable-but-not-
  multidrawable bins, and `unbatchable` the entities that get a draw each.
  Sorted phases (`transparent_3d`, `transmissive_3d`) publish only `draws` —
  they have no bins, so every item is its own draw. This is the number the
  batching levers in `perf-future-levers.md` are defined in terms of: collapsing
  terrain materials, restoring effect batching and merging object mesh parts all
  mean "make these go down".
- `mesh_allocator_{slabs,slabs_size,allocations}` and
  `render_asset/*` — what the render world actually holds. Meshes sharing a slab
  are what makes a batch possible, so slab count is a batching signal; the
  render-asset counts are where a GPU-side texture or mesh leak shows up, which
  the CPU-side `cache_counts/*` maps cannot see.
- `process/mem_usage` (GB), `process/cpu_usage`, `system/*` — the client's
  own footprint via `SystemInformationDiagnosticsPlugin`. This is the way to
  hunt memory growth remotely: sample it over time and A/B against suspected
  churn sources (sandboxed sessions cannot `ps`/`vmmap` the game).

`avg` is the mean over the diagnostic's ~120-measurement history — about 2 s
of frames at 60 fps but ~8 s at 15 fps. Settle times (`SECS`) must exceed
`120 / expected_fps` or averages still contain pre-change frames. `smoothed`
is the EMA the on-screen FPS overlay shows.

## Attribution mode

`make perf attribute` measures what each subsystem costs per frame: it reads
`RenderDebugSettings`, measures a baseline, then for each of
`render_terrain`, `render_objects`, `render_water`, `render_effects`,
`play_animations`, `enable_fog`, `backface_culling`, `automatic_batching`
toggles the field off
via `world.mutate_resources`, settles, samples `frame_time.avg`, and restores
it (`cost_ms = frame_time_on − frame_time_off`). `enable_shadows` is skipped
(off by default). The baseline is re-measured at the end and a >10 % drift
prints a warning — run it standing still in a fully loaded area; a streaming
world makes the numbers noisy.

The `play_animations` row only means something from 2026-10-02 on: the switch
works through `AnimationCullingPlugin`, whose registration in `main.rs` was
commented out until then, so toggling it changed nothing and earlier captures
read ~0 ms for animation regardless of the real cost.

## GPU upload budget

`main.rs` caps texture + mesh uploads at `UPLOAD_BYTES_PER_FRAME` (16 MiB,
Bevy's `RenderAssetBytesPerFrame`); the rest waits a frame. It targets the
render-thread stalls in `prepare_assets<GpuImage>` / `allocate_and_free_meshes`
(up to ~200 ms in the 2026-09-17 trace) when a region's assets land together.
When tuning it, read `frame_time/max_window` while crossing region boundaries
against how late distant assets appear; Bevy logs a debug line whenever the
budget is exhausted with assets still queued.

## Raw curl (what the CLI sends)

Bevy 0.19 method names are dotted (`world.get_resources`,
`world.mutate_resources`) — not the pre-0.16 `bevy/*` names — and resource
params need the *full* type path.

```bash
curl -s -X POST http://127.0.0.1:15702/ -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"openroad/diagnostics"}'

curl -s -X POST http://127.0.0.1:15702/ -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"world.mutate_resources","params":{
        "resource":"client::plugins::dev::render_debug::RenderDebugSettings",
        "path":"render_effects","value":false}}'

curl -s -X POST http://127.0.0.1:15702/ -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"world.get_resources","params":{
        "resource":"client::plugins::dev::render_debug::RenderDebugSettings"}}'
```

`rpc.discover` lists all available methods (Bevy built-ins,
`brp_extras/*`, and `openroad/diagnostics`). Reflection-based methods
(`world.query`, `world.get_resources`) only see the few types the client
registers with `register_type` — the marker components behind
`world_counts/*` are deliberately *not* registered; that is what the
`openroad/diagnostics` dump is for.

## Per-system CPU profiles (chrome traces)

BRP diagnostics answer *what* is loaded and *how long* frames take; per-system
CPU attribution comes from a chrome trace, which writes `trace-<nanos>.json` to
the working directory each run.

It is a **build flag**, not a source edit — `client/Cargo.toml` declares
`profile-chrome = ["bevy/trace_chrome"]`, so profiling never means modifying
tracked files:

```bash
make profile chrome            # Linux client (see the WSL caveat below)
make profile windows           # cross-compile a profiling client.exe
make profile summary           # rank the newest trace by span self-time
```

The feature stays out of the default set because it instruments every system
span in every build that carries it, which is a cost no shipped artifact should
pay.

On WSL, `make profile chrome` builds and runs a *Linux* client, which has no
GPU (`/dev/dri` is absent) and renders through llvmpipe — its frame times are
the software rasterizer's. The CPU span ranking is still valid, but for
representative numbers use `make profile windows` and run the exe from a local
Windows disk.

Caveats that make traces silently useless:

- **`RUST_LOG` must not filter bevy targets.** System/schedule spans are
  INFO-level with `bevy_ecs::*` targets; a filter like `warn,client=info`
  keeps the client's own log lines (so everything *looks* fine) but drops
  every span — the trace ends up a few KB of log events. The make targets
  unset `RUST_LOG` for exactly this reason.
- **Exit the game cleanly** (close the window, don't kill it): the tracing
  writer buffers and only reliably flushes on shutdown.
- Expect ~5-10 MB per second of gameplay; 10 s standing still is plenty.
- Write the trace to a **local Windows disk**, not across the WSL 9p bridge —
  at tens of MB per second that write distorts the frame times being measured.

Rank it with `make profile summary` (or `cargo run -p tools --bin trace_summary
-- trace-*.json --top 30`), which reports per-span **self** time. Use
`--last-secs 10` to drop the loading phase. Spans nest, so wall time puts the
root schedule on top of every capture and says nothing.

Read `mean us` as per-frame cost for anything running once per frame. Note the
instrumented build runs at roughly half the frame rate, so treat these as
*relative* weights — with one exception: a span's mean is comparable **across**
traces when the thing it works on has not changed.

## GPU timelines (Tracy)

A chrome trace shows the CPU threads. It cannot tell you whether the GPU is
busy for the whole frame or busy for half of it and idle for the rest, and
those imply opposite next moves — the first says reduce GPU work, the second
says find the stall. Tracy answers it directly, because bevy's
`RenderDiagnosticsPlugin` uploads per-pass GPU timestamps as Tracy GPU zones
(`bevy_render/src/diagnostic/internal.rs`), reachable through
`bevy/trace_tracy` → `bevy_internal/trace_tracy` → `bevy_render/tracing-tracy`.
No extra code: the diagnostics tier already registers the plugin.

```bash
make profile windows-tracy release   # cross-compile a Tracy-instrumented client.exe
make profile tracy                   # Linux client; software rendering under WSL
```

Cross-compiling this one needs the mingw **C++** compiler on top of the usual
`gcc-mingw-w64-x86-64` — Tracy's client is C++, and `tracy-client-sys` compiles
`TracyClient.cpp` as part of the build:

```bash
sudo apt install g++-mingw-w64-x86-64
```

`make profile windows-tracy` checks for it up front, because without it the
build fails deep inside a cc-rs build script (`failed to find tool
"x86_64-w64-mingw32-g++"`) rather than saying what is missing.

Linking C++ also means the exe needs the mingw runtime DLLs beside it, which
the normal pure-Rust client does not. Windows reports these one at a time
(`libstdc++-6.dll`, then `libgcc_s_seh-1.dll` which the first one pulls in), so
copy the whole import closure at once rather than chasing the error boxes:

```bash
scripts/mingw-runtime-dlls.sh \
    target/x86_64-pc-windows-gnu/release/client.exe /mnt/c/coding/openroad
```

That script walks the closure with `objdump` and resolves it against the
directory the *building* g++ reports, so it stays correct across toolchain
upgrades and does not confuse the win32 and posix mingw variants.

The Tracy **viewer is a third-party download you fetch yourself** — it is not
vendored here and not linked from the repo. Its release must match the bundled
`tracy-client-sys` (see `Cargo.lock`; the crate's README names the Tracy
version it speaks). A mismatched viewer refuses the connection with a protocol
error rather than misbehaving quietly.

Run the viewer **on Windows**, next to the client: Tracy connects over TCP
8086 and both ends are then host-local, so none of the WSL NAT problem that
forced a cross-compiled `brp_perf.exe` applies. Start the viewer first; the
client connects on launch and streams live, so there is no truncated-file
hazard — save from the viewer once you have enough.

What to read off it:

- **GPU busy time per frame** against the frame time. This is the number the
  `render/` diagnostics cannot give you: they measure *inside* pass spans, so
  they miss both the gaps between passes and the passes bevy never
  instruments — the shadow pass and tonemapping among them.
- **Where `prepare_windows` blocks.** It is a swapchain acquire, so it shows up
  as a long main-thread wait either way; the GPU track says whether the GPU was
  saturated underneath it (fill-bound) or idle (a stall to find).

## Worked example: MSAA is the `prepare_windows` lever, confirmed

A 2026-09-16 Tracy capture found `prepare_windows` costing ~8.7-11ms/frame —
13-17% of frame time by itself — with the GPU only ~40-50% utilized
underneath it (`main_opaque_pass_3d`'s own GPU time was a few ms, nowhere
near the CPU-side wait). Opening the capture in the Tracy viewer and
inspecting the worker-thread tracks during a `PostUpdate`/`Render` window
confirmed `prepare_windows` runs as a dispatched task on the compute task
pool (not a dedicated render thread) and is consistently the single widest
block among all parallel work that frame — i.e. it is a real cost, not a
measurement artifact.

**`graphics.msaa: 1` (`Msaa::Off`, see `MsaaSamples::to_msaa` in
`client/src/plugins/config/graphics.rs`) measurably shrinks it.** A/B in the
deterministic `SCENE=world` sandbox (not a live server — see the caveat
below), `dev_tools: false`, both runs Tracy-captured:

| metric | msaa: 2 | msaa: 1 (off) | delta |
|---|---|---|---|
| FPS | 41.2 | 46.6 | +13% |
| `prepare_windows` | 11.02 ms/frame | 8.79 ms/frame | −20% |
| `schedule{name=Render}` self | 14.29 ms/frame | 12.11 ms/frame | −15% |
| `sub app{name=RenderExtractApp}` | 11.99 ms/frame | 9.87 ms/frame | −18% |
| `msaa_writeback` GPU pass | 0.93 ms/frame | absent | the resolve step itself disappears |

The mechanism: `msaa_writeback` (the MSAA resolve pass) vanishes entirely at
`Msaa::Off` since there is nothing to downsample, and the swapchain-acquire
wait drops right along with it — `prepare_windows`'s cost tracks GPU render
workload, and MSAA is a direct lever on that workload, same axis the `msaa`
config comment already named ("the single biggest frame-time win available"
on fill-limited hardware). FXAA (already always-on, independent of `msaa`,
~2ms/frame either way per its own GPU zone) keeps doing edge AA on top, so
`msaa: 1` is not "no AA", it is "no MSAA, FXAA only".

**Caveat that cost an iteration**: the same A/B run once against a live
server capture (`SceneState::GameWorld`) came back *backwards*
(`prepare_windows` higher at `msaa: 1`, lower FPS) — a real result, just not
of the variable being tested. `main_opaque_pass_3d`'s raw GPU time had nearly
doubled between the two captures, which MSAA alone cannot cause (it
multiplies per-sample cost, not geometry drawn); the honest read is that the
second capture simply hit a heavier moment on a shared, non-reproducible
server (more nearby players/mobs, different terrain-streaming state). Any
config A/B needs a deterministic scene (`SCENE=world` or `SCENE=skills`) to
mean anything — a live-server capture is fine for "what does a real session
cost" but not for isolating one setting's effect.

## Worked example: water quality is a real, confirmed lever

A 2026-09-16 A/B in the deterministic `SCENE=world` sandbox (`dev_tools:
false`, identical `world_counts/*` — same 4677 map objects, 1265 water, 3744
terrain tiles both runs, confirming the sandbox loads the same content every
launch) compared `graphics.water.quality: high` vs `low`, restarting between
each (this setting is read once at `OnExit(GameState::Loading)` in
`map::setup_terrain_mesh`, not live-toggleable):

| metric | high (default) | low | delta |
|---|---|---|---|
| `frame_time.avg` | ~20-21 ms | ~19.4 ms | ~5-8% faster |
| `fps.avg` | ~48-50 | ~52 | +~8% |
| `render_phase/transmissive_3d/draws` | 2 | **0** | mechanism confirmed |

The `transmissive_3d` draw count dropping to exactly 0 at Low is the direct
mechanistic proof: Low never enters Bevy's Transmissive phase at all (it uses
`AlphaMode::Blend` instead — see `client/src/plugins/map/mod.rs:125-138`),
so the full-screen `view_transmission_texture` copy and the lost early-Z that
High pays "whenever ANY water is on screen" are both gone entirely, not just
reduced. This test location only had 2 transmissive draws in view, so treat
the ~8% figure as a floor — a view with more water on screen should show a
larger gap, since the fixed per-frame costs (the texture copy, opting out of
the depth prepass) are paid once regardless of how much water is visible,
while the win compounds with how much *other* geometry avoids losing early-Z
because of it.

Reproduced twice (high → low → high) with consistent readings each time.

## Worked example: shadows — a real but modest lever

Same session, same sandbox, live-toggled via BRP (no restart needed —
`RenderDebugSettings.enable_shadows` is genuinely live despite `make perf
attribute` excluding it; see the field's own doc comment in
`client/src/plugins/dev/render_debug.rs:204-209`). `enable_shadows: true`
(the runtime-seeded value from `graphics.shadows.enabled`, not the struct's
`false` default) vs `false`, reproduced twice (on → off → on):

| | on | off |
|---|---|---|
| `frame_time.avg` | ~20.1-20.3 ms | ~19.0 ms |
| `fps.avg` | ~50.0-50.2 | ~53.4 |

A consistent ~5-7% frame-time reduction, real but well short of an
MSAA-or-water-sized win — this scene's 2 scoped vanilla-mode cascades
(`map::sun_cascade_config`) apparently don't cost much here. Worth
retesting in a scene with more shadow-casting geometry in view before
concluding this is capped everywhere.

**Foliage `view_distance` (300 and 0/unlimited, vs the default 1920) showed
no measurable difference** at this test location — frame_time stayed within
~1ms of baseline at every setting. Not necessarily a dead end: this
particular camera position may simply not have much foliage in view: retest
somewhere foliage-dense (a grass field, not open terrain) before ruling it
out. Confirmed live-toggleable with zero rebuild either way (`VisibilityRange`
swap, `client/src/plugins/map/foliage/mod.rs:211-262`), so re-testing it is
cheap whenever there's a better vantage point.

## Worked example: `frame_time/max_window` catches a real region-crossing hitch

`frame_time.avg`/`.smoothed` are both means, so a single hitch buried in an
otherwise-smooth window barely moves either one — the exact frame-*pacing*
blind spot `frame_time/max_window` (see above) exists to close. A
2026-09-16 live-server session (`SceneState::GameWorld`, `dev_tools: false`,
polling `openroad/diagnostics` at ~1.5 Hz while actually playing) caught two
real spikes this way:

| time | `frame_time.avg` | `frame_time/max_window` | `world_counts/map_objects` | `world_counts/terrain_tiles` |
|---|---|---|---|---|
| 21:03:38 | 24.5 ms | 106 ms | 4607 | 3708 |
| 21:03:45 | 31.4 ms | **162 ms** | **4390** ↓ | **3168** ↓ |
| 21:03:47-53 | 28-32 ms | **208 ms** (peak, 7.3x avg) | 4535 ↑ | 3420 ↑ |
| 21:04:28-35 | 26-29 ms | 66-68 ms (2.5x avg) | 4873→4878 ↑ | 3384→3420 |

`map_objects`/`terrain_tiles` dropping then partially recovering is the
signature of crossing a region boundary: old regions unloading behind the
player, new ones loading ahead. `frame_time.avg` moved by single-digit
milliseconds across this whole window — a 208 ms frame was completely
invisible to it.

**`world_counts/terrain_building` stayed at 0 throughout both spikes.** That
rules out the mesh-*build* stage (`GROUP_BUILDS_PER_FRAME` is working
correctly — nothing ever queued) and narrows the cause to the spawn/despawn
side of terrain streaming: `load_terrain_objects_system`'s unbounded
per-region object-spawn loop and/or the unbounded despawn in
`load_terrain_dynamically`'s unload pass (`client/src/plugins/map/objects.rs`,
`client/src/plugins/map/terrain/mod.rs`) — the same systems a code-review
pass had already flagged as unbounded-consumers-downstream-of-a-budget
*before* this capture, now confirmed against a real spike instead of resting
on code review alone.

**Fixed (same session).** Applied the `GROUP_BUILDS_PER_FRAME` idiom to both
stages: `OBJECT_SPAWNS_PER_FRAME` (`client/src/plugins/map/objects.rs`,
`load_terrain_objects_system`) caps object spawns per frame, leaning on the
existing `SpawnedMapObjects` dedup so a region that doesn't finish this frame
is safely re-walked next frame (only newly-loaded regions transition to
`TerrainLoadState::Completed`, and only once fully drained);
`REGION_UNLOADS_PER_FRAME` (`client/src/plugins/map/terrain/mod.rs`, the
unload pass in `load_terrain_dynamically`) caps region despawns per frame —
`out_of_range` is recomputed fresh every run, so a deferred region is simply
re-evaluated (and despawned once budget allows) the next frame, no new state
needed.

Re-measured with the identical method (live server, `dev_tools: false`,
`pace_poll.js` polling `openroad/diagnostics` while crossing regions):

| | before | after |
|---|---|---|
| Peak `frame_time/max_window` | **207.68 ms** | **58.46 ms** |
| Peak ratio vs `frame_time.avg` | 7.33x | 2.41x |

`map_objects`/`terrain_tiles` still showed the same load/unload churn in the
after-capture (3959↔4887, confirming real boundary crossings happened), so
this is a like-for-like comparison — the streaming work didn't go away, it's
just spread across enough frames that no single one spikes nearly as badly.
~3.6x reduction in the worst observed frame. 58 ms is still ~2x the average,
so `OBJECT_SPAWNS_PER_FRAME`/`REGION_UNLOADS_PER_FRAME` (currently 64 and 2)
have room to tune tighter if a smoother result is wanted — start there
before looking elsewhere if this needs another pass.

## GPU capability and feature switches

Bevy requests every optional GPU feature the adapter offers and the adapter's own
limits, then picks code paths from them. Three environment variables (read once at
startup, `render_plugin` in `client/src/main.rs`) take that apart for measurement:

| switch | effect |
|---|---|
| `OPENROAD_GPU_BASELINE=1` | WebGPU-baseline limits and no optional features except BC texture compression: roughly what a ~2014 D3D11/12 or Vulkan 1.0 GPU guarantees (see ADR 0011). Anything the client needs beyond that shows up as a validation error on a current machine. `WGPU_SETTINGS_PRIO=webgpu` alone also drops BC, which no real old PC lacks and every DDJ texture uses. |
| `OPENROAD_WGPU_DISABLE=<list>` | Withholds features on top of the config: the groups `bindless`, `indirect` (GPU mesh preprocessing + multi-draw indirect), `mappable` (`MAPPABLE_PRIMARY_BUFFERS`, which Bevy enables on integrated GPUs), or raw wgpu feature names (`TIMESTAMP_QUERY`), comma-separated. |
| `OPENROAD_GPU_LIMITS=webgpu` | WebGPU-default limits, features kept. Combine with `bindless` off: Bevy enables bindless by feature alone, and those limits allow no binding-array elements, so the client crashes otherwise. |
| `OPENROAD_GPU_BASELINE=gl33` | What wgpu's GL 3.3 backend offers a DX10-class card: WebGL2-class limits (no storage buffers, storage textures or compute) but 8192 textures, BC only. Run it on Vulkan/DX12 to exercise Bevy's uniform-buffer fallbacks; `graphics.preset: auto` resolves to `low` under it. See "The GL 3.3 floor" below. |

The native OpenGL backend is a build feature: `cargo build --release -p client
--features gles`, then `WGPU_BACKEND=gl`. It is inert unless selected.

Two traps: a variable set in a PowerShell window stays set for every later launch from
that window (`Remove-Item Env:OPENROAD_GPU_BASELINE`), and the pass timers and the
`fragment_shader_invocations` counts need timestamp and pipeline-statistics queries,
so under `OPENROAD_GPU_BASELINE` they read 0 — compare FPS and frame time there.

## Worked example: two Bevy defaults cost ~3 ms at the waterfall

Facing the Jangan West waterfall (AMD integrated GPU, 1920x1080, vanilla lighting),
`OPENROAD_GPU_BASELINE=1` ran at 82 FPS against 57-59 normally. Bisecting with the
switches above, one launch per row, same view, window focused:

| configuration | FPS | opaque pass (GPU) |
|---|---|---|
| normal (then-defaults) | 57-59 | 7.4-8.0 ms |
| diagnostics tier off (`diagnostics: false`) | 59 | — |
| `mappable` off | 59 | 7.1 ms |
| `indirect` off | 60 | 7.8 ms |
| `bindless` off | 63-65 | 5.8 ms |
| `bindless` + `indirect` off | 66.5 | 6.5 ms |
| `bindless` off + WebGPU limits | 73-77 | 5.1-5.8 ms |
| `OPENROAD_GPU_BASELINE=1` | 82-84 | (no timers) |

- **Bindless material slabs** (`StandardMaterial` binding its textures from shared
  arrays indexed per fragment) cost ~1.7 ms with identical output. Now off by default:
  `graphics.bindless_materials`.
- **The WebGPU limits looked like a win but were a side effect.** They allow 4 storage
  textures per stage; Bevy's environment-map generation needs 6, so the sky
  environment map was never built and no pixel sampled it. Measured on its own
  (`graphics.sky_reflections: false`, normal limits): 64 -> 71 FPS, ~1.6 ms, with no
  visible difference in vanilla lighting — off by default.
- GPU-driven indirect drawing, mappable buffers and the per-pass GPU timers cost
  nothing measurable. The last ~1 ms between 76 and 82 FPS is not attributed yet.

## Worked example: vegetation overdraw, attributed by hiding

The same view shaded **11.8M opaque fragments for a 2.07M-pixel screen**; elsewhere the
count is about 2x the pixels. Fragments were attributed by hiding one group of visible
meshes at a time over BRP (`world.insert_components` with `Visibility::Hidden`,
filtered by `MeshMaterial3d<...>` type, then by the resource name of each mesh's
parent) and reading `render/main_opaque_pass_3d/fragment_shader_invocations` before
and after, restoring the original `Visibility` afterwards:

- Every `StandardMaterial` mesh together: -4.6M; the other material types
  (sheen, UV-scroll, water, sky, clouds) under 0.1M each.
- Within those, four kinds of vegetation: `c_hhm_tree` -0.93M, `tre_pine` -0.87M,
  `tre_tree` -0.80M, `grs_weed` grass -0.78M. Everything else under 0.1M per group.

They are alpha-tested cards. Their `discard` stops early depth rejection, so every
layer is shaded in full:

- **Depth prepass** (`graphics.depth_prepass`): the opaque pass dropped from 11.8M to
  7.6M fragments and 7.1 -> 5.0 ms, but the prepass itself took 1.5 ms (it alpha-tests
  too) — ~0.6 ms net. Opt-in.
- **Cheaper shading** (render-debug `unlit_materials`, every `StandardMaterial` unlit
  live): only ~0.9 ms (12.2 -> 11.4 ms). Shading math is not where the time goes; a
  cheaper object material is not worth it on its own.
- **Drawing fewer distant cards** is the lever. `graphics.objects.nature_view_distance`
  caps the LOD range of every `res/nature/` resource (render-debug knob of the same
  name to try values live):

| vegetation view distance | FPS | shaded fragments | alpha-mask draws |
|---|---|---|---|
| unlimited (fog, 5760) | 68 | 12.6M | 22 |
| 3000 | 72 | 11.0M | 17 |
| 2000 | 79 | 10.1M | 14 |
| 1200 | 86 | 7.3M | 9 |

With all three changes (bindless off, sky reflections off, vegetation at 2000) the
waterfall went from 57-59 to 75 FPS.

## Worked example: per-entity CPU costs

Bevy's per-frame passes (visibility, transforms, extraction, scheduling) cost per
entity and per system, so a lot of the CPU wins came from doing less of either.
Measured at the waterfall or a busy town with the levers each change introduced:

| change | where | measured |
|---|---|---|
| effect particles drawn GPU-instanced, one transparent item per (effect, mesh, material) | `plugins/effects/instanced.rs` | transparent draws added by effects ~2,000-2,700 -> ~330-450; their render-thread CPU ~2.8 -> ~1 ms |
| instance gather across the compute pool, one flat sort | `instanced.rs` | effects' main-thread cost ~2.5-3.5 -> ~1.5 ms |
| closed windows' content parked under a non-UI holder | `plugins/hud/ui_parking.rs` | main thread 9.3 -> 8.2 ms with all windows closed (~1,900 of ~2,800 UI nodes) |
| one packet dispatcher instead of one system per opcode | `packets/src/lib.rs` | ~0.4 ms; ~250 idle systems fewer |
| directional-light shadow visibility only with shadow maps on | `plugins/environment/light_visibility.rs` | ~0.37 ms/frame in vanilla mode (it rebuilt a world query over ~2,500 archetypes) |
| off-screen map-prop animation paused | `plugins/animation_culling.rs` | bones animated per frame 1,668 -> 138 |
| single-resource map objects anchored on their placement | `plugins/map/objects.rs` | ~3,500 fewer entities in a loaded area |

The per-system numbers came from a chrome trace (`--features profile-chrome`,
`trace_summary`); self time per system summed per crate/module is the quickest way to
see that engine passes, not game systems, dominate (game systems were ~16% of system
time at the waterfall).

## Worked example: view distance is a memory and entity lever

`graphics.view.view_distance` decides how many regions are streamed: `ceil(view /
1920)` rings around the camera's region, plus one ring of head start. In the
`SCENE=world` sandbox at Jangan (2026-10-07, AMD integrated GPU, debug build):

| `view_distance` | regions streamed | `world_counts/terrain_tiles` | `world_counts/map_objects` |
|---|---|---|---|
| 5760 (default, `high`) | 9x9 | 4068 | 4823 |
| 2880 (`low`) | 7x7 | 2016 | 4174 |
| 3840 (`medium`) | 7x7 | 2016 | 4174 |

Map objects fall less than terrain here because the sandbox pins a 7x7 preload
around Jangan until the camera reaches it (`scenes/world_scene.rs`). Medium and
low stream the same ring; they differ in fog, cull distance and the per-part
LOD. Read `process/mem_usage` and the `render_asset/*` counters the same way
to see what a ring costs on a given machine.

## Worked example: GPU light clustering, off by default

Bevy clusters point lights with a compute and raster pass, every frame for
every 3D view, whenever the GPU has compute shaders and storage buffers, even
with no point light anywhere. The overworld in vanilla lighting has none: the
sun is directional, and directional lights are never clustered. A/B/A in
`SCENE=world` (2026-10-07, AMD integrated GPU, debug build, 75 s settle):

| GPU clustering | `frame_time.avg` | `fps.avg` | `render/clustering/elapsed_gpu` |
|---|---|---|---|
| on (Bevy default) | 9.84 ms | 102.8 | 0.23 ms |
| off | 9.32 ms | 108.5 | (no pass) |
| on | 9.93 ms | 102.7 | 0.22 ms |

About 0.5 ms per frame, more than the pass's own timer shows. Off is now the
default (`graphics.gpu_light_clustering: false`). Bevy then assigns the few
dungeon lights on the CPU. The setting applies live.

## Worked example: terrain LOD, and why draw calls are not the lever here

`graphics.view.terrain_lod: normal` (the medium preset) against `"off"`, in
`SCENE=world` at Jangan with the camera pitched to the horizon over BRP
(`CameraRig` is reflected for this: `world.mutate_resources`, path `pitch` =
0.1, `distance` = 400), 2026-10-07, AMD integrated GPU, debug build:

| terrain LOD | opaque `vertex_shader_invocations` | opaque pass GPU | `frame_time.avg` |
|---|---|---|---|
| off | 163,045 | 2.52 ms | 10.22 ms |
| normal | 138,027 | 2.44 ms | 10.10 ms |

That is 15 % fewer vertices for ~0.1 ms. At these view distances the city
walls and the fog leave little far terrain on screen, and this GPU is not
vertex-bound. LOD is worth more at `ultra` distances and on old cards with
weak vertex throughput, which is where the presets use it.

The same captures read 23-54 `render_phase/opaque_3d/draws` for the whole
view. Bevy's GPU-driven multi-draw indirect already folds thousands of mesh
instances into a few dozen draws, so merging static map objects into bigger
meshes would buy nothing on a Vulkan or DX12 GPU. It only matters for the GL
tier, which has no indirect draws and pays one call per instance.

## Worked example: drawing the sky last

The sky cube used to be an opaque mesh, binned with the rest of the opaque
phase in no particular order. It shaded the whole screen before the terrain
drew over it. It is now drawn first in the transparent phase, depth-tested
against the opaque scene, so it shades only the pixels nothing covers
(`plugins/skybox.rs`). Same `SCENE=world` view (2026-10-07, AMD integrated
GPU, debug build):

| view | | opaque fragments | transparent fragments | opaque pass GPU | `frame_time.avg` |
|---|---|---|---|---|---|
| top-down (default) | before | 6.92M | 0.05M | 2.77 ms | 9.22 ms |
| | after | 4.84M | 0.05M | 2.73 ms | 9.13 ms |
| horizon (pitch 0.1) | before | 6.05M | 0.63M | 2.79 ms | 10.56 ms |
| | after | 3.97M | 1.07M | 2.50 ms | 10.01 ms |

The 2.07M opaque fragments removed are exactly one 1920x1080 screen. Looking
down, the sky shader was cheap enough to cost almost nothing. Toward the
horizon the frame is 0.55 ms faster, and on a card with little fill rate the
same 2M fragments are worth proportionally more. One sky-last run of the
top-down view read 9.70 ms, and a repeat read 9.13 ms. Single runs on an
integrated GPU vary by about that much, so repeat a reading before trusting a
small difference.

## Levers studied but not built yet

Two options came up in the old-hardware work, were looked at closely, and
were left for later. Each section says why, and what would make it worth
doing.

### Merging static map objects

**Idea.** Combine the static, unanimated parts of a region's map objects
that share a material into one mesh at spawn, the way the terrain already
merges a region's 36 blocks into one ground mesh (`merge_block_meshes`).
Fewer meshes means fewer draw items and less per-entity work.

**Why not now.** On Vulkan or DX12 the whole view already draws in 23–54
opaque draw calls (`render_phase/opaque_3d/draws`, see the terrain LOD
example above). Bevy's GPU-driven multi-draw indirect folds the instances,
so merging would save almost nothing there. It would cost:
- the per-part distance LOD (`commands::mesh_visibility_range`), which works
  per mesh. Merging could only combine parts in the same LOD band.
- the per-object culls and picking, which work per placement.
- the mesh deduplication that lets a common tree share one GPU mesh across
  regions. Merged meshes are unique per region, so memory goes up.

**When it would pay.** The GL 3.3 tier has no indirect drawing and pays one
draw call per mesh instance, which is exactly what merging removes. Measure
`render_phase/*/draws` under `WGPU_BACKEND=gl` in a busy town first. If draw
submission dominates the CPU frame there, build it for that tier only
(e.g. behind `GpuPreprocessingSupport::None`).

### Pausing off-screen effects

**Idea.** `cull_effect_simulation` (`effects/systems.rs`) pauses effects
past the cull distance. Map-prop animation is also paused while off screen
(`animation_culling.rs`, after `OFFSCREEN_GRACE_SECS`). Doing the same for
effects would stop simulating torches and glows behind the camera.

**Why not now.** Pausing an effect stamps `EffectSimPaused` on its whole
node subtree, which moves every one of those entities to another archetype,
and unpausing moves them back. Effects have no visibility signal of their
own while paused (they are hidden), so the test would be a frustum check
on an estimated bounding sphere. Turning the camera would then churn
hundreds of entities between archetypes, which could cost more than the
simulation it saves. The animation cull avoids the same trap with its grace
period, but a prop's pause is one component on one root.

**What would make it worth it.** First, a chrome trace in a dense town that
shows effect simulation (`tick_effect_nodes`, `emit_particles`) as a real
share of the frame. Second, a pause that does not restructure the subtree:
for example a per-root "paused" flag the simulation systems read, instead of
a marker component on every node.

## Worked example: texture memory

GPU memory of the client process, read from Windows' `\GPU Process
Memory(pid_*)` counters (dedicated plus shared), same `SCENE=world` view,
70 s after launch (2026-10-07, AMD integrated GPU, debug build):

| change | GPU memory |
|---|---|
| before: every ground tile also uploaded as its own texture | 1,142 MB |
| ground tiles only in the tile arrays (the per-tile asset is a 1x1 placeholder) | 885 MB |
| plus `graphics.texture_detail: half` | 821 MB |

The ground tiles were uploaded twice: once into the texture arrays the splat
shader samples, and once more as one never-sampled texture per tile. wgpu
suballocates textures from large memory blocks, so the counter moves in coarse
steps. Read single readings as bounds, not exact sizes.

## The GL 3.3 floor

Radeon HD 5000/6000 and GeForce 8 to 200 cards have no Vulkan or DX12 driver.
wgpu's GL backend (`--features gles`, `WGPU_BACKEND=gl`) is their only path,
and their GL 3.3 offers no compute shaders and no storage buffers.
`OPENROAD_GPU_BASELINE=gl33` reproduces those limits on a current GPU.
`graphics.preset: auto` then resolves to `low`, and `FpsOverlayPlugin` is left
out, because its frame-time graph material binds a storage buffer even when the
graph is hidden.

What happened under it (2026-10-07, `SCENE=world`):

- Bevy gates its own compute features. The log reports SSAO, the atmosphere
  and OIT as "not loaded", and GPU light clustering checks the same limits in
  code before it registers (`bevy_pbr` `cluster/mod.rs`).
- **One Bevy 0.19 bug stops the client:** `pbr_opaque_mesh_pipeline` fails
  validation at mesh-view binding 14 (`visibility_ranges`). Without storage
  buffers the shader declares it as a 64-element uniform array (1024 bytes),
  but the bind-group layout keeps the storage path's `min_binding_size` of one
  `Vec4` (16 bytes) (`bevy_pbr` `render/mesh_view_bindings.rs`, the `(14, …)`
  entry). wgpu validates this on every backend, so a real GL 3.3 card fails the
  same way. Bevy's error policy then quits.
- With that one size corrected, the world scene loads and renders correctly,
  with no other validation error. The fix is not carried in the repository,
  because without the runtime WebGL paths below it makes no player's GPU
  work. To re-apply it, vendor `bevy_pbr` 0.19.1 and wire it in with
  `[patch.crates-io]`. Then, in its `src/render/mesh_view_bindings.rs`, give
  the `(14, buffer_layout(...))` entry a `min_binding_size` of
  `64 * Vec4::min_size()` when the binding type is `BufferBindingType::Uniform`,
  instead of `Vec4::min_size()`. Until then `OPENROAD_GPU_BASELINE=gl33`
  stops at this validation error.

The **real** GL backend (`cargo build -p client --features gles`, then
`WGPU_BACKEND=gl`, on a current AMD driver, 2026-10-07) goes further, and
stops short of a playable picture:

- SSAO's compute shader does not translate to GLSL (`textureGatherOffset`),
  so Bevy quits. Fixed: under the GL backend `render_plugin` caps storage
  textures at 4, and Bevy then skips SSAO, as it would on a real GL 3.3 card
  that has none. `graphics.preset: auto` resolves to `low` there.
- The world then runs (~50 FPS on this iGPU) and the terrain and characters
  draw, but **every map object is missing**, and wgpu logs thousands of "view
  dimension heuristics" errors. wgpu's GL backend picks a texture's GL type
  from its layer count when the texture is created: 1 layer is a 2D texture,
  6 is a cube, any other count a 2D array. A *cube array* is never possible.
  Bevy binds its shadow maps to every PBR material as a 2D array and a cube
  array. With no shadow-casting light those textures hold 1 and 6 layers,
  and a cube array cannot exist on GL anyway.
- Bevy already has the non-array shader path for exactly this
  (`NO_ARRAY_TEXTURES_SUPPORT`, `NO_CUBE_ARRAY_TEXTURES_SUPPORT`). But it is
  chosen by compile-time `cfg(feature = "webgl", target_arch = "wasm32")`,
  so a native build never takes it. Making it a runtime choice means changing
  ~52 `cfg` sites across `bevy_pbr` (34), `bevy_core_pipeline` (11),
  `bevy_render` (6) and `bevy_light` (1): the shader defs, the bind-group
  layouts, the shadow texture views and the WebGL-only light limits. That is
  a fork of Bevy's renderer to maintain, not a local fix, so it has not been
  done.
- Our own part is fixed: the ground-tile arrays are never created with
  exactly one layer (`tile_arrays.rs`), so GL makes them arrays.

So on Windows the floor stays at DX12/Vulkan GPUs (ADR 0011). A GL 3.3 tier
needs either Bevy to choose its WebGL2 paths at runtime, upstream, or that
fork.

## Gotcha: Windows caps an unfocused window at 60 FPS

Readings of exactly 60.0 FPS / 16.67 ms with `render/pipeline/acquire_ms` near 1 ms
mean the game window was not in the foreground — typing into a terminal next to it is
enough. The cap hides any improvement above 60. Keep the game window focused while
sampling, and treat a flat 16.67 ms as "capped", not as a result.

## Gotcha: a background `cargo`/`rustc` build skews every reading

Discovered mid-session the hard way: a `make perf fps`/`snapshot` reading can
silently include CPU contention from an unrelated background compile running
on the same machine. A shadows toggle once appeared to make frame time
*worse* by 2-3x and kept climbing over several readings — restoring the
setting didn't recover it either, which is what exposed the real cause:
leftover `rustc.exe`/`cargo.exe` processes from an earlier crashed build were
still running and starving the client of CPU. Killing them dropped
`frame_time.avg` from ~90ms back to ~19ms with no config change at all.
**Before trusting any BRP perf delta, check `tasklist` (or equivalent) for
stray `rustc`/`cargo`/`link` processes first** — a real effect and "something
else is compiling in the background" look identical in the numbers, and only
one of them is what you're testing.

## Offline analysis of samples

```bash
jq -s 'map(.frame_time.value) | add/length' before.jsonl     # mean frame time
jq -c '{t: .t_ms, particles: ."world_counts/particles".value}' before.jsonl
```

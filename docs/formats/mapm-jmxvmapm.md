# MapMesh (*.m) - JMXVMAPM

The terrain heightmap of one world region: 6×6 blocks, each a 17×17 vertex grid
plus a 16×16 tile grid.

Layout derived from openroad's parser (`client/src/assets/m/mod.rs`) and
corpus-verified across 4,674 regions. Upstream reference:
`SilkroadDoc.wiki/JMXVMAPM` —
https://github.com/DummkopfOfHachtenduden/SilkroadDoc/wiki/JMXVMAPM

## Layout

| size | type | field | notes |
|---|---|---|---|
| 12 | char[12] | `Signature` | `JMXVMAPM1000` |

Then 36 blocks, iterated `z` outer, `x` inner (6 × 6). The region's vertex grid
is therefore 97×97 (`TERRAIN_NUM_VERTICES`), the blocks sharing edge vertices.

### Block header

| size | type | field | notes |
|---|---|---|---|
| 4 | u32 LE | `Flag` | `0` none, `1` culled |
| 2 | u16 LE | `EnvironmentID` | keys into `environment.ifo` |

### Vertices — 17 × 17, `z` outer

| size | type | field | notes |
|---|---|---|---|
| 4 | f32 LE | `Height` | |
| 2 | u16 LE | `Texture` | bits 0-9 tile id (`tile2d.ifo`), bits 10-15 see below |
| 1 | u8 | `Brightness` | |

The upper bits of `Texture` are named "Scale" upstream. **That interpretation is
withdrawn** — see the next section; openroad renders at a constant repeat factor
and the field's meaning is UNKNOWN.

### Water

| size | type | field | notes |
|---|---|---|---|
| 1 | i8 | `WaterType` | `-1` none, `0` water, `1` ice |
| 1 | u8 | `WaterWaveType` | not a wave parameter — see below |
| 4 | f32 LE | `WaterHeight` | |

#### `WaterWaveType` is a water-coverage class, not a wave strength (census 2026-10-23)

Measured over this corpus (4,644 valid regions, 167,184 blocks) by comparing each
block's 289 vertex heights against its own `WaterHeight`. "Coverage" below is the
share of those vertices that lie *below* the water plane.

| value | water blocks | coverage | reading |
|---|---|---|---|
| 0 | 5,937 (16.1%) | exactly 0.000 | the plane is below every vertex: no water surface in this block |
| 1 | 23,988 (64.9%) | exactly 1.000 | every vertex is under it: the block is fully submerged |
| 2 | 2,963 (8.0%) | 0.003 .. 0.997 | the water line crosses this block |
| 3 | 4,088 (11.1%) | 0.003 .. 0.997 | the water line crosses this block |

On the 36,976 `WaterType == 0` blocks that is **exceptionless in all three
directions**: no `0` with a vertex below the plane, no `1` with a vertex above it,
no `2`/`3` with full or empty coverage. Read as two bits, bit 1 is "the water line
crosses this block" (set for exactly `2` and `3`).

So the field carries no wave, swell or current information: it restates what the
vertex grid already says. Hanging a wave strength on it would make water calm or
rough depending on whether a block is a shore block.

**What separates `2` from `3` is UNKNOWN.** Refuted over the 7,051 partial blocks:
coverage amount (`2` quartiles 0.121/0.388/0.723, `3` 0.287/0.616/0.886 — a tilt, but
both span 0.003..0.997), the number of the block's four corners under water (`2`
spread 261/929/1171/519/83 over 0..4, `3` 68/786/1800/1317/117), the `(0,0)` vertex,
the centre vertex, and the region (256 regions contain both values). It is also not a
property of a water *body*: of the 8,021 adjacent partial-block pairs that sit at one
and the same `WaterHeight`, 729 (9.1%) disagree — so the value is spatially clustered
but flips inside one continuous surface.

**On ice (`WaterType == 1`, 955 blocks) the byte is only approximately true**: 103
blocks contradict the reading — 8 of the 111 `0`-blocks have something below the
plane, 20 of the 641 `1`-blocks have something above it, and 75 of the 203 `2`/`3`
blocks are fully covered or empty. The vertex grid is therefore the source that
cannot be wrong, not this byte.

Reproduce: `python3 scripts/re/census_water_wave_type.py --map-dir <your Map>`.

Consequence recorded in `plugins/map/terrain`: a plane that lies below every vertex
of its own block cannot be seen (the terrain is a triangulated heightfield, and a
linear interpolant's minimum is at a vertex), so 5,937 water and 109 ice planes were
submitted for nothing before 2026-10-23.

### Tiles — 16 × 16, `z` outer

| size | type | field | notes |
|---|---|---|---|
| 2 | u16 LE | `Flag` | `1` = manually blocked |

### Block footer

| size | type | field | notes |
|---|---|---|---|
| 4 | f32 LE | `HeightMax` | highest point, objects included |
| 4 | f32 LE | `HeightMin` | lowest point, objects included |
| 20 | u8[20] | `Reserved` | UNKNOWN — byte 0 is 1 in some blocks |

### The vertex "Scale" field does NOT drive texture tiling (playtest verdict 2026-08-10)

The community wiki names bits 10-15 of `vertex.Texture` "Scale" with no
semantics. openroad long interpreted it as a per-vertex tiling-density code —
that interpretation is now **withdrawn**: side-by-side playtests against the
vanilla client (city pavement for code 16; the census cluster centroids for
24/32, reachable via the dev Teleport window's "Splat code" destinations)
show **every ground texture tiles at a constant repeat factor of 0.25 — one
repeat per 80 world units — regardless of the field's value**. The constant
also eliminates the hard tiling seams that any per-texel varying factor
produces (the original, had it scaled at all, would have interpolated
per-vertex UVs — but the point is moot at a constant factor).

What the field ACTUALLY means is **UNKNOWN**. Data facts from the full
Map.pk2 census (`probe_splat_scale_census` in `client/src/assets/m/mod.rs`;
4674 regions, 5 parse failures on degenerate border regions):

| Code | Vertices | Notes |
|--|--|--|
| 0  | 22,629,310 | |
| 8  | 5,656,622 | |
| 16 | 20,107,524 | dominant on city ground (tile 29: 1.01M of its 1.02M vertices) |
| 24 | 233,670 | clusters, e.g. regions 83x94 / 90x89 (mirrored pair) |
| 32 | 1,170 | tiny patches, e.g. regions 156x98, 161x100 |

- Only multiples of 8 occur → really a 3-bit value 0..4 in bits 13-15.
- It is genuinely per-vertex, not a texture property: 435 of 685 tile ids
  carry several codes (tile 7 splits ~50/50 between codes 0 and 8).
- Ruled out: tiling density (this section), per-texture property (census).
  Open hypotheses: editor metadata, LOD/detail hint, lighting/AO-related.
  Do not implement semantics for it without new evidence.

Interpretation history, so nobody re-derives a dead end: 24 → 0.125 /
32 → 0.0625 (first guess, inverted); ×2-per-+8 (0.25/0.5/1.0/2.0/4.0 —
far too coarse in-game); mod-3 wrap (0.25/0.5/1.0/0.25/0.5 — refuted on
city pavement, where code 16 also matches 0.25).

Openroad renders with a constant 0.25 (shader `get_splat_scale` defaults +
legacy `Mesh::from(JMXVMAPM)`); the per-code factors remain live-tunable in
the render-debug panel (`terrain_splat_factor_*`, shared
`TerrainRenderParams` buffer) should new evidence appear.

## Corpus re-verification (openroad, 2026-08-12)

Re-probed over `<pk2-corpus>/Map` (4,649 `.m`): block stride 2,575 B, file
92,712 B, **EOF-exact on 4,644/4,649**. The 5 non-conforming files are all in
the world-origin corner (`Map/0/{0,1,2}.m`, `Map/1/{0,1}.m`) — valid signature,
bodies that are not 36 blocks.

- `Block.Flag = 1` (Culled) occurs in only **2** of 167,184 blocks.
- `WaterType` ∈ {−1 ×129,253, 0 ×36,976, 1 (ice) ×955}; `WaterWaveType` ∈ {0,1,2,3}
  on every block of every valid file, water or not (the byte is not cleared where
  there is no water: 3,430 of the 129,253 `WaterType == -1` blocks carry a non-zero
  value, so a consumer must gate on `WaterType` first). Meaning: see the Water section.
- Texture ids span **0..718** (comfortably inside the 10-bit field).
- The bits-10-15 "Scale" code is **always a multiple of 8** (0 ×23.2M, 16 ×19.1M,
  8 ×5.8M, 24 ×218,685, 32 ×770) ⇒ bits 10-12 are always zero and the field is
  really 3 bits at 13-15.
- Heights span −21,589..6,803 with **zero NaN** in any valid file.
- `HeightMax`/`HeightMin` **include objects** — 384 blocks exceed their own
  vertex range, so they are not a terrain-only bound.
- Reserved[20] is zero in every block except byte 0 ∈ {0,1}.
- Note: the splat-scale census recorded above (4,674 regions / 48,628,296
  vertices) does not match this corpus (4,644 / 48,316,176) — same distribution
  shape, different region set; worth reconciling.


# Vertex shaders (`Data/shader/*.c`) — the original's own rendering math

The v1.188 client ships its vertex shaders as **D3D assembly source**, not as
compiled blobs: 40 `.c` files under `Data/shader/` (31), `Data/shader_maptool/`
(7) and `Particles/shader/` (2). They are plain text with C preprocessor
directives, so the original's lighting, fog and skinning math is readable
directly — no decompilation involved.

This matters for ADR 0009: where the original is the tie-breaker, this is the
tie-breaker in source form. Nothing here is copied into the repository; the
facts below are cited by path so a reader can open the user's own files.

## Layout: techniques compose bodies

Four technique files are the entry points. Each `#include`s one or more shader
bodies and defines the permutation switches:

| technique | includes | covers |
|---|---|---|
| `vstech_o11.c` (1170 lines) | `vss0.c`, `vss2.c` | objects and characters |
| `vstech_m11.c` (322 lines) | `vsm0.c`, `vss0.c` | map / terrain |
| `vstech_m2sw.c` (322 lines) | `vsm0.c`, `vss0.c` | second map variant |
| `vstech_p11.c` (14 lines) | `vsp0.c` | particles |

The bodies carry no `vs.x.y` directive of their own — the technique supplies it.
Vertex declarations differ per body, and that is what each one is for:

| body | `dcl_` inputs |
|---|---|
| `vss0.c` | position, normal, texcoord0 |
| `vss2.c` | position, normal, texcoord0, **blendindices, blendweight** |
| `vsm0.c` | position, color0, texcoord0, **texcoord1** (second UV set) |
| `vsp0.c` | position, color0, texcoord0 |

The `vertexshader0*.c` / `vertexshader2*.c` families (23 files) are
pre-expanded single permutations of the same material — `0` without skinning,
`2` with it.

## The constant register ABI

Every shader shares one constant layout (`Data/shader/vss2.c`, the `VS_CONSTANT`
block). The comments in the `c8` line are the file's own:

| register | name | note |
|---|---|---|
| `c0` | `VSC_WORLD_VIEW_PROJECTION` | WVP, 4 registers |
| `c4` | `VSC_WORLDVIEW` | World, 4 registers |
| `c8` | `VSC_UTIL` | `1.0f, 100.0f, 0.0f, 765.01f` |
| `c9` | `VSC_DLIGHT_DIRECTION` | directional light |
| `c10` | `VSC_DLIGHT_AMBIENT` | |
| `c11` | `VSC_DLIGHT_DIFFUSE` | |
| `c12` | `VSC_PLIGHT_POSITION` | point light |
| `c13` | `VSC_PLIGHT_AMBIENT` | |
| `c14` | `VSC_PLIGHT_DIFFUSE` | |
| `c15` | `VSC_PLIGHT_ATTENUATION` | |
| `c16` | `VSC_FOG` | |
| `c17` | `VSC_ENV_TEXTURE_MATRIX` | 4 registers |
| `c21` | `VSC_WORLD_POS` | 1 register |
| `c22` | `VSC_HEIGHTFOG` | |
| `c23` | `VSC_TEXTUREANI_MATRIX` | 3 registers |
| `c26` | `VSC_COMMON` | bone matrices start here |

## Skinning: two weights, and a bone budget that follows from `c26`

`vss2.c` indexes bone matrices as `c[a0.x + VSC_COMMON]` and transforms with
`m4x3` — three constant registers per bone, starting at `c26`. The budget is
therefore arithmetic, not a guess:

```
vs_1_1:  (96 - 26) / 3 = 23 bones
vs_2_0: (256 - 26) / 3 = 76 bones
```

The switch names say two weights, not four: `VS_DCL_SKIN2`,
`VS_POS_NORMAL_SKIN2`, and a `VS_POS_NORMAL_SKIN2_V2` variant beside it. The
rigid path is the `SKIN0` family in the same files.

OpenRoad does not inherit this limit — it uses Bevy's `SkinnedMesh`, which has
its own budget — but the number is what the original's own meshes were authored
against, so it is the figure to check a skeleton against before blaming a
renderer.

## The permutation matrix (characters, `vstech_o11.c`)

Four orthogonal axes:

- **skin**: `SKIN0` (rigid) or `SKIN2` (two weights)
- **fog**: `FOGEXP`, `FOGLINEAR` or none — each with or without `_HFOG`
  (height fog, `c22`)
- **light**: default, `_NOLIGHT`, or `_PLIGHT` (point light)
- **uv**: base, `_MTEX` (multi-texture), `_SPEC` (specular), `_TEXANI`
  (+`_STAGE2`), `_LIGHTMAP`

The map techniques swap the light axis for a lightmap one, which is what
`vsm0.c`'s second UV set is for.

## Status

Catalogued 2026-11-02. Nothing in `client/` reads these files today — they are a
reference for rendering decisions, not an asset the client loads. The pixel
shaders, if the install carries them, are not covered here.

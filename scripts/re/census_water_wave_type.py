#!/usr/bin/env python3
"""`WaterWaveType` census over a Map.pk2 extraction (JMXVMAPM `*.m`).

The block header of every terrain block carries a `WaterWaveType` byte whose
meaning the format references leave blank. The name suggests a wave parameter —
something a water shader could read. This census answers that from the data
instead of guessing, and the answer is no: on every water block of the
reference corpus the byte restates what the block's own 17x17 vertex grid
already says.

Method: for each block, compare all 289 vertex heights against that block's
`WaterHeight` and call the share that lies below it the block's *coverage*.
Then cross the coverage against the byte. A byte that carried wave information
would be free of that relationship; one that encodes coverage is pinned by it.

What the run prints, and what each part decides:

* the value/coverage table, plus the number of blocks that contradict the
  reading in each direction (0 with something below the plane, 1 with
  something above it, 2/3 with full or empty coverage)
* the same table for ice, where the byte is only approximately true
* the tests that fail to separate 2 from 3 (coverage quantiles, how many of the
  block's four corners are under water, per-region mixing, and whether two
  adjacent blocks of one continuous surface agree)
* how many water planes a renderer would draw for a block whose terrain is
  entirely above them

The corpus is the user's own extraction and is not in this repository, so a
missing directory is reported and the run exits 0 rather than failing.

Run: `python3 scripts/re/census_water_wave_type.py [--map-dir DIR]`
     (`OPENROAD_MAP_DIR` is the environment fallback; default `./Map`)
"""

import argparse
import collections
import os
import pathlib
import struct
import sys

VERTEX = struct.Struct("<fHB")  # Height, Texture, Brightness
VERTICES = 17 * 17
BLOCK_SIZE = 2575
BLOCKS = 36
FILE_SIZE = 12 + BLOCKS * BLOCK_SIZE
WATER = struct.Struct("<bBf")  # WaterType, WaterWaveType, WaterHeight
WATER_AT = 6 + VERTICES * VERTEX.size  # after header (6) and vertices


def blocks(data):
    """Yield `(index, wave, water_type, water_height, below, corners)` per block."""
    for i in range(BLOCKS):
        base = 12 + i * BLOCK_SIZE
        water_type, wave, height = WATER.unpack_from(data, base + WATER_AT)
        verts = data[base + 6 : base + WATER_AT]
        heights = [h for h, _, _ in VERTEX.iter_unpack(verts)]
        below = sum(1 for h in heights if h < height)
        # The 17x17 grid is row-major in z, so these are the block's corners.
        corners = sum(1 for k in (0, 16, 272, 288) if heights[k] < height)
        yield i, wave, water_type, height, below, corners


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--map-dir",
        default=os.environ.get("OPENROAD_MAP_DIR", "Map"),
        help="directory holding the extracted Map.pk2 (<z>/<x>.m)",
    )
    args = parser.parse_args()
    root = pathlib.Path(args.map_dir)
    if not root.is_dir():
        print(f"{root} is not a directory — supply an extracted Map.pk2 to census")
        return 0

    files = sorted(root.glob("*/*.m"))
    if not files:
        print(f"no <z>/<x>.m files under {root}")
        return 0

    cover = collections.defaultdict(collections.Counter)  # (water_type, wave) -> class
    corner_hist = collections.defaultdict(collections.Counter)
    region_values = collections.defaultdict(set)
    partial_cov = collections.defaultdict(list)
    waves = collections.defaultdict(collections.Counter)
    surface = {}  # (gz, gx) -> (wave, height) for water blocks
    skipped = []
    for path in files:
        data = path.read_bytes()
        if len(data) != FILE_SIZE:
            skipped.append((str(path), len(data)))
            continue
        rz, rx = int(path.parent.name), int(path.stem)
        for i, wave, water_type, height, below, corners in blocks(data):
            waves[water_type][wave] += 1
            if water_type not in (0, 1):
                continue
            klass = "none" if below == 0 else "full" if below == VERTICES else "partial"
            cover[(water_type, wave)][klass] += 1
            if water_type == 0:
                region_values[(rz, rx)].add(wave)
                bz, bx = divmod(i, 6)
                surface[(rz * 6 + bz, rx * 6 + bx)] = (wave, height)
                if klass == "partial":
                    corner_hist[wave][corners] += 1
                    partial_cov[wave].append(below / VERTICES)

    print(f"{len(files) - len(skipped)} regions censused, {len(skipped)} skipped")
    for path, size in skipped:
        print(f"  skipped {path}: {size} bytes, not {FILE_SIZE}")

    print("\nWaterWaveType values, by WaterType")
    for water_type in sorted(waves):
        print(f"  WaterType {water_type:>3}: {dict(sorted(waves[water_type].items()))}")

    for water_type, label in ((0, "water"), (1, "ice")):
        print(f"\ncoverage of the 289 vertices, {label} blocks")
        for wave in sorted(v for t, v in cover if t == water_type):
            counts = cover[(water_type, wave)]
            total = sum(counts.values())
            print(
                f"  {wave}: {total:>6} blocks  "
                f"none={counts['none']:>6} partial={counts['partial']:>6} "
                f"full={counts['full']:>6}"
            )
        contradictions = (
            cover[(water_type, 0)]["partial"]
            + cover[(water_type, 0)]["full"]
            + cover[(water_type, 1)]["none"]
            + cover[(water_type, 1)]["partial"]
            + sum(cover[(water_type, v)][k] for v in (2, 3) for k in ("none", "full"))
        )
        print(f"  contradicting the reading: {contradictions}")

    print("\n2 vs 3 — what does not separate them")
    for wave in (2, 3):
        cov = sorted(partial_cov[wave])
        if not cov:
            continue
        q = [cov[int(p * (len(cov) - 1))] for p in (0.0, 0.25, 0.5, 0.75, 1.0)]
        print("  coverage %d: min/q1/median/q3/max = %s" % (wave, [round(v, 3) for v in q]))
    for wave in (2, 3):
        print(f"  corners under water {wave}: {dict(sorted(corner_hist[wave].items()))}")
    mixed = sum(1 for values in region_values.values() if {2, 3} <= values)
    print(f"  regions using both 2 and 3: {mixed} of {len(region_values)} water regions")
    agree = differ = 0
    for (gz, gx), (wave, height) in surface.items():
        for dz, dx in ((0, 1), (1, 0)):
            other = surface.get((gz + dz, gx + dx))
            if not other or other[1] != height or wave not in (2, 3):
                continue
            if other[0] not in (2, 3):
                continue
            if other[0] == wave:
                agree += 1
            else:
                differ += 1
    pairs = agree + differ
    if pairs:
        print(
            f"  adjacent partial blocks at one water height: {agree} agree, "
            f"{differ} differ ({100.0 * differ / pairs:.1f}%)"
        )

    hidden = cover[(0, 0)]["none"]
    hidden_ice = sum(cover[(1, v)]["none"] for v in sorted(v for t, v in cover if t == 1))
    water_total = sum(sum(cover[(0, v)].values()) for v in (0, 1, 2, 3))
    ice_total = sum(sum(cover[(1, v)].values()) for v in (0, 1, 2, 3))
    print("\nplanes a renderer would draw under their own terrain")
    print(f"  water: {hidden} of {water_total} ({100.0 * hidden / water_total:.1f}%)")
    if ice_total:
        print(f"  ice:   {hidden_ice} of {ice_total} ({100.0 * hidden_ice / ice_total:.1f}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())

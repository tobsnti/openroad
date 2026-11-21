#!/usr/bin/env python3
"""Ratchet on the original binary's addresses and source-file names in the repository.

CONTRIBUTING.md keeps decompiler output out of the repository, function
addresses and the original's internal source-file names included, but the tree
still carries a few hundred from before that rule. This gate counts them per
top-level directory and holds each count at its ceiling below. A new one fails
the gate, and so does a removal that the ceiling does not follow, so the
counts can only go down.

Three shapes are counted:
- a decompiler name, `FUN_`, `DAT_`, `LAB_` or `PTR_` followed by the address;
- a hex literal in the image range of a 32-bit Windows executable (from
  0x400000, below 0x1000000), written with six digits or with eight and a
  leading `00`;
- a source-file name as the original's embedded paths spell them, a CamelCase
  `<Name>.cpp` or `<Name>.h`.
A literal whose low 16 bits are zero is a mask or a base (`0x00ff0000`,
`0x400000`) and is not counted; neither are Blowfish's S-boxes, whose digits
of pi only look like addresses. A constant that is not an address takes a
digit separator (`0x80_0000`), which the pattern does not match. A foreign
source file of the same shape is not counted when `FOREIGN_SOURCES` names it
with its licence class, so a new one takes a deliberate line there.

Run: `python3 scripts/check_re_addresses.py`
"""

import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (decompiler names, addresses, source-file names) per top-level directory; a
# directory that is not listed has a ceiling of (0, 0, 0).
CEILING = {
    "assets": (0, 1, 0),
    "client": (17, 73, 4),
    "docs": (161, 182, 16),
    "packets": (178, 48, 9),
    "scripts": (8, 10, 11),
}

NAME = re.compile(r"(?:FUN|DAT|LAB|PTR)_[0-9a-fA-F]{6,8}")
ADDRESS = re.compile(r"\b0x(?:00)?[4-9a-fA-F][0-9a-fA-F]{5}\b")
SOURCE = re.compile(r"\b[A-Z][A-Za-z0-9_]*\.(?:cpp|h)\b")
NOT_ADDRESSES = {"bevy_pk2/src/pk2/blowfish.rs", "sro_net/src/blowfish.rs"}
# Source files of other projects that the tree names on purpose, with the
# project and its licence class (AGENTS.md, "External sources").
FOREIGN_SOURCES = {
    "TracyClient.cpp": "Tracy profiler, BSD-3-Clause, built by `make profile tracy`",
    "Bmt.cpp": "SroPk2Studio, MIT",
    "Bmt.h": "SroPk2Studio, MIT",
}
KINDS = ("decompiler names", "addresses", "source-file names")


def find(line: str) -> list[tuple[int, str]]:
    """The (kind, token) pairs in one line; kind indexes `KINDS`."""
    found = [(0, name) for name in NAME.findall(line)]
    found += [(1, a) for a in ADDRESS.findall(line) if int(a, 16) & 0xFFFF]
    found += [(2, s) for s in SOURCE.findall(line) if s not in FOREIGN_SOURCES]
    return found


def scan() -> list[tuple[str, int, int, str]]:
    """Every (path, line, kind, token) in the tracked text files."""
    listed = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout.decode()
    hits = []
    for path in filter(None, listed.split("\0")):
        if path in NOT_ADDRESSES:
            continue
        data = (ROOT / path).read_bytes()
        if b"\0" in data:
            continue
        for number, line in enumerate(data.decode(errors="replace").splitlines(), 1):
            hits += [(path, number, kind, token) for kind, token in find(line)]
    return hits


def directory(path: str) -> str:
    return path.split("/")[0] if "/" in path else "."


def problems(counts: Counter, ceiling: dict) -> list[tuple[str, int, int, int]]:
    """(directory, kind, count, ceiling) wherever a count is not its ceiling."""
    return [
        (top, kind, counts[top, kind], ceiling.get(top, (0, 0, 0))[kind])
        for top in sorted({d for d, _ in counts} | set(ceiling))
        for kind in range(len(KINDS))
        if counts[top, kind] != ceiling.get(top, (0, 0, 0))[kind]
    ]


def main() -> int:
    hits = scan()
    counts = Counter((directory(path), kind) for path, _, kind, _ in hits)
    found = problems(counts, CEILING)
    if not found:
        total = Counter(kind for _, _, kind, _ in hits)
        rows = ", ".join(f"{d} {n}/{a}/{s}" for d, (n, a, s) in sorted(CEILING.items()))
        print(
            f"check_re_addresses: OK ({total[0]} decompiler names, {total[1]} addresses, "
            f"{total[2]} source-file names; per directory: {rows})"
        )
        return 0
    print("check_re_addresses: what the tree names of the original binary changed\n")
    for top, kind, have, cap in found:
        if have < cap:
            print(f"  {top}: {have} {KINDS[kind]}, ceiling {cap}; lower the ceiling to {have}")
            continue
        print(f"  {top}: {have} {KINDS[kind]}, ceiling {cap}; found:")
        for path, number, k, token in hits:
            if k == kind and directory(path) == top:
                print(f"    {path}:{number}  {token}")
    print(
        "\nDecompiler names, addresses and source-file names of the original binary stay"
        "\nin the local RE store (CONTRIBUTING.md, \"What this repository does not"
        "\npublish\"). A constant that only looks like an address takes a digit separator"
        "\n(0x80_0000); another project's source file is named in FOREIGN_SOURCES with its"
        "\nlicence class. After a removal, set the ceiling in scripts/check_re_addresses.py"
        "\nto the new count."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())

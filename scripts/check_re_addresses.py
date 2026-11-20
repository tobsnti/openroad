#!/usr/bin/env python3
"""Ratchet on the original binary's addresses in the repository.

CONTRIBUTING.md keeps decompiler output out of the repository, function
addresses included, but the tree still carries a few hundred from before that
rule. This gate counts them per top-level directory and holds each count at
its ceiling below. A new address fails the gate, and so does a removal that
the ceiling does not follow, so the counts can only go down.

Two shapes are counted:
- a decompiler name, `FUN_`, `DAT_`, `LAB_` or `PTR_` followed by the address;
- a hex literal in the image range of a 32-bit Windows executable (from
  0x400000, below 0x1000000), written with six digits or with eight and a
  leading `00`.
A literal whose low 16 bits are zero is a mask or a base (`0x00ff0000`,
`0x400000`) and is not counted; neither are Blowfish's S-boxes, whose digits
of pi only look like addresses. A constant that is not an address takes a
digit separator (`0x80_0000`), which the pattern does not match.

Run: `python3 scripts/check_re_addresses.py`
"""

import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# (decompiler names, addresses) per top-level directory; a directory that is
# not listed has a ceiling of (0, 0).
CEILING = {
    "assets": (0, 1),
    "client": (17, 73),
    "docs": (161, 182),
    "packets": (178, 48),
    "scripts": (8, 10),
}

NAME = re.compile(r"(?:FUN|DAT|LAB|PTR)_[0-9a-fA-F]{6,8}")
ADDRESS = re.compile(r"\b0x(?:00)?[4-9a-fA-F][0-9a-fA-F]{5}\b")
NOT_ADDRESSES = {"bevy_pk2/src/pk2/blowfish.rs", "sro_net/src/blowfish.rs"}
KINDS = ("decompiler names", "addresses")


def find(line: str) -> list[tuple[int, str]]:
    """The (kind, token) pairs in one line; kind indexes `KINDS`."""
    found = [(0, name) for name in NAME.findall(line)]
    found += [(1, a) for a in ADDRESS.findall(line) if int(a, 16) & 0xFFFF]
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
        (top, kind, counts[top, kind], ceiling.get(top, (0, 0))[kind])
        for top in sorted({d for d, _ in counts} | set(ceiling))
        for kind in (0, 1)
        if counts[top, kind] != ceiling.get(top, (0, 0))[kind]
    ]


def main() -> int:
    hits = scan()
    counts = Counter((directory(path), kind) for path, _, kind, _ in hits)
    found = problems(counts, CEILING)
    if not found:
        total = Counter(kind for _, _, kind, _ in hits)
        rows = ", ".join(f"{d} {n}/{a}" for d, (n, a) in sorted(CEILING.items()))
        print(
            f"check_re_addresses: OK ({total[0]} decompiler names, {total[1]} addresses; "
            f"per directory: {rows})"
        )
        return 0
    print("check_re_addresses: the original binary's addresses changed\n")
    for top, kind, have, cap in found:
        if have < cap:
            print(f"  {top}: {have} {KINDS[kind]}, ceiling {cap}; lower the ceiling to {have}")
            continue
        print(f"  {top}: {have} {KINDS[kind]}, ceiling {cap}; found:")
        for path, number, k, token in hits:
            if k == kind and directory(path) == top:
                print(f"    {path}:{number}  {token}")
    print(
        "\nDecompiler names and addresses of the original binary stay in the local RE"
        "\nstore (CONTRIBUTING.md, \"What this repository does not publish\"). A constant"
        "\nthat only looks like an address takes a digit separator (0x80_0000). After a"
        "\nremoval, set the ceiling in scripts/check_re_addresses.py to the new count."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())

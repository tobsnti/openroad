#!/usr/bin/env python3
"""Count unfilled rows in a textuisystem.txt and compare the two error tables against it.

Why a script and not a unit test: the data belongs to the user and is not in the
repository, so the number can only be produced where the data is. The parser here
is a replica of `UiSystemText::parse` (client/src/assets/textdata/uisystem.rs):
UTF-16LE, tab separated, column 0 must be "1", column 1 is the key, and the value
is the LAST non-empty column. Cross-check: the client logs "loaded 5324 UI strings"
for the same file, which is the row count this replica reports.

Usage: count_unfilled_rows.py <textuisystem.txt> <openroad checkout>
"""
import re
import sys


def parse_rows(path):
    raw = open(path, "rb").read()
    text = raw.decode("utf-16-le" if raw[:2] == b"\xff\xfe" else "utf-8", errors="replace")
    rows = {}
    for line in text.splitlines():
        fields = line.split("\t")
        if len(fields) < 3 or fields[0].lstrip("\ufeff") != "1":
            continue
        key = fields[1].strip()
        if not key:
            continue
        value = next((f.strip() for f in reversed(fields[2:]) if f.strip()), None)
        if value is not None:
            rows[key] = value
    return rows


def table_pairs(checkout):
    """(key, fallback) pairs of login_error_text and lobby_error_text."""
    pairs, seen = [], set()
    for rel, fn in (("packets/src/login.rs", "login_error_text"),
                    ("packets/src/agent/lobby.rs", "lobby_error_text")):
        src = open(f"{checkout}/{rel}").read()
        body = src[src.index(f"pub fn {fn}"):]
        body = body[:body.index("\n}\n") if "\n}\n" in body else body.index("\n}")]
        for m in re.finditer(r'"(UI[A-Z0-9_]+)",\s*\n?\s*(?:fallback:\s*)?"((?:[^"\\]|\\.)*)"', body):
            if m.group(1) in seen:
                continue
            seen.add(m.group(1))
            pairs.append((m.group(1), m.group(2).encode().decode("unicode_escape")))
    return pairs


def main():
    rows = parse_rows(sys.argv[1])
    unfilled = [k for k, v in rows.items() if v.strip().isdigit() or not v.strip()]
    print(f"rows parsed:              {len(rows)}")
    print(f"rows that are digits only:{len(unfilled):>6}")
    print(f"rows that are exactly 0:  {sum(1 for k in unfilled if rows[k] == '0'):>6}")
    print()
    same = differ = missing = 0
    for key, fallback in table_pairs(sys.argv[2]):
        data = rows.get(key)
        if data is None:
            missing += 1
            print(f"MISSING    {key}")
        elif data == fallback:
            same += 1
        else:
            differ += 1
            print(f"DIFFERS    {key}\n  data:     {data!r}\n  fallback: {fallback!r}")
    print()
    print(f"error-table keys: {same + differ + missing}  identical: {same}  differs: {differ}  missing: {missing}")


if __name__ == "__main__":
    main()

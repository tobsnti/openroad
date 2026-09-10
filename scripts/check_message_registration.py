#!/usr/bin/env python3
"""Refuse a MessageReader whose message type nobody registers.

# The idea

Bevy does not skip a system whose `MessageReader<T>` has no registered `T` —
the parameter fails validation and the panic takes the whole schedule with it.
The client then dies at startup, in every scene that runs the system.

Neither of the two checks we run before merging can see this:
`make ci` builds and unit-tests but never starts the app, and the `NETCHECK`
smoke run returns before the full app is assembled (`client/src/main.rs`), so
it only proves the network path. A `MessageReader<QuestUpdate>` can reach a
branch this way and panic every HUD scene; finding it needs a window run, which
costs minutes. This check costs seconds.

There are two ways a message becomes registered, and both count:
  * the `packets!` macro in `packets/src/lib.rs` calls `add_message::<$name>()`
    for every opcode it declares, so every packet type is registered by import;
  * anything else — HUD or scene messages — needs an explicit `add_message::<T>()`.

Bevy's own messages (`MouseMotion`, `AssetEvent<_>`, …) come from
`DefaultPlugins` and are listed as known-external instead of being guessed at.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLIENT = ROOT / "client" / "src"
PACKETS_LIB = ROOT / "packets" / "src" / "lib.rs"

# Registered by Bevy itself (DefaultPlugins) rather than by this workspace.
# Messages bevy itself registers (winit/window plugins), so no `add_message`
# of ours can exist for them.
EXTERNAL = {
    "MouseMotion",
    "MouseWheel",
    "KeyboardInput",
    "CursorMoved",
    "Packet",
    "WindowCloseRequested",
    "WindowDestroyed",
}

READER = re.compile(r"MessageReader<\s*(?:'[a-z_]+\s*,\s*)*(?:'[a-z_]+\s*,\s*)?([A-Za-z0-9_:<>]+)\s*>")
ADD = re.compile(r"add_message::<\s*([A-Za-z0-9_:<>]+)\s*>")
OPCODE = re.compile(r"=>\s*([A-Za-z_][A-Za-z0-9_]*)")
QUOTED = re.compile(r'"[^"]*"')


def bare(name: str) -> str:
    """`packets::agent::quest::QuestUpdate` -> `QuestUpdate`."""
    return name.split("::")[-1]


def main() -> int:
    if not CLIENT.is_dir() or not PACKETS_LIB.is_file():
        print("check_message_registration: cannot find the tree", file=sys.stderr)
        return 2

    registered = {bare(m) for m in OPCODE.findall(PACKETS_LIB.read_text(errors="ignore"))}
    n_from_macro = len(registered)

    readers: dict[str, list[str]] = {}
    files = sorted(CLIENT.rglob("*.rs"))
    for path in files:
        text = path.read_text(errors="ignore")
        # Comment lines are skipped on purpose: this file's own doc comment
        # names `add_message::<QuestUpdate>()` while explaining the defect, and
        # a checker that counts prose as a registration proves itself right.
        for line in text.splitlines():
            # Two kinds of line are skipped on purpose, and both bit this file
            # while it was being written:
            #   * a doc comment naming `add_message::<T>()` while explaining
            #     the defect, and
            #   * a registration test asserting that the source *contains*
            #     "add_message::<T>()" as a string literal.
            # A checker that counts prose or an assertion as a registration
            # proves itself right — the exact failure mode it exists to catch.
            if line.lstrip().startswith("//"):
                continue
            outside = QUOTED.sub("", line)
            for m in ADD.findall(outside):
                registered.add(bare(m))
        for lineno, line in enumerate(text.splitlines(), 1):
            # Comment lines are skipped here for the same reason they are
            # skipped above, and the asymmetry was a real defect: the
            # registration pass ignored prose while the reader pass counted
            # it, so a doc comment explaining `MessageReader<T>` reported a
            # reader for a type named `T` that no code ever asks for.
            if line.lstrip().startswith("//"):
                continue
            for m in READER.findall(line):
                name = bare(m)
                if name.startswith("AssetEvent"):
                    continue
                readers.setdefault(name, []).append(f"{path.relative_to(ROOT)}:{lineno}")

    # Positive control: the check is worthless if it reads nothing.
    if not files or n_from_macro == 0 or not readers:
        print(
            "check_message_registration: read nothing "
            f"({len(files)} files, {n_from_macro} macro types, {len(readers)} readers) "
            "— refusing to report OK",
            file=sys.stderr,
        )
        return 2

    missing = {n: w for n, w in readers.items() if n not in registered and n not in EXTERNAL}
    if not missing:
        print(
            f"check_message_registration: OK ({len(readers)} message types read, "
            f"{len(registered)} registered, {n_from_macro} of them by the packets! macro)"
        )
        return 0

    print(f"check_message_registration: {len(missing)} unregistered message type(s)\n")
    for name in sorted(missing):
        print(f"  {name} — read but never add_message::<{name}>()")
        for where in missing[name][:4]:
            print(f"    {where}")
        print("    a system reading this panics the schedule at startup")
        print()
    return 1


if __name__ == "__main__":
    raise SystemExit(main())

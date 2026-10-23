# ADR 0013: Where the server work lives

Date: 2026-10-23
Status: Proposed

## Context

The client needs a server to talk to. Today that peer is the stub in the
sibling workspace `sro-rs-client` (same owner), and it works: a client reaches
handshake, login, character list, world entry, movement and chat against it,
and over 2026-10-23 five further slices landed there (monsters in view, a shop
purchase, a hit that arrives as damage, death, level-up). The question this ADR
settles is not whether that stub is useful. It is **where the next years of
server work should live**, because the answer decides how many packet models we
maintain.

Everything below was measured on 2026-10-23. Where a number comes from a tree
that is not published, the measurement says so.

### There are two packet models, and they have drifted

| Measurement | Value | How it was taken |
|---|---|---|
| Opcodes this repo wires | **256** | `python3 scripts/check_opcode_ledger.py` → `OK (256 opcodes match)` on this branch |
| Opcodes the stub wires | **197** | the `packets!` table in its own `packets/src/lib.rs` |
| Shared opcodes | **157** | set intersection |
| …of those, carrying a **different type name** | **100** | e.g. `0x304D` is `DropUnlocked` here and `CharacterDataChunkBroadcast` there |
| Opcodes only the stub has | **40** | set difference |
| …of those, in neither this repo's model nor any capture | **39** | cross-checked against the original client's three dispatch tables (320 inbound handlers, the figure `docs/protocol/opcodes.md` already states) and against every local `packet_dump` tree |
| S→C opcodes the stub actually emits | **95** | every `Packet::X` construction in its server sources, plus one raw body |
| …of those this repo does not model at all | **28** | the client drops them: `network: unhandled opcode 0x….` (`client/src/plugins/net/plugin.rs`) |

The last row is the cost in one line: a working shop purchase, a resolved hit,
a death and a gold balance were all computed server-side and then announced on
numbers no v1.188 client dispatches. Five of the 28 were confirmed at the
client on 2026-10-23 (`0x3030`, `0x3041`, `0x3043`, `0x3061`, `0x30C0`); the
dumps that show it are gitignored, so that evidence stays local by
`CONTRIBUTING.md`'s rule.

### This repo already owns the parts a server needs

- **`packets` is bidirectional.** The `packets!` macro generates
  `Packet::into_serialize` for every wired type, so each of the 256 has a
  serializer as well as a parser; there is no `unimplemented!` or `todo!` in
  `packets/src`. A server can therefore *emit* with the same crate the client
  *reads* with — one model, two directions.
- **The wire can be gated against the recorded corpus.** A corpus replay
  (every recorded frame through `Packet::deserialize` and back out through
  `into_serialize`, compared byte for byte) exists in the maintainer's working
  tree, not on this branch — **[U] on published code**. The capability matters
  for this decision; its availability here does not yet exist.
- **Frame, crypto and sequencing exist twice.** This repo's are 3,603 lines
  under `client/src/net/`, inside the `client` **binary** crate
  (`client/Cargo.toml` declares no `[lib]`), so no other crate can call them.
  The stub's are a library crate, `sro_protocol`, 2,869 lines — and the sibling
  workspace is same-owner material, which `AGENTS.md` lists as free to adopt.

### A crate in this workspace joins every gate

`make test` is `cargo test` and `make build` is `cargo build`, both
workspace-wide (`Makefile`), and `ci:` runs both. So a `server/` member is
compiled and tested on every gate run, forever. Measured on the stub's crate as
it stands: **83,420 lines** of server sources and **1,561** `#[test]`
attributes; `cargo test -p server` runs 1,828 tests plus three integration
tests in about six seconds once warm, but a cold build of that crate takes
**13m00s** because its `packets` depends on `bevy` and the whole bevy tree
compiles with it (`packets/Cargo.toml` here declares the same dependency, so
this is not avoidable by moving the crate).

### Licence classes are not symmetric

`AGENTS.md` §External sources governs this: **AGPL-3.0** sources (skrillax,
RSBot) may be observed for behaviour and **never ported**; **no-licence**
sources (xBot-WinForms, SilkroadDoc) are all-rights-reserved, so facts and
field layouts only, never code or text; the sibling workspace is same-owner and
adoptable. Measured in the stub's own sources: 338 references to xBot, 100 to
go-sro, 5 to SilkroadDoc, and **zero** to skrillax or RSBot — so adopting from
it does not import an AGPL dependency, but it does inherit a dense trail of
no-licence citations that a port has to keep honest (facts kept, wording not
copied).

## Decision

**A small `server/` crate in this workspace, with this repo's `packets` as the
only packet model — and not yet.**

1. **Order.** The stub stays the test peer while the remaining wire mismatches
   are fixed there. Each fix is a few lines behind the stub's own
   `SRO_STUB_VANILLA_WIRE` gate and is measurable the same day; a new crate
   would buy none of that faster.
2. **One model.** The new crate depends on `packets` and adds no packet types of
   its own. If a server needs an opcode this repo does not model, the opcode is
   added to `packets` with its evidence and lands in `docs/protocol/opcodes.md`
   through the existing gate — never as a second, server-private type.
3. **Small means small.** Game logic is re-grown against the real wire, in
   slices a reviewer can read. The stub's 83k-line hub is a reference and a
   source of measured behaviour, not a thing to port in one move.
4. **Frame and crypto: decide when the crate is created, not now.** Two
   routes, both open: extract `client/src/net/` into a library crate that both
   the client and the server use, or adopt `sro_protocol` from the sibling
   workspace. The first removes a duplication this repo already has; the second
   is 2,869 lines that already work. Whichever is chosen, there must be exactly
   one implementation afterwards.
5. **The headless rule applies from the first commit.** `AGENTS.md` already
   requires every system under `client/src/plugins/net/**` to run without the
   HUD; a server crate is the same discipline, one step further: no HUD
   resources, no window, no asset server.
6. **The wire is gated against captures, not against our own tests.** A server
   body is correct when it matches a recorded frame — the corpus replay is the
   gate to publish alongside the crate (see the `[U]` above).

## Consequences

- **The 28 invisible opcodes stop being produced.** Server work becomes
  client-visible work by construction, because both sides serialise the same
  type.
- **Gate time and disk grow, measurably.** Every `make ci` gains the crate's
  build and tests. The levers are known: a dedicated `CARGO_TARGET_DIR` and
  `CARGO_PROFILE_DEV_DEBUG=0`, which cut the stub's target directory from
  **7,589 MB to 2,054 MB** (measured 2026-10-23, same build).
- **`bevy` ends up in a headless server.** It is already a dependency of
  `packets` in both trees, so this is a cost we pay today in the stub as well;
  it is not new, but it is real (the 13-minute cold build above).
- **The duplication of frame/crypto has to end.** Either extraction or
  adoption — leaving both in place would be the worst of the three.
- **Delivery stays small.** `CONTRIBUTING.md`'s review limits do not change
  because a server exists; a crate that lands in slices is reviewable, a 83k
  line import is not.
- **The sibling repo keeps a job.** It remains the test peer and the place
  where behaviour is cheap to discover, and it stays the reference for
  measured game logic (damage formula, spawn layout, loot tables).

## Alternatives considered

### (a) Keep all server work in the sibling repo

*What it costs:* the second packet model stays, with its 100 diverging type
names and 39 invented opcodes, and every feature is built twice — once as a
stub type, once as a reader here. There is no shared gate: the stub's own tests
cannot see this repo's ledger or corpus.

*What it is worth:* it is the test peer today, five slices landed in it in a
day, and it costs this repo's gate nothing. This is why the decision above
keeps it for now rather than replacing it immediately.

### (c) A server in its own repository

*What it costs:* `packets` would have to be published as a crate or duplicated
— and duplication is the exact problem this ADR exists to end. A third tree
also means a third place to keep the opcode ledger honest.

*What it is worth:* the client's gate stays as fast as it is today, and the
server's release cadence is independent. Worth revisiting only if the measured
gate time in this workspace becomes the dominant cost.

## What is not decided here, and what is unmeasured

- This ADR creates nothing. No crate, no module, no dependency; it is a
  direction, and the first slice of it is still the stub work.
- **[U]** Whether the corpus replay tool ships here. It exists in the
  maintainer's working tree and is not on this branch.
- **[U]** The cost of extracting `client/src/net/` into a library crate — no
  attempt was made, and the figure 3,603 lines is a size, not an estimate.
- **[U]** Whether a bevy-free `packets` is worth it. Both trees depend on bevy
  today and nothing in this measurement set says what removing it would cost.
- The opcode comparison was taken against this branch (256) on 2026-10-23;
  an earlier run against `origin/main` the same day read 247. The numbers move
  as the ledger grows, the ratio is the point.

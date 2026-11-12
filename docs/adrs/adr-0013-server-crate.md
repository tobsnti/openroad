# ADR 0013: Where the server work lives

Date: 2026-10-23 (proposed) · 2026-11-11 (accepted)
Status: Accepted

## Context

The client needs a server to talk to. Today that peer is the stub in the
sibling workspace `sro-rs-client`, and it works as a test peer: sixteen
gameplay chains (movement, combat, kill and loot, NPC talk, shop, storage,
chat, party, trade, skills, stat points, teleport refusal) run against it
with zero decode errors on this client. The question this ADR settles is
**where the real server grows**, because the answer decides how many packet
models we maintain.

### Two packet models, and the cost of the second one

| Measurement (2026-11-11) | Value | How it was taken |
|---|---|---|
| Opcodes this repo wires | **267** | `0xNNNN => Name,` entries in `packets/src/lib.rs` |
| Opcodes the stub wires | **199** | the same pattern in its `packets/src/lib.rs` |
| Shared opcodes | **162** | set intersection |
| …carrying a **different type name** | **100** | e.g. `0x304D` |
| Stub server sources | **87,066 lines** | `wc -l` over `server/src`; one hub file holds 30,218 |
| Shared mutable state in the stub's hub | **86 fields, 43 behind a `Mutex`** | the hub struct; 333 `.lock()` calls |
| Crates in the stub server's dependency closure | **401** | `cargo tree -p server`, including renderer crates a headless server never uses |

The stub speaks its own dialect internally and translates to the original
wire at the socket, behind a runtime environment switch. The wire defects
measured against this client on 2026-11-11 all sit at that translation
boundary: a loot record five bytes too long, an experience update one byte
short, an acknowledgement on an opcode the client does not know, a consent
answer expected on the wrong opcode, and requests read with the stub's own
layout into a different action (a mount request, a skill cast read as an
attack).

A v1.188 client is stricter than this client: in the game state it checks
after every frame that the handler consumed the whole body, and terminates
otherwise — including for opcodes it does not dispatch at all (measured
locally on a crash dump). A server that is not byte-exact is therefore not
"a little off" for the original client; it kills it.

### This repo already owns the parts a server needs

- **`packets` is bidirectional.** Every wired type has a serializer as well as
  a parser. A round trip over all locally recorded frames the original client
  sent (56,829 frames, 49 opcodes) came back **byte-identical for 56,824**;
  one form is missing (`0x705A`, a five-byte variant) and three opcodes are
  not modelled yet (`0x7302`, `0x730B`, `0x730D`); no panic.
- **`packets` does not need Bevy.** Bevy is used for the `Message` derive and
  the dispatch registration only. Making `bevy` an optional, default-on feature
  and re-exporting an empty derive under the same name shrinks the dependency
  closure of `packets` from **405 to 21 crates** (measured in a throwaway copy,
  about +50/−30 lines over 30 files).
- **The transport is small.** `client/src/net` holds 4,102 lines, of which the
  transport (frame, codec, Blowfish, CRC, sequence, security, handshake,
  connection) is **2,583 lines in 9 files**; the rest is spawn parsing. It
  implements the client role only.

### Provenance

The stub's own `packets` and macro crates carry commits by other authors, and
the sibling tree has no licence file. Its remote is not ours, so nothing
developed there can be published from this project.

## Decision

**A `server/` crate in this workspace, with this repo's `packets` as the only
packet model. Nothing is ported from the stub's code.**

1. **One model.** The server reads and writes `packets` only. A server-side
   type this repo lacks is added to `packets` with its evidence, through the
   same review as any client change — never as a second, server-private type.
2. **No code port.** The stub stays the test peer and a reference for measured
   behaviour (formulas, sequences, timings). Behaviour is read there and
   written anew here, with its origin stated. Until the provenance above is
   cleared, the stub's code is treated like a source without a licence.
3. **The stub is frozen.** It receives wire-fidelity fixes only. The new server
   replaces it as the test peer when every chain of the test-bench contract
   passes against the new server with **zero decode errors on the client and
   zero opcodes outside the 320 that the original client dispatches**.
4. **One transport.** The transport moves out of `client/src/net` into a
   library crate used by the client, the headless netcheck and the server; the
   server (handshake responder) role is written new against recorded handshake
   frames. Afterwards exactly one implementation exists.
5. **Strict in, exact out.** The server decodes every request with a round
   trip (decode, re-encode, compare) and refuses anything that is not
   byte-identical, naming both lengths. The client keeps reading leniently —
   it should survive where the original would terminate — but counts unread
   bytes per opcode and reports each once.
6. **One writer.** Network I/O runs per connection; decoded requests go to a
   single simulation loop that owns the world. Each session has a bounded
   outbound queue; overflow disconnects visibly instead of dropping events.
   One registration table maps every opcode to its handler and its kind:
   *answer*, *silent*, or *locked* — the original client blocks two request
   groups until their answer (`0x7046`→`0xB046`, `0x70B5`→`0xB0B5`), so those
   handlers must answer exactly once, errors included, and the type system
   enforces it. The simulation steps at a fixed **1/30 s**, and AI and
   movement run on a **300 ms** timer `[V original server]`. Timers count
   whole ticks (300 ms = 9 ticks): the 307 ms cadence seen in recorded
   vanilla traffic is the original truncating milliseconds on every tick,
   not a design value. With exact ticks a mob covers its nominal speed per
   second — each AI step is speed × 0.3 s. `bevy_ecs` is used only if the
   tick-latency target of the first spike is missed.
7. **PostgreSQL from the first persisted slice**, clean schema, embedded
   migrations, admin-seeded accounts with argon2id hashes. Persistence is one
   interface — "commit the changes of one accepted action" — because a trade
   spans two characters. Queries are not checked against a live database at
   build time, so `make ci` needs none. Unit tests use an in-memory store;
   database tests run only when `DATABASE_URL` is set and say so when skipped.
   There is no interim file or SQLite store. A test-bench mode `--ephemeral`
   (nothing survives, announced at start) is the only database-less run.
8. **Configuration** is a file plus command-line flags. Test hooks (forced
   spawns, empty bags, experience grants) are admin commands behind a static
   Cargo feature `test-hooks`; later gameplay modules are static features too.
9. **The wire is gated against captures, not against our own encoder.** A
   corpus-replay tool ships in `tools/` and takes the capture directory as an
   argument; captures stay local. Tests quote single recorded lines.
10. **Small means small.** Slices of at most 400 lines, collected on a group
    branch; the headless rule of `AGENTS.md` applies from the first commit.

## Consequences

- **The translation boundary disappears.** Server work is client-visible by
  construction, and the defect class of 2026-11-11 cannot recur.
- **Atomicity is built, not inherited.** The stub never settles a trade (both
  confirmations end the session without moving items); the first spike must
  pass a property test: under any interleaving of two sessions, including
  cancel and disconnect, items and gold are conserved and a trade happens
  completely or not at all.
- **Gate time and disk grow.** The bevy-free `packets` keeps the server's
  closure small; the build cost of the crate itself is measured in its first
  slice.
- **`packets` gains a second consumer.** Changes to it are reviewed for both
  directions; types that today encode only one direction get both.
- **The sibling repo keeps a job** as test peer and behavioural reference until
  the switch criterion in point 3 is met.

## Alternatives considered

### (a) Rebuild the stub into the real server

*What it costs:* 87k lines with 43 shared locks, a second packet model with
100 diverging type names, a runtime dialect switch, and a tree that cannot be
published from this project. Every refactoring slice would be reviewed against
code that is being thrown away.

*What it is worth:* it runs today. That value is kept by freezing it as the
test peer rather than by growing it.

### (c) A server in its own repository

*What it costs:* `packets` would have to be published or duplicated — the
duplication this ADR exists to end — and a third tree keeps the opcode ledger.

*What it is worth:* an independent gate. Worth revisiting only if the measured
gate time in this workspace becomes the dominant cost.

## What is not decided here, and what is unmeasured

- **[U]** Local PostgreSQL provisioning (installation is the owner's decision);
  persisted runs wait for it.
- **[U]** The build time of the new crate and of `packets` with the default
  feature after the Bevy split; measured in those slices.
- **[U]** Whether `bevy_ecs` is needed at all; decided by the spike's tick
  latency, not in advance.

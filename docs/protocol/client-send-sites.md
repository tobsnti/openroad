# What the v1.188 client can send — 157 opcodes, with their send sites

Until now the c2s side had no denominator. "How much of the protocol do we
cover" was answered against our own ledger, which is circular: the ledger lists
what we already knew. This page gives the figure the binary itself carries.

## The shape that finds a send site

Every outbound packet in `sro_client.exe` goes through the same three steps, and
they are recognisable as a byte pattern rather than as a guess:

```
push   <opcode>
call   FUN_00841780      ; per-opcode gate -> eax
test   eax, eax
je     <skip>            ; gate says no: nothing is sent
push   <opcode>
call   FUN_0053cea0      ; build a frame with that opcode
...
call   FUN_008418d0      ; send it
```

`FUN_00841780` takes the opcode as a `u16` plus a flag, looks it up in two
containers at `0xEECDEC` and `0xEECE14`, and returns 0 or 1 — a gate keyed by
opcode. The flag, when non-zero, writes back into the entry (`mov [ecx+0xc], eax`);
the selection site passes 0, i.e. it asks without marking.

Scanning `.text` for `push imm32` followed within a few bytes by a call to that
gate yields **159** opcodes. Requiring a build or send call within the same basic
block leaves **157**. The two that fall out — `0x70ED` and `0x7104` — reach the
gate but no frame builder within reach, and are not counted here.

## The numbers

| | |
|---|---|
| opcodes the client can send | **157** |
| of those, modelled in `packets/src` | **86** |
| of those, **not** modelled | **71** |
| of those, ever seen in our captures | **36** (23 %) |

The last row is the useful one. Our capture corpus — every original-client
session recorded in this project — contains 36 of the 157 outbound opcodes the
client is capable of. Three quarters of the client's outbound surface has never
been exercised, which is why "the client never sends X" has rarely been evidence
of anything.

## Not modelled (71)

```
0x3014 0x3091 0x30C4 0x3303 0x347F 0x34A9 0x7023 0x7031 0x703F 0x705D 0x705E 0x70C0 0x70C7 0x70D8 0x70D9 0x70DB 0x70E1 0x70E2 0x70E4 0x70E5 0x70F6 0x7113 0x7118 0x7119 0x711A 0x7121 0x715F 0x7160 0x716A 0x7256 0x7258 0x7259 0x725A 0x7302 0x7304 0x7308 0x730A 0x730B 0x7402 0x7450 0x7461 0x7462 0x7463 0x7470 0x7471 0x7473 0x7474 0x7475 0x7476 0x7478 0x747A 0x747B 0x747C 0x747E 0x7483 0x74B2 0x74D3 0x74D4 0x74DF 0x74E0 0x7506 0x7507 0x7508 0x7509 0x750B 0x7515 0x7516 0x7519 0x751A 0x751C 0x751D
```

Full table with one row per opcode — send-site count, first site VA, containing
function, and whether we model it or have ever captured it — in
[`client-send-sites.tsv`](client-send-sites.tsv).

## What this is not

- **Not a dispatch table.** It says what the client is *capable* of sending from
  some code path, not what it sends in any given state.
- **Not a layout.** No field, width or order is claimed here; the pattern finds
  the opcode and the site, nothing more.
- **Not filtered by direction convention.** Twelve of the 157 are in the `0x3xxx`
  range usually seen inbound. That is not an artefact: `0x3012` (GameReady) is a
  known client send, and `0x3014` sits in the same `switch` one case away, with
  the identical gate/build shape at `0x0084C170`. The range is a convention, not
  a rule.

Measured 2026-11-04 against `sro_client.exe` (ImageBase `0x400000`), read as data
with a linear sweep. The binary is not redistributed and is never executed.

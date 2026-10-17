# Evidence — `packet_dump: true` stalls the agent login

Ten logs from five runs of the same binary against the same local stub peer,
2026-10-16. Home paths rewritten to `~`; no key, salt or credential is in them
(the stub answers a made-up token). Nothing here is a fix — the cause is **unknown**.

## The five runs

The client prints its whole `NetworkSettings` at start-up, so the single-variable
claim is checkable in the logs themselves (`client-m*.log`, line 13):

| run | `packet_dump` | everything else in `NetworkSettings` | outcome |
|---|---|---|---|
| m1 | `false` | identical | character list reached |
| m2 | **`true`** | identical | **silent after the agent handshake** |
| m3 | **`true`** | identical | **silent after the agent handshake** |
| m4 | `false` | identical | character list reached |
| m5 | `false` | identical | character list, delete, restore, create |

## Where it stops, from the peer side

The gateway leg **completes with the dump on** — `peer-m2.log`:

```
17:24:07.978 sro_peer[gateway]: sent 0x5000 setup (43 bytes)
17:24:07.979 sro_peer[gateway]: handshake accepted
17:24:07.980 sro_peer[gateway]: c2s 0x2001 (12 bytes)      <- the client answers
17:24:08.024 sro_peer[gateway]: c2s 0x6106 ... 0x6101 ... 0x6102 (login)
17:24:17.439 sro_peer[gateway]: client closed the connection
17:24:17.439 sro_peer[agent]:   client connected from 127.0.0.1:56760
17:24:17.440 sro_peer[agent]:   sent 0x5000 setup (43 bytes)
17:24:17.440 sro_peer[agent]:   handshake accepted
17:27:26.492 sro_peer[agent]:   client closed the connection   <- 3 minutes, nothing in between
```

The same place in `peer-m1.log`, dump off:

```
17:22:53.551 sro_peer[agent]: handshake accepted
17:22:53.551 sro_peer[agent]: c2s 0x2001 (12 bytes)
17:22:53.555 sro_peer[agent]: c2s 0x6103 (29 bytes)
17:22:53.555 sro_peer[agent]: token 0xbf8b9f45 accepted for 'player'
```

So the client answers the setup frame on the **first** connection and not on the
**second**, and the difference between the two runs is one config line.

## What the logs rule out

* **Not an I/O error in the dump.** `PacketDump::write` disables itself after the
  first error and warns once (`packet_dump: disabled after I/O error on …`). That
  line appears **0 times** in all five logs.
* **Not a crash and not a hang of the whole client.** m2 keeps serving BRP
  screenshot requests for minutes after the stall (`client-m2.log`, last lines).
* **Not the peer.** It accepted the handshake in every run and logged the same
  sequence up to that point.

## What stays unknown

Why the second connection produces no `0x2001` while the resource exists. The dump
class itself only appends a line per packet and cannot swallow a frame. That is the
whole point of filing this instead of patching something.

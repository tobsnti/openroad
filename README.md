# Defect report — `packet_dump: true` stalls the agent login

*Filed here because issues are disabled on this fork. Measured 2026-10-16; this is a report with evidence, not a fix — the cause is unknown.*

Measured 2026-10-16 over five runs of the same binary against the same local stub peer. **Not a fix — the cause is unknown**, and a fix without one would be a guess.

## What happens

With `network_settings.packet_dump: true` the client goes **silent on the agent connection**: the peer accepts the handshake and the client never sends `0x2001`, so `0x6103` never follows and the character list never arrives. With `packet_dump: false` the same clicks run through in seconds.

| run | `packet_dump` | outcome |
|---|---|---|
| m1 | `false` | character list reached |
| m2 | **`true`** | **silent after `agent: handshake accepted`** |
| m3 | **`true`** | **silent after `agent: handshake accepted`** |
| m4 | `false` | character list reached |
| m5 | `false` | character list, delete, restore, create |

Same build, same stub, same click sequence, same window. The client prints its whole `NetworkSettings` at start-up, so the single-variable claim is checkable in the logs rather than asserted:

```
m1: NetworkSettings { enabled: true, gateway_address: Some("127.0.0.1:21136"), packet_dump: false, item_use_enabled: false, outbound_encryption: false }
m2: NetworkSettings { enabled: true, gateway_address: Some("127.0.0.1:21136"), packet_dump: true,  item_use_enabled: false, outbound_encryption: false }
```

## Where exactly it stops

The **gateway** leg completes *with the dump on* — it is the **second** connection that stays mute (`peer-m2.log`):

```
17:24:07.980 sro_peer[gateway]: c2s 0x2001 (12 bytes)        <- client answers the setup frame
17:24:08.024 sro_peer[gateway]: c2s 0x6106 ... 0x6101 ... 0x6102 (login)
17:24:17.439 sro_peer[agent]:   client connected from 127.0.0.1:56760
17:24:17.440 sro_peer[agent]:   sent 0x5000 setup (43 bytes)
17:24:17.440 sro_peer[agent]:   handshake accepted
17:27:26.492 sro_peer[agent]:   client closed the connection  <- 3 minutes, nothing in between
```

Same place with the dump off (`peer-m1.log`): `c2s 0x2001` arrives in the same millisecond, `c2s 0x6103` 4 ms later, token accepted.

## What the logs already rule out

* **Not an I/O error in the dump.** `PacketDump::write` disables itself after the first error and warns once (`packet_dump: disabled after I/O error on …`, `client/src/plugins/net/packet_dump.rs:78`). That line appears **0 times** in all five logs.
* **Not a crash or a frozen client.** m2 keeps answering BRP screenshot requests for minutes after the stall.
* **Not the peer.** It accepted the handshake in every run and logged the same sequence up to that point.

## Why it matters more than a normal defect

`packet_dump` is **our own diagnostic switch**. Whoever investigates a login problem turns it on and thereby stops the very path they wanted to measure — and the switch is `true` in the committed `config.example.yaml` default of several working trees.

## Evidence

The ten logs of those five runs are the other files on this branch (home paths rewritten to `~`, no credentials). `client-m*.log` line 13 carries the `NetworkSettings` line quoted above; `peer-m*.log` carries the frame trace.

## Open, deliberately

The dump class appends one line per packet and cannot swallow a frame, yet the frame that fails to leave is the first one of the second connection. Whoever picks this up should start there, with `receive_packets` / `send_packets` (`client/src/plugins/net/plugin.rs:260`, `:361`) and the handshake state of the second `SilkroadConnection`.

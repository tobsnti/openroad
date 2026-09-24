# Protocol opcode coverage ledger

The CI-checkable scoreboard of protocol coverage. It lists every opcode the
client currently wires through the `packets!` macro in
[`packets/src/lib.rs`](../../packets/src/lib.rs) — the single source of truth for
opcode ↔ type ↔ event wiring. `scripts/check_opcode_ledger.py` fails CI if this
table and the macro disagree, so coverage stays honest.

- **Wired opcode count:** not written down here on purpose — it is derived.
  `python3 scripts/check_opcode_ledger.py` prints it (`OK (N opcodes match)`), and
  the gate rejects a hand-maintained number reappearing in this prose.
- **Original client's opcode surface:** the v1.188 client dispatches roughly 320
  server→client handlers and 169 client→server builders (10 opcodes appear in both
  directions). Use 320 as the S→C denominator when quoting coverage: it counts
  server→client handlers only, so C→S requests never count against it.
- **Status legend:** `wired` = typed and fanned out; `experimental` = layout not
  yet confirmed on the wire (marked EXPERIMENTAL in-code).
- **Counter-list:** some ids must **never** become packet types — the original's
  internal pseudo-ids, and real opcodes dispatched outside the three handler
  tables. An absence from this ledger is therefore not automatically a gap.

> Direction is inferred from the opcode range/naming convention
> (`0x2xxx`/`0x6xxx`/`0x7xxx` client→server; `0xAxxx`/`0xBxxx`/`0x3xxx`
> server→client) and cross-checked against the macro's own section comments.

## Handshake & global

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x2001` | ModuleIdentification | both | wired | first packet on every connection; `u16`-length-prefixed module name. C→S `SR_Client` + a trailing zero; S→C the peer's own name, matched against `GatewayServer`/`AgentServer`/`DownloadServer` by `ModuleIdentification::peer_kind()`. The trailing byte is optional inbound |
| `0x2002` | KeepAlive | C→S | wired | 5s ping, empty body |
| `0x2005` | GlobalStateUpdate | S→C | wired |  |
| `0x6005` | GlobalStateRequest | C→S | wired | typed but not sent yet |
| `0x2113` | XTrapIdentification | both | wired | anti-cheat challenge/response, 1026 bytes both ways (`u8 kind`, `u8 sub`, `u8[0x400]`); the blob stays opaque, the body is kept whole, and nothing sends a reply |

## Login & gateway

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x6102` | LoginRequest | C→S | wired |  |
| `0xA102` | LoginResponse | S→C | wired |  |
| `0x6323` | LoginCaptchaConfirmRequest | C→S | wired |  |
| `0xA323` | LoginCaptchaConfirmResponse | S→C | wired |  |
| `0x2322` | LoginCaptchaChallenge | S→C | wired |  |
| `0x6100` | PatchRequest | C→S | wired | the original launcher's version check (locale byte, `u16`-prefixed module name, build `u32`), as the v1.208 client sends it. OpenRoad's client does not send it — it does an `SV.T` preflight |
| `0xA100` | PatchResponse | S→C | wired | patch verdict; `result == 1` is a single byte, `result == 2` carries a `PatchErrorCode` (+ the download triple on code `2`). The code-`2` file list is deliberately unmodelled — we never send it |
| `0x6104` | NoticeRequest | C→S | wired | launcher news request, one content-id byte (`0x16` on the v1.208 client); sent a few ms after `0x6100` |
| `0xA104` | NoticeResponse | S→C | wired | **only `noticeCount` is modelled**: the per-notice entries need a counted list of structs the derive cannot express, and the only answer we ever send is the empty one. The launcher blocks on this packet — with a dead notice service it never offers its Start button |
| `0x6101` | ShardListRequest | C→S | wired |  |
| `0xA101` | ShardListResponse | S→C | wired |  |
| `0x6106` | ShardListPingRequest | C→S | wired |  |
| `0xA106` | ShardListPingResponse | S→C | wired |  |
| `0x6103` | AgentLoginRequest | C→S | wired |  |
| `0xA103` | AgentLoginResponse | S→C | wired |  |

## Character select / join

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7007` | CharacterSelectionActionRequest | C→S | wired |  |
| `0xB007` | CharacterSelectionActionResponse | S→C | wired |  |
| `0x7001` | CharacterJoinRequest | C→S | wired |  |
| `0xB001` | CharacterJoinResponse | S→C | wired |  |

## World enter & celestial

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x34A5` | CharacterDataBegin | S→C | wired |  |
| `0x3013` | CharacterDataBody | S→C | wired |  |
| `0x34A6` | CharacterDataEnd | S→C | wired |  |
| `0x3020` | CelestialPosition | S→C | wired |  |
| `0x3027` | CelestialUpdate | S→C | wired |  |
| `0x34BE` | ServerTime | S→C | wired | real-world server clock, packed u32, ~10 min cadence; confirmed |
| `0x3012` | GameReady | C→S | wired | client loading-finished ack |

## Entity spawn / despawn

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3017` | GroupEntitySpawnBegin | S→C | wired | batch begin (kind + count); [`net-entity-spawn-0x3019.md`](../net-entity-spawn-0x3019.md) |
| `0x3018` | GroupEntitySpawnEnd | S→C | wired | empty End marker; 0x3019 carries the data; [`net-entity-spawn-0x3019.md`](../net-entity-spawn-0x3019.md) |
| `0x3019` | GroupEntitySpawnData | S→C | wired | group spawn payload (players/NPCs/monsters/item drops); [`net-entity-spawn-0x3019.md`](../net-entity-spawn-0x3019.md) |
| `0x3015` | SingleEntitySpawn | S→C | wired | single spawn (one 0x3019 record); [`net-entity-spawn-0x3019.md`](../net-entity-spawn-0x3019.md) |
| `0x3016` | SingleEntityDespawn | S→C | wired | single despawn (unique_id); [`net-entity-spawn-0x3019.md`](../net-entity-spawn-0x3019.md) |

## Movement & entity state

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7021` | MovementRequest | C→S | wired |  |
| `0xB021` | MovementResponse | S→C | wired |  |
| `0xB023` | MovementPositionUpdate | S→C | wired |  |
| `0xB024` | MovementAngleResponse | S→C | wired | turn-in-place; C→S half 0x7024 unwired (no builder in the original) |
| `0x30D0` | EntitySpeedUpdate | S→C | wired |  |
| `0x30BF` | EntityStateUpdate | S→C | wired |  |
| `0x3054` | EntityLevelUp | S→C | wired |  |
| `0x300C` | NoticeUpdate | S→C | wired | unique spawned/killed; only subtype 5 is decoded, others kept raw |
| `0x3011` | CharacterDied | S→C | wired | 1-byte `death_cause` |
| `0x304D` | DropUnlocked | S→C | wired | drop `unique_id` |
| `0x704F` | CharacterActionRequest | C→S | wired | posture/gait, one byte (2 walk / 3 run / 4 sit-stand toggle); the 2/3 pair is 0x30BF's own `MOTION_STATE_*` encoding |
| `0x3091` | EmoteRequest | C→S | wired | emote code, one byte; C→S in the 0x3xxx range — the same documented exception as `0x3053` (builder only, no parser in the original) |

## Progression & vitals

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3053` | GetUpRequest | C→S | wired | death-window "Return"/get-up (CLIENT_GETUP) |
| `0x3056` | ReceiveExperience | S→C | wired |  |
| `0x3057` | EntityBarsUpdate | S→C | wired |  |
| `0x304E` | CharacterPointsUpdate | S→C | wired |  |
| `0x303D` | CharacterStatsUpdate | S→C | wired |  |
| `0x70A7` | HwanActionRequest | C→S | wired | jahwan/berserk activation; action byte unknown |
| `0xB0A7` | HwanActionResponse | S→C | wired | error code only when `result != 1` |
| `0x30DF` | HwanLevelUpdate | S→C | wired | HWANLEVEL push |
| `0x7050` | IncreaseStrRequest | C→S | wired | stat-spend |
| `0xB050` | IncreaseStrResponse | S→C | wired |  |
| `0x7051` | IncreaseIntRequest | C→S | wired |  |
| `0xB051` | IncreaseIntResponse | S→C | wired |  |

## Misc server pushes

Small S→C pushes the vSRO 1.188 server sends at world join. Typed so the
fan-out exists;
client-side handling lands per-epic. The `0x3153`/`0x3809` bodies are
confirmed; the list-entry shapes of `0x3305`/`0x3077` are not, because both
have only ever been seen empty (see the struct docs in `agent/ingame.rs`).

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3153` | SilkUpdate | S→C | experimental | account silks `{own, gift, point}` (u32×3); confirmed; feeds cash-shop epic |
| `0x3809` | WeatherUpdate | S→C | experimental | `{weather_type, intensity}` (u8×2); EP-27 (environment/weather) |
| `0x3305` | FriendListInfo | S→C | experimental | friend roster; only ever seen empty; EP-32 (social) |
| `0x3077` | CharacterFinished | S→C | experimental | join-time cooldown replay (item + skill lists); only ever seen empty; EP-07 |

## Invites & petitions

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3080` | GameInvite | S→C **and** C→S | wired | dual-mapped: the petition popup one way, accept/decline the other. **Two petition arms carry tails openroad drops**: petition **9** (academy / training-camp join) ends with a `u16`-length **ASCII name** (rendered into `UIIT_STT_TC_JOIN_REQUEST`), and petition **7** (enter-instance) carries **23+ bytes** (`u32, u8, u32, u8, u8, u32, u32, u16 count, count × u32`). Petitions 1–6 and 8 are byte-exact as modelled |
| `0x7062` | PartyInviteRequest | C→S | wired | funnel: server raises a 0x3080 petition on the target |
| `0x7081` | ExchangeInviteRequest | C→S | wired | funnel |
| `0x70F3` | GuildInviteRequest | C→S | wired | funnel |
| `0x7472` | AcademyInviteRequest | C→S | wired | funnel |
| `0x7477` | AcademyNoticeEditRequest | C→S | wired | academy notice edit, `strS strS` |
| `0x747D` | AcademyMatchListRequest | C→S | wired | matching-board page request, one byte |
| `0xB47D` | AcademyMatchListResponse | S→C | experimental | arms typed; the record block stays raw until its fields are named |
| `0xB081` | ExchangeInviteResponse | S→C | wired | inviter-side ack `{success, uid}` |

Not wired: `0xB060` party-ack — **but the premise for leaving it unwired is false.** The original
**does** register a handler, whose own debug string names it
`"OnCreatePartyAck[%d]"`; it is the ack for `0x7060 PartyCreationRequest`, which openroad already
sends. Body: `u8 result`; on `result == 1` a `u32` party value (**5 bytes**); on
`result == 2` a `u16` error code (**3 bytes**). That settles the standing
`0xB060`-vs-`0xB062` conflict: **0xB060 = create ack, 0xB062 = invite ack.
Different acks, not aliases.** Unknown: whether the u32 is a party number or
a member count (the client only stores it, and `0xB067` writes the same field).
`0x70B3` is **not** a 0x3080 arm — it pairs with `0xB0B3` stall-talk-response —
and party-match `0x706D`/`0x306E` has its own richer popup.

## Targeting

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7045` | SelectEntityRequest | C→S | wired |  |
| `0xB045` | SelectEntityResponse | S→C | wired |  |

## Combat & skills

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7074` | ObjectActionRequest | C→S | wired |  |
| `0xB074` | ObjectActionResponse | S→C | wired |  |
| `0xB070` | ObjectActionUpdate | S→C | wired |  |
| `0xB071` | SkillEnd | S→C | wired |  |
| `0x70A1` | SkillLearnRequest | C→S | wired | AGENT_SKILL_LEARN |
| `0xB0A1` | SkillLearnResponse | S→C | wired |  |
| `0x70A2` | MasteryLearnRequest | C→S | wired | AGENT_SKILL_MASTERY_LEARN |
| `0xB0A2` | MasteryLearnResponse | S→C | wired |  |
| `0x7202` | SkillLevelDownRequest | C→S | experimental | level-down mirror of 0x70A1; body unknown (no builder in the original) |
| `0xB202` | MasterySkillLevelDownResponse | S→C | experimental | success returns the new (lower) skill id; failure shape unknown |
| `0x7203` | MasteryLevelDownRequest | C→S | experimental | mirror of 0x70A2 minus its `amount` byte, which stays unknown |
| `0xB203` | MasteryLevelDownResponse | S→C | experimental | exact mirror of 0xB0A2; failure shape unknown |
| `0xB0BD` | BuffAdd | S→C | wired |  |
| `0xB072` | BuffRemove | S→C | wired |  |

## GM commands

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7010` | GmCommand | C→S | wired | `{u16 sub_id, …typed args}`; sub-ids from the original's own command registry — `0x06` LoadMonster, `0x07` MakeItem, `0x0C` Zoe, `0x0E`/`0x0F` the toggles. **MakeItem was wrongly `0x06` (= LoadMonster) before.** Note `/zoe` and `/zoe2` share `0x0C`: Zoe2 is a client-side batching wrapper, not a separate sub-command. |
| `0xB010` | GmResponse | S→C | wired | `{u8 result, u16 gm_command_id, …}` — the u16 echoes the request's sub-command on **both** the ok and fail arms, which is what makes it a probe.  |

## Logout

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7005` | LogoutRequest | C→S | wired |  |
| `0xB005` | LogoutResponse | S→C | wired |  |
| `0x7006` | LogoutCancelRequest | C→S | wired |  |
| `0xB006` | LogoutCancelResponse | S→C | wired |  |
| `0x300A` | LogoutSuccess | S→C | wired |  |

## Chat

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7025` | ChatRequest | C→S | wired |  |
| `0xB025` | ChatResponse | S→C | wired |  |
| `0x3026` | ChatUpdate | S→C | wired |  |
| `0x302D` | ChatRestriction | S→C | wired |  |

## Inventory & equipment

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7034` | InventoryOperationRequest | C→S | wired |  |
| `0xB034` | InventoryOperationResponse | S→C | wired |  |
| `0x3036` | PlayerPickupAnimation | S→C | wired |  |
| `0x3038` | EntityEquip | S→C | wired | 9 or **10** bytes — the trailing byte is read only when `ref_id` is TypeID 3.1.x |
| `0x3039` | EntityUnequip | S→C | wired |  |
| `0x3052` | InventoryItemDurabilityUpdate | S→C | wired | confirmed, 5 bytes |
| `0x3040` | InventoryItemUpdate | S→C | wired | **byte 1 is a bitmask, not an updateType** — 8 bit-gated blocks, read off the original client; the shipped struct models 2 of 8 |
| `0x3092` | InventoryCapacityUpdate | S→C | wired | fixed 2 bytes, read off the original client: byte 0 is a **target kind** (1 inventory / 2 storage), not a success flag — there is no failure tail |
| `0x704C` | ItemUseRequest | C→S | experimental | CLIENT_ITEM_USE; unconfirmed on the wire |
| `0xB04C` | ItemUseResponse | S→C | experimental | unconfirmed on the wire |
| `0x7158` | QuickSlotSaveRequest | C→S | wired | under-bar quickslot persistence; kind 1 of a kind-discriminated opcode (kind 2 is the auto-potion settings, unwired) |

## NPC interaction (talk / teleport / storage / repair)

NPC dialog, teleporter, storage, repair.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7046` | TalkRequest | C→S | wired |  |
| `0xB046` | TalkResponse | S→C | wired |  |
| `0x704B` | CloseTalkRequest | C→S | wired |  |
| `0xB04B` | CloseTalkResponse | S→C | wired |  |
| `0x705A` | TeleportRequest | C→S | wired |  |
| `0xB05A` | TeleportResponse | S→C | wired |  |
| `0x705B` | TransitionCastingCancelRequest | C→S | wired | cancels a teleport/transition cast in progress |
| `0xB05B` | TransitionCastingCancelResponse | S→C | wired | ack for the cancel above |
| `0x7059` | TeleportRecallRequest | C→S | experimental | designate a teleporter as the recall point |
| `0xB059` | TeleportRecallResponse | S→C | experimental | body kept whole — no parser exists in any source |
| `0x34B5` | GameReset | S→C | wired | teleport handshake |
| `0x34B6` | GameResetComplete | C→S | wired | client ack of 0x34B5 |
| `0x703C` | StorageDataRequest | C→S | wired | NPC storage open |
| `0xB03C` | StorageDataResponse | S→C | wired | storage-open error ack (repeat 0x703C refused) |
| `0x3047` | StorageDataBegin | S→C | wired |  |
| `0x3049` | StorageDataChunk | S→C | wired |  |
| `0x3048` | StorageDataEnd | S→C | wired |  |
| `0x7250` | GuildStorageOpenRequest | C→S | wired | guild warehouse open, `npc_unique_id:u32` |
| `0x7251` | GuildStorageCloseRequest | C→S | wired | same builder/body as 0x7250 |
| `0x7252` | GuildStorageListRequest | C→S | wired | echoes the id 0xB250 handed us — the item stream is this request's reply |
| `0xB250` | GuildStorageResponse | S→C | wired | open ack; `0x4C48` names the member holding the guild-wide lock |
| `0x3253` | GuildStorageDataBegin | S→C | wired | guild storage gold `u64` |
| `0x3255` | GuildStorageDataChunk | S→C | wired | concatenate before parsing |
| `0x3254` | GuildStorageDataEnd | S→C | wired | empty marker |
| `0x34D2` | BArenaOperation | S→C | wired | Battle Arena scheduler broadcast; tagged union, body length varies per op; ops `02/03/05/0D/0E` are confirmed, the rest and the `0xFF` sub-stream are not |
| `0x703E` | ItemRepairRequest | C→S | wired |  |
| `0xB03E` | ItemRepairResponse | S→C | wired |  |
| `0x7157` | AlchemyDismantleRequest | C→S | wired | alchemy dismantle, `{u8 SlotCount, u8[] Slots}` — the family's **only** published body |
| `0xB157` | AlchemyDismantleResponse | S→C | wired | `{u8 result, if result == 2 u16 errorCode}`; the error-code table is a dead page, so the code stays unnamed |
| `0x7150` | AlchemyReinforceRequest | C→S | experimental | elixir fuse / cancel; `{u8 2, u8 op=3, u8 count, count × u8 inventory slot}`, confirmed against a real server |
| `0xB150` | AlchemyReinforceResponse | S→C | experimental | `{u8 result, …}`; the outcome is classified by two flag bytes, as the original's handler does |
| `0x7151` | AlchemyStoneRequest | C→S | experimental | stone attach / cancel; leads with the `AlchemyType` byte (4 magic / 5 attribute), then the same count-prefixed slot list |
| `0xB151` | AlchemyStoneResponse | S→C | experimental | as `0xB150` minus the breakdown flag — a stone attach always delivers a record |

## Guild (wire only — see EP-15)

Read from the original client's parser and builder; none of them is confirmed
on the wire, so all are `experimental`.
The record arrives chunked (BEGIN → DATA… → END) and is parsed once assembled.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x34B3` | GuildDataBegin | S→C | experimental | marker; whether it carries a prefix is unknown |
| `0x3101` | GuildDataBody | S→C | experimental | one chunk, raw; assemble then `GuildData::parse` |
| `0x34B4` | GuildDataEnd | S→C | experimental | marker |
| `0x30FF` | EntityGuildUpdate | S→C | wired | **the original does have a parser**: `u32 gid`, `u32 guild_id`, `u16+N guild name`, then **only if the name length ≠ 0** `u16+M grant name`, `u32 crest rev`, `u32 union id`, `u32 union crest rev`, `u8 fortress position` (a **bit-valued** enum: 1 commander, 2 sub-commander, 4 battle-manager, 8 product-manager, 0x10 trainer-manager, 0x20 engineer) and `u8 relation flag`. It is a guild-tag update, not an activity log |
| `0x38F5` | GuildUpdate | S→C | experimental | `update_type` only; per-type payload raw |
| `0xB0F0` | GuildCreatedData | S→C | experimental | success + the same record inline |
| `0x70F9` | GuildNoticeEditRequest | C→S | experimental | title + message |

### Guild lifecycle & membership

The command acks share **one body form** — `{result:u8}` on success, plus a
`u16` error code on refusal — because their handlers sit in one cluster and each
reads the same two fields; the matching server writers emit the single byte
`01`. They follow the original's reader *and* writer, but none of them is
confirmed on the wire, so all are `experimental`.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3100` | EntityGuildRemove | S→C | experimental | bare `u32` entity id; one per member while disbanding |
| `0xB0F1` | GuildDisbandAck | S→C | experimental | shared ack form; disband vs leave is unconfirmed, see the doc |
| `0xB0F2` | GuildLeaveAck | S→C | experimental | shared ack form |
| `0xB0F3` | GuildInviteAck | S→C | experimental | shared ack form; success arm inert by design |
| `0xB0F4` | GuildKickAck | S→C | experimental | shared ack form |
| `0xB0F6` | GuildDonateAck | S→C | experimental | the one exception: `u32` donated GP on success |
| `0xB0F9` | GuildNoticeEditAck | S→C | experimental | the ack for the `0x70F9` we already send |
| `0xB0FA` | GuildPromoteAck | S→C | experimental | shared ack form |
| `0xB104` | GuildPermissionUpdateAck | S→C | experimental | shared ack form; the mask itself travels in the record |
| `0x70F0` | GuildCreateRequest | C→S | experimental | `u32` npc id + guild name |
| `0x70F1` | GuildDisbandRequest | C→S | experimental | one `u32` from the guild window; its meaning is unknown |
| `0x70F2` | GuildLeaveRequest | C→S | experimental | same accessor, same width |
| `0x70F4` | GuildKickRequest | C→S | experimental | **by name**, not by member id |
| `0x70FA` | GuildPromoteRequest | C→S | experimental | one `u32`; target or grade is unknown |
| `0x7104` | GuildPermissionUpdateRequest | C→S | experimental | one `u8`; inlined ctor |

### Guild union / alliance

Requests and acks share the same handler cluster as the guild ack form above.
`0x3102` (the union roster push) is decoded from the original's own handler.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3102` | UnionRoster | S→C | experimental | union roster push; decoded from the original's handler |
| `0x70FB` | UnionInviteRequest | C→S | experimental | target-addressed `u32`, like the guild invite |
| `0x70FC` | UnionLeaveRequest | C→S | experimental | **empty body** — a real layout, not a missing one |
| `0x70FD` | UnionExpelRequest | C→S | experimental | one `u32`; guild or its master is unknown |
| `0xB0FB` | UnionInviteAck | S→C | experimental | shared guild ack form |
| `0xB0FC` | UnionLeaveAck | S→C | experimental | shared guild ack form |
| `0xB0FD` | UnionExpelAck | S→C | experimental | shared guild ack form |

### Guild leadership (transfer, elections, GP history)

Three of the four acks are the shared guild ack form; `0xB106` and `0xB501` are
the only guild bodies with a repeating record. `0xB501`'s row leads with a 32-bit
`time_t`, which the original passes to the CRT's `_localtime32_s`. The request
fields have known widths but no names.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x7103` | GuildMasterTransferRequest | C→S | experimental | `u32`, `u32` |
| `0x7105` | GuildElectionStartRequest | C→S | experimental | `u32` |
| `0x7106` | GuildElectionParticipateRequest | C→S | experimental | `u32` |
| `0x7107` | GuildElectionVoteRequest | C→S | experimental | `u32`, `u32`, `u8` |
| `0x7501` | GuildGpHistoryRequest | C→S | experimental | `u32` |
| `0xB103` | GuildMasterTransferAck | S→C | experimental | shared ack form; carries no successor id |
| `0xB105` | GuildElectionStartAck | S→C | experimental | shared ack form; `0x4C33` = "not vote time" |
| `0xB106` | GuildElectionRoster | S→C | experimental | `10 + 5*N`; matches the server writer field for field |
| `0xB107` | GuildElectionVoteAck | S→C | experimental | shared ack form (error category `0x15`) |
| `0xB501` | GuildGpHistoryResponse | S→C | experimental | `{time_t, name, gp, reason}` rows |

### Guild war & siege authority

`0x30EF` keys a **guild-id** set, not an entity flag — one packet changes the
relation to every member of that guild. `0xB114`'s `u32` is the compensation
amount, named by the server writer itself (`0x4C45` = "no compensation" appears
on both sides). Details: . `0x3109`
`GuildWarInfo` and `0x7113` stay unwired, with reasons in that doc.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x30EF` | GuildRelationUpdate | S→C | experimental | `u8 flag, u32 guild_id`; set polarity unknown |
| `0x70FF` | SiegeAuthorityUpdateRequest | C→S | experimental | `u32`, `u8` |
| `0x7110` | GuildWarStartRequest | C→S | experimental | opposing guild name + four scalars |
| `0x7112` | GuildWarEndRequest | C→S | experimental | `u32` |
| `0x7114` | GuildWarRewardRequest | C→S | experimental | `u32` |
| `0xB110` | GuildWarStartAck | S→C | experimental | shared ack form; tears down the dialog on **both** arms |
| `0xB112` | GuildWarEndAck | S→C | experimental | shared ack form |
| `0xB114` | GuildWarRewardAck | S→C | experimental | `u32` compensation on success |

## Fortress war (wire only — no consumer yet)

`0x385F` is a `u8` sub-command family (0x35 arms). Two arms are decoded; the
other 52 keep their bytes. Five record fields have a known width and no name, so
they are carried as `unk_*`; outside a war they are all zero. Arithmetic:
`1 + 1 + count x 28 + 1 + 4 = 91`, the exact body length.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x385F` | SiegeUpdate | S→C | experimental | sub 0 fortress list (confirmed), sub 0x34 application-period end, everything else raw |

## Party (wire only — no consumer yet, see EP-14)

Read from the original client's parser and builder; none of them is confirmed
on the wire, so all are `experimental`.

| opcode | type | dir | status | notes |
|---|---|---|---|---|
| `0x3065` | PartyData | S→C | wired | full roster; the header is `u8 presence mask` + `u32 number` + *conditional* `u32 master_jid`/`u8 setup` |
| `0x3864` | PartyUpdate | S→C | experimental | delta; type 9 (new master) body unknown |
| `0x306E` | PartyMatchJoinResponse | C→S | experimental | C→S despite the 0x3xxx range |
| `0x7060` | PartyCreationRequest | C→S | experimental | whether `unique_id` is the invitee or self is unknown |
| `0x7061` | PartyLeave | C→S | experimental | empty body |
| `0x7063` | PartyKickRequest | C→S | experimental |  |
| `0x7069` | PartyMatchCreationRequest | C→S | experimental | second u32 always 0, purpose unknown |
| `0x706A` | PartyMatchEditedRequest | C→S | experimental | **SPEC only** — no original-client builder |
| `0x706B` | PartyMatchDeleteRequest | C→S | experimental |  |
| `0x706C` | PartyMatchListRequest | C→S | experimental | number shared with CLIENT_PET_DESTROY |
| `0x706D` | PartyMatchJoin | S→C | experimental | bidirectional opcode with two unrelated bodies; one codec serves both, as for `0x3080` |
| `0xB069` | PartyMatchCreationResponse | S→C | experimental | **SPEC only** — no original-client parser |
| `0xB06A` | PartyMatchEditedResponse | S→C | experimental | **SPEC only** — no original-client parser |
| `0xB06B` | PartyMatchDeleteResponse | S→C | experimental | number present only on success |
| `0xB06C` | PartyMatchListResponse | S→C | experimental | `race_type` semantics unknown |
| `0x3068` | PartyDistribution | S→C | experimental | item handed to a member; the tail's width is the ITEM's class, read behind a resolver |
| `0xB060` | PartyCreateResponse | S→C | experimental | the **create** ack (not the invite ack — attribution corrected); JID on success, `u16` code on failure |
| `0xB062` | PartyInviteResponse | S→C | experimental | the invite ack; empty on success, the invitation itself is `0x3080` |
| `0xB067` | PartyJoinResponse | S→C | experimental | the join ack; its body is read from the wire |
| `0xB06D` | PartyMatchJoinAck | S→C | experimental | branches on `result == 1`, not `== 2` — both tails are `u16` |

## Quest marks (wire only — the quest system itself is not built yet)

The three opcodes of the quest family whose bodies are known. Everything else
the family has is listed unwired, with its reason, in `docs/net-quest.md` —
including the correction that `0x30D0`, `0x30D2`, `0x30D3` and `0x30DF` are not
quest opcodes at all.

| opcode | type | dir | status | notes |
|---|---|---|---|---|
| `0x30D6` | QuestMarkAdd | S→C | wired | 24 bytes; `mark_id` is a server pool handle, the four trailing `u32`s are unknown |
| `0x30D7` | QuestMarkRemove | S→C | wired | whole body is a `mark_id` an earlier `0x30D6` introduced |
| `0x30D5` | QuestUpdate | S→C | wired | add / modify / delete of one active quest; the body after the `kind` byte is the CHARACTER_DATA quest record, read by the same parser |

## Job (trader / hunter / thief — `packets::agent::job`)

Read off the original's own handlers and, where noted, off a vSRO server that
writes the same fields. Only the two rows marked `wired` below have been seen
on the wire, so the section is `experimental` unless a row says otherwise.

| opcode | type | dir | status | notes |
|---|---|---|---|---|
| `0x70E1` | JobJoinRequest | C→S | experimental | `{u32 npc_gid, u8 enroll_job_union_type}` |
| `0x70E2` | JobLeaveRequest | C→S | wired | `{u32 npc_gid}` — matches the frame a server answers |
| `0x70E3` | JobAliasRequest | C→S | experimental | `{u32 npc_gid, u8 job_type, u16 len + ASCII alias}` |
| `0x70E4` | JobRankingRequest | C→S | experimental | `{u8 job_type, u8 rank_kind}` |
| `0x70E5` | JobOutcomeRequest | C→S | experimental | `{u32 npc_gid, u8 job_type}` — the name of the second byte is unconfirmed |
| `0x70E6` | JobPrevInfoRequest | C→S | experimental | `{u32 npc_gid}` |
| `0xB0E4` | JobRankingResponse | S→C | experimental | `{u8 result, u8 job_type, u8 rank_kind, u8 count, count × row}`; the row is one byte wider on the activity ranking |
| `0xB0E6` | JobPrevInfoResponse | S→C | experimental | `{u8 result, 3 × (u8 level, u32 exp)}` = 16 bytes; the pair order is unconfirmed |
| `0x30E0` | JobPriceUpdate | S→C | experimental | `u8`-counted list of `(ref_id, price)` pairs; which of the two `u32`s the UI shows is unconfirmed |
| `0x30E8` | JobTradeScaleUpdate | S→C | experimental | `{u8 trade_scale}` |
| `0x34D5` | JobSafeTradeUpdate | S→C | experimental | `{u8 state, u8 code}` plus two counter bytes on exactly one arm; 15 of the codes are indistinguishable on the wire |
| `0x74D4` | JobExportDetailRequest | C→S | experimental | `{u32 selected_ref_id}`; the one request of the family without an NPC pre-check |
| `0xB4D4` | JobExportDetailResponse | S→C | experimental | `{u16 count, count × goods row}` |

## Player stall / private shop (wire only — no consumer yet, see EP-16 / Trading)

None of these is confirmed on the wire, so all are `experimental`.

The stall listing streams as a **0xFF-sentinel-terminated** row list whose rows
embed the shared item body, so `0x30B7` and `0xB0BA` keep their rows raw behind
resolver-taking accessors — no derive list mode can express either half.

| opcode | type | dir | status | notes |
|---|---|---|---|---|
| `0x70B1` | StallCreateRequest | C→S | experimental | title only; the original's 63-char cap is UI policy, not wire framing |
| `0x70B2` | StallDestroyRequest | C→S | experimental | empty body |
| `0x70B3` | StallTalkRequest | C→S | experimental | one `u32` unique id, from the builder's single 4-byte write |
| `0x70B4` | StallBuyRequest | C→S | experimental | one slot byte |
| `0x70B5` | StallLeaveRequest | C→S | experimental | empty body |
| `0x70BA` | StallUpdateRequest | C→S | experimental | derived enum over the update type; trailing `unknown0` on the item types and State only |
| `0xB0B1` | StallCreateResponse | S→C | experimental | `result == 2` carries a u16 error code |
| `0xB0B2` | StallDestroyResponse | S→C | experimental | same result/error shape |
| `0xB0B4` | StallBuyResponse | S→C | experimental | slot on success, u16 error otherwise (branches on `!= 1`) |
| `0xB0B5` | StallLeaveResponse | S→C | experimental | same result/error shape; the leading byte is unconfirmed |
| `0xB0BA` | StallUpdateResponse | S→C | experimental | per-type body; add/remove carry the whole listing as raw rows |
| `0x30B7` | StallEntityAction | S→C | experimental | enter/exit carry a viewer id; buy carries the buyer and the remaining listing |
| `0x30B8` | EntityStallCreate | S→C | experimental | uid + title + decoration id |
| `0x30B9` | EntityStallDestroy | S→C | experimental | trailing u16 is unconfirmed |
| `0x30BB` | EntityStallTitleUpdate | S→C | experimental | title edits arrive here, not on 0xB0BA |
| `0xB0B3` | StallTalkResponse | S→C | experimental | the viewer's snapshot on entering a stall: header, the listing as raw rows, then the viewer id list |

Not wired, with the reason written down rather than a guessed layout
(`net-stall-0x30B7.md` §9): **`0xB0BC` / `0xB0BE`**, which are not this family at
all: their handlers sit next to `0xB0BD BuffAdd`, not with the stall handlers.
`0x70B3`, the C→S half of stall-talk, is wired in the table above.

## Mail/memo + consignment (wire only — no consumer yet, see EP-17)

None of these is confirmed on the wire, so all are `experimental`.

| opcode | type | dir | status | notes |
|---|---|---|---|---|
| `0x750E` | ConsignmentListRequest | C→S | experimental | empty body |
| `0xB508` | ConsignmentRegisterResponse | S→C | experimental | fixed 30-byte listing rows; `result == 2` carries a u16 error code |
| `0xB509` | ConsignmentUnregisterResponse | S→C | experimental | records are variable-length and itemdata-dependent, so the list stays raw behind a resolver-taking accessor |
| `0x7309` | MailSendRequest | C→S | experimental | confirmed head (title + message) only; everything after it is unknown and kept as a raw tail |

Deliberately **not** wired — all three bodies are entirely unverified, so a layout
would have to be invented: `0xB309 SERVER_MAIL_SEND_RESPONSE` (declared in the
original's opcode enum but it has no parser and no dispatch case at all),
`0x7508 CLIENT_CONSIGNMENT_REGISTER_REQUEST` and `0x7509
CLIENT_CONSIGNMENT_UNREGISTER_REQUEST` (enum-only, no builder).

## Pet / COS (consumed by `client/src/plugins/cos/`, EP-19)

Layouts come from the original client's own handlers and builders, cross-checked
against the vSRO server's writers where one exists.

Only the vehicle half is confirmed on the wire. The growth block, the pick-pet
settings word and `0xB0C5` follow the binaries alone and are unconfirmed.

| opcode | type | dir | status | notes |
|---|---|---|---|---|
| `0x30C8` | PetData | S→C | experimental | header only; the tail layout is chosen by the model's `tid4`, which is not in the body — read through `PetData::body` |
| `0x30C9` | PetUpdate | S→C | experimental | all seven arms modelled (`PetUpdatePayload`). Arm 3 is `{i64 exp_delta, u32 source_uid}` — **signed**, a death subtracts. Arm 2's item records need the itemdata resolver, so they are read through `PetUpdate::bag_items` |
| `0x30CA` | PetStateUpdate | S→C | experimental | `{u32 uid, u8 mask}` plus one byte per set mask bit. The two bits are not confirmed: `COS_STATE_MASK_A/B` = 0x01/0x02 are the lowest bits of the only mask value we know, 0x03, so the *reader* is honest about the frame's width while the meaning of the bytes is not claimed |
| `0x30E7` | StuckDistanceWarning | S→C | experimental | one reason byte; not COS-only (1 = job trade cart, 2 = quest monster). The distances the original prints are its own literals |
| `0xB0C5` | PetActionResponse | S→C | experimental | 0x70C5 ack. The error code sits **between** the action echo and the uid, so the uid's offset moves with `result`; `item_gid` only on action 8 |
| `0xB0C6` | PetTerminateResponse | S→C | experimental | success body empty; the guard is `result != 1`, not `== 2` |
| `0xB0CB` | PetPlayerMounted | S→C | experimental | mount/dismount ack. `riding_unique_id` is **unconditional** — a dismount is 10 bytes, and gating it on `is_mounting` consumed 6 |
| `0xB116` | PetUnsummonResponse | S→C | experimental | success body empty; COS error category `0x0C` |
| `0xB117` | PetRenameResponse | S→C | experimental | success body empty — the new name arrives as `0x30C9` arm 5, so nothing is applied optimistically |
| `0xB420` | PetSettingsChangeResponse | S→C | experimental | `settings` is present on **both** `settings_type` arms (1 = gold pet, 2 = cash pet), not just type 1 |
| `0x70C5` | PetActionRequest | C→S | experimental | three builder shapes: `Movement`, `Turn` (`u16` heading), and the 9-byte `Attack`/`ItemPickUp`. Actions 2 and 8 are confirmed by the server's own writer; `Follow` (9) is unconfirmed — 0xB0C5 echoes the action byte, which is what will settle it. Unknown codes keep a raw tail |
| `0x70C6` | PetTerminateRequest | C→S | experimental | `{u32}`. Distinct from unsummon |
| `0x70CB` | PetMountRequest | C→S | experimental | `{u8 mount_state, u32 cos_unique_id}` — **byte first** |
| `0x7116` | PetUnsummonRequest | C→S | experimental | `{u32}` |
| `0x7117` | PetRenameRequest | C→S | experimental | `{u32, string}` (`IFCOSInfo.cpp`) |
| `0x7420` | PetSettingsChangeRequest | C→S | experimental | shape from the vSRO server's own reader (`u32, u8, u32`), not from a client builder |

Deliberately **not** wired: `0xB0C7` has a known layout but unknown semantics,
so nothing could drive it honestly; `0x70C0`/`0x70C7` likewise.
`0x706C CLIENT_PET_DESTROY` shares its number with `PartyMatchListRequest` above,
and one opcode maps to one type.

## Player exchange / trade (wire only — see EP-17)

Read from the original client's parser and builder; none of them is confirmed
on the wire, so all are `experimental`.
The own-side staging half is not here — it rides `0x7034`/`0xB034` sub-ops 4/5/13,
still missing from `InventoryOperationRequest`.

| Opcode | Name | Direction | Status | Notes |
|---|---|---|---|---|
| `0x3085` | ExchangeStarted | S→C | experimental | window opened; partner uid |
| `0x3086` | ExchangePlayerConfirmed | S→C | experimental | empty body |
| `0x3087` | ExchangeCompleted | S→C | experimental | empty body |
| `0x3088` | ExchangeCanceled | S→C | wired | **fixed 2 bytes, not empty**: `u16 reason` — the original *does* read it and renders it as an exchange-failed message; the server writes exactly one field. A lost field, not a decode failure |
| `0x3089` | ExchangeGoldUpdate | S→C | wired | the leading byte is a **mode discriminator**, not an unknown: `1` = a staged **item** (`u8 slot` + the shared item block), `2` = staged **gold** (`u64`); other modes read nothing. Decoding a mode-1 body with the current `{u8, u64}` struct yields garbage gold |
| `0x308C` | ExchangeItemsUpdate | S→C | experimental | partner's staged items; raw tail, resolver-decoded |
| `0x7082` | ExchangeConfirmRequest | C→S | experimental | empty body |
| `0x7083` | ExchangeApproveRequest | C→S | experimental | empty body |
| `0x7084` | ExchangeExitRequest | C→S | experimental | empty body **inferred** — no original builder |
| `0xB082` | ExchangeConfirmResponse | S→C | experimental |  |
| `0xB083` | ExchangeApproveResponse | S→C | experimental |  |
| `0xB084` | ExchangeExitResponse | S→C | experimental |  |

## Not yet wired

Major subsystems with **no** opcodes wired yet. These are the coverage frontier:

- **Guild** chat / war / storage / alliance — EP-15 (the guild record, log, update and notice edit are wired above)
- **Storage** — EP-17 (player exchange is wired above; own-side staging still missing)
- **Quest** — EP-21
- **Alchemy** (elixir/stone fusion) — P4/P5
- **Friends / block list** — the join-time roster push `0x3305` is wired
  (experimental, only ever seen empty); the request/manage path is still later
- **Item cooldown replay** — `0x3077` CharacterFinished is wired (experimental,
  only ever seen empty); its entry shape is still unknown.
  (Item use `0x704C`/`0xB04C` itself is now wired — see Inventory & equipment.)
- **Character creation send path** (`0x7007` Create/CheckName/Restore actions)
  — the response enum exists but the requests are not sent yet (EP-10)

Received and deliberately **ignored** by the original, so not wired here either
(kept out of the table above because `scripts/check_opcode_ledger.py` requires
every table row to exist in the `packets!` macro):

- **`0x2110`** — the original's receive loop has an explicit branch that skips the
  whole body without reading a byte, and the opcode is in none of its three
  registration tables. There is no layout to
  wire. Listed in `KNOWN_IGNORED_OPCODES` (`client/src/plugins/net/plugin.rs`) so
  it is logged as known-and-ignored, not as an unhandled opcode, and does not
  attract a speculative layout.

Seen on the wire but deliberately **not** wired:

- **`0x3206`** — **not a "ticket" and no longer unresolved.** Its handler is named by
  its own log string `"OnRefreshBuffRemaintime:%d"`: a `u8` sub-type family for **buff /
  status-effect refresh**. `sub 8` = `u32 uid`, `u32 buff instance id`, `u32 duration in
  milliseconds` (stored `÷ 1000`). Wire it alongside the buff HUD rather than
  waiting for a "ticket state" that does not exist.

---

_Generated for EP-04.2; keep in sync with `packets/src/lib.rs` via
`scripts/check_opcode_ledger.py` (wired into CI by EP-04.3)._

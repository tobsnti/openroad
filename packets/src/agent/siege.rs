//! Fortress war (siege) wire opcode `0x385F`.
//!
//! Idea: `0x385F` is not one message but a **`u8` sub-command family** — the
//! original's handler `FUN_00895BB0` accepts sub-commands `0x00..=0x34` and
//! dispatches them through a **byte index table** at `0x008980A4` into 28 arms
//! at `0x00898034` (`0f b6 88 a4 80 89 00` / `ff 24 8d 34 80 89 00` at
//! `0x00895C0F`). The table is not 0x35 arms: subs `0x17..=0x30` all land on
//! the default arm `0x1B`, and only `0x00..=0x16` plus `0x31..=0x34` have one
//! of their own.
//!
//! Four arms are modelled. Sub `0x00` (the fortress list) and sub `0x34` (the
//! 1-byte application-period-end push) came from our own capture; sub `0x0E`
//! and sub `0x11` are read out of the original's own code, because our capture
//! is peacetime and has no frame of either. Every other arm keeps its bytes.
//!
//! That the opcode is ours is not an assumption either: the client registers
//! the pair itself at `0x0089853A`, `MOV [ESP+0xC], 0x385F` immediately before
//! `MOV [ESP+0x10], 0x00895BB0`.
//!
//! Sub 0's record is what makes this wireable without a wartime capture: the
//! arithmetic closes exactly on the captured body,
//! `1 sub + 1 count + 3 x 28 record + 1 + 4 = 91`. Five fields inside the record
//! have a verified width but no naming site in the binary, so they are carried
//! as `unk_*` rather than named — that is the honest half of this decode, and a
//! wartime capture is what resolves them.
//!
//! Layouts, VAs and the per-field evidence: `docs/net-siege-0x385F.md`.

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use sro_macro::SerializationError;

/// Sub-command 0: the fortress status list.
pub const SIEGE_FORTRESS_LIST: u8 = 0x00;
/// Sub-command 0x34: the application period ended. Empty body — captured on the
/// full hour, which is what identifies it as a scheduled period transition.
pub const SIEGE_APPLICATION_PERIOD_END: u8 = 0x34;
/// Sub-command 0x0E, arm `0x008972B6`.
pub const SIEGE_RANK: u8 = 0x0E;
/// Sub-command 0x11, arm `0x00897549`.
pub const SIEGE_SCORE: u8 = 0x11;

/// One fortress in the sub-0 list. Minimum 28 bytes: `4 + 2 + 4 + 2 + 2 + 4 + 4
/// + 4 + 1 + 1`, with all three strings empty and both flags clear — which is
/// exactly the record the peacetime capture carries three times.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FortressStatus {
    /// Capture: 1, 3 and 6 — the same three ids the user's own
    /// `textdata/siegefortress.txt` lists (Jangan, Hotan, Bijeokdan).
    pub fortress_id: u32,
    pub name: String,
    /// [U] — verified `u32` width, no formatting site names it.
    pub unk_u32_00: u32,
    /// Rendered next to the fortress name by the original, hence the name; [S].
    pub owning_guild: String,
    /// [S], same rendering site.
    pub owning_guild_master: String,
    /// [U] — verified width only.
    pub unk_u32_01: u32,
    /// [U] — verified width only.
    pub unk_u32_02: u32,
    /// [U] — verified width only.
    pub unk_u32_03: u32,
    /// Gates [`Self::flag_a_value`]: the original reads the extra `u32` only
    /// when this byte is exactly 1.
    pub flag_a: u8,
    pub flag_a_value: Option<u32>,
    /// Gates [`Self::flag_b_value`], same shape.
    pub flag_b: u8,
    pub flag_b_value: Option<u32>,
}

/// 0x385F — server → client siege update.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum SiegeUpdate {
    /// Sub 0. `period` is the trailing byte: our captures show it as `2` while
    /// the server was in the application period and `0` afterwards, so it is a
    /// state field rather than a constant — but the value table itself is [U].
    FortressList {
        fortresses: Vec<FortressStatus>,
        period: u8,
        unk_u32_00: u32,
    },
    /// Sub 0x34, empty body.
    ApplicationPeriodEnd,
    /// Sub 0x0E. The shape is measured in the original's own arm at
    /// `0x008972B6`: a `u8`, then — **only when that byte is zero** — a
    /// length-prefixed string, then a `u8` that the arm formats into a
    /// localized line through the same string table sub 0 uses (`0x00EEDF08`).
    /// The gate is ours: `cmp byte ptr [esp+0x14], 0` / `jne` at `0x0089733C`
    /// skips the string read.
    ///
    /// The *meaning* is [S], not measured. An external AGPL-3.0 project
    /// (v1.150 client, read as behaviour only — no code and no text taken)
    /// describes this arm as a rank announcement carrying a local flag, an
    /// optional name and a rank byte. Our own capture is peacetime and holds no
    /// 0x0E frame, so the fields keep neutral names until one does.
    Rank {
        flag: u8,
        name: Option<String>,
        value: u8,
    },
    /// Sub 0x11, arm `0x00897549`: three `u32`, nothing else. The second and
    /// third are stored into **adjacent** fields of the siege window object,
    /// `+0x20C0` and `+0x20C4` (`0x009D4DA0`, `0x009D4DC0`), and a UI refresh
    /// follows; the first is read and not consumed by this arm.
    ///
    /// [S] for the same reason as [`SiegeUpdate::Rank`]: the external source
    /// calls the three fortress, kills and deaths. Three `u32` is what our
    /// disassembly confirms — which of them is which, it does not.
    Score {
        unk_u32_00: u32,
        counter_a: u32,
        counter_b: u32,
    },
    /// Any of the other arms. They are `do-not-wire` until one disassembly pass
    /// names them, so their bytes are kept whole rather than guessed at.
    Other { sub: u8, tail: Bytes },
}

fn short() -> SerializationError {
    SerializationError::IoError(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "short 0x385F body",
    ))
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SerializationError> {
        let end = self.pos.checked_add(n).ok_or_else(short)?;
        let slice = self.buf.get(self.pos..end).ok_or_else(short)?;
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8, SerializationError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, SerializationError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, SerializationError> {
        let len = u16::from_le_bytes(self.take(2)?.try_into().unwrap()) as usize;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }
}

impl TryFrom<Bytes> for SiegeUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut c = Cursor {
            buf: &value,
            pos: 0,
        };
        let sub = c.u8()?;
        match sub {
            SIEGE_APPLICATION_PERIOD_END if value.len() == 1 => {
                Ok(SiegeUpdate::ApplicationPeriodEnd)
            }
            SIEGE_FORTRESS_LIST => {
                let parsed = (|| -> Option<SiegeUpdate> {
                    let count = c.u8().ok()?;
                    let mut fortresses = Vec::with_capacity(count as usize);
                    for _ in 0..count {
                        let fortress_id = c.u32().ok()?;
                        let name = c.string().ok()?;
                        let unk_u32_00 = c.u32().ok()?;
                        let owning_guild = c.string().ok()?;
                        let owning_guild_master = c.string().ok()?;
                        let unk_u32_01 = c.u32().ok()?;
                        let unk_u32_02 = c.u32().ok()?;
                        let unk_u32_03 = c.u32().ok()?;
                        let flag_a = c.u8().ok()?;
                        let flag_a_value = (flag_a == 1).then(|| c.u32().ok()).flatten();
                        let flag_b = c.u8().ok()?;
                        let flag_b_value = (flag_b == 1).then(|| c.u32().ok()).flatten();
                        fortresses.push(FortressStatus {
                            fortress_id,
                            name,
                            unk_u32_00,
                            owning_guild,
                            owning_guild_master,
                            unk_u32_01,
                            unk_u32_02,
                            unk_u32_03,
                            flag_a,
                            flag_a_value,
                            flag_b,
                            flag_b_value,
                        });
                    }
                    let period = c.u8().ok()?;
                    let unk_u32_00 = c.u32().ok()?;
                    Some(SiegeUpdate::FortressList {
                        fortresses,
                        period,
                        unk_u32_00,
                    })
                })()
                // The body must be consumed exactly. A list that does not close
                // on the last byte is a layout surprise, and keeping it raw is
                // better than reporting a half-read fortress table.
                .filter(|_| c.pos == value.len());
                Ok(parsed.unwrap_or(SiegeUpdate::Other {
                    sub,
                    tail: value.slice(1..),
                }))
            }
            SIEGE_RANK => {
                let parsed = (|| -> Option<SiegeUpdate> {
                    let flag = c.u8().ok()?;
                    let name = (flag == 0).then(|| c.string().ok()).flatten();
                    let value = c.u8().ok()?;
                    Some(SiegeUpdate::Rank { flag, name, value })
                })()
                .filter(|_| c.pos == value.len());
                Ok(parsed.unwrap_or(SiegeUpdate::Other {
                    sub,
                    tail: value.slice(1..),
                }))
            }
            SIEGE_SCORE => {
                let parsed = (|| -> Option<SiegeUpdate> {
                    Some(SiegeUpdate::Score {
                        unk_u32_00: c.u32().ok()?,
                        counter_a: c.u32().ok()?,
                        counter_b: c.u32().ok()?,
                    })
                })()
                .filter(|_| c.pos == value.len());
                Ok(parsed.unwrap_or(SiegeUpdate::Other {
                    sub,
                    tail: value.slice(1..),
                }))
            }
            _ => Ok(SiegeUpdate::Other {
                sub,
                tail: value.slice(1..),
            }),
        }
    }
}

impl From<SiegeUpdate> for Bytes {
    fn from(p: SiegeUpdate) -> Self {
        let mut buf = BytesMut::new();
        match p {
            SiegeUpdate::ApplicationPeriodEnd => buf.put_u8(SIEGE_APPLICATION_PERIOD_END),
            SiegeUpdate::FortressList {
                fortresses,
                period,
                unk_u32_00,
            } => {
                buf.put_u8(SIEGE_FORTRESS_LIST);
                buf.put_u8(fortresses.len() as u8);
                for f in fortresses {
                    buf.put_u32_le(f.fortress_id);
                    put_string(&mut buf, &f.name);
                    buf.put_u32_le(f.unk_u32_00);
                    put_string(&mut buf, &f.owning_guild);
                    put_string(&mut buf, &f.owning_guild_master);
                    buf.put_u32_le(f.unk_u32_01);
                    buf.put_u32_le(f.unk_u32_02);
                    buf.put_u32_le(f.unk_u32_03);
                    buf.put_u8(f.flag_a);
                    if let Some(v) = f.flag_a_value {
                        buf.put_u32_le(v);
                    }
                    buf.put_u8(f.flag_b);
                    if let Some(v) = f.flag_b_value {
                        buf.put_u32_le(v);
                    }
                }
                buf.put_u8(period);
                buf.put_u32_le(unk_u32_00);
            }
            SiegeUpdate::Rank { flag, name, value } => {
                buf.put_u8(SIEGE_RANK);
                buf.put_u8(flag);
                if let Some(name) = name {
                    put_string(&mut buf, &name);
                }
                buf.put_u8(value);
            }
            SiegeUpdate::Score {
                unk_u32_00,
                counter_a,
                counter_b,
            } => {
                buf.put_u8(SIEGE_SCORE);
                buf.put_u32_le(unk_u32_00);
                buf.put_u32_le(counter_a);
                buf.put_u32_le(counter_b);
            }
            SiegeUpdate::Other { sub, tail } => {
                buf.put_u8(sub);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

fn put_string(buf: &mut BytesMut, s: &str) {
    buf.put_u16_le(s.len() as u16);
    buf.extend_from_slice(s.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The captured 91-byte body, verbatim from `packet_dump/0x385f.log`:
    /// sub 0, three fortresses (ids 1, 3, 6 — the ids of the user's own
    /// `siegefortress.txt`), every string empty, both flags clear, trailer
    /// `02 00000000`. It must close on byte 91 exactly.
    #[test]
    fn the_captured_peacetime_fortress_list_closes_on_ninety_one_bytes() {
        let hex = "0003\
                   01000000000000000000000000000000000000000000000000000000\
                   03000000000000000000000000000000000000000000000000000000\
                   06000000000000000000000000000000000000000000000000000000\
                   0200000000";
        let body = Bytes::from(hex_to_bytes(hex));
        assert_eq!(body.len(), 91);

        let decoded = SiegeUpdate::try_from(body.clone()).unwrap();

        let SiegeUpdate::FortressList {
            fortresses,
            period,
            unk_u32_00,
        } = &decoded
        else {
            panic!("expected the fortress list, got {decoded:?}");
        };
        assert_eq!(
            fortresses.iter().map(|f| f.fortress_id).collect::<Vec<_>>(),
            vec![1, 3, 6]
        );
        assert!(fortresses.iter().all(|f| f.name.is_empty()
            && f.owning_guild.is_empty()
            && f.flag_a == 0
            && f.flag_a_value.is_none()));
        assert_eq!((*period, *unk_u32_00), (2, 0));

        let back: Bytes = decoded.into();
        assert_eq!(back, body);
    }

    /// The same push later in the capture carries a trailing `00` instead of
    /// `02` — which is why `period` is modelled as a state field and not as a
    /// constant.
    #[test]
    fn the_trailing_period_byte_varies_between_captures() {
        let hex = "0003\
                   01000000000000000000000000000000000000000000000000000000\
                   03000000000000000000000000000000000000000000000000000000\
                   06000000000000000000000000000000000000000000000000000000\
                   0000000000";
        let decoded = SiegeUpdate::try_from(Bytes::from(hex_to_bytes(hex))).unwrap();
        let SiegeUpdate::FortressList { period, .. } = decoded else {
            panic!("expected the fortress list");
        };
        assert_eq!(period, 0);
    }

    /// `packet_dump/0x385f.log` line 2: the single byte `34`, logged on the
    /// full hour.
    #[test]
    fn the_application_period_end_is_a_lone_sub_command() {
        let body = Bytes::from_static(&[0x34]);
        let decoded = SiegeUpdate::try_from(body.clone()).unwrap();
        assert_eq!(decoded, SiegeUpdate::ApplicationPeriodEnd);
        assert_eq!(Bytes::from(decoded), body);
    }

    /// A record with a name and both flag-gated `u32`s present round-trips —
    /// the wartime shape the capture cannot show.
    #[test]
    fn a_named_fortress_with_both_optional_fields_round_trips() {
        let fortress = FortressStatus {
            fortress_id: 6,
            name: "Hotan".into(),
            owning_guild: "Wanderers".into(),
            owning_guild_master: "Kong".into(),
            flag_a: 1,
            flag_a_value: Some(0xDEAD),
            flag_b: 1,
            flag_b_value: Some(0xBEEF),
            ..Default::default()
        };
        let msg = SiegeUpdate::FortressList {
            fortresses: vec![fortress.clone()],
            period: 1,
            unk_u32_00: 7,
        };

        let wire: Bytes = msg.clone().into();
        assert_eq!(SiegeUpdate::try_from(wire).unwrap(), msg);
    }

    /// An arm nobody has decoded keeps its bytes instead of being misread.
    #[test]
    fn an_untraced_sub_command_is_kept_raw() {
        let body = Bytes::from_static(&[0x0B, 0xAA, 0xBB]);
        let decoded = SiegeUpdate::try_from(body.clone()).unwrap();
        assert_eq!(
            decoded,
            SiegeUpdate::Other {
                sub: 0x0B,
                tail: Bytes::from_static(&[0xAA, 0xBB]),
            }
        );
        assert_eq!(Bytes::from(decoded), body);
    }

    /// A truncated list is not reported as a half-read fortress table.
    #[test]
    fn a_truncated_fortress_list_degrades_to_raw() {
        let body = Bytes::from_static(&[0x00, 0x03, 0x01, 0x00]);
        assert!(matches!(
            SiegeUpdate::try_from(body).unwrap(),
            SiegeUpdate::Other { sub: 0, .. }
        ));
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }

    /// Sub 0x0E with the flag clear: the original reads the string only on that
    /// branch (`0x0089733C`), so a body with a name must decode and come back
    /// byte for byte.
    #[test]
    fn the_rank_arm_reads_its_name_only_when_the_flag_is_zero() {
        let with_name = Bytes::from_static(&[0x0E, 0x00, 0x03, 0x00, b'A', b'b', b'c', 0x07]);
        let parsed = SiegeUpdate::try_from(with_name.clone()).unwrap();
        assert_eq!(
            parsed,
            SiegeUpdate::Rank {
                flag: 0,
                name: Some("Abc".to_string()),
                value: 7,
            }
        );
        assert_eq!(Bytes::from(parsed), with_name);

        // Flag set: no string on the wire at all, and the byte right after the
        // flag is the value.
        let without = Bytes::from_static(&[0x0E, 0x01, 0x09]);
        let parsed = SiegeUpdate::try_from(without.clone()).unwrap();
        assert_eq!(
            parsed,
            SiegeUpdate::Rank {
                flag: 1,
                name: None,
                value: 9,
            }
        );
        assert_eq!(Bytes::from(parsed), without);
    }

    /// Sub 0x11 is three `u32` and nothing else — 13 bytes with the sub byte.
    #[test]
    fn the_score_arm_is_three_u32() {
        let body = Bytes::from_static(&[0x11, 0x01, 0, 0, 0, 0x2A, 0, 0, 0, 0x07, 0, 0, 0]);
        assert_eq!(body.len(), 13);
        let parsed = SiegeUpdate::try_from(body.clone()).unwrap();
        assert_eq!(
            parsed,
            SiegeUpdate::Score {
                unk_u32_00: 1,
                counter_a: 42,
                counter_b: 7,
            }
        );
        assert_eq!(Bytes::from(parsed), body);
    }

    /// A new arm that does not close exactly on its body is kept raw, the same
    /// rule sub 0 already follows: a half-read score is worse than bytes.
    #[test]
    fn a_new_arm_that_does_not_close_keeps_its_bytes() {
        for short in [
            Bytes::from_static(&[0x11, 0x01, 0, 0, 0, 0x2A, 0, 0, 0]),
            Bytes::from_static(&[0x11, 0x01, 0, 0, 0, 0x2A, 0, 0, 0, 7, 0, 0, 0, 0xFF]),
            Bytes::from_static(&[0x0E, 0x00, 0x03, 0x00, b'A', b'b', b'c']),
        ] {
            let parsed = SiegeUpdate::try_from(short.clone()).unwrap();
            assert!(
                matches!(parsed, SiegeUpdate::Other { .. }),
                "{short:?} should have stayed raw"
            );
            assert_eq!(Bytes::from(parsed), short);
        }
    }
}

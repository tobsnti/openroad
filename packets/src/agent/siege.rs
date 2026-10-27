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
//! `1 sub + 1 count + 3 x 28 record + 1 + 4 = 91`.
//!
//! That arithmetic is a weaker witness than it looks, and this module learnt it
//! the hard way: it is blind to **labels**. The record's first string was
//! called the fortress name for as long as nobody held a fortress, because a
//! peacetime capture is all-empty strings and the widths are identical whatever
//! the fields mean. The name is now read as the holding guild's, with the
//! reasons on [`FortressStatus`]. The read order itself is pinned to the
//! original's own sequence by
//! `tests::the_record_follows_the_original_read_order`.
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
///
/// **The record carries no fortress name.** This struct called its first string
/// `name` and the arithmetic never noticed, because both readings have the same
/// width sequence (`4, 2, 4, 2, 2, 4, 4, 4`) — a wrong label costs nothing until
/// a guild actually holds a fortress. Two measurements of our own say the label
/// was wrong: the client loads `%stextdata\siegefortress.txt` and the user's
/// copy maps exactly the captured ids 1, 3 and 6 to the name keys
/// `SN_FORTRESS_JANGAN`, `_HOTAN` and `_BIJEOKDAN`, so the name is already on
/// disk; and in all 117 captured frames — 351 records — **all three** strings
/// are empty, which a fortress name could not be while a peacetime owner-side
/// string is exactly that.
///
/// The three strings are therefore owner-side. Their individual labels are [S],
/// from two filled frames of two unrelated private servers (guild, master,
/// notice); our own capture cannot separate them, because peacetime sends
/// nothing in any of the three.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FortressStatus {
    /// Capture: 1, 3 and 6 — the same three ids the user's own
    /// `textdata/siegefortress.txt` lists (Jangan, Hotan, Bijeokdan). The
    /// fortress *name* comes from that file, not from this record.
    pub fortress_id: u32,
    /// [S] — the holding guild. Empty in every captured (peacetime) record.
    pub owning_guild: String,
    /// [S] — read as the guild's number by the filled frames; [U] for us.
    pub guild_number: u32,
    /// [S] — the guild master. Empty in every captured record.
    pub owning_guild_master: String,
    /// [S] — a holder-authored notice. Empty in every captured record, and the
    /// field this struct had no slot for at all before.
    pub notice: String,
    /// [U] — verified width only.
    pub unk_u32_01: u32,
    /// [S] — read as `100 - tax percent` by the filled frames (80 at 20 %);
    /// [U] for us, since every captured record has it at 0.
    pub item_price_percent: u32,
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
                        let owning_guild = c.string().ok()?;
                        let guild_number = c.u32().ok()?;
                        let owning_guild_master = c.string().ok()?;
                        let notice = c.string().ok()?;
                        let unk_u32_01 = c.u32().ok()?;
                        let item_price_percent = c.u32().ok()?;
                        let unk_u32_03 = c.u32().ok()?;
                        let flag_a = c.u8().ok()?;
                        let flag_a_value = (flag_a == 1).then(|| c.u32().ok()).flatten();
                        let flag_b = c.u8().ok()?;
                        let flag_b_value = (flag_b == 1).then(|| c.u32().ok()).flatten();
                        fortresses.push(FortressStatus {
                            fortress_id,
                            owning_guild,
                            guild_number,
                            owning_guild_master,
                            notice,
                            unk_u32_01,
                            item_price_percent,
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
                    put_string(&mut buf, &f.owning_guild);
                    buf.put_u32_le(f.guild_number);
                    put_string(&mut buf, &f.owning_guild_master);
                    put_string(&mut buf, &f.notice);
                    buf.put_u32_le(f.unk_u32_01);
                    buf.put_u32_le(f.item_price_percent);
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
        // All three strings empty in all three records — repeated across the
        // whole capture, which is what rules the first one out as the fortress
        // name: the names exist, and they are in `siegefortress.txt`.
        assert!(fortresses.iter().all(|f| f.owning_guild.is_empty()
            && f.owning_guild_master.is_empty()
            && f.notice.is_empty()
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
            owning_guild: "Wanderers".into(),
            guild_number: 12,
            owning_guild_master: "Kong".into(),
            notice: "recruiting".into(),
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

    /// The record order, nailed to the original's own read sequence so a later
    /// relabel cannot quietly move a field. `FUN_00895BB0`'s sub-0 arm calls
    /// the byte reader (`0x004F7220`, with the count pushed) and the
    /// length-prefixed string reader (`0x004F7A70`) in exactly this order:
    ///
    /// ```text
    /// 00895cb5  PUSH 4 .. CALL 0x4F7220     fortress_id
    /// 00895cd3  LEA/PUSH .. CALL 0x4F7A70   string 1
    /// 00895ce2  PUSH 4 .. CALL 0x4F7220     u32
    /// 00895cf3  LEA/PUSH .. CALL 0x4F7A70   string 2
    /// 00895d02  LEA/PUSH .. CALL 0x4F7A70   string 3
    /// 00895d11  PUSH 4 .. CALL 0x4F7220     u32
    /// 00895d22  PUSH 4 .. CALL 0x4F7220     u32
    /// 00895d33  PUSH 4 .. CALL 0x4F7220     u32
    /// 00895d44  PUSH 1 .. CALL 0x4F7220     flag_a
    /// 00895d7b  CMP BYTE [ESP+0x14], 1 / JNE 0x895d90
    /// 00895d82  PUSH 4 .. CALL 0x4F7220     flag_a's u32, only when flag == 1
    /// 00895d90  PUSH 1 .. CALL 0x4F7220     flag_b
    /// 00895d9e  CMP BYTE [ESP+0x14], 1 / JNE 0x895db3
    /// 00895da5  PUSH 4 .. CALL 0x4F7220     flag_b's u32, same gate
    /// ```
    ///
    /// Two things follow that the arithmetic alone never showed. The flag gate
    /// is a comparison against **1**, not "non-zero" — and there is no version
    /// check anywhere in the loop, so the 2-vs-10 byte tail is this flag pair
    /// and nothing else.
    #[test]
    fn the_record_follows_the_original_read_order() {
        let fortress = FortressStatus {
            fortress_id: 3,
            owning_guild: "G".into(),
            guild_number: 0x11111111,
            owning_guild_master: "M".into(),
            notice: "N".into(),
            unk_u32_01: 0x22222222,
            item_price_percent: 80,
            unk_u32_03: 0x33333333,
            flag_a: 0,
            flag_a_value: None,
            flag_b: 0,
            flag_b_value: None,
        };
        let wire: Bytes = SiegeUpdate::FortressList {
            fortresses: vec![fortress],
            period: 0,
            unk_u32_00: 0,
        }
        .into();

        #[rustfmt::skip]
        let expected: &[u8] = &[
            0x00, 0x01,                             // sub, count
            0x03, 0, 0, 0,                          // fortress_id
            0x01, 0x00, b'G',                       // string 1
            0x11, 0x11, 0x11, 0x11,                 // u32
            0x01, 0x00, b'M',                       // string 2
            0x01, 0x00, b'N',                       // string 3
            0x22, 0x22, 0x22, 0x22,                 // u32
            0x50, 0x00, 0x00, 0x00,                 // u32 (80)
            0x33, 0x33, 0x33, 0x33,                 // u32
            0x00, 0x00,                             // both flags clear
            0x00, 0, 0, 0, 0,                       // period, tail u32
        ];
        assert_eq!(wire.as_ref(), expected);
    }

    /// The flag pair is the whole of the record's variable tail: clear is two
    /// bytes, set is ten (`u8 u32 u8 u32`). Nothing else in the record moves.
    #[test]
    fn the_flag_pair_is_two_bytes_clear_and_ten_bytes_set() {
        let base = FortressStatus {
            fortress_id: 1,
            ..Default::default()
        };
        let clear: Bytes = SiegeUpdate::FortressList {
            fortresses: vec![base.clone()],
            period: 0,
            unk_u32_00: 0,
        }
        .into();
        let set: Bytes = SiegeUpdate::FortressList {
            fortresses: vec![FortressStatus {
                flag_a: 1,
                flag_a_value: Some(0),
                flag_b: 1,
                flag_b_value: Some(0),
                ..base
            }],
            period: 0,
            unk_u32_00: 0,
        }
        .into();
        assert_eq!(clear.len(), 2 + 28 + 5, "sub, count, record, period, u32");
        assert_eq!(set.len() - clear.len(), 8);
    }
}

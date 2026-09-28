//! Fortress war (siege) wire opcode `0x385F`.
//!
//! Idea: `0x385F` is not one message but a **`u8` sub-command family** — the
//! original's handler dispatches 0x35 arms through a jump table, the same shape
//! `0x3080` and `0x3101` use elsewhere in this protocol. Decoded are the two
//! arms our own capture contains — sub `0x00`, the fortress list, and sub
//! `0x34`, the 1-byte application-period-end push — plus eight arms whose body
//! is a flat field sequence. Their field *meaning* is unknown, so they carry
//! `unk_*` names. Every other arm keeps its bytes, and so does an arm whose
//! body is only partly readable.
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

use crate::agent::cursor::{put_string, Cursor};

/// Sub-command 0: the fortress status list.
pub const SIEGE_FORTRESS_LIST: u8 = 0x00;
/// Sub-command 0x34: the application period ended. Empty body — captured on the
/// full hour, which is what identifies it as a scheduled period transition.
pub const SIEGE_APPLICATION_PERIOD_END: u8 = 0x34;

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
    /// Sub 7.
    Sub07 { unk_u32_00: u32, unk_u16_00: u16 },
    /// Sub 8.
    Sub08 {
        unk_u32_00: u32,
        unk_str_00: String,
        unk_str_01: String,
        unk_str_02: String,
        unk_u32_01: u32,
        unk_u32_02: u32,
        unk_u32_03: u32,
        unk_u32_04: u32,
    },
    /// Sub 10.
    Sub0A { unk_u32_00: u32 },
    /// Sub 12.
    Sub0C { unk_u32_00: u32, unk_u8_00: u8 },
    /// Sub 13.
    Sub0D { unk_u32_00: u32, unk_u8_00: u8 },
    /// Sub 17.
    Sub11 {
        unk_u32_00: u32,
        unk_u32_01: u32,
        unk_u32_02: u32,
    },
    /// Sub 18.
    Sub12 { unk_u32_00: u32, unk_u8_00: u8 },
    /// Sub 20.
    Sub14 { unk_u8_00: u8 },
    /// Every remaining arm. Arms whose body is only partly readable — a field
    /// behind a condition this decode cannot source, or a list whose count
    /// field is not settled — stay here on purpose: raw beats half-read.
    Other { sub: u8, tail: Bytes },
}

impl TryFrom<Bytes> for SiegeUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut c = Cursor::new(&value, "short 0x385F body");
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
                .filter(|_| c.at_end());
                Ok(parsed.unwrap_or(SiegeUpdate::Other {
                    sub,
                    tail: value.slice(1..),
                }))
            }
            7 | 8 | 10 | 12 | 13 | 17 | 18 | 20 => {
                let parsed = (|| -> Option<SiegeUpdate> {
                    let arm = match sub {
                        7 => SiegeUpdate::Sub07 {
                            unk_u32_00: c.u32().ok()?,
                            unk_u16_00: c.u16().ok()?,
                        },
                        8 => SiegeUpdate::Sub08 {
                            unk_u32_00: c.u32().ok()?,
                            unk_str_00: c.string().ok()?,
                            unk_str_01: c.string().ok()?,
                            unk_str_02: c.string().ok()?,
                            unk_u32_01: c.u32().ok()?,
                            unk_u32_02: c.u32().ok()?,
                            unk_u32_03: c.u32().ok()?,
                            unk_u32_04: c.u32().ok()?,
                        },
                        10 => SiegeUpdate::Sub0A {
                            unk_u32_00: c.u32().ok()?,
                        },
                        12 => SiegeUpdate::Sub0C {
                            unk_u32_00: c.u32().ok()?,
                            unk_u8_00: c.u8().ok()?,
                        },
                        13 => SiegeUpdate::Sub0D {
                            unk_u32_00: c.u32().ok()?,
                            unk_u8_00: c.u8().ok()?,
                        },
                        17 => SiegeUpdate::Sub11 {
                            unk_u32_00: c.u32().ok()?,
                            unk_u32_01: c.u32().ok()?,
                            unk_u32_02: c.u32().ok()?,
                        },
                        18 => SiegeUpdate::Sub12 {
                            unk_u32_00: c.u32().ok()?,
                            unk_u8_00: c.u8().ok()?,
                        },
                        _ => SiegeUpdate::Sub14 {
                            unk_u8_00: c.u8().ok()?,
                        },
                    };
                    Some(arm)
                })()
                // Same rule as the list arm: a body that does not close on its
                // last byte is kept whole instead of half-read.
                .filter(|_| c.at_end());
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
            SiegeUpdate::Sub07 {
                unk_u32_00,
                unk_u16_00,
            } => {
                buf.put_u8(7);
                buf.put_u32_le(unk_u32_00);
                buf.put_u16_le(unk_u16_00);
            }
            SiegeUpdate::Sub08 {
                unk_u32_00,
                unk_str_00,
                unk_str_01,
                unk_str_02,
                unk_u32_01,
                unk_u32_02,
                unk_u32_03,
                unk_u32_04,
            } => {
                buf.put_u8(8);
                buf.put_u32_le(unk_u32_00);
                put_string(&mut buf, &unk_str_00);
                put_string(&mut buf, &unk_str_01);
                put_string(&mut buf, &unk_str_02);
                buf.put_u32_le(unk_u32_01);
                buf.put_u32_le(unk_u32_02);
                buf.put_u32_le(unk_u32_03);
                buf.put_u32_le(unk_u32_04);
            }
            SiegeUpdate::Sub0A { unk_u32_00 } => {
                buf.put_u8(10);
                buf.put_u32_le(unk_u32_00);
            }
            SiegeUpdate::Sub0C {
                unk_u32_00,
                unk_u8_00,
            } => {
                buf.put_u8(12);
                buf.put_u32_le(unk_u32_00);
                buf.put_u8(unk_u8_00);
            }
            SiegeUpdate::Sub0D {
                unk_u32_00,
                unk_u8_00,
            } => {
                buf.put_u8(13);
                buf.put_u32_le(unk_u32_00);
                buf.put_u8(unk_u8_00);
            }
            SiegeUpdate::Sub11 {
                unk_u32_00,
                unk_u32_01,
                unk_u32_02,
            } => {
                buf.put_u8(17);
                buf.put_u32_le(unk_u32_00);
                buf.put_u32_le(unk_u32_01);
                buf.put_u32_le(unk_u32_02);
            }
            SiegeUpdate::Sub12 {
                unk_u32_00,
                unk_u8_00,
            } => {
                buf.put_u8(18);
                buf.put_u32_le(unk_u32_00);
                buf.put_u8(unk_u8_00);
            }
            SiegeUpdate::Sub14 { unk_u8_00 } => {
                buf.put_u8(20);
                buf.put_u8(unk_u8_00);
            }
            SiegeUpdate::Other { sub, tail } => {
                buf.put_u8(sub);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
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

    /// Body bytes for a 0x385F arm, decoded and written back.
    fn siege_update(hex: &str) -> SiegeUpdate {
        let wire = Bytes::from(hex_to_bytes(hex));
        let decoded = SiegeUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(Bytes::from(decoded.clone()), wire, "write-back differs");
        decoded
    }

    /// The eight flat arms, one body each.
    #[test]
    fn the_flat_siege_arms_decode_by_width_and_order() {
        assert_eq!(
            siege_update("07 00222222 0111"),
            SiegeUpdate::Sub07 {
                unk_u32_00: 0x2222_2200,
                unk_u16_00: 0x1101,
            }
        );
        assert_eq!(
            siege_update(
                "08 00222222 0200 6631 0200 6632 0200 6633 04222222 05222222 06222222 07222222"
            ),
            SiegeUpdate::Sub08 {
                unk_u32_00: 0x2222_2200,
                unk_str_00: "f1".into(),
                unk_str_01: "f2".into(),
                unk_str_02: "f3".into(),
                unk_u32_01: 0x2222_2204,
                unk_u32_02: 0x2222_2205,
                unk_u32_03: 0x2222_2206,
                unk_u32_04: 0x2222_2207,
            }
        );
        assert_eq!(
            siege_update("0A 00222222"),
            SiegeUpdate::Sub0A {
                unk_u32_00: 0x2222_2200,
            }
        );
        assert_eq!(
            siege_update("0C 00222222 11"),
            SiegeUpdate::Sub0C {
                unk_u32_00: 0x2222_2200,
                unk_u8_00: 0x11,
            }
        );
        assert_eq!(
            siege_update("0D 00222222 11"),
            SiegeUpdate::Sub0D {
                unk_u32_00: 0x2222_2200,
                unk_u8_00: 0x11,
            }
        );
        assert_eq!(
            siege_update("11 00222222 01222222 02222222"),
            SiegeUpdate::Sub11 {
                unk_u32_00: 0x2222_2200,
                unk_u32_01: 0x2222_2201,
                unk_u32_02: 0x2222_2202,
            }
        );
        assert_eq!(
            siege_update("12 00222222 11"),
            SiegeUpdate::Sub12 {
                unk_u32_00: 0x2222_2200,
                unk_u8_00: 0x11,
            }
        );
        assert_eq!(
            siege_update("14 10"),
            SiegeUpdate::Sub14 { unk_u8_00: 0x10 }
        );
    }

    /// The first field of sub 7 is four bytes wide and the second two — a decode
    /// that reads both as `u32` would swallow the arm.
    #[test]
    fn a_flat_siege_arm_that_does_not_close_stays_raw() {
        assert!(matches!(
            siege_update("07 00222222 0111 FF"),
            SiegeUpdate::Other { sub: 7, .. }
        ));
    }

    /// Arms with a field behind a condition this decode cannot source, and arms
    /// with an unsettled list count, stay raw rather than half-read.
    #[test]
    fn partly_readable_siege_arms_stay_raw() {
        for sub in [11u8, 14, 15, 16] {
            let body = Bytes::from(vec![sub, 0x01, 0x02, 0x03, 0x04]);
            assert!(
                matches!(
                    SiegeUpdate::try_from(body).unwrap(),
                    SiegeUpdate::Other { .. }
                ),
                "sub {sub} should stay raw"
            );
        }
    }

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        (0..clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
            .collect()
    }
}

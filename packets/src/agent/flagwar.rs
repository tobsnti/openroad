//! Flag war (`0x34B1`) — the `u8` sub-command family behind the flag-war
//! announcements.
//!
//! Idea: `0x34B1` looks like eight scattered reads in one long function, and it
//! is not: the leading `u8` indexes a **jump table**, and one of its arms
//! (`0xFF`) reads a second `u8` and indexes a second table. Each arm then reads
//! its own body — most of them read nothing — which is why a linear list of the
//! reads is not the frame order.
//!
//! Where the variant names come from: each arm carries the message key the
//! original shows for it (`UIIT_MSG_FLAGWAR_*`), so the names are the original's
//! own words rather than a guess. The *fields* have no such source and keep
//! `unk_*` names.
//!
//! Arms outside the two tables, and arms whose body does not close on its last
//! byte, keep their bytes in an `Other` variant instead of being half-read.

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use sro_macro::SerializationError;

use crate::agent::cursor::{put_string, Cursor};

/// Sub-command `0xFF`: a second selector byte follows.
pub const FLAG_WAR_NOTICE: u8 = 0xFF;

/// 0x34B1 — server → client: a flag-war state change.
#[derive(Message, Clone, Debug, PartialEq)]
pub enum FlagWarUpdate {
    /// Sub 2 — `UIIT_MSG_FLAGWAR_STANDBY_10`.
    Standby10,
    /// Sub 4 — `UIIT_MSG_FLAGWAR_GAME_START`.
    GameStart,
    /// Sub 5 — no message and no body; the arm only calls into the flag-war
    /// window.
    Sub05,
    /// Sub 8 — `UIIT_MSG_FLAGWAR_GAME_START_MISSION`. The `u16` is compared
    /// against a value the client holds, and the message is shown only on a
    /// match, so it is a selector of some kind; which one is not readable here.
    GameStartMission { unk_u16_00: u16 },
    /// Sub 9 — `UIIT_MSG_FLAGWAR_GAME_END`.
    GameEnd,
    /// Sub 10 — the `u32` is looked up in the entity table, so it is an entity
    /// id; what is done with it is one virtual call away.
    Sub0A { unk_u32_00: u32 },
    /// Sub 13 — `UIIT_MSG_FLAGWAR_STANDBY_5`.
    Standby5,
    /// Sub 14 — `UIIT_MSG_FLAGWAR_STANDBY_1`.
    Standby1,
    /// Sub `0xFF` — the second table.
    Notice(FlagWarNotice),
    /// Every arm the first table sends to its default case, plus any arm whose
    /// body does not close on its last byte.
    Other { sub: u8, tail: Bytes },
}

/// The second-stage arms of [`FlagWarUpdate::Notice`].
#[derive(Clone, Debug, PartialEq)]
pub enum FlagWarNotice {
    /// Sub 0 — `UIIT_MSG_FLAGWAR_GAME_JOIN_SUCCES` or
    /// `..._GAME_JOIN_CANCEL`; which one is decided by client state, not by the
    /// body.
    GameJoin,
    /// Sub 4 — `UIIT_MSG_FLAGWAR_GAME_START_NOT_ENTER`.
    GameStartNotEnter,
    /// Sub 5 — `UIIT_MSG_FLAGWAR_GAME_JOIN_LEVEL_ERORR` (the original's
    /// spelling).
    GameJoinLevelError,
    /// Sub 6 — `UIIT_MSG_FLAGWAR_GAME_CANCEL`.
    GameCancel,
    /// Sub 8 — `UIIT_MSG_FLAGWAR_FLAGZONE_CANNOT_ENTER`.
    FlagZoneCannotEnter,
    /// Sub 9 — `UIIT_MSG_FLAGWAR_PICKUP_FLAG_OURTEAM`; the string is formatted
    /// into that message.
    PickupFlagOurTeam { unk_str_00: String },
    /// Sub 10 — `UIIT_MSG_FLAGWAR_PICKUP_FLAG_OTHERTEAM`.
    PickupFlagOtherTeam,
    /// Sub 11 — `UIIT_MSG_FLAGWAR_PICKUP_MASTERKEY`. The `u8` is compared
    /// against a byte the client holds and only a match shows the message.
    PickupMasterKey {
        unk_u32_00: u32,
        unk_u8_00: u8,
        unk_str_00: String,
    },
    /// Sub 12 — no message and no body; the arm opens a window.
    Sub0C,
    /// Sub 14 — `UIIT_MSG_FLAGWAR_FLAG_DROP`; the `u32` is looked up in the
    /// entity table.
    FlagDrop { unk_u32_00: u32 },
    /// Sub 15 — `UIIT_MSG_FLAGWAR_FLAG_MISSION_CLEAR`.
    FlagMissionClear,
    /// Sub 16 — `UIIT_MSG_FLAGWAR_FLAG_MISSION_FAIL`, same
    /// compare-then-show shape as [`Self::PickupMasterKey`].
    FlagMissionFail { unk_u8_00: u8, unk_str_00: String },
    /// Sub 17 — `UIIT_MSG_FLAGWAR_GAME_WIN`.
    GameWin,
    /// Sub 18 and 19 — `UIIT_MSG_FLAGWAR_PLAYER_FLAG_PICKUP`; both selectors
    /// reach the same arm, so the byte is kept to write the body back.
    PlayerFlagPickup { sub: u8 },
    /// Sub 20 and 21 — `UIIT_MSG_FLAGWAR_GAME_ENTERD_ERORR` (the original's
    /// spelling); two selectors, one arm.
    GameEnteredError { sub: u8 },
    /// Sub 22 — `UIIT_MSG_FLAGWAR_GAME_LOSE`.
    GameLose,
    /// Sub 23 — `UIIT_MSG_FLAGWAR_GAME_DRAW`.
    GameDraw,
    /// Every selector the second table sends to its default case, plus any arm
    /// whose body does not close on its last byte.
    Other { sub: u8, tail: Bytes },
}

impl TryFrom<Bytes> for FlagWarUpdate {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut c = Cursor::new(&value, "short 0x34B1 body");
        let sub = c.u8()?;
        let raw = || FlagWarUpdate::Other {
            sub,
            tail: value.slice(1..),
        };
        if sub == FLAG_WAR_NOTICE {
            // `FF` without its selector byte is not a notice at all.
            return Ok(match c.u8() {
                Ok(notice) => FlagWarUpdate::Notice(parse_notice(&mut c, &value, notice)),
                Err(_) => raw(),
            });
        }
        let parsed = (|| -> Option<FlagWarUpdate> {
            Some(match sub {
                2 => FlagWarUpdate::Standby10,
                4 => FlagWarUpdate::GameStart,
                5 => FlagWarUpdate::Sub05,
                8 => FlagWarUpdate::GameStartMission {
                    unk_u16_00: c.u16().ok()?,
                },
                9 => FlagWarUpdate::GameEnd,
                10 => FlagWarUpdate::Sub0A {
                    unk_u32_00: c.u32().ok()?,
                },
                13 => FlagWarUpdate::Standby5,
                14 => FlagWarUpdate::Standby1,
                _ => return None,
            })
        })()
        // The arm must consume the body exactly; anything else is a layout
        // surprise and keeps its bytes.
        .filter(|_| c.at_end());
        Ok(parsed.unwrap_or_else(raw))
    }
}

/// The second-stage decode; `sub` is the selector byte the caller read.
fn parse_notice(c: &mut Cursor<'_>, value: &Bytes, sub: u8) -> FlagWarNotice {
    let parsed = (|| -> Option<FlagWarNotice> {
        Some(match sub {
            0 => FlagWarNotice::GameJoin,
            4 => FlagWarNotice::GameStartNotEnter,
            5 => FlagWarNotice::GameJoinLevelError,
            6 => FlagWarNotice::GameCancel,
            8 => FlagWarNotice::FlagZoneCannotEnter,
            9 => FlagWarNotice::PickupFlagOurTeam {
                unk_str_00: c.string().ok()?,
            },
            10 => FlagWarNotice::PickupFlagOtherTeam,
            11 => FlagWarNotice::PickupMasterKey {
                unk_u32_00: c.u32().ok()?,
                unk_u8_00: c.u8().ok()?,
                unk_str_00: c.string().ok()?,
            },
            12 => FlagWarNotice::Sub0C,
            14 => FlagWarNotice::FlagDrop {
                unk_u32_00: c.u32().ok()?,
            },
            15 => FlagWarNotice::FlagMissionClear,
            16 => FlagWarNotice::FlagMissionFail {
                unk_u8_00: c.u8().ok()?,
                unk_str_00: c.string().ok()?,
            },
            17 => FlagWarNotice::GameWin,
            18 | 19 => FlagWarNotice::PlayerFlagPickup { sub },
            20 | 21 => FlagWarNotice::GameEnteredError { sub },
            22 => FlagWarNotice::GameLose,
            23 => FlagWarNotice::GameDraw,
            _ => return None,
        })
    })()
    .filter(|_| c.at_end());
    parsed.unwrap_or_else(|| FlagWarNotice::Other {
        sub,
        tail: value.slice(2..),
    })
}

impl From<FlagWarUpdate> for Bytes {
    fn from(p: FlagWarUpdate) -> Self {
        let mut buf = BytesMut::new();
        match p {
            FlagWarUpdate::Standby10 => buf.put_u8(2),
            FlagWarUpdate::GameStart => buf.put_u8(4),
            FlagWarUpdate::Sub05 => buf.put_u8(5),
            FlagWarUpdate::GameStartMission { unk_u16_00 } => {
                buf.put_u8(8);
                buf.put_u16_le(unk_u16_00);
            }
            FlagWarUpdate::GameEnd => buf.put_u8(9),
            FlagWarUpdate::Sub0A { unk_u32_00 } => {
                buf.put_u8(10);
                buf.put_u32_le(unk_u32_00);
            }
            FlagWarUpdate::Standby5 => buf.put_u8(13),
            FlagWarUpdate::Standby1 => buf.put_u8(14),
            FlagWarUpdate::Notice(notice) => {
                buf.put_u8(FLAG_WAR_NOTICE);
                write_notice(&mut buf, notice);
            }
            FlagWarUpdate::Other { sub, tail } => {
                buf.put_u8(sub);
                buf.extend_from_slice(&tail);
            }
        }
        buf.freeze()
    }
}

fn write_notice(buf: &mut BytesMut, notice: FlagWarNotice) {
    match notice {
        FlagWarNotice::GameJoin => buf.put_u8(0),
        FlagWarNotice::GameStartNotEnter => buf.put_u8(4),
        FlagWarNotice::GameJoinLevelError => buf.put_u8(5),
        FlagWarNotice::GameCancel => buf.put_u8(6),
        FlagWarNotice::FlagZoneCannotEnter => buf.put_u8(8),
        FlagWarNotice::PickupFlagOurTeam { unk_str_00 } => {
            buf.put_u8(9);
            put_string(buf, &unk_str_00);
        }
        FlagWarNotice::PickupFlagOtherTeam => buf.put_u8(10),
        FlagWarNotice::PickupMasterKey {
            unk_u32_00,
            unk_u8_00,
            unk_str_00,
        } => {
            buf.put_u8(11);
            buf.put_u32_le(unk_u32_00);
            buf.put_u8(unk_u8_00);
            put_string(buf, &unk_str_00);
        }
        FlagWarNotice::Sub0C => buf.put_u8(12),
        FlagWarNotice::FlagDrop { unk_u32_00 } => {
            buf.put_u8(14);
            buf.put_u32_le(unk_u32_00);
        }
        FlagWarNotice::FlagMissionClear => buf.put_u8(15),
        FlagWarNotice::FlagMissionFail {
            unk_u8_00,
            unk_str_00,
        } => {
            buf.put_u8(16);
            buf.put_u8(unk_u8_00);
            put_string(buf, &unk_str_00);
        }
        FlagWarNotice::GameWin => buf.put_u8(17),
        FlagWarNotice::PlayerFlagPickup { sub } | FlagWarNotice::GameEnteredError { sub } => {
            buf.put_u8(sub)
        }
        FlagWarNotice::GameLose => buf.put_u8(22),
        FlagWarNotice::GameDraw => buf.put_u8(23),
        FlagWarNotice::Other { sub, tail } => {
            buf.put_u8(sub);
            buf.extend_from_slice(&tail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body, decoded and written back byte for byte.
    fn update(hex: &str) -> FlagWarUpdate {
        let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        let wire = Bytes::from(
            (0..clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        );
        let decoded = FlagWarUpdate::try_from(wire.clone()).unwrap();
        assert_eq!(Bytes::from(decoded.clone()), wire, "write-back differs");
        decoded
    }

    /// The bodyless arms of the first table, in the order the table lists them.
    #[test]
    fn the_first_table_arms_without_a_body() {
        assert_eq!(update("02"), FlagWarUpdate::Standby10);
        assert_eq!(update("04"), FlagWarUpdate::GameStart);
        assert_eq!(update("05"), FlagWarUpdate::Sub05);
        assert_eq!(update("09"), FlagWarUpdate::GameEnd);
        assert_eq!(update("0D"), FlagWarUpdate::Standby5);
        assert_eq!(update("0E"), FlagWarUpdate::Standby1);
    }

    /// The two first-table arms that read: two bytes and four bytes.
    #[test]
    fn the_first_table_arms_with_a_body() {
        assert_eq!(
            update("08 3412"),
            FlagWarUpdate::GameStartMission { unk_u16_00: 0x1234 }
        );
        assert_eq!(
            update("0A 11111111"),
            FlagWarUpdate::Sub0A {
                unk_u32_00: 0x1111_1111
            }
        );
    }

    /// The second table: the four arms that read, and one that does not.
    #[test]
    fn the_second_table_arms() {
        assert_eq!(
            update("FF 00"),
            FlagWarUpdate::Notice(FlagWarNotice::GameJoin)
        );
        assert_eq!(
            update("FF 09 0200 4142"),
            FlagWarUpdate::Notice(FlagWarNotice::PickupFlagOurTeam {
                unk_str_00: "AB".into()
            })
        );
        assert_eq!(
            update("FF 0B 11111111 22 0200 4142"),
            FlagWarUpdate::Notice(FlagWarNotice::PickupMasterKey {
                unk_u32_00: 0x1111_1111,
                unk_u8_00: 0x22,
                unk_str_00: "AB".into(),
            })
        );
        assert_eq!(
            update("FF 0E 11111111"),
            FlagWarUpdate::Notice(FlagWarNotice::FlagDrop {
                unk_u32_00: 0x1111_1111
            })
        );
        assert_eq!(
            update("FF 10 22 0200 4142"),
            FlagWarUpdate::Notice(FlagWarNotice::FlagMissionFail {
                unk_u8_00: 0x22,
                unk_str_00: "AB".into(),
            })
        );
    }

    /// Two selectors share one arm in each of two places, and the body must
    /// write back as the selector it arrived with.
    #[test]
    fn the_shared_second_table_arms_keep_their_selector() {
        for sub in [18u8, 19] {
            assert_eq!(
                update(&format!("FF{sub:02X}")),
                FlagWarUpdate::Notice(FlagWarNotice::PlayerFlagPickup { sub })
            );
        }
        for sub in [20u8, 21] {
            assert_eq!(
                update(&format!("FF{sub:02X}")),
                FlagWarUpdate::Notice(FlagWarNotice::GameEnteredError { sub })
            );
        }
    }

    /// An arm outside the tables keeps its bytes, and so does an arm whose body
    /// does not close on its last byte — in both tables.
    #[test]
    fn arms_outside_the_tables_and_arms_that_do_not_close_stay_raw() {
        assert_eq!(
            update("03 AABBCC"),
            FlagWarUpdate::Other {
                sub: 3,
                tail: Bytes::from_static(&[0xAA, 0xBB, 0xCC]),
            }
        );
        assert_eq!(
            update("02 AA"),
            FlagWarUpdate::Other {
                sub: 2,
                tail: Bytes::from_static(&[0xAA]),
            }
        );
        assert_eq!(
            update("FF 01 AABBCC"),
            FlagWarUpdate::Notice(FlagWarNotice::Other {
                sub: 1,
                tail: Bytes::from_static(&[0xAA, 0xBB, 0xCC]),
            })
        );
        assert_eq!(
            update("FF 00 AA"),
            FlagWarUpdate::Notice(FlagWarNotice::Other {
                sub: 0,
                tail: Bytes::from_static(&[0xAA]),
            })
        );
    }

    /// `FF` with no selector byte is not a notice: it keeps its one byte.
    #[test]
    fn the_notice_selector_without_a_selector_byte_stays_raw() {
        assert_eq!(
            update("FF"),
            FlagWarUpdate::Other {
                sub: 0xFF,
                tail: Bytes::new(),
            }
        );
    }
}

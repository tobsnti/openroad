//! Guild war and siege authority: the war lifecycle (0x7110 start, 0x7112 end,
//! 0x7114 reward and their acks), the siege-authority update 0x70FF, and the
//! guild-hostility toggle 0x30EF.
//!
//! Idea: two of the three war acks are the shared guild ack form again; the
//! reward ack is not, and it is the one field in this family that a *server*
//! decompile names outright. `0x30EF` is the interesting one — it looks like a
//! per-entity flag and is not: the handler keys a **global guild-id set**, so
//! one packet changes the relation to every member of that guild at once.
//!
//! Layouts and evidence: `docs/net-guild-war.md`.

use bevy::prelude::Message;
use bytes::{BufMut, Bytes, BytesMut};

use crate::agent::cursor::{put_string, Cursor};
use crate::agent::guild::guild_op_ack;

use sro_macro::ByteSize;
use sro_macro::Deserialize;
use sro_macro::SerializationError;
use sro_macro::Serialize;
use sro_macro_derive::*;

/// The one guild-war error code recovered from both sides of the wire: the
/// client special-cases it with `UIIT_CTL_GUILDWAR_NOTCOMPENSATION` and the
/// server writes it when the compensation value is `<= 0`
/// (`FUN_005c72a0 @5c72d2`).
pub const GUILD_WAR_NO_COMPENSATION: u16 = 0x4C45;

/// 0x30EF — server → client: add or remove a guild from the client's
/// **guild-relation set**.
///
/// Despite the inherited name "entity update", the handler `FUN_0088a770` keys a
/// global red-black tree by *guild id*, not by entity uid: the same set is
/// queried elsewhere as `FUN_00469e00(it, guild + 0x30)`, and `+0x30` is where
/// the guild id lives (it is the `%u` in the `"G%u_%u_%u.crb"` crest filename).
/// So one packet affects every member of that guild.
///
/// `flag == 1` inserts, `flag == 0` erases. Whether membership means "hostile"
/// or "friendly" is [U] — the two consumer functions gate name-tag and targeting
/// behaviour, but their branch sense is not readable without their full context.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildRelationUpdate {
    /// 1 = add to the set, 0 = remove.
    pub flag: u8,
    pub guild_id: u32,
}

impl GuildRelationUpdate {
    pub fn is_add(&self) -> bool {
        self.flag == 1
    }
}

/// 0x70FF — client → server: update a member's siege authority.
/// Builder `FUN_0081f9e0@23`, body `b4 b1`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct SiegeAuthorityUpdateRequest {
    /// [U] — verbatim; the builder gives the width, nothing names it.
    pub unk_u32_00: u32,
    /// [U] — verbatim.
    pub unk_u8_00: u8,
}

/// 0x7110 — client → server: declare a guild war.
/// Builder `FUN_00824a30@45`, body `strS b1 b4 b1 b4`.
///
/// The only request in the guild family with a mixed body, and the only one that
/// names anything: the string is the opposing guild, which is how the original's
/// war dialog addresses it. The four scalars are [U].
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildWarStartRequest {
    pub target_guild: String,
    /// [U] — verbatim.
    pub unk_u8_00: u8,
    /// [U] — verbatim.
    pub unk_u32_00: u32,
    /// [U] — verbatim.
    pub unk_u8_01: u8,
    /// [U] — verbatim.
    pub unk_u32_01: u32,
}

/// 0x7112 — client → server: end a guild war. Builder `FUN_00820540@23`, `b4`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildWarEndRequest {
    /// [U] — verbatim.
    pub unk_u32_00: u32,
}

/// 0x7114 — client → server: claim the war compensation.
/// Builder `FUN_00820600@23`, `b4`.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildWarRewardRequest {
    /// [U] — verbatim.
    pub unk_u32_00: u32,
}

guild_op_ack! {
    /// 0xB110 — ack for [`GuildWarStartRequest`], handler `FUN_00881ec0`.
    ///
    /// Its tail runs **unconditionally**: the war-declaration dialog is torn
    /// down whether the request succeeded or failed. A consumer should close the
    /// dialog on both arms rather than only on success.
    GuildWarStartAck
}

guild_op_ack! {
    /// 0xB112 — ack for [`GuildWarEndRequest`], handler `FUN_00881f30`.
    GuildWarEndAck
}

/// 0xB114 — ack for [`GuildWarRewardRequest`], handler `FUN_00885ae0`.
///
/// The one war ack with a payload, and the one field in this family that a
/// server decompile names outright: `FUN_005c72a0` loads the compensation value,
/// refuses with [`GUILD_WAR_NO_COMPENSATION`] when it is `<= 0`, and otherwise
/// writes `u8 result = 1` followed by that `u32`. Reader and writer agree,
/// including the error constant.
#[derive(Message, Serialize, Deserialize, ByteSize, Clone, Debug, PartialEq)]
pub struct GuildWarRewardAck {
    pub result: u8,
    #[sro_packet(when = "result == 1")]
    pub compensation: Option<u32>,
    #[sro_packet(when = "result != 1")]
    pub error_code: Option<u16>,
}

impl GuildWarRewardAck {
    pub fn is_success(&self) -> bool {
        self.result == 1
    }

    /// The refusal the original words itself
    /// (`UIIT_CTL_GUILDWAR_NOTCOMPENSATION`) instead of sending it to the
    /// generic error box.
    pub fn is_no_compensation(&self) -> bool {
        self.error_code == Some(GUILD_WAR_NO_COMPENSATION)
    }
}

/// 0x3109 — server → client: the guild-war list.
///
/// A `u8` count, then that many entries. An entry leads with a `u32` that also
/// decides its length: on zero the entry ends there, otherwise seven more
/// scalars and a name follow. The original's reader is the pair
/// "record reader + the name behind it": the record reader stops right after
/// the leading `u32` when that `u32` is zero, and the caller reads the string
/// only for a non-zero one. So a list mixes long and short entries, and a
/// decoder that reads a fixed record length loses the rest of the list.
///
/// Field *meanings* are not readable from the reader — it stores the record and
/// hands it on whole — so everything but the structural count keeps an `unk_*`
/// name.
#[derive(Message, Clone, Debug, PartialEq)]
pub struct GuildWarInfo {
    pub entries: Vec<GuildWarInfoEntry>,
}

/// One entry of [`GuildWarInfo`].
#[derive(Clone, Debug, PartialEq)]
pub struct GuildWarInfoEntry {
    /// Zero ends the entry here.
    pub unk_u32_00: u32,
    /// Present exactly when [`Self::unk_u32_00`] is non-zero.
    pub detail: Option<GuildWarInfoDetail>,
}

/// The long form of a [`GuildWarInfoEntry`].
#[derive(Clone, Debug, PartialEq)]
pub struct GuildWarInfoDetail {
    pub unk_u32_00: u32,
    pub unk_u8_00: u8,
    pub unk_u32_01: u32,
    pub unk_u32_02: u32,
    pub unk_u32_03: u32,
    pub unk_u32_04: u32,
    pub unk_u32_05: u32,
    pub unk_str_00: String,
}

/// A body that carries more bytes than the list describes. It is refused rather
/// than truncated: the surplus would mean the entry shape is wrong, and a
/// half-read war list is worse than a logged decode failure.
fn trailing() -> SerializationError {
    SerializationError::IoError(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "trailing bytes in 0x3109 body",
    ))
}

impl TryFrom<Bytes> for GuildWarInfo {
    type Error = SerializationError;
    fn try_from(value: Bytes) -> Result<Self, SerializationError> {
        let mut c = Cursor::new(&value, "short 0x3109 body");
        let count = c.u8()?;
        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let unk_u32_00 = c.u32()?;
            let detail = if unk_u32_00 == 0 {
                None
            } else {
                Some(GuildWarInfoDetail {
                    unk_u32_00: c.u32()?,
                    unk_u8_00: c.u8()?,
                    unk_u32_01: c.u32()?,
                    unk_u32_02: c.u32()?,
                    unk_u32_03: c.u32()?,
                    unk_u32_04: c.u32()?,
                    unk_u32_05: c.u32()?,
                    unk_str_00: c.string()?,
                })
            };
            entries.push(GuildWarInfoEntry { unk_u32_00, detail });
        }
        if !c.at_end() {
            return Err(trailing());
        }
        Ok(GuildWarInfo { entries })
    }
}

impl From<GuildWarInfo> for Bytes {
    fn from(p: GuildWarInfo) -> Self {
        let mut buf = BytesMut::new();
        buf.put_u8(p.entries.len() as u8);
        for entry in &p.entries {
            buf.put_u32_le(entry.unk_u32_00);
            // The leading `u32` decides the length, so a detail is written only
            // where the reader would look for one.
            if entry.unk_u32_00 == 0 {
                continue;
            }
            if let Some(detail) = &entry.detail {
                buf.put_u32_le(detail.unk_u32_00);
                buf.put_u8(detail.unk_u8_00);
                buf.put_u32_le(detail.unk_u32_01);
                buf.put_u32_le(detail.unk_u32_02);
                buf.put_u32_le(detail.unk_u32_03);
                buf.put_u32_le(detail.unk_u32_04);
                buf.put_u32_le(detail.unk_u32_05);
                put_string(&mut buf, &detail.unk_str_00);
            }
        }
        buf.freeze()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, Bytes, BytesMut};

    /// Five fixed bytes, and the `u32` is a guild id — not an entity uid.
    #[test]
    fn the_relation_update_is_a_flag_and_a_guild_id() {
        let wire = Bytes::from_static(&[0x01, 0x2A, 0x00, 0x00, 0x00]);
        let decoded = GuildRelationUpdate::try_from(wire.clone()).unwrap();
        assert!(decoded.is_add());
        assert_eq!(decoded.guild_id, 0x2A);
        assert_eq!(Bytes::from(decoded), wire);

        let removal = Bytes::from_static(&[0x00, 0x2A, 0x00, 0x00, 0x00]);
        assert!(!GuildRelationUpdate::try_from(removal).unwrap().is_add());
    }

    /// The war declaration is the family's only mixed body: a name first, then
    /// four scalars in the builder's order.
    #[test]
    fn the_war_declaration_leads_with_the_opposing_guild_name() {
        let mut body = BytesMut::new();
        body.put_u16_le(5);
        body.put_slice(b"Rival");
        body.put_u8(1);
        body.put_u32_le(2);
        body.put_u8(3);
        body.put_u32_le(4);
        let wire = body.freeze();

        let decoded = GuildWarStartRequest::try_from(wire.clone()).unwrap();
        assert_eq!(decoded.target_guild, "Rival");
        assert_eq!(
            (
                decoded.unk_u8_00,
                decoded.unk_u32_00,
                decoded.unk_u8_01,
                decoded.unk_u32_01
            ),
            (1, 2, 3, 4)
        );
        assert_eq!(Bytes::from(decoded), wire);
    }

    /// 5 bytes carrying the compensation on success, 3 on refusal — and the
    /// refusal the original words itself is recognised by name.
    #[test]
    fn the_reward_ack_carries_the_compensation_or_the_no_compensation_code() {
        let mut ok = BytesMut::new();
        ok.put_u8(1);
        ok.put_u32_le(5000);
        let ok = ok.freeze();
        let decoded = GuildWarRewardAck::try_from(ok.clone()).unwrap();
        assert_eq!(decoded.compensation, Some(5000));
        assert!(!decoded.is_no_compensation());
        assert_eq!(Bytes::from(decoded), ok);

        let refused = Bytes::from_static(&[0x02, 0x45, 0x4C]);
        let decoded = GuildWarRewardAck::try_from(refused.clone()).unwrap();
        assert!(decoded.is_no_compensation());
        assert_eq!(decoded.compensation, None);
        assert_eq!(Bytes::from(decoded), refused);
    }

    /// Start and end acks are the shared cluster form.
    #[test]
    fn the_war_lifecycle_acks_are_the_shared_form() {
        let ok = Bytes::from_static(&[0x01]);
        assert!(GuildWarStartAck::try_from(ok.clone()).unwrap().is_success());
        assert!(GuildWarEndAck::try_from(ok).unwrap().is_success());

        let refused = Bytes::from_static(&[0x02, 0x0E, 0x1C]);
        let ack = GuildWarStartAck::try_from(refused.clone()).unwrap();
        assert_eq!(ack.error_code, Some(0x1C0E));
        assert_eq!(Bytes::from(ack), refused);
    }

    /// A war-list body, decoded and written back byte for byte.
    fn war_info(hex: &str) -> GuildWarInfo {
        let clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        let wire = Bytes::from(
            (0..clean.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>(),
        );
        let decoded = GuildWarInfo::try_from(wire.clone()).unwrap();
        assert_eq!(Bytes::from(decoded.clone()), wire, "write-back differs");
        decoded
    }

    /// The list mixes entry lengths: the leading `u32` decides whether the
    /// seven scalars and the name follow.
    #[test]
    fn the_war_list_mixes_short_and_long_entries() {
        assert_eq!(war_info("00").entries, vec![]);

        assert_eq!(
            war_info("01 00000000").entries,
            vec![GuildWarInfoEntry {
                unk_u32_00: 0,
                detail: None,
            }]
        );

        let both = war_info(
            "02 01000000 02000000 03 04000000 05000000 06000000 07000000 \
             08000000 0200 4142 00000000",
        );
        assert_eq!(
            both.entries,
            vec![
                GuildWarInfoEntry {
                    unk_u32_00: 1,
                    detail: Some(GuildWarInfoDetail {
                        unk_u32_00: 2,
                        unk_u8_00: 3,
                        unk_u32_01: 4,
                        unk_u32_02: 5,
                        unk_u32_03: 6,
                        unk_u32_04: 7,
                        unk_u32_05: 8,
                        unk_str_00: "AB".into(),
                    }),
                },
                GuildWarInfoEntry {
                    unk_u32_00: 0,
                    detail: None,
                },
            ]
        );
    }

    /// A fixed-length record would read the second entry out of the first
    /// entry's name — the count must drive the walk, and the walk must end on
    /// the last byte.
    #[test]
    fn a_war_list_that_does_not_close_is_refused() {
        let short = Bytes::from_static(&[0x01, 0x01, 0x00, 0x00, 0x00]);
        assert!(GuildWarInfo::try_from(short).is_err());

        let surplus = Bytes::from_static(&[0x01, 0x00, 0x00, 0x00, 0x00, 0xFF]);
        assert!(GuildWarInfo::try_from(surplus).is_err());
    }

    /// The two single-`u32` requests round-trip at four bytes.
    #[test]
    fn the_end_and_reward_requests_are_a_single_u32() {
        let wire = Bytes::from_static(&[0x07, 0x00, 0x00, 0x00]);
        assert_eq!(
            GuildWarEndRequest::try_from(wire.clone())
                .unwrap()
                .unk_u32_00,
            7
        );
        assert_eq!(GuildWarRewardRequest::try_from(wire).unwrap().unk_u32_00, 7);
    }
}

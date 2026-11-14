//! A server's refusal, in the original's own words.
//!
//! Idea: the original shows an error code through one function. The handler
//! that received it names an error category, and per category a table maps the
//! code to a `textuisystem` key; a code without a row shows nothing at all
//! `[V original handler]`. This is that mechanism once, so a feature that
//! receives a refusal looks its text up here instead of writing its own table.
//! A category holds only the codes some consumer of ours can receive, each
//! with the original's key; the English after it is what a player without
//! the data reads.

use crate::plugins::textdata::ClientUiStrings;

/// One code of a category: its `textuisystem` key and the English fallback.
pub struct ErrorRow {
    pub code: u16,
    pub key: &'static str,
    pub fallback: &'static str,
}

/// Category 13 — an NPC interaction refused (teleport and its kin). The rows
/// are the codes the original server's 0x705A handler can answer with a text;
/// its other codes (1, 2, 3, `0x1C0F`) have none in the original either.
pub const INTERACTION: &[ErrorRow] = &[
    ErrorRow {
        code: 0x0000,
        key: "UIIT_STT_ERR_COMMON_INVALID_TARGET",
        fallback: "Invalid object",
    },
    ErrorRow {
        code: 0x0004,
        key: "UIIT_MSG_INTERACTION_FAIL_TOO_FAR",
        fallback: "The object is too far.",
    },
    ErrorRow {
        code: 0x1C07,
        key: "UIIT_MSG_INTERACTION_FAIL_NOT_ENOUGH_MONEY",
        fallback: "Not enough gold",
    },
    ErrorRow {
        code: 0x1C14,
        key: "UIIT_MSG_INTERACTION_FAIL_TELESCROLL_ALREADY_WORKING",
        fallback: "Cannot choose another teleport method because you are now being teleported.",
    },
    ErrorRow {
        code: 0x1C21,
        key: "UIIT_MSG_NEWBIE_RETURN_ERROR",
        fallback: "Cannot reverse return because you have not returned or died.",
    },
    ErrorRow {
        code: 0x1C31,
        key: "UIIT_MSG_INSTANCEDUNGEON_ERROR_MSG_02",
        fallback:
            "You are already inside of instance dungeon. Therefore unable to enter other dungeon.",
    },
];

/// Category 4 — a skill or attack refused, by the server's failed 0xB070 or by
/// the client's own checks before a cast goes out; the original raises both
/// through this category `[V original handler]`. The rows are the codes a vSRO
/// server refused actions with `[V capture, peer vSRO]` and the ones our cast
/// gates raise; cooldown (0x3005) is in the capture and has no text in the
/// original. The original shows MP, HP, weapon and obstacle refusals in its
/// warning banner rather than the chat; error_text has one channel, so they
/// are chat lines.
pub const SKILL_USE: &[ErrorRow] = &[
    ErrorRow {
        code: SKILL_NOT_ENOUGH_MP,
        key: "UIIT_SKILL_USE_FAIL_NOTENOUGHMP",
        fallback: "Cannot use due to insufficient MP",
    },
    ErrorRow {
        code: 0x3006,
        key: "UIIT_SKILL_USE_FAIL_WRONGTARGET",
        fallback: "Invalid target",
    },
    ErrorRow {
        code: SKILL_WRONG_WEAPON,
        key: "UIIT_SKILL_USE_FAIL_WRONGWEAPON",
        fallback: "You wear equipment which prohibits you from using the selected skill.",
    },
    ErrorRow {
        code: 0x300F,
        key: "UIIT_SKILL_USE_FAIL_BROKEN_WEAPON",
        fallback: "Cannot attack because the weapon is broken",
    },
    ErrorRow {
        code: 0x3010,
        key: "UIIT_SKILL_USE_FAIL_PATH_INTERRUPTED",
        fallback: "Cannot attack due to an obstacle.",
    },
    ErrorRow {
        code: SKILL_NO_TARGET,
        key: "UIIT_SKILL_USE_FAIL_NONESELECTETARGET",
        fallback: "The target is not selected.",
    },
    ErrorRow {
        code: SKILL_NOT_ENOUGH_HP,
        key: "UIIT_SKILL_USE_FAIL_NOTENOUGHHP",
        fallback: "Cannot use due to insufficient HP",
    },
];

/// The category-4 codes our cast gates raise: the ones the original's server
/// (MP, HP, weapon) or its own client check (no target) gives the same refusal.
pub const SKILL_NOT_ENOUGH_MP: u16 = 0x3004;
pub const SKILL_WRONG_WEAPON: u16 = 0x300D;
pub const SKILL_NO_TARGET: u16 = 0x3011;
pub const SKILL_NOT_ENOUGH_HP: u16 = 0x3013;

/// The line for `code` in `category`, or `None` where the original says
/// nothing.
pub fn error_line(category: &[ErrorRow], code: u16, ui: &ClientUiStrings) -> Option<String> {
    let row = category.iter().find(|row| row.code == code)?;
    Some(ui.get_message_or(row.key, row.fallback))
}

#[cfg(test)]
mod test {
    use super::*;

    /// A coded row reads through the player's data; a code the original shows
    /// no text for stays silent rather than getting one of ours.
    #[test]
    fn a_row_reads_its_key_and_an_unlisted_code_is_silent() {
        let ui = ClientUiStrings::from_rows(&[(
            "UIIT_MSG_INTERACTION_FAIL_NOT_ENOUGH_MONEY",
            "Not enough gold!",
        )]);
        assert_eq!(
            error_line(INTERACTION, 0x1C07, &ui).as_deref(),
            Some("Not enough gold!")
        );
        assert_eq!(
            error_line(INTERACTION, 0x0004, &ClientUiStrings::default()).as_deref(),
            Some("The object is too far.")
        );
        assert_eq!(error_line(INTERACTION, 1, &ui), None);
        assert_eq!(error_line(INTERACTION, 0x1C0F, &ui), None);
    }
}

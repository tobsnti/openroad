//! Error text for the pre-game screens.
//!
//! IDEA. The packet crate already carries the original's own error tables —
//! `login_error_text` and `lobby_error_text` read the dispatcher's jump table
//! and hand back a `textuisystem` key plus the shipped English fallback. They
//! landed with tests and then nothing called them: the screens kept printing
//! either a raw decimal ("error 1035") or one fixed sentence for every code.
//! This module is the one place that turns a table row into the line the
//! player reads, so the three screens stop each inventing their own wording.

use packets::agent::lobby::{lobby_error_text, LobbyErrorText};
use packets::login::login_error_text;

use crate::plugins::textdata::ClientUiStrings;

/// The line for a lobby failure (character list, create, delete, restore,
/// name check).
///
/// `None` means the original shows nothing at all for this code — matching its
/// silence matters as much as matching its text.
pub fn lobby_failure_line(ui_strings: &ClientUiStrings, code: u16) -> Option<String> {
    match lobby_error_text(code) {
        LobbyErrorText::Silent => None,
        LobbyErrorText::CodeOnly => Some(format!("(S{code})")),
        LobbyErrorText::Text {
            key,
            fallback,
            code_suffix,
        } => {
            let text = ui_strings.get_message_or(key, fallback);
            Some(if code_suffix {
                format!("{text} (S{code})")
            } else {
                text
            })
        }
    }
}

/// The line for a gateway login failure.
///
/// Codes 1 and 2 return `None`: the original renders those from the payload
/// (the attempt counter and the ban reason), which the login screen already
/// does itself.
pub fn login_failure_line(ui_strings: &ClientUiStrings, code: u8) -> Option<String> {
    let (key, fallback) = login_error_text(code)?;
    Some(ui_strings.get_message_or(key, fallback))
}

#[cfg(test)]
mod test {
    use super::*;
    /// No `textuisystem` rows: every line then comes from the table's own
    /// shipped fallback, which is what a player without that data sees.
    fn no_strings() -> ClientUiStrings {
        ClientUiStrings::from_rows(&[])
    }

    #[test]
    fn a_captured_lobby_code_reads_as_text_not_as_a_decimal() {
        let s = no_strings();
        // 0x040B is the delete rejection our captures carry; the screen used
        // to print "Character deletion failed (error 1035)."
        let line = lobby_failure_line(&s, 0x040B).expect("0x040B has a row");
        assert!(!line.is_empty());
        assert_ne!(line, "1035");
        assert!(!line.contains("deletion failed"));
    }

    #[test]
    fn the_silent_code_stays_silent() {
        // The original returns before selecting any string for 0x0401, so
        // matching its silence matters as much as matching its text.
        assert_eq!(lobby_failure_line(&no_strings(), 0x0401), None);
    }

    #[test]
    fn an_out_of_table_code_shows_only_the_suffix() {
        // 0x0419 (restore) is past the jump table: the original looks up the
        // empty string and only the appended code remains.
        assert_eq!(
            lobby_failure_line(&no_strings(), 0x0419),
            Some("(S1049)".to_string())
        );
    }

    /// Login error `0x0B` is the measured case: the shipped row for
    /// `UIIO_CLIENT_START_CONTENT_FAIL_BILLING_FAILED` is the bare `0` this
    /// data set uses for an unfilled entry (339 of its 5324 rows). Preferring
    /// the row there explains a failed login with a digit.
    #[test]
    fn an_unfilled_row_does_not_become_the_error_message() {
        let strings = ClientUiStrings::from_tsv(
            "1\tUIIO_CLIENT_START_CONTENT_FAIL_BILLING_FAILED\t\t\t\t\t\t\t0\r\n",
        );
        let line = login_failure_line(&strings, 0x0B).expect("0x0B has a row");
        assert_ne!(line, "0");
        assert!(line.contains("Billing"), "got {line:?}");
    }

    /// ...and the data still wins wherever it carries a sentence, which is the
    /// whole point of the lookup: 19 of the 21 keys in the two tables are
    /// byte-identical to the shipped rows, and those rows must keep priority.
    #[test]
    fn a_filled_row_still_beats_the_shipped_fallback() {
        let strings = ClientUiStrings::from_tsv(
            "1\tUIO_MSG_ERROR_OVERLAP\t\t\t\t\t\t\tAlready connected, from the data.\r\n",
        );
        assert_eq!(
            login_failure_line(&strings, 3),
            Some("Already connected, from the data.".to_string())
        );
    }

    #[test]
    fn login_codes_with_a_payload_render_nothing_here() {
        // 1 and 2 carry the attempt counter and the ban reason; the login
        // screen renders those from the payload itself.
        let s = no_strings();
        assert_eq!(login_failure_line(&s, 1), None);
        assert_eq!(login_failure_line(&s, 2), None);
        assert!(login_failure_line(&s, 3).is_some());
    }
}

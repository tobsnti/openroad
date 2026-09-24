//! Chat window settings: per-channel text colors as AARRGGBB hex strings so
//! they can be customized in `config.yaml` without recompiling. Invalid hex
//! values warn and fall back to the built-in default at resolve time.

use bevy::prelude::*;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Default)]
pub struct ChatSettings {
    #[serde(default)]
    pub colors: ChatColorSettings,
    /// Fade the chat chrome out after a few idle seconds.
    ///
    /// Non-original: the v1.188 client has no idle fade — it ships a manual
    /// transparency slider instead. Off by
    /// default so the stock client matches the original; the locked rule keeps
    /// non-original behaviour behind a flag.
    #[serde(default)]
    pub idle_fade: bool,
}

/// AARRGGBB hex per chat channel.
///
/// Sourced from the original client: the PK2 really has no colour table — `ifchatviewer.txt`
/// carries white on every `GDR_LIST_*` — but the v1.188 client compiles one
/// in, as a switch on the wire `chat_type` inside the chat-line formatter: a
/// 16-entry jump table with one `mov ebp, imm32` per case, the immediate being
/// the `0xAARRGGBB` colour. The defaults below are those immediates verbatim,
/// and each field names the `chat_type` case it was read from — that case
/// number is the citation a reader can re-derive the value from. They stay
/// config-driven so a user can override them; a deliberate deviation from the
/// original (contrast, accessibility) is fine but must say why.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(default)]
pub struct ChatColorSettings {
    /// All/local chat, stall lines and every unnamed type — the switch's
    /// *default* arm, which sets the colour with `or ebp,-1` (= `0xFFFFFFFF`).
    pub normal: String,
    /// Whisper — `chat_type` case 2, shared with case 10.
    pub whisper: String,
    /// Party — `chat_type` case 4.
    pub party: String,
    /// Guild — `chat_type` case 5.
    pub guild: String,
    /// GM chat and notices — `chat_type` cases 3 and 7 share one arm; our own
    /// client info lines reuse it.
    pub gm_notice: String,
    /// Union/alliance — `chat_type` case 11.
    pub union: String,
    /// Academy — `chat_type` case 16.
    pub academy: String,
    /// Global — `chat_type` case 6.
    pub global: String,
    /// NPC dialog lines — `chat_type` case 13. Its own arm in the original,
    /// not the `normal` white we used to fold it into.
    pub npc: String,
}

impl Default for ChatColorSettings {
    fn default() -> Self {
        Self {
            normal: "FFFFFFFF".into(),
            whisper: "FF9FFFFE".into(),
            party: "FF9AFFD0".into(),
            guild: "FFFFB541".into(),
            gm_notice: "FFFFAEC3".into(),
            union: "FFC2F573".into(),
            academy: "FF64C7FF".into(),
            global: "FFFFFF00".into(),
            npc: "FFDBADF8".into(),
        }
    }
}

/// The parsed, ready-to-use colors (a resource so UI systems don't re-parse
/// hex strings per frame).
#[derive(Resource, Debug, Clone)]
pub struct ChatColors {
    pub normal: Color,
    pub whisper: Color,
    pub party: Color,
    pub guild: Color,
    pub gm_notice: Color,
    pub union: Color,
    pub academy: Color,
    pub global: Color,
    pub npc: Color,
}

impl ChatColorSettings {
    pub fn resolved(&self) -> ChatColors {
        let defaults = ChatColorSettings::default();
        let parse = |field: &str, value: &str, default: &str| {
            parse_argb(value).unwrap_or_else(|| {
                warn!("config: invalid chat color {field}={value:?}, using {default}");
                parse_argb(default).expect("default chat colors are valid")
            })
        };
        ChatColors {
            normal: parse("normal", &self.normal, &defaults.normal),
            whisper: parse("whisper", &self.whisper, &defaults.whisper),
            party: parse("party", &self.party, &defaults.party),
            guild: parse("guild", &self.guild, &defaults.guild),
            gm_notice: parse("gm_notice", &self.gm_notice, &defaults.gm_notice),
            union: parse("union", &self.union, &defaults.union),
            academy: parse("academy", &self.academy, &defaults.academy),
            global: parse("global", &self.global, &defaults.global),
            npc: parse("npc", &self.npc, &defaults.npc),
        }
    }
}

/// `AARRGGBB` (or `RRGGBB`, alpha FF) hex → sRGB color.
pub(crate) fn parse_argb(hex: &str) -> Option<Color> {
    let hex = hex.trim().trim_start_matches("0x").trim_start_matches('#');
    let value = u32::from_str_radix(hex, 16).ok()?;
    let (a, r, g, b) = match hex.len() {
        8 => (
            (value >> 24) as u8,
            (value >> 16) as u8,
            (value >> 8) as u8,
            value as u8,
        ),
        6 => (0xFF, (value >> 16) as u8, (value >> 8) as u8, value as u8),
        _ => return None,
    };
    Some(Color::srgba_u8(r, g, b, a))
}

/// The palette a default (or absent) `ChatColorSettings` block resolves to.
///
/// Needed because the resource is now `init_resource`d and (re)filled by an
/// apply system whenever `ClientConfig` changes — see
/// [`crate::plugins::settings::live`] — instead of being derived once in
/// `Plugin::build`, where a later config edit could never reach it.
impl Default for ChatColors {
    fn default() -> Self {
        ChatColorSettings::default().resolved()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard the sourced table: each default is an
    /// instruction immediate out of the original client, so a future edit that
    /// swaps one for an invented value fails here. The `chat_type` case each
    /// immediate was read from is named on the field; this asserts the
    /// resolved sRGB bytes.
    #[test]
    fn defaults_are_the_original_clients_compiled_colors() {
        let colors = ChatColorSettings::default().resolved();
        let expect = |argb: u32| {
            Color::srgba_u8(
                (argb >> 16) as u8,
                (argb >> 8) as u8,
                argb as u8,
                (argb >> 24) as u8,
            )
        };
        assert_eq!(colors.normal, expect(0xFFFF_FFFF), "ALL (default arm)");
        assert_eq!(colors.whisper, expect(0xFF9F_FFFE), "PM, chat_type 2");
        assert_eq!(
            colors.gm_notice,
            expect(0xFFFF_AEC3),
            "GM/notice, chat_type 3/7"
        );
        assert_eq!(colors.party, expect(0xFF9A_FFD0), "party, chat_type 4");
        assert_eq!(colors.guild, expect(0xFFFF_B541), "guild, chat_type 5");
        assert_eq!(colors.global, expect(0xFFFF_FF00), "global, chat_type 6");
        assert_eq!(colors.union, expect(0xFFC2_F573), "union, chat_type 11");
        assert_eq!(colors.npc, expect(0xFFDB_ADF8), "NPC, chat_type 13");
        assert_eq!(colors.academy, expect(0xFF64_C7FF), "academy, chat_type 16");
    }

    /// A partial `chat.colors` block keeps the sourced defaults for the keys it
    /// omits (`#[serde(default)]` per field), so an override cannot silently
    /// blank the rest of the table.
    #[test]
    fn partial_config_block_keeps_sourced_defaults() {
        let settings: ChatColorSettings =
            serde_yaml::from_str("party: \"FF102030\"\n").expect("must parse");
        let colors = settings.resolved();
        assert_eq!(colors.party, Color::srgba_u8(0x10, 0x20, 0x30, 0xFF));
        assert_eq!(colors.npc, Color::srgba_u8(0xDB, 0xAD, 0xF8, 0xFF));
        assert_eq!(colors.whisper, Color::srgba_u8(0x9F, 0xFF, 0xFE, 0xFF));
    }

    /// The public-tree scrub (264c3176) removed the local RE file paths and
    /// the instruction addresses from this file's docs, but cut several
    /// sentences mid-clause: what was left promised a citation ("each with its
    /// address", "— @.") that no longer stood anywhere, which under ADR 0009
    /// reads as nine unsourced magic numbers. Pinned here because prose is
    /// exactly what no other test looks at.
    #[test]
    fn no_doc_promises_a_citation_it_no_longer_carries() {
        let src = include_str!("chat.rs");
        // The production half only: this test names the stubs verbatim.
        let docs = src.split("#[cfg(test)]").next().expect("a first half");
        for stub in ["— @.", "each with its address", "(jump table\n/// at,"] {
            assert!(
                !docs.contains(stub),
                "a scrubbed citation stub is back: {stub:?}"
            );
        }
    }

    #[test]
    fn parses_aarrggbb() {
        assert_eq!(
            parse_argb("FFFFB541"),
            Some(Color::srgba_u8(0xFF, 0xB5, 0x41, 0xFF))
        );
        assert_eq!(
            parse_argb("8000FF00"),
            Some(Color::srgba_u8(0x00, 0xFF, 0x00, 0x80))
        );
        assert_eq!(parse_argb("B541"), None);
        assert_eq!(parse_argb("nothex00"), None);
    }
}

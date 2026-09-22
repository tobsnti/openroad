//! The playable races, derived from the corpus instead of enumerated.
//!
//! Idea: which races a client can offer is a property of the *data*, not of
//! this program. The character table is the only place that says which bodies
//! really exist, and it says it in the code name itself:
//! `CHAR_<XX>_{MAN,WOMAN}_<SUFFIX>` — `<XX>` is the race code. So the race
//! list is read off those rows ([`races_from_bodies`]), and a two-letter
//! [`Race`] code is all the rest of the pre-game screens carry around. A
//! corpus with only `CHAR_CH_*` rows offers one race, a corpus that also ships
//! `CHAR_EU_*` offers two, and a corpus with a third prefix we have never seen
//! offers three — without a code change here.
//!
//! What is *not* derivable is the per-race presentation: which plate art,
//! which caption key, which loading picture. Those are authored per race in
//! the interface resinfo and the string table, and we can only cite the ones
//! our reference corpus authors ([`KNOWN_RACES`]). A race the corpus carries
//! bodies for but no presentation for is still offered — it renders with its
//! data code as the caption and no baked plate art, and says so in the log.
//! That is a deliberate deviation (the original ships exactly the races its
//! own interface files know), stated here rather than hidden behind a
//! two-value enumeration.

use crate::plugins::textdata::ClientCharacterData;

/// A playable race, identified by the two-letter code the character table uses
/// in `CHAR_<XX>_{MAN,WOMAN}_*`. Not an enumeration: the set of valid values
/// is whatever the corpus holds.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Race {
    /// ASCII, uppercase, exactly as the row spells it.
    code: [u8; 2],
}

impl Race {
    /// `CHAR_CH_*` — the Chinese/Asian body set.
    pub const CHINESE: Race = Race::from_ascii(*b"CH");
    /// `CHAR_EU_*` — the European body set.
    pub const EUROPEAN: Race = Race::from_ascii(*b"EU");

    pub const fn from_ascii(code: [u8; 2]) -> Race {
        Race { code }
    }

    /// The race of a body code name, or `None` when the name is not a body row
    /// of the `CHAR_<XX>_{MAN,WOMAN}_<SUFFIX>` shape. The gender segment is
    /// required on purpose: it is what separates a *playable* body row from
    /// every other `CHAR_` row in the same table (monsters, NPCs, pets), and
    /// it is the same rule the figure picker already filters by.
    pub fn from_body_code_name(code_name: &str) -> Option<Race> {
        let rest = code_name.strip_prefix("CHAR_")?;
        let (code, rest) = rest.split_once('_')?;
        let bytes = code.as_bytes();
        if bytes.len() != 2 || !bytes.iter().all(|b| b.is_ascii_uppercase()) {
            return None;
        }
        let suffix = rest
            .strip_prefix("MAN_")
            .or_else(|| rest.strip_prefix("WOMAN_"))?;
        if suffix.is_empty() {
            return None;
        }
        Some(Race::from_ascii([bytes[0], bytes[1]]))
    }

    /// The code as it appears in the data, e.g. `"CH"`.
    pub fn code(&self) -> &str {
        // built from two ASCII-uppercase bytes in every constructor
        std::str::from_utf8(&self.code).unwrap_or("??")
    }

    /// The authored presentation of this race, when our reference corpus has
    /// one. `None` = a race the data carries bodies for but whose interface
    /// files we cannot cite; callers degrade instead of inventing art.
    pub fn presentation(&self) -> Option<&'static RacePresentation> {
        KNOWN_RACES.iter().find(|p| p.code == self.code())
    }

    /// The body-row prefix of this race for one gender, e.g. `CHAR_CH_MAN_`.
    /// Built from the code, so it exists for a race we have never seen.
    pub fn body_prefix_of(&self, gender_segment: &str) -> String {
        format!("CHAR_{}_{gender_segment}_", self.code())
    }

    /// Caption key + fallback of the race name. A race without authored
    /// strings falls back to its data code, which is the only name we have for
    /// it and is at least checkable against the table.
    pub fn label(&self) -> (&'static str, String) {
        match self.presentation() {
            Some(p) => (p.label_key, p.label_fallback.to_string()),
            None => ("", self.code().to_string()),
        }
    }
}

impl Default for Race {
    /// A placeholder for a screen entered without a plate click (the dev
    /// jump): the first race [`KNOWN_RACES`] lists. Every real path takes the
    /// race from the region board, i.e. from [`available_races`].
    fn default() -> Self {
        Race::from_ascii([
            KNOWN_RACES[0].code.as_bytes()[0],
            KNOWN_RACES[0].code.as_bytes()[1],
        ])
    }
}

impl std::fmt::Debug for Race {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Race({})", self.code())
    }
}

/// Everything the interface data authors per race, for the races our reference
/// corpus authors. Each field carries its own citation.
pub struct RacePresentation {
    /// Body-table code this presentation belongs to.
    pub code: &'static str,
    /// Plate texture from `pscharacterselect.txt`, section `Select`
    /// (`GDR_STA_CHINA` (9) / `GDR_STA_EUROPE` (11); the third declared plate
    /// `GDR_STA_ISLAM` (10) has art in the archive but no body code we can
    /// name, so it is not listed here — inventing that mapping is exactly the
    /// unsourced guess this table exists to avoid).
    pub plate_ddj: &'static str,
    /// Full-screen loading picture the region cut shows.
    pub loading_ddj: &'static str,
    /// `GDR_STATIC1` label of the plate tab: key + English fallback.
    pub label_key: &'static str,
    pub label_fallback: &'static str,
    /// Tooltip/description key shown in the plate body rect.
    pub desc_key: &'static str,
    /// x of `GDR_STATIC1` inside the plate — genuinely per race
    /// (section `China` 318, section `Europe` 250).
    pub plate_label_x: f32,
    /// Infix of the figure/garment/weapon `UIO_NEWCHAR_*` keys of this race
    /// (`""` for the China tree, `"EU_"` for the Europe tree).
    pub ui_key_infix: &'static str,
    /// Whether this race's creation tree swaps the protector and weapon rows
    /// against the China tree (`_europe :276-294` / `:295-313`).
    pub gear_rows_swapped: bool,
}

/// The races whose presentation our reference corpus authors, in the order the
/// resinfo declares their plates. Adding a race here is a *data citation*, not
/// a feature: a race missing from this list is still playable when the
/// character table holds its bodies.
pub const KNOWN_RACES: [RacePresentation; 2] = [
    RacePresentation {
        code: "CH",
        plate_ddj: "media://interface/outer/china.ddj",
        loading_ddj: "media://interface/loading/loading_charactercustom.ddj",
        label_key: "UIO_NEWCHAR_CTL_CHINESE",
        label_fallback: "Chinese",
        desc_key: "UIO_NEWCHAR_CTL_CHINESE_TT",
        plate_label_x: 318.0,
        ui_key_infix: "",
        gear_rows_swapped: false,
    },
    RacePresentation {
        code: "EU",
        plate_ddj: "media://interface/outer/europe.ddj",
        loading_ddj: "media://interface/loading/loading_charactercustom_europe.ddj",
        label_key: "UIO_NEWCHAR_CTL_EUROPEAN",
        label_fallback: "European",
        desc_key: "UIO_NEWCHAR_CTL_EUROPEAN_TT",
        plate_label_x: 250.0,
        ui_key_infix: "EU_",
        gear_rows_swapped: true,
    },
];

/// The races a set of character-table code names carries bodies for.
///
/// Order: the races [`KNOWN_RACES`] declares first, in that (resinfo) order,
/// then any further race the corpus holds, sorted by code — so the screen
/// order of a known corpus is unchanged and an unknown race lands
/// deterministically at the end.
pub fn races_from_bodies<'a>(code_names: impl Iterator<Item = &'a str>) -> Vec<Race> {
    let mut found: Vec<Race> = Vec::new();
    for name in code_names {
        if let Some(race) = Race::from_body_code_name(name) {
            if !found.contains(&race) {
                found.push(race);
            }
        }
    }
    found.sort_by_key(|race| {
        let known = KNOWN_RACES.iter().position(|p| p.code == race.code());
        (known.unwrap_or(KNOWN_RACES.len()), *race)
    });
    found
}

/// The playable races of the loaded corpus. Empty when no character table is
/// loaded (headless/netcheck), which is what keeps the callers from offering a
/// race nothing can build.
pub fn available_races(char_data: &ClientCharacterData) -> Vec<Race> {
    let Some(table) = char_data.data() else {
        return Vec::new();
    };
    races_from_bodies(table.iter().map(|(_, row)| row.code_name().as_str()))
}

/// The plates the region board shows: every race the corpus can build, plus
/// the races the interface data itself declares a plate for. The second half
/// is why a corpus without European bodies still shows a *dimmed* Europe
/// plate with the original's "Out of service area." reason (#643) instead of
/// silently dropping a race the client's own interface files announce.
pub fn plate_races(char_data: &ClientCharacterData) -> Vec<Race> {
    let mut races = available_races(char_data);
    for known in KNOWN_RACES.iter() {
        let race = Race::from_ascii([known.code.as_bytes()[0], known.code.as_bytes()[1]]);
        if !races.contains(&race) {
            races.push(race);
        }
    }
    races.sort_by_key(|race| {
        let known = KNOWN_RACES.iter().position(|p| p.code == race.code());
        (known.unwrap_or(KNOWN_RACES.len()), *race)
    });
    races
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::textdata::characterdata::{CharacterData, CharacterDataRow};
    use std::collections::HashMap;

    /// Minimal characterdata row: the parser keeps any tab row with >10
    /// columns and reads the code name from column 2.
    fn row(id: i32, code_name: &str) -> (i32, CharacterDataRow) {
        let mut cols = vec![String::new(); 60];
        cols[1] = id.to_string();
        cols[2] = code_name.to_string();
        (id, CharacterDataRow(cols))
    }

    fn char_data(rows: Vec<(i32, CharacterDataRow)>) -> ClientCharacterData {
        ClientCharacterData::from_table(CharacterData(rows.into_iter().collect::<HashMap<_, _>>()))
    }

    fn codes(races: &[Race]) -> Vec<&str> {
        races.iter().map(|r| r.code()).collect()
    }

    /// The four corpus shapes, on one derivation. (a) and (b) are the two
    /// single-race corpora, (c) is a corpus with both, and (d) is the point of
    /// the whole module: a race whose code this program has never heard of must
    /// appear, because its bodies are in the table.
    #[test]
    fn the_race_list_is_whatever_the_body_rows_hold() {
        // (a) only CH
        assert_eq!(
            codes(&races_from_bodies(
                ["CHAR_CH_MAN_ADVENTURER", "CHAR_CH_WOMAN_WARRIOR"].into_iter()
            )),
            ["CH"]
        );
        // (b) only EU
        assert_eq!(
            codes(&races_from_bodies(["CHAR_EU_MAN_FIGHTER"].into_iter())),
            ["EU"]
        );
        // (c) both, in the resinfo plate order regardless of table order
        assert_eq!(
            codes(&races_from_bodies(
                ["CHAR_EU_MAN_FIGHTER", "CHAR_CH_MAN_ADVENTURER"].into_iter()
            )),
            ["CH", "EU"]
        );
        // (d) a third, unknown race: bodies exist, so it is offered
        let three = races_from_bodies(
            [
                "CHAR_CH_MAN_ADVENTURER",
                "CHAR_EU_WOMAN_FIGHTER",
                "CHAR_JP_MAN_RONIN",
                "CHAR_JP_WOMAN_RONIN",
            ]
            .into_iter(),
        );
        assert_eq!(codes(&three), ["CH", "EU", "JP"]);
        let unknown = three[2];
        assert!(
            unknown.presentation().is_none(),
            "we cannot have authored art for a race we have never seen"
        );
        // and it still names itself and finds its own body rows
        assert_eq!(unknown.label().1, "JP");
        assert_eq!(unknown.body_prefix_of("MAN"), "CHAR_JP_MAN_");
    }

    /// A race with no body rows is not offered — the rule that makes the list
    /// data-borne in both directions. Rows of the same table that merely
    /// mention a race token (monsters, NPCs, pets, and the un-gendered
    /// `CHAR_` rows) are not bodies and must not conjure a race.
    #[test]
    fn a_race_without_body_rows_is_not_offered() {
        let derived = races_from_bodies(
            [
                "CHAR_CH_MAN_ADVENTURER",
                "MOB_EU_MOVOI",
                "NPC_EU_EVENT_CARNIVAL_OBJECT_2011",
                "CHAR_EU_HORSE",
                "CHAR_XX_MAN_",
            ]
            .into_iter(),
        );
        assert_eq!(codes(&derived), ["CH"]);
        assert!(!derived.contains(&Race::EUROPEAN));

        // …and the same through the resource the screens actually hold
        let data = char_data(vec![
            row(1907, "CHAR_CH_MAN_ADVENTURER"),
            row(5851, "MOB_EU_MOVOI"),
        ]);
        assert_eq!(codes(&available_races(&data)), ["CH"]);
        // the board still shows the plate the interface data declares, dimmed
        assert_eq!(codes(&plate_races(&data)), ["CH", "EU"]);
    }

    /// An unknown race the corpus can build reaches the board too — the plate
    /// list is the race list plus the declared plates, never a fixed pair.
    #[test]
    fn an_unknown_race_with_bodies_reaches_the_board() {
        let data = char_data(vec![
            row(1907, "CHAR_CH_MAN_ADVENTURER"),
            row(30000, "CHAR_JP_WOMAN_RONIN"),
        ]);
        assert_eq!(codes(&plate_races(&data)), ["CH", "EU", "JP"]);
    }

    /// The presentation table stays a citation of the corpus, not a race list:
    /// every entry names a code the body rows could hold, and the two we can
    /// cite keep their authored values.
    #[test]
    fn the_presentation_table_keeps_its_cited_values() {
        assert_eq!(Race::CHINESE.presentation().unwrap().plate_label_x, 318.0);
        assert_eq!(Race::EUROPEAN.presentation().unwrap().plate_label_x, 250.0);
        assert_eq!(Race::CHINESE.presentation().unwrap().ui_key_infix, "");
        assert_eq!(Race::EUROPEAN.presentation().unwrap().ui_key_infix, "EU_");
        assert!(!Race::CHINESE.presentation().unwrap().gear_rows_swapped);
        assert!(Race::EUROPEAN.presentation().unwrap().gear_rows_swapped);
        for p in KNOWN_RACES.iter() {
            assert_eq!(p.code.len(), 2);
            assert_eq!(
                Race::from_body_code_name(&format!("CHAR_{}_MAN_X", p.code))
                    .unwrap()
                    .code(),
                p.code
            );
        }
    }
}

//! Creation-choice model shared by the pregame screens.
//!
//! Lives here, not in `character_create`, so the chrome (which reads the
//! current race for its per-race frames) and the creation screen (which reads
//! the chrome's info text) do not import each other. The races themselves are
//! not enumerated: `race_catalog` reads them off the character table.

use bevy::prelude::*;

use super::character_create::DEFAULT_SCALE;
pub use super::race_catalog::Race;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gender {
    #[default]
    Male,
    Female,
}

impl Gender {
    /// The segment the character table spells between race code and suffix in
    /// a body row (`CHAR_CH_MAN_ADVENTURER`).
    pub fn body_segment(self) -> &'static str {
        match self {
            Gender::Male => "MAN",
            Gender::Female => "WOMAN",
        }
    }
}

/// The three `_DEF` starter garment sets (original `GDR_SLI_PROTECTOR`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Garment {
    #[default]
    Clothes,
    Light,
    Heavy,
}

/// The current creation choice; its change drives the live preview, the
/// toggle highlights and the picker labels. `figure`/`weapon` index into the
/// corpus-enumerated option lists (clamped on resolve).
#[derive(Resource)]
pub struct CharCreateSelection {
    pub race: Race,
    pub gender: Gender,
    pub figure: usize,
    pub weapon: usize,
    /// Height step, `0..=SCALE_STEPS - 1`; the **low** nibble of the wire
    /// scale byte.
    pub height: u8,
    /// Volume step, same range; the **high** nibble.
    pub volume: u8,
    pub garment: Garment,
    /// Whether the weapon row was ever committed. The original keeps the
    /// weapon ref in `this+0x138` and it starts at **0**; the two pre-send
    /// gates refuse while it is 0, which is
    /// why a fresh screen with the visually obvious default still answers
    /// "Select a Weapon." — see [`gate_selection`].
    pub weapon_chosen: bool,
    /// The same for the protector/chest ref `this+0x12c`, which only the
    /// European gate reads.
    pub garment_chosen: bool,
}

/// Hand-written because the two scale axes do **not** default to 0: an
/// untouched original screen sends `0x22`, i.e. both nibbles on the middle
/// step.
impl Default for CharCreateSelection {
    fn default() -> Self {
        Self {
            race: Race::default(),
            gender: Gender::default(),
            figure: 0,
            weapon: 0,
            // split out of the default byte itself, so the two
            // fields cannot drift away from what an untouched screen sends
            height: DEFAULT_SCALE & 0x0f,
            volume: DEFAULT_SCALE >> 4,
            garment: Garment::default(),
            weapon_chosen: false,
            garment_chosen: false,
        }
    }
}

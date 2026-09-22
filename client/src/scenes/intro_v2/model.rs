//! Creation-choice model shared by the pregame screens.
//!
//! Lives here, not in `character_create`, so the chrome (which reads the
//! current race for its per-race frames) and the creation screen (which reads
//! the chrome's info text) do not import each other.

use bevy::prelude::*;

/// The two v1.188 playable races (Islam/Arabia is not a v1.188 body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Race {
    #[default]
    Chinese,
    European,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Gender {
    #[default]
    Male,
    Female,
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
#[derive(Resource, Default)]
pub struct CharCreateSelection {
    pub race: Race,
    pub gender: Gender,
    pub figure: usize,
    pub weapon: usize,
    pub garment: Garment,
}

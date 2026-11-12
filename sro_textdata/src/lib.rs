//! Row types of the game's reference tables (the `textdata` folder), shared by
//! the client and the server so that each table's columns are named in one
//! place. A table moves here when a second user needs it (decision 0017 of the
//! studio; the client reached them first). The Bevy asset derive is behind the
//! `bevy` feature, which only the client turns on.

pub mod characterdata;

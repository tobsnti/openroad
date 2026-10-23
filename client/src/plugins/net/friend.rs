//! Client-side friend roster: the join-time 0x3305 push, kept.
//!
//! Idea: `packets::agent::ingame::FriendListInfo` has parsed 0x3305 since the
//! opcode was wired, but nothing in `client/` ever read the message, so the
//! roster the server pushes shortly after world-enter was decoded and dropped.
//! This module is the single place that keeps it, exactly like
//! [`super::party`] keeps 0x3065 — the community window's friend page then
//! only renders what is here.
//!
//! **What this deliberately does not do.** 0x3305 is the only friend opcode
//! this tree models at all: add, delete and the presence push are not in the
//! `packets!` table (`rg "7302|7304|3303|3B07" packets/src/lib.rs` -> empty).
//! So the roster has no delta path and no outbound verb: a friend added or
//! removed in this session is only reflected after the next 0x3305. Folding in
//! a guessed presence opcode would be inventing wire semantics; saying it
//! plainly is the cheaper honesty.
//!
//! Note on the capture: the only one this tree documents is an **empty**
//! roster, a single `00` count byte (`packets/src/agent/ingame.rs:896`). The
//! count-prefixed shell is therefore capture-VERIFIED and the *entry* layout is
//! not; see [`FriendEntry`], whose four fields are read off the original's own
//! parser. Nothing here adds a fifth reading of its own — the fifth field a
//! third-party record suggested is exactly what #546 removed from the wire
//! type.

use bevy::prelude::*;

use packets::agent::ingame::{FriendEntry, FriendListInfo};

/// The friend roster as the server last pushed it (0x3305). Empty until the
/// push arrives, and empty is also the legitimate steady state — the only
/// capture this tree owns is exactly that.
#[derive(Resource, Default, Debug)]
pub struct FriendRoster {
    pub friends: Vec<FriendEntry>,
}

impl FriendRoster {
    /// 0x3305 is a full list, not a delta: it replaces whatever we had.
    pub fn apply_list(&mut self, list: &FriendListInfo) {
        self.friends = list.friends.clone();
    }

    /// How many entries the roster holds; the page's count readout shows this.
    pub fn len(&self) -> usize {
        self.friends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.friends.is_empty()
    }
}

pub fn on_friend_list_info(
    mut reader: MessageReader<FriendListInfo>,
    mut roster: ResMut<FriendRoster>,
) {
    for list in reader.read() {
        // `count` is the wire's own header and the decoder already used it to
        // size the vector, so a mismatch cannot happen here. Logged because
        // this is the first client-side consumer of the opcode, and a
        // *non-empty* capture is the one thing the entry layout still lacks.
        info!(
            "friend: 0x3305 roster push, {} entr{}",
            list.friends.len(),
            if list.friends.len() == 1 { "y" } else { "ies" }
        );
        roster.apply_list(list);
    }
}

/// Owns [`FriendRoster`]; part of the networking core so the roster exists
/// wherever 0x3305 can arrive — including the headless netcheck harness, which
/// has no HUD at all.
pub struct FriendPlugin;

impl Plugin for FriendPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FriendRoster>()
            .add_systems(Update, on_friend_list_info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// The one shape this tree actually owns: an empty roster, the single byte
    /// `00`. It must decode and it must leave the resource empty — including
    /// when a previous push had left something in it.
    #[test]
    fn the_captured_empty_roster_leaves_the_list_empty() {
        let body = Bytes::from_static(&[0x00]);
        let list: FriendListInfo = body.try_into().unwrap();
        let mut roster = FriendRoster {
            friends: vec![entry(1, "Stale")],
        };
        roster.apply_list(&list);
        assert!(roster.is_empty());
    }

    /// A push replaces, it does not merge — 0x3305 is the full list. This is
    /// the opposite of the party roster next door, which *must* merge because
    /// its records are partial; getting the two the wrong way round would
    /// blank a friend list on every push or keep a deleted friend forever.
    #[test]
    fn a_second_push_replaces_the_roster() {
        let mut roster = FriendRoster::default();
        roster.apply_list(&FriendListInfo {
            count: 2,
            friends: vec![entry(1, "Alpha"), entry(2, "Beta")],
        });
        assert_eq!(roster.len(), 2);
        roster.apply_list(&FriendListInfo {
            count: 1,
            friends: vec![entry(3, "Gamma")],
        });
        assert_eq!(names(&roster), vec!["Gamma".to_string()]);
    }

    /// Wire order is list order: the page renders row `n` from entry `n`, so a
    /// silent reorder here would be a silent reorder on screen.
    #[test]
    fn entries_keep_their_wire_order() {
        let mut roster = FriendRoster::default();
        roster.apply_list(&FriendListInfo {
            count: 3,
            friends: vec![entry(7, "Cho"), entry(8, "Ann"), entry(9, "Bex")],
        });
        assert_eq!(
            names(&roster),
            vec!["Cho".to_string(), "Ann".to_string(), "Bex".to_string()]
        );
    }

    fn names(roster: &FriendRoster) -> Vec<String> {
        roster.friends.iter().map(|f| f.name.clone()).collect()
    }

    fn entry(char_id: u32, name: &str) -> FriendEntry {
        FriendEntry {
            char_id,
            name: name.to_string(),
            char_model: 1907,
            is_online: 1,
        }
    }
}

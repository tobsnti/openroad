//! Idea: an offline preview of the **guild** surfaces of the Community window
//! (`hud/community/guild.rs`) — the guild page with its roster, notice strip and
//! command column, and the guild-relations page's alliance sub-pane. Run with
//! `SCENE=ui_testing`.
//!
//! Like `party_ui.rs` this registers the window's **own** public spawn system
//! for `SceneState::UiTesting` instead of widening the production
//! `OnEnter(GameWorld)` registration, so what gets photographed is the in-game
//! window rather than a copy of it; the page's `Update` systems already run in
//! `UiTesting` (`hud/community/mod.rs`).
//!
//! **Where the record comes from — and why it is not invented.** The whole
//! point of this module is that the guild family finally has a *measured*
//! record: `docs/re/systems/guild-live-capture-2026-08-24.md` §4/§11a, guild
//! "OpenRoad" (id 1, level 5) on the user's own server, one member `Devi`
//! (`member_id` 5, level 68, `permissions` = the master sentinel
//! `0xFFFFFFFF`, `model_id` 1931), notice title "OpenRoad" and the message the
//! **server's own filter rewrote** on the way in ("Founded 2026 08 24 by
//! netcheck probe" — every `-` became a space, §7a). So the values below are
//! the bytes that arrived, dashes included as they came back, not
//! plausible-looking placeholder copy.
//!
//! The alliance pane is the opposite case and is treated the opposite way: no
//! 0x3102 has ever been captured, because that guild is in no alliance. The
//! preview therefore shows the pane in its **true empty state** ("Not in the
//! alliance") by default, and only seeds a list behind
//! `OPENROAD_PREVIEW_GUILD_ALLIANCE=1` — with deliberately obvious placeholder
//! names (`Alliance Guild 1..`), because a preview must not be mistakable for
//! decoded data (ADR-0009). What that seeded picture *is* good for is the one
//! thing a test cannot judge: whether eight rows and the header line fit the
//! authored geometry.
//!
//! Ported from the fork with two field edits the chain's own wire work forced:
//! `GuildMember` no longer carries `unk_u8_02`, and `GuildData` now closes with
//! `election_count`/`elections` (seeded empty — this capture has no vote block).
//!
//! `OPENROAD_PREVIEW_GUILD_PAGE=relations` photographs the relations page
//! instead of the guild page; the two share the window's page rect, so one run
//! can only show one of them.

use bevy::prelude::*;

use packets::agent::guild::{GuildData, GuildMember, GuildPermissions};
use packets::agent::guild_union::{UnionGuild, UnionRoster};

use crate::plugins::hud::community::model::{CommunityPage, CommunityState};
use crate::plugins::hud::community::ui;
use crate::plugins::net::guild::{GuildRoster, GuildUnion};
use crate::scenes::SceneState;

pub struct GuildUiPreviewPlugin;

impl Plugin for GuildUiPreviewPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            OnEnter(SceneState::UiTesting),
            (
                seed_guild_record,
                seed_alliance,
                open_guild_page,
                ui::spawn_community_window,
            )
                .chain(),
        );
    }
}

/// The measured record of guild "OpenRoad", field for field as it arrived.
fn seed_guild_record(mut roster: ResMut<GuildRoster>) {
    roster.data = Some(GuildData {
        guild_id: 1,
        name: "OpenRoad".to_string(),
        // A freshly founded guild starts at 5, which is itself one of the
        // capture's findings — a preview seeded with 1 would have hidden it.
        level: 5,
        guild_points: 0,
        notice: "OpenRoad".to_string(),
        message: "Founded 2026 08 24 by netcheck probe".to_string(),
        unk_u32_00: 0,
        unk_u8_00: 0,
        member_count: 1,
        members: vec![GuildMember {
            member_id: 5,
            name: "Devi".to_string(),
            unk_u8_01: 0,
            level: 68,
            guild_points: 0,
            permissions: GuildPermissions::MASTER,
            unk_u32_01: 0,
            unk_u32_02: 0,
            unk_u32_03: 0,
            nickname: String::new(),
            model_id: 1931,
            is_master: true,
            is_offline: false,
        }],
        // The chain's `GuildData` closes with the vote block this capture has
        // none of (`election_count = 0`), so the record ends where the capture
        // ended rather than seeding an election nobody measured.
        election_count: 0,
        elections: Vec::new(),
    });
}

/// Seed an alliance **only** on request, and only with obvious placeholders —
/// no 0x3102 has been captured, so anything here is our own construction.
fn seed_alliance(mut union: ResMut<GuildUnion>) {
    if std::env::var("OPENROAD_PREVIEW_GUILD_ALLIANCE").is_err() {
        return;
    }
    let guilds: Vec<UnionGuild> = (1..=4)
        .map(|n| UnionGuild {
            guild_id: n,
            guild_name: format!("Alliance Guild {n}"),
            unknown_a: 0,
            master_name: format!("Master {n}"),
            master_object_id: 0,
            unknown_b: 0,
        })
        .collect();
    union.data = Some(UnionRoster {
        union_id: 1,
        union_crest_rev: 0,
        // The second entry leads, so the header line is visibly keyed rather
        // than showing whatever is first.
        leader_guild_id: 2,
        count: guilds.len() as u8,
        guilds,
    });
}

fn open_guild_page(mut state: ResMut<CommunityState>) {
    state.open = true;
    state.page = match std::env::var("OPENROAD_PREVIEW_GUILD_PAGE").as_deref() {
        Ok("relations") => CommunityPage::GuildRelations,
        _ => CommunityPage::Guild,
    };
}

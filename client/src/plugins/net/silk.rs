//! Silk balance (`0x3153 SilkUpdate`).
//!
//! Idea: the server pushes the account's three silk counters and nothing in
//! this client read them — 239 captured frames in 18 session logs, decoded and
//! dropped. The original shows the triple in the Item Mall's "My information"
//! pane, three statics with empty `Text` filled at runtime
//! (`resinfo/ifitemmallmyinfo.txt:158` silk, `:139` gift, `:120` point), but
//! that window has no purchase opcode we know of ([U]), so this keeps the value
//! in a resource and says it in the chat log: the numbers are the original's,
//! the surface is ours.
//!
//! The labels are the pane's own keys against the player's `textuisystem.txt`,
//! not our words: `UIIT_STT_SILKMALL_SILK` (L1166, "Silk"),
//! `UIIT_CTL_SILK_INQUIRY_GIFT_SILK` (L3488, "Bonus Silk"),
//! `UIIT_CTL_SILK_INQUIRY_POINT` (L3490, "Point"). Only a *changed* triple is
//! reported: the 239 frames carry 56 changes, so a line per frame would be 183
//! lines of nothing.

use bevy::prelude::*;
use packets::agent::ingame::SilkUpdate;

use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::textdata::ClientUiStrings;

/// The account's silk counters as last pushed, `None` before the first
/// `0x3153`. `None` rather than zeroes because zero is a real balance (40 of
/// the 239 captured frames carry it), and a later window has to tell "nothing
/// pushed yet" from "nothing left".
#[derive(Resource, Default, Debug, PartialEq)]
pub struct SilkBalance(pub Option<SilkCounters>);

/// The triple as the wire carries it (`packets::agent::ingame::SilkUpdate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SilkCounters {
    pub own: u32,
    pub gift: u32,
    pub point: u32,
}

const SILK_KEY: &str = "UIIT_STT_SILKMALL_SILK";
const GIFT_KEY: &str = "UIIT_CTL_SILK_INQUIRY_GIFT_SILK";
const POINT_KEY: &str = "UIIT_CTL_SILK_INQUIRY_POINT";

/// The labels in the player's wording; the fallbacks are the English values
/// their own table carries, for scenes that load no textdata at all.
fn balance_line(counters: SilkCounters, strings: Option<&ClientUiStrings>) -> String {
    let label = |key: &str, fallback: &'static str| match strings {
        Some(strings) => strings.get_or(key, fallback),
        None => fallback,
    };
    format!(
        "{}: {}, {}: {}, {}: {}",
        label(SILK_KEY, "Silk"),
        counters.own,
        label(GIFT_KEY, "Bonus Silk"),
        counters.gift,
        label(POINT_KEY, "Point"),
        counters.point,
    )
}

/// `0x3153` — keep the counters, and report them when they changed.
///
/// `ChatHistory` and `ClientUiStrings` are **HUD** resources and this is the
/// **net** layer: the headless netcheck harness builds networking with no HUD,
/// and Bevy does not skip a system whose `ResMut` is missing — it fails
/// parameter validation and panics the schedule. Both are therefore optional by
/// construction, the same reason `party::report_ack` gives.
pub fn on_silk_update(
    mut reader: MessageReader<SilkUpdate>,
    mut balance: ResMut<SilkBalance>,
    mut history: Option<ResMut<ChatHistory>>,
    strings: Option<Res<ClientUiStrings>>,
) {
    for msg in reader.read() {
        let counters = SilkCounters {
            own: msg.own,
            gift: msg.gift,
            point: msg.point,
        };
        if balance.0 == Some(counters) {
            continue;
        }
        balance.0 = Some(counters);
        let text = balance_line(counters, strings.as_deref());
        match &mut history {
            Some(history) => history.push(ChatLine::system(text)),
            None => info!("silk (headless): {text}"),
        }
    }
}

/// Owns [`SilkBalance`]; part of the networking core so the balance exists
/// wherever `0x3153` can arrive — it lands with the login push, before a HUD.
pub struct SilkPlugin;

impl Plugin for SilkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SilkBalance>()
            .add_systems(Update, on_silk_update);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three rows verbatim from the player's `textuisystem.txt`
    /// (L1166 / L3488 / L3490), through the production parser so the format is
    /// pinned too, not just the pairs.
    const REAL_ROWS: &str = "1\tUIIT_STT_SILKMALL_SILK\t\t\t\t\t\t\t\tSilk\n\
         1\tUIIT_CTL_SILK_INQUIRY_GIFT_SILK\t\t\t\t\t\t\t\tBonus Silk\n\
         1\tUIIT_CTL_SILK_INQUIRY_POINT\t\t\t\t\t\t\t\tPoint\n";

    /// `packet_dump/0x3153.log` body `f4cb9a3b50c3000000000000`, 16 of the 239
    /// captured frames.
    const CAPTURED: SilkUpdate = SilkUpdate {
        own: 1_000_000_500,
        gift: 50_000,
        point: 0,
    };

    #[test]
    fn a_changed_balance_is_kept_and_said_in_the_players_own_words() {
        let mut app = App::new();
        app.init_resource::<ChatHistory>()
            .insert_resource(ClientUiStrings::from_tsv(REAL_ROWS))
            .add_message::<SilkUpdate>()
            .add_plugins(SilkPlugin);

        app.world_mut().write_message(CAPTURED);
        app.update();
        assert_eq!(
            app.world().resource::<SilkBalance>().0,
            Some(SilkCounters {
                own: 1_000_000_500,
                gift: 50_000,
                point: 0,
            })
        );
        let line = app
            .world()
            .resource::<ChatHistory>()
            .iter()
            .next()
            .expect("one line")
            .display();
        assert_eq!(line, "Silk: 1000000500, Bonus Silk: 50000, Point: 0");

        // 183 of the 239 captured frames repeat their predecessor, and a repeat
        // is not news
        app.world_mut().write_message(CAPTURED);
        app.update();
        assert_eq!(app.world().resource::<ChatHistory>().iter().count(), 1);

        // a different triple is
        app.world_mut().write_message(SilkUpdate {
            own: 1000,
            gift: 80,
            point: 50,
        });
        app.update();
        assert_eq!(app.world().resource::<ChatHistory>().iter().count(), 2);
    }

    #[test]
    fn the_silk_system_runs_without_a_hud() {
        let mut app = App::new();
        app.add_message::<SilkUpdate>().add_plugins(SilkPlugin);
        assert!(
            !app.world().contains_resource::<ChatHistory>(),
            "this test is only meaningful without the HUD resource"
        );
        app.world_mut().write_message(CAPTURED);
        app.update();
        assert!(app.world().resource::<SilkBalance>().0.is_some());
    }
}

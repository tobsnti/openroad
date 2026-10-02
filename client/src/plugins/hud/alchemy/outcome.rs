//! What comes back from a fuse: the `0xB150`/`0xB151` acks, the outcome the
//! player reads, and the item the server sends back.
//!
//! Idea: this is the client half of the original's *result presenter* — the
//! routine that decides which of the six outcome lines you see after a fuse. Two
//! things about it shape this module:
//!
//! 1. **The ack carries no deltas.** It says *what* happened (success /
//!    breakdown / failure, off two discriminator bytes) and re-sends the mutated
//!    item. Enhancement level and durability *changes* exist only as a diff
//!    against the item that was in the slot, which is why the handler below
//!    reads the old item out of [`Inventory`] **before** it stores the new one.
//! 2. **Every line here is the archive's own**, keyed exactly as the archive
//!    keys it. Where the original formats an error table we do not have, this
//!    module logs the raw code instead of inventing a sentence — the one
//!    deliberate gap, and it is marked at the call site.
//!
//! **What has and has not been seen on the wire:** only the *refusal* arm of
//! `0xB150` has ever arrived. The success, breakdown and cancel arms are read
//! off the client that answers them, so this presenter is built from the layout
//! rather than from traffic, and the diff-against-the-old-item rule is what
//! keeps it from needing fields the ack may not carry.
//!
//! The visual half of the presenter (the window animation and its sounds) is not
//! built here: that is an animation on the box's own art, and a sound path this
//! module has no business opening. The chat line first.

use bevy::prelude::*;

use packets::agent::alchemy::{
    AlchemyOutcome, AlchemyReinforceResponse, AlchemyStoneResponse, ALCHEMY_ACTION_CANCEL_ACK,
    ALCHEMY_ERROR_STONE_FAILED,
};
use packets::agent::character_data::{InventoryItem, ItemTypeData};

use crate::plugins::hud::alchemy::model::AlchemyState;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

/// `textuisystem.txt:2151`. The single `%d` is filled with the value the string
/// itself names — the endurability of the item that came back. The alternative
/// reading, that the original passes the new enhancement level and the English
/// wording is a mistranslation, has no source we can check, so the string's own
/// word decides it.
const MSG_SUCCESS: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_SUCCESS",
    "Success! Endurability has been changed to [%d].",
);
/// `:2152` — the plain failure.
const MSG_FAIL: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAIL",
    "The alchemy enhancement has failed.",
);
/// `:2153` — a failure that took the last enhancement level.
const MSG_FAIL_OPTLV_ZERO: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAIL_RESULT_OPTLV_ZERO",
    "The enhancement level on the equipment is gone, because the alchemy enhancement failed.",
);
/// `:2154` — `%d` = the new level, `%d` = how far it dropped.
const MSG_FAIL_OPTLV_DOWN: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAIL_RESULT_OPTLV_DOWN",
    "The enhanced level on the equipment has changed to [%d]because the alchemy enhancement failed.(enhanced level %d decreased)",
);
/// `:2155` — `%d` = the new durability, `%d` = how much was lost.
const MSG_FAIL_DURABILITY: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAILDOWN_DURABILITY",
    "The endurability has been changed to [%d]because the alchemy enhancement failed. (Endurability %d reduced)",
);
/// `:2156` — the item is gone.
const MSG_BREAKDOWN: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_BREAKDOWN",
    "The item has been destroyed because the alchemy enhancement failed.",
);
/// `:783` — the cancel ack's own line.
const MSG_CANCELLED: (&str, &str) = (
    "UIIT_MSG_ALCHEMY_CANCELED_COMPOUND",
    "Fusing has been cancelled.",
);

/// The enhancement level and durability of an equipment item, or `None` for
/// anything that is not equipment (a stone cannot be reinforced, and the diff
/// below is meaningless for a stack).
fn equipment_state(item: &InventoryItem) -> Option<(u8, u32)> {
    match &item.data {
        ItemTypeData::Equipment(equipment) => Some((equipment.opt_level, equipment.durability)),
        _ => None,
    }
}

/// Fill the `%d` holes of a shipped template left to right. The archive's
/// strings carry positional `%d` only, so this is a plain left-to-right
/// substitution and deliberately not a new formatting abstraction.
fn fill(template: &str, values: &[i64]) -> String {
    let mut out = template.to_string();
    for value in values {
        out = out.replacen("%d", &value.to_string(), 1);
    }
    out
}

/// Which of the six lines the player reads, from the wire outcome plus the diff
/// of the old and new item. A failure is refined by *what actually changed*,
/// because the ack states no deltas of its own.
fn outcome_line(
    ui_strings: &ClientUiStrings,
    outcome: AlchemyOutcome,
    before: Option<(u8, u32)>,
    after: Option<(u8, u32)>,
) -> String {
    let line = |(key, fallback): (&str, &str)| ui_strings.get_or(key, fallback).to_string();
    match outcome {
        AlchemyOutcome::Success => match after {
            Some((_, durability)) => fill(&line(MSG_SUCCESS), &[durability as i64]),
            // the record did not parse: say the outcome, not a made-up number
            None => line(MSG_SUCCESS).replace("%d", "?"),
        },
        AlchemyOutcome::Breakdown => line(MSG_BREAKDOWN),
        AlchemyOutcome::Failure => {
            let (Some((old_level, old_durability)), Some((level, durability))) = (before, after)
            else {
                return line(MSG_FAIL);
            };
            if level != old_level {
                if level == 0 {
                    line(MSG_FAIL_OPTLV_ZERO)
                } else {
                    fill(
                        &line(MSG_FAIL_OPTLV_DOWN),
                        &[level as i64, old_level as i64 - level as i64],
                    )
                }
            } else if durability != old_durability {
                fill(
                    &line(MSG_FAIL_DURABILITY),
                    &[durability as i64, old_durability as i64 - durability as i64],
                )
            } else {
                line(MSG_FAIL)
            }
        }
    }
}

/// Apply one decoded fuse ack: say what happened, store the item the server
/// sent, and release the page slots.
///
/// The slots are released **only** when the ack reports a fuse or a cancel — a
/// refusal leaves the page exactly as the player built it, so the fuse can be
/// pressed again without rebuilding it.
fn apply_ack(
    response: &AlchemyReinforceResponse,
    opcode: &str,
    state: &mut AlchemyState,
    inventory: &mut Inventory,
    item_data: &ClientItemData,
    ui_strings: &ClientUiStrings,
    history: &mut ChatHistory,
) {
    state.pending = false;
    if !response.is_success() {
        let code = response.error_code.unwrap_or_default();
        // The original resolves this through an error-string table that is not
        // in our data. One value has a known meaning; the rest stay numbers on
        // purpose, because a plausible sentence here would be a lie.
        if code == ALCHEMY_ERROR_STONE_FAILED {
            history.push(ChatLine::system(
                ui_strings.get_or(MSG_FAIL.0, MSG_FAIL.1).to_string(),
            ));
        } else {
            warn!("alchemy: fuse refused ({opcode} error {code:#06x})");
            history.push(ChatLine::system(format!(
                "{} (code {code:#06x})",
                ui_strings.get_or(MSG_FAIL.0, MSG_FAIL.1)
            )));
        }
        return;
    }
    let Some((outcome, slot)) = response.outcome else {
        if response.action == Some(ALCHEMY_ACTION_CANCEL_ACK) {
            history.push(ChatLine::system(
                ui_strings
                    .get_or(MSG_CANCELLED.0, MSG_CANCELLED.1)
                    .to_string(),
            ));
            state.clear();
        }
        return;
    };
    // the old item must be read before the new one replaces it
    let before = inventory.get(slot).and_then(equipment_state);
    let item = response.item(item_data);
    let after = item.as_ref().and_then(equipment_state);
    history.push(ChatLine::system(outcome_line(
        ui_strings, outcome, before, after,
    )));
    match outcome {
        AlchemyOutcome::Breakdown => {
            inventory.take_slot(slot);
        }
        _ => {
            if let Some(item) = item {
                // the server's copy is authoritative, which is the reason
                // nothing is predicted before the ack arrives
                inventory.gain_item(item);
            } else {
                warn!(
                    "alchemy: {opcode} item record did not parse, inventory slot {slot} is stale"
                );
            }
        }
    }
    // The materials are consumed either way, so the page is cleared and the
    // player rebuilds it. What the server removed from the bag arrives on the
    // inventory opcodes, not here.
    state.clear();
}

/// `0xB150` (Equip Enhance) and `0xB151` (Att.Grant) in one system: the bodies
/// differ by one byte and the presentation not at all, so splitting them would
/// duplicate the table above.
pub fn apply_fuse_response(
    mut reinforce: MessageReader<AlchemyReinforceResponse>,
    mut stone: MessageReader<AlchemyStoneResponse>,
    mut state: ResMut<AlchemyState>,
    mut inventories: Query<&mut Inventory, With<Player>>,
    item_data: Res<ClientItemData>,
    ui_strings: Res<ClientUiStrings>,
    mut history: ResMut<ChatHistory>,
) {
    // The bag is a component on the player entity, not a resource; before the
    // world scene has one there is nothing to fuse into either.
    let Ok(mut inventory) = inventories.single_mut() else {
        return;
    };
    for response in reinforce.read() {
        apply_ack(
            response,
            "0xB150",
            &mut state,
            &mut inventory,
            &item_data,
            &ui_strings,
            &mut history,
        );
    }
    for response in stone.read() {
        apply_ack(
            &response.0,
            "0xB151",
            &mut state,
            &mut inventory,
            &item_data,
            &ui_strings,
            &mut history,
        );
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn strings() -> ClientUiStrings {
        ClientUiStrings::default()
    }

    fn equipment(opt_level: u8, durability: u32) -> Option<(u8, u32)> {
        Some((opt_level, durability))
    }

    /// Success reports the value the shipped string names, filled from the item
    /// that came back.
    #[test]
    fn success_reports_the_value_the_string_names() {
        let line = outcome_line(
            &strings(),
            AlchemyOutcome::Success,
            equipment(3, 40),
            equipment(4, 55),
        );
        assert_eq!(line, "Success! Endurability has been changed to [55].");
        assert!(!line.contains("%d"), "every hole is filled");

        // no parsable record: the outcome still reads, the number does not lie
        let line = outcome_line(&strings(), AlchemyOutcome::Success, equipment(3, 40), None);
        assert!(line.starts_with("Success!") && line.contains('?'));
    }

    /// Which failure line shows is decided by the item diff, because the ack
    /// states no deltas. All three branches, plus the "nothing changed" floor.
    #[test]
    fn failure_picks_its_line_from_the_item_diff() {
        let s = strings();
        // the level dropped to zero
        let line = outcome_line(
            &s,
            AlchemyOutcome::Failure,
            equipment(1, 40),
            equipment(0, 40),
        );
        assert_eq!(
            line,
            "The enhancement level on the equipment is gone, because the alchemy enhancement failed."
        );
        // the level dropped but not to zero: new level and the drop
        let line = outcome_line(
            &s,
            AlchemyOutcome::Failure,
            equipment(5, 40),
            equipment(3, 40),
        );
        assert!(
            line.contains("[3]") && line.contains("level 2 decreased"),
            "{line}"
        );
        assert!(!line.contains("%d"));
        // only durability moved
        let line = outcome_line(
            &s,
            AlchemyOutcome::Failure,
            equipment(5, 40),
            equipment(5, 31),
        );
        assert!(
            line.contains("[31]") && line.contains("9 reduced"),
            "{line}"
        );
        assert!(!line.contains("%d"));
        // nothing measurable changed, or the item did not parse
        for (before, after) in [
            (equipment(5, 40), equipment(5, 40)),
            (equipment(5, 40), None),
            (None, equipment(5, 40)),
        ] {
            assert_eq!(
                outcome_line(&s, AlchemyOutcome::Failure, before, after),
                "The alchemy enhancement has failed."
            );
        }
    }

    /// A breakdown needs no item at all — the record is absent by layout, so the
    /// line must not depend on it.
    #[test]
    fn breakdown_needs_no_item() {
        let s = strings();
        let expected = "The item has been destroyed because the alchemy enhancement failed.";
        assert_eq!(
            outcome_line(&s, AlchemyOutcome::Breakdown, equipment(5, 40), None),
            expected
        );
        assert_eq!(
            outcome_line(&s, AlchemyOutcome::Breakdown, None, None),
            expected
        );
    }

    /// Every line is a distinct shipped key, so no two outcomes read the same.
    #[test]
    fn the_six_outcome_lines_are_six_distinct_shipped_keys() {
        let keys = [
            MSG_SUCCESS,
            MSG_FAIL,
            MSG_FAIL_OPTLV_ZERO,
            MSG_FAIL_OPTLV_DOWN,
            MSG_FAIL_DURABILITY,
            MSG_BREAKDOWN,
            MSG_CANCELLED,
        ];
        let unique: std::collections::BTreeSet<_> = keys.iter().map(|(key, _)| key).collect();
        assert_eq!(unique.len(), keys.len(), "two outcomes share a key");
        for (key, fallback) in keys {
            assert!(
                key.starts_with("UIIT_MSG_"),
                "{key} is not a shipped message key"
            );
            // with the table absent the fallback is the shipped English
            assert_eq!(strings().get_or(key, fallback), fallback);
        }
    }

    /// The `%d` holes are filled left to right and counted: a template with two
    /// holes must receive two values, or a `%d` reaches the player.
    #[test]
    fn every_template_hole_gets_a_value() {
        for (key, template) in [MSG_SUCCESS, MSG_FAIL_OPTLV_DOWN, MSG_FAIL_DURABILITY] {
            let holes = template.matches("%d").count();
            let values: Vec<i64> = (1..=holes as i64).collect();
            let filled = fill(template, &values);
            assert!(!filled.contains("%d"), "{key} still has a hole: {filled}");
        }
        // and the ones without holes are left alone
        for (_, template) in [MSG_FAIL, MSG_BREAKDOWN, MSG_CANCELLED, MSG_FAIL_OPTLV_ZERO] {
            assert_eq!(template.matches("%d").count(), 0);
        }
    }

    /// The presenter is a HUD system: it writes chat lines and reads the box's
    /// page state, so it belongs behind the world-scene gate like the rest of the
    /// module.
    ///
    /// The lines are counted **after** the comment lines are dropped. A plain
    /// `contains` is satisfied by a commented-out registration, which is exactly
    /// the shape a careless edit leaves behind.
    #[test]
    fn the_fuse_ack_system_is_gated_on_the_world_scene() {
        let live = |needle: &str| {
            include_str!("mod.rs")
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with("//"))
                .filter(|line| line.contains(needle))
                .count()
        };
        assert_eq!(
            live("outcome::apply_fuse_response"),
            1,
            "the presenter is not registered on a live line"
        );
        assert_eq!(
            live("super::hud_scenes"),
            1,
            "the module's Update systems are not scene-gated"
        );
    }
}

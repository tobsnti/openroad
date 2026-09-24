//! What comes back from a fuse: the `0xB150`/`0xB151` acks, the outcome
//! message the player reads, and the item the server sends back.
//!
//! Idea: this is the client half of the original's *result presenter*
//! `(slot, flags, oldDurability, oldOptLevel)`, the function that decides
//! which of the six outcome strings you see after a fuse. Two things about it
//! shape this module:
//!
//! 1. **The ack carries no deltas.** It says *what* happened (success /
//!    breakdown / failure, off the `A` and `C` discriminators) and re-sends the
//!    mutated item. Enhancement level and durability *changes* exist only as a
//!    diff against the item that was in the slot, which is why the handler
//!    below reads the old item out of [`Inventory`] **before** it stores the
//!    new one.
//! 2. **Every string here is the shipped one**, keyed exactly as the original
//!    keys it. Where the original formats an error table that is not in the
//!    shipped data, this module logs the raw code instead of inventing a
//!    sentence — the one deliberate gap, and it is marked at the call site.
//!
//! The visual half of that presenter (`alcm_effect_success.ddj`,
//! `snd_elixir_success`/`snd_elixir_failure`) is not built here: the effect is
//! a window animation on the box's own art, whose placement is unknown, and a
//! sound path this module has no business opening. The chat line comes first
//! because it is the part that is settled.

use bevy::prelude::*;

use packets::agent::alchemy::{
    AlchemyOutcome, AlchemyReinforceResponse, AlchemyStoneResponse, ALCHEMY_ACTION_CANCEL,
    ALCHEMY_ERROR_STONE_FAILED,
};
use packets::agent::character_data::{InventoryItem, ItemTypeData};

use crate::plugins::hud::alchemy::model::AlchemyState;
use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};
use crate::plugins::net::inventory::Inventory;
use crate::plugins::player::Player;
use crate::plugins::textdata::{ClientItemData, ClientUiStrings};

/// `textuisystem.txt:2151` — `Success! Endurability has been changed to [%d].`
/// The `%d` is the **new enhancement level**, not durability: the presenter
/// formats it from the enhancement-level getter. The English wording is a
/// mistranslation in the original data; the string and its argument are kept
/// as they are.
const MSG_SUCCESS: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_SUCCESS",
    "Success! Endurability has been changed to [%d].",
);
/// `:2152` — the plain failure, presenter flag `0x20`.
const MSG_FAIL: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAIL",
    "The alchemy enhancement has failed.",
);
/// `:2153` — flag `0x20|1` with a new `+N` of zero.
const MSG_FAIL_OPTLV_ZERO: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAIL_RESULT_OPTLV_ZERO",
    "The enhancement level on the equipment is gone, because the alchemy enhancement failed.",
);
/// `:2154` — flag `0x20|1`, `%d` = new level, `%d` = how far it dropped.
const MSG_FAIL_OPTLV_DOWN: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAIL_RESULT_OPTLV_DOWN",
    "The enhanced level on the equipment has changed to [%d]because the alchemy enhancement failed.(enhanced level %d decreased)",
);
/// `:2155` — flag `0x20|2`, `%d` = new durability, `%d` = how much was lost.
const MSG_FAIL_DURABILITY: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_FAILDOWN_DURABILITY",
    "The endurability has been changed to [%d]because the alchemy enhancement failed. (Endurability %d reduced)",
);
/// `:2156` — flag `0x40`, the item is gone.
const MSG_BREAKDOWN: (&str, &str) = (
    "UIIT_MSG_REINFORCERR_BREAKDOWN",
    "The item has been destroyed because the alchemy enhancement failed.",
);
/// `:783` — the cancel ack's own line, the one string the handler
/// the original names outright.
const MSG_CANCELLED: (&str, &str) = (
    "UIIT_MSG_ALCHEMY_CANCELED_COMPOUND",
    "Fusing has been cancelled.",
);

/// The enhancement level and durability of an equipment item, or `None` for
/// anything that is not equipment (a stone cannot be reinforced, and the
/// diff below is meaningless for a stack).
fn equipment_state(item: &InventoryItem) -> Option<(u8, u32)> {
    match &item.data {
        ItemTypeData::Equipment(equipment) => Some((equipment.opt_level, equipment.durability)),
        _ => None,
    }
}

/// Fill the `%d` holes of a shipped template left to right. The archive's
/// strings carry positional `%d` only (no `%1$d`), so this is the same
/// left-to-right substitution `net/job.rs:136` already uses — deliberately not
/// a new formatting abstraction.
fn fill(template: &str, values: &[i64]) -> String {
    let mut out = template.to_string();
    for value in values {
        out = out.replacen("%d", &value.to_string(), 1);
    }
    out
}

/// The presenter's own decision table (`0059eb20`), with the flags replaced by
/// what produced them: the wire outcome plus the diff of old and new item.
fn outcome_line(
    ui_strings: &ClientUiStrings,
    outcome: AlchemyOutcome,
    before: Option<(u8, u32)>,
    after: Option<(u8, u32)>,
) -> String {
    let line = |(key, fallback): (&str, &str)| ui_strings.get_or(key, fallback).to_string();
    match outcome {
        // flag 0x10
        AlchemyOutcome::Success => match after {
            Some((level, _)) => fill(&line(MSG_SUCCESS), &[level as i64]),
            None => line(MSG_SUCCESS).replace("%d", "?"),
        },
        // flag 0x40
        AlchemyOutcome::Breakdown => line(MSG_BREAKDOWN),
        // flag 0x20, refined by the diff the original makes at :74-88
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
/// The slots are released **only** when the ack reports a fuse or a cancel —
/// a refusal leaves the page exactly as the player built it, the same rule
/// `enchant.rs::apply_dismantle_response` follows for dismantle.
fn apply_ack(
    response: &AlchemyReinforceResponse,
    opcode: &str,
    state: &mut AlchemyState,
    inventory: &mut Inventory,
    item_data: &ClientItemData,
    ui_strings: &ClientUiStrings,
    history: &mut ChatHistory,
) {
    if !response.is_success() {
        let code = response.error_code.unwrap_or_default();
        // The original resolves this through an error-string table that the
        // shipped data does not carry and whose published description is a
        // dead page. One value is known, the rest stay numbers on purpose
        // (ADR-0009): a plausible sentence would be a lie.
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
        state.pending = false;
        return;
    }
    state.pending = false;
    let Some((outcome, slot)) = response.outcome else {
        if response.action == Some(ALCHEMY_ACTION_CANCEL) {
            history.push(ChatLine::system(
                ui_strings
                    .get_or(MSG_CANCELLED.0, MSG_CANCELLED.1)
                    .to_string(),
            ));
            state.clear();
        }
        return;
    };
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
                // The server's copy is authoritative — the same rule the
                // pickup path follows, and the reason nothing is predicted
                // before the ack.
                inventory.gain_item(item);
            } else {
                warn!(
                    "alchemy: {opcode} item record did not parse, inventory slot {slot} is stale"
                );
            }
        }
    }
    // The materials are consumed either way, so the page is cleared and the
    // player re-builds it. (What the *server* removed from the bag arrives on
    // the inventory opcodes, not here.)
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

    /// A tiny stand-in for the string table: the tests assert on the *shipped*
    /// English, so they fail if a key is renamed or a `%d` goes missing.
    fn strings() -> ClientUiStrings {
        ClientUiStrings::default()
    }

    fn equipment(opt_level: u8, durability: u32) -> Option<(u8, u32)> {
        Some((opt_level, durability))
    }

    /// Success quotes the new `+N`, and it is the *level*, not durability —
    /// the mistranslation is in the original's own string.
    #[test]
    fn success_reports_the_new_enhancement_level() {
        let line = outcome_line(
            &strings(),
            AlchemyOutcome::Success,
            equipment(3, 100),
            equipment(4, 100),
        );
        assert!(line.contains("[4]"), "{line}");
        assert!(!line.contains("%d"), "every hole is filled: {line}");
    }

    /// The three failure sub-messages are a *diff*, not a wire field: same
    /// outcome byte, three different lines.
    #[test]
    fn failure_picks_its_message_from_the_item_diff() {
        let plain = outcome_line(
            &strings(),
            AlchemyOutcome::Failure,
            equipment(3, 100),
            equipment(3, 100),
        );
        assert_eq!(plain, MSG_FAIL.1);

        let down = outcome_line(
            &strings(),
            AlchemyOutcome::Failure,
            equipment(5, 100),
            equipment(3, 100),
        );
        assert!(
            down.contains("[3]") && down.contains("2 decreased"),
            "{down}"
        );

        let gone = outcome_line(
            &strings(),
            AlchemyOutcome::Failure,
            equipment(1, 100),
            equipment(0, 100),
        );
        assert_eq!(gone, MSG_FAIL_OPTLV_ZERO.1);

        let worn = outcome_line(
            &strings(),
            AlchemyOutcome::Failure,
            equipment(3, 100),
            equipment(3, 88),
        );
        assert!(
            worn.contains("[88]") && worn.contains("12 reduced"),
            "{worn}"
        );
    }

    /// A breakdown has no item to diff — the ack does not even carry one.
    #[test]
    fn breakdown_needs_no_item() {
        let line = outcome_line(
            &strings(),
            AlchemyOutcome::Breakdown,
            equipment(3, 100),
            None,
        );
        assert_eq!(line, MSG_BREAKDOWN.1);
    }

    /// This system holds `Res<ClientItemData>` / `Res<ClientUiStrings>`,
    /// which do not exist outside the world scene, so its registration must be
    /// gated. Neither `cargo test` nor the netcheck harness can see that
    /// structurally, hence the text assertion (same device as
    /// `both_job_windows_are_gated_on_the_world_scene`).
    #[test]
    fn the_fuse_ack_system_is_gated_on_the_world_scene() {
        let source = include_str!("mod.rs");
        let registered = source
            .find("outcome::apply_fuse_response")
            .expect("system is registered");
        let gate = source
            .find(".run_if(super::hud_scenes)")
            .expect("the update tuple is gated");
        assert!(
            registered < gate,
            "apply_fuse_response must sit inside the hud_scenes-gated tuple"
        );
    }
}

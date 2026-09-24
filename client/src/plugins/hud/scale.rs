//! The one HUD scale multiplier.
//!
//! # The idea
//!
//! Vanilla authors every HUD rect in a fixed 1:1 window space sized for the
//! era's 1024x768–1280x1024 screens, so transcribing those rects verbatim
//! gives a HUD that is pixel-exact and unreadably small on a modern display.
//! OpenRoad therefore multiplies the transcribed geometry by one uniform
//! factor. The factor is **ours, not the original's** (ADR-0009: a stated
//! deliberate improvement, not a transcribed value), and until #620 it was
//! restated as an anonymous `const *_SCALE: f32 = 1.5` in 25 modules — 25
//! places that each read like data and none of which a user could change.
//!
//! # Why a process global rather than `Res<ClientConfig>`
//!
//! The multiplier is consumed inside the windows' *pure layout helpers*
//! (`fn tab(..) -> impl Bundle`, `fn label(..)`, rect math) which take no
//! system parameters at all. Threading a `f32` through every one of them is a
//! far larger diff than the value is worth, and it would still not be a single
//! source. So the value lives here, is seeded and refreshed from
//! `config.hud.hud_scale` by [`apply_hud_scale`] on the same
//! `resource_changed::<ClientConfig>` path every other live setting uses
//! (`plugins::settings::live`), and is read with [`hud_scale`].
//!
//! A change lands on the next spawn of a surface: the windows compute their
//! geometry while they are built, so already-open windows keep the scale they
//! were built with until they are reopened.

use bevy::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::plugins::config::ClientConfig;
use crate::plugins::settings::live::config_changed;

/// The factor the whole HUD was authored against, and the default of
/// `config.hud.hud_scale`. Not from the original's data — see the module doc.
pub const DEFAULT_HUD_SCALE: f32 = 1.5;

/// `f32` has no atomic, so the bits are. Relaxed ordering is right here: the
/// value is a single independent scalar, written by one system on the main
/// schedule and read by spawn code; there is no other state whose visibility
/// has to be ordered against it.
static HUD_SCALE: AtomicU32 = AtomicU32::new(DEFAULT_HUD_SCALE.to_bits());

/// The current HUD scale multiplier. Every HUD surface multiplies its
/// transcribed geometry by this and nothing else.
pub fn hud_scale() -> f32 {
    f32::from_bits(HUD_SCALE.load(Ordering::Relaxed))
}

/// Set the multiplier. A non-finite or non-positive value would collapse or
/// NaN out every rect in the HUD, so it is rejected with a message instead of
/// being clamped silently to some other invented number.
pub fn set_hud_scale(scale: f32) {
    if !scale.is_finite() || scale <= 0.0 {
        warn!("ignoring hud.hud_scale = {scale}: it must be finite and > 0");
        return;
    }
    HUD_SCALE.store(scale.to_bits(), Ordering::Relaxed);
}

/// Seeds the global at boot and follows later `config.yaml` edits — one path
/// for both, because `resource_changed` is true on the frame the resource is
/// inserted (`plugins::settings::live`).
pub fn apply_hud_scale(config: Res<ClientConfig>) {
    set_hud_scale(config.hud.hud_scale);
}

// --- The original's font ladder ---------------------------------------------

/// The v1.188 client's five UI text sizes in **device pixels**, indexed by the
/// `FontIndex` key that every `resinfo/if*.txt` control carries.
///
/// # Where the numbers come from
///
/// The original creates exactly five fonts and nothing else ever calls the
/// creator: every call site of its font-slot constructor pushes one of the
/// pairs `(0, 9) (1, 8) (2, 12) (3, 11) (4, 15)`. The shipped data states the
/// same table independently:
/// `Media/server_dep/silkroad/event/event_interface.txt:2` reads
/// `//titlefont : 0 = "9", 1 = "8", 2 = "12", 3 = "11", 4 = "15"`.
///
/// Those five are **point** sizes. The slot constructor hands them to
/// `CreateFontIndirectA` as `lfHeight = -MulDiv(pt, 96, 72)`, so at the era's
/// 96 dpi the ladder is 12, 11, 16, 15, 20 pixels. A *negative* `lfHeight`
/// asks GDI for a character height, which for a TrueType face is the em size —
/// exactly what Bevy's `FontSize::Px` means, so the numbers transfer 1:1.
///
/// Across the 247 `resinfo` files: 3740 authored controls carry a
/// `FontIndex` — index 0 appears 3547 times, 2 → 156, 1 → 30, 3 → 2, 4 → 2.
/// Three sites in `ifchatbubblewindow.txt` (`:12,:31,:50`) ask for index **7**,
/// which no slot in the binary covers; only one of the three carries text
/// (`GDR_CHATBUBBLEWINDOW_CHATBOX:CIFTextBox`, `:44`), the other two are empty
/// `CIFWnd` bubble ends.
///
/// The intro scene keeps its own copy of this ladder
/// (`scenes::intro_v2::intro_font_px`) with the *opposite* out-of-range rule —
/// it clamps to the largest entry. That is not a second opinion about the same
/// question: the intro trees (`ps*.txt`) author only index 0 and 2, so no
/// out-of-range value reaches it, and it is unscaled because the intro draws at
/// the art's native size. The rule below is the HUD's, and the paragraph on
/// [`ladder_px`] states the data it rests on.
pub const FONT_INDEX_PX: [f32; 5] = [12.0, 11.0, 16.0, 15.0, 20.0];

/// The `FontSize::Px` value for a control whose resinfo `FontIndex` is
/// `index`, scaled by [`hud_scale`] like every other transcribed rect.
///
/// An out-of-range index falls back to 0, the index 3547 of 3740 authored
/// controls use, rather than panicking on a data value.
pub fn font_px(index: usize) -> f32 {
    text_px(ladder_px(index))
}

/// The ladder entry for a resinfo `FontIndex`. Split out so it can be
/// asserted without reading the process global the tests below race on.
///
/// An index the binary has no slot for falls back to **index 0**, the 9 pt
/// body size 3547 of 3740 authored controls carry — not to the largest entry,
/// which would render the one `ifchatbubblewindow.txt` FontIndex-7 site that
/// carries text at nearly twice the size the rest of the chat uses.
///
/// The data behind the choice: the only out-of-range index in the shipped
/// resinfo is 7, and it occurs in chat bubbles only. The eight other chat trees
/// (`ifchatviewer`, `ifchatmodule`, `ifwholechat`, `ifchatoptionboard`,
/// `ifchattingblocking(+slot)`, `ifcaschatview`, `ifsupporterchatwnd`) carry
/// **104 controls, every one of them FontIndex 0**, so index 0 is the size of
/// the surrounding chat text. What the original itself does with index 7 was
/// not read out of the binary — this is a stated openroad decision under
/// ADR-0009, not a transcribed rule.
fn ladder_px(index: usize) -> f32 {
    FONT_INDEX_PX
        .get(index)
        .copied()
        .unwrap_or(FONT_INDEX_PX[0])
}

/// [`text_px`]'s rule without the process global, so it can be asserted
/// without writing the one value the other tests here race on.
///
/// Private on purpose: a *fit* check elsewhere would have the same need (know
/// what a design value rounds to at a given scale without writing the global),
/// but no such call site exists in this tree yet, and `pub(crate)` on a
/// helper nobody outside the module calls only invites the re-derivation of
/// `(design * scale).round()` this module exists to remove.
fn round_text_px(design_px: f32, scale: f32) -> f32 {
    (design_px * scale).round()
}

/// Scale a text size the way [`hud_scale`] scales a rect, then **round to a
/// whole pixel**.
///
/// The rounding is the point of this function. A rect may land on a half
/// pixel — that is half a pixel of stretched texture and nobody sees it — but
/// a font size may not: `hud_scale` 1.5 turns every odd entry of the ladder
/// into a half (11 -> 16.5), and a fractional em rasterises the glyphs off the
/// pixel grid, which is the blurry half of "font and spacing do not fit". One
/// rule, one place; a caller that multiplies by `hud_scale()` itself is the
/// bug this replaces.
pub fn text_px(design_px: f32) -> f32 {
    round_text_px(design_px, hud_scale())
}

pub struct HudScalePlugin;

impl Plugin for HudScalePlugin {
    fn build(&self, app: &mut App) {
        // PreUpdate, so a fresh value is in place before any window spawned
        // this frame reads it.
        app.add_systems(PreUpdate, apply_hud_scale.run_if(config_changed));
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The point of #620: the multiplier exists once. A module that
    /// re-declares its own `1.5` is back to a constant a config value cannot
    /// reach, which is the defect this issue is about — so the tree is
    /// scanned for that shape rather than trusted.
    #[test]
    fn no_module_declares_its_own_hud_scale_constant() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                // This file is where the one value is allowed to live.
                if path.file_name().and_then(|f| f.to_str()) == Some("scale.rs") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (line, text) in text.lines().enumerate() {
                    let text = text.trim();
                    if (text.starts_with("const ") || text.starts_with("pub const "))
                        && text.contains("SCALE: f32 = 1.5")
                    {
                        offenders.push(format!("{}:{}", path.display(), line + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these sites re-declare the HUD scale instead of calling \
             `hud_scale()`, so `config.hud.hud_scale` cannot reach them: {offenders:?}"
        );
    }

    /// The ladder is data + binary, not taste: both sources are quoted in
    /// `FONT_INDEX_PX`'s doc comment, and the order is the resinfo `FontIndex`
    /// order (0 = 9 pt, the default of 3547 of 3740 authored controls), not
    /// ascending size. A future edit that "tidies" it into 11,12,15,16,20
    /// would silently re-point every FontIndex-0 label at 8 pt.
    #[test]
    fn the_font_ladder_is_the_originals_five_sizes_in_font_index_order() {
        assert_eq!(FONT_INDEX_PX, [12.0, 11.0, 16.0, 15.0, 20.0]);
        // -MulDiv(pt, 96, 72) for the pushed point sizes 9, 8, 12, 11, 15.
        for (index, pt) in [9.0f32, 8.0, 12.0, 11.0, 15.0].into_iter().enumerate() {
            assert_eq!(FONT_INDEX_PX[index], (pt * 96.0 / 72.0).round());
        }
    }

    /// A fractional em is the blurry half of "font and spacing do not fit":
    /// 11 px at scale 1.5 is 16.5, and it has to leave the rule as 17. Asserted
    /// on the pure rule, not through the process global, because the test below
    /// owns that global.
    #[test]
    fn a_scaled_text_size_is_always_a_whole_pixel() {
        assert_eq!(round_text_px(12.0, 1.5), 18.0);
        assert_eq!(round_text_px(11.0, 1.5), 17.0, "16.5 must not be rendered");
        assert_eq!(round_text_px(15.0, 1.5), 23.0);
        assert_eq!(round_text_px(12.0, 1.0), 12.0);
    }

    /// A resinfo `FontIndex` the binary has no slot for (7, in
    /// `ifchatbubblewindow.txt`) must not index out of bounds — and it must
    /// land on the body size, not on the 20 px headline entry: a clamp to the
    /// last slot renders the one chat-bubble site that carries text at nearly
    /// twice the size of the text around it. The intro scene's copy of the
    /// ladder clamps instead, and says why in its own doc — it never sees an
    /// out-of-range index.
    #[test]
    fn an_out_of_range_font_index_falls_back_to_the_body_size() {
        assert_eq!(ladder_px(7), ladder_px(0));
        assert_eq!(ladder_px(7), 12.0, "must not clamp to the 20 px entry");
        assert_eq!(ladder_px(0), 12.0);
        assert_eq!(ladder_px(4), 20.0, "an in-range index still resolves");
    }

    /// The knob has to move the number every surface multiplies by — and a
    /// garbage value in `config.yaml` must not NaN out the whole HUD. Both
    /// live in **one** test on purpose: the value under test is a process
    /// global and cargo runs tests in threads, so two tests writing it would
    /// race each other.
    #[test]
    fn the_scale_follows_a_valid_value_and_refuses_an_invalid_one() {
        let restore = hud_scale();
        set_hud_scale(2.25);
        assert_eq!(hud_scale(), 2.25);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            set_hud_scale(bad);
            assert_eq!(hud_scale(), 2.25, "{bad} was accepted");
        }
        set_hud_scale(restore);
    }
}

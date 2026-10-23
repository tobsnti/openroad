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

/// Scale a **text** size the way [`hud_scale`] scales a rect, then round to a
/// whole pixel.
///
/// The rounding is the point. A rect may land on a half pixel — that is half a
/// pixel of stretched texture and nobody sees it — but a font size may not:
/// at the shipped scale 1.5 every odd design value becomes a half
/// (`11 -> 16.5`, `9 -> 13.5`, `8.5 -> 12.75`), and a fractional em rasterises
/// the glyphs off the pixel grid. That is the blurry half of "font and spacing
/// do not fit" — the one open `[U]` the font arc left
/// (`FONT-PX-REST-UND-BILANZ.md`).
///
/// One rule, one place; a surface that writes `FontSize::Px(design * s)`
/// itself is the bug this replaces.
///
/// # What is taken from where
///
/// The *rule* is the one `port/07-hud-frame` `1a19b65dc` already settled.
/// Two deliberate differences from that branch:
///
/// * **Its font ladder is not taken.** Ours lives in
///   [`crate::assets::resinfo::interface_text::font_index_px`] and falls back
///   to slot **0** — the index 3547 of 3739 authored controls use — where that
///   branch clamps to the largest slot. Ours is the corpus-backed one.
/// * **The scale is a parameter, not a global read.** That branch's helper
///   reads [`hud_scale`] because its call sites had no scale in hand; nearly
///   every surface here already holds one (`let s = hud_scale()`, or an `s:
///   f32` a window builder was handed), and silently re-reading the global
///   would ignore the value the caller passed.
pub fn text_px(design_px: f32, scale: f32) -> f32 {
    (design_px * scale).round()
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

    /// The defect this exists for: at the shipped scale every odd design size
    /// lands on a half pixel. The scale is passed in, so this test does not
    /// race the process global the one above writes.
    #[test]
    fn the_shipped_scale_turns_odd_design_sizes_into_whole_pixels() {
        // The three shapes the tree actually holds: the font ladder's odd
        // entries, the inline halves, and an even value that was never broken.
        assert_eq!(text_px(11.0, DEFAULT_HUD_SCALE), 17.0); // was 16.5
        assert_eq!(text_px(9.0, DEFAULT_HUD_SCALE), 14.0); // was 13.5
        assert_eq!(text_px(8.5, DEFAULT_HUD_SCALE), 13.0); // was 12.75
        assert_eq!(text_px(12.0, DEFAULT_HUD_SCALE), 18.0); // already whole
        for design in [
            6.0f32, 7.0, 7.5, 8.0, 8.5, 9.0, 9.5, 11.0, 12.0, 15.0, 16.0, 20.0,
        ] {
            let px = text_px(design, DEFAULT_HUD_SCALE);
            assert_eq!(px.fract(), 0.0, "{design} scaled to {px}");
        }
    }

    /// Rounding must not collapse a size to zero — a 0 px font draws nothing,
    /// which would be a worse bug than a blurry one. `round` keeps anything
    /// from 0.5 px up, and the smallest design size in the tree is 6.0.
    #[test]
    fn a_small_design_size_survives_the_rounding() {
        assert_eq!(text_px(6.0, 1.0), 6.0);
        assert_eq!(text_px(6.0, 0.25), 2.0);
        // the degenerate end is the caller's to avoid; stated, not hidden
        assert_eq!(text_px(6.0, 0.01), 0.0);
    }

    /// The rule has to stay in one place. A surface that writes
    /// `FontSize::Px(design * s)` again is back to a fractional em, and no
    /// compile error says so — so the tree is scanned for that shape, the same
    /// way the test above scans for a re-declared scale constant.
    ///
    /// Rects are deliberately not covered: `ICON_SIZE * hud_scale()` is a
    /// *rect*, and half a pixel of stretched texture is what the module doc
    /// says is fine.
    #[test]
    fn no_surface_scales_a_font_size_without_the_rounding_rule() {
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
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (line, text) in text.lines().enumerate() {
                    let text = text.trim();
                    if text.starts_with("//") || !text.contains("FontSize::Px(") {
                        continue;
                    }
                    let scaled = text.contains("* s)") || text.contains("* hud_scale())");
                    if scaled {
                        offenders.push(format!("{}:{}", path.display(), line + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these sites scale a font size without rounding it to a whole              pixel — call `text_px(design, scale)` instead: {offenders:?}"
        );
    }
}

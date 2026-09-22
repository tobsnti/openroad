use bevy::prelude::*;

use super::assets::IntroV2Assets;
use super::IntroV2State;
use crate::scenes::loading_screen::{design_pct, DesignFit};

/// Root marker of the splash screen (big logo, click to continue).
#[derive(Component, Default, Clone)]
pub struct SplashRoot;

/// `GDR_STA_BIGLOGO` (`Media/resinfo/pstitle_europe.txt:624`,
/// `Rect="360,448,896,300"`) — the rect the title tree authors for the big
/// logo, verbatim, in the tree's **1600x1200** design space.
///
/// A control with a real rect in that space is *scaled*, not drawn native: the
/// whole title tree is one flat 1600x1200 canvas (`GDR_FADE` `0,0,1600,1200`),
/// so the drawn rect is `(x, y, w, h) * k` with `k = view / 1600`. On an
/// 800x600 client `k = 0.5`: the gold hairline of `blackbar_up_18_europe.ddj`
/// covers art rows 156..165 but screen rows 78..82, and the red "E" of
/// `logo.ddj` is 77x106 in the art and 39x53 on screen. This rect therefore
/// draws **448x150 at (180,224)**, not its native 896x300.
///
/// The factor is **not** a constant here. The rect goes into the same
/// `DesignFit`/`design_pct` pair the loading chrome already uses, so `k` is the
/// size of one centred 4:3 box and the number in this file is the data line.
///
/// Non-4:3 windows are **unknown**: whether the original stretches or
/// letterboxes on a wide screen is undecided. `DesignFit::CONTAIN`
/// picks the conservative half — `k` from the height, centred horizontally —
/// which keeps the art's aspect and agrees with `chrome.rs`, where the bands
/// are already a fraction of the height (172/1200).
const BIG_LOGO_RECT: (f32, f32, f32, f32) = (360.0, 448.0, 896.0, 300.0);

pub fn splash_logo(assets: &IntroV2Assets) -> impl Scene {
    let logo = assets.logo_big.clone();
    let (logo_l, logo_t, logo_w, logo_h) = design_pct(BIG_LOGO_RECT);
    bsn! {
        SplashRoot
        Name("Splash Logo V2")
        // The 4:3 box the rect above is a percentage of; `fit_design_surfaces`
        // writes the four `px(0)` placeholders every time the window changes.
        DesignFit { cover: false }
        Node {
            position_type: PositionType::Absolute,
            left: px(0), top: px(0), width: px(0), height: px(0),
        }
        Pickable::IGNORE
        Children [
            (
                ImageNode { image: {logo}, image_mode: NodeImageMode::Stretch }
                Node {
                    position_type: PositionType::Absolute,
                    left: percent(logo_l),
                    top: percent(logo_t),
                    width: percent(logo_w),
                    height: percent(logo_h),
                }
                Pickable::IGNORE
            ),
        ]
    }
}

/// How the original advanced splash -> login is **UNKNOWN** — the data carries
/// no string or control for it. Advancing on a left click is therefore an
/// openroad affordance, not a reproduction.
pub fn on_splash_click(
    mut next_state: ResMut<NextState<IntroV2State>>,
    buttons: Res<ButtonInput<MouseButton>>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        next_state.set(IntroV2State::LoginForm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::loading_screen::design_fit_rect;

    /// What a design-space rect actually covers in a `win_w` x `win_h` client:
    /// the `DesignFit::CONTAIN` box, then the percentages of it the scene
    /// authors. Same two calls the scene makes, so the assertions below are
    /// about the shipped geometry and not about a second formula.
    fn drawn(rect: (f32, f32, f32, f32), win_w: f32, win_h: f32) -> (f32, f32, f32, f32) {
        let (bx, by, bw, bh) = design_fit_rect(win_w, win_h, DesignFit::CONTAIN);
        let (l, t, w, h) = design_pct(rect);
        (
            bx + bw * l / 100.0,
            by + bh * t / 100.0,
            bw * w / 100.0,
            bh * h / 100.0,
        )
    }

    /// `GDR_STA_BIGLOGO` `Rect="360,448,896,300"` at `k = 800/1600 = 0.5` is
    /// **448x150 at (180,224)**. Drawn at its native 896x300 the art is twice
    /// too large and runs off the edge of a 640x480 window.
    #[test]
    fn the_splash_logo_is_half_size_on_an_800x600_client() {
        assert_eq!(
            drawn(BIG_LOGO_RECT, 800.0, 600.0),
            (180.0, 224.0, 448.0, 150.0)
        );
        // The wrong draw, spelled out so this test fails if anyone puts it
        // back: native size is what 1600x1200 — and only 1600x1200 — gives.
        assert_ne!(drawn(BIG_LOGO_RECT, 800.0, 600.0).2, BIG_LOGO_RECT.2);
    }

    /// `k = 1` at the design size: the rect is drawn verbatim. This is the
    /// positive control for `drawn` itself — a broken helper cannot return the
    /// input rect here *and* half of it above.
    #[test]
    fn at_the_design_size_the_logo_is_native() {
        assert_eq!(drawn(BIG_LOGO_RECT, 1600.0, 1200.0), BIG_LOGO_RECT);
    }

    /// One more 4:3 step, so the test pins the *rule* and not one resolution:
    /// 1024/1600 = 0.64.
    #[test]
    fn the_rule_is_one_factor_not_a_table() {
        let (x, y, w, h) = drawn(BIG_LOGO_RECT, 1024.0, 768.0);
        for (got, want) in [(x, 230.4), (y, 286.72), (w, 573.44), (h, 192.0)] {
            assert!((got - want).abs() < 0.01, "got {got}, want {want}");
        }
    }

    /// Non-4:3 is unknown: the conservative choice this scene takes is one
    /// factor from the height with the box centred, so the art keeps its aspect
    /// and nothing stretches. Pinned because it is a decision, not a fact about
    /// the original.
    #[test]
    fn a_wide_window_letterboxes_rather_than_stretching() {
        let (_, _, w, h) = drawn(BIG_LOGO_RECT, 1920.0, 1200.0);
        assert!((w / h - BIG_LOGO_RECT.2 / BIG_LOGO_RECT.3).abs() < 1e-4);
        // 1200-tall window: k is the same 1.0 the 1600x1200 case gets.
        assert!((w - BIG_LOGO_RECT.2).abs() < 0.01);
    }
}

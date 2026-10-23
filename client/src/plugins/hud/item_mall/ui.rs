//! The Item Mall's "My information" pane (`resinfo/ifitemmallmyinfo.txt`).
//!
//! Idea: `0x3153` pushes the account's three silk counters, and the one surface
//! the original shows them on is this pane — three statics whose `Text` is
//! empty in the descriptor and filled at runtime, one per counter, above them
//! the character's portrait, name and level. The pane rides on the shared
//! `game_window` chrome: the title is the enclosing sub-frame's own key
//! `UIIT_STT_SILKMALL_MY_INFO` (`GDR_ITEM_MALL_MYINFO_FRAME1`, `:405`), and the
//! interior tile letter is the one the pane declares, `com_bg_tile_b.ddj`
//! (`GDR_ITEM_MALL_MYINFO_BG`, `:386`) — not the shell's default `_d`.
//!
//! **The content box is that background plate** (`:386`, `Rect="16,36,224,135"`),
//! and every element is placed by subtracting its origin — one translation,
//! straight from the data. It used to be the inner frame
//! `GDR_ITEM_MALL_MYINFO_FRAME2` (`:367`, `6,31,244,149`) minus our chrome, and
//! that stopped working the moment the identity row joined: the original's
//! `frameg01_wnd_` eats 5 units above its plate, our `mframe_wnd_` eats 36, so
//! the portrait would have landed under the title band. Sizing the *interior*
//! from the data and letting the outer box fall out of it keeps every distance
//! inside the pane exactly as authored; the window is 4 units wider and 38
//! taller than the original's box, which is the thickness of our chrome and
//! nothing else.
//!
//! **Deliberately left out, and why:** `GDR_ITEM_MALL_MYINFO_BUY_SILK_BTN`
//! (`:255`) and `..._COUPON_EXCHANGE_BTN` (`:233`). Both would need a
//! purchase/redeem opcode, and there is none: no opcode catalog, wiki page or
//! xBot handler covers "buy from the mall", `packet_dump/` holds no frame of
//! it, and reference servers run the shop on the web (`SK_SilkChange_BY_Web`).
//! A button that cannot send is a dead wire, so the pane ships without them
//! rather than with two that lie — the empty strip under the silk rows is
//! where they sit in the original.
//!
//! Every string is the pane's own key through the player's `textuisystem.txt`,
//! never our words — including the level, whose `Lv %d` template the table
//! carries as `UIIT_STT_LV_LEVEL` (L594). The font size is the descriptor's own
//! `FontIndex` through [`font_index_px`]; all nine text blocks carry index 0.

use bevy::prelude::*;
use bevy::ui_widgets::Activate;

use crate::assets::resinfo::interface_text::font_index_px;
use crate::assets::FontAssets;
use crate::plugins::hud::art::art_path;
use crate::plugins::hud::game_window::{self, abs_node, GameWindowStyle};
use crate::plugins::hud::item_mall::ItemMallState;
use crate::plugins::hud::player_mini_info::{PlayerVitals, PortraitTarget};
use crate::plugins::hud::scale::hud_scale;
use crate::plugins::net::silk::SilkBalance;
use crate::plugins::textdata::ClientUiStrings;

// --- Layout (resinfo, pane units) -------------------------------------------

/// `GDR_ITEM_MALL_MYINFO_BG` (`ifitemmallmyinfo.txt:386`): the pane's own
/// background plate inside its frame. Its **size** is this window's content
/// box and its **origin** is what every element below is measured from.
const PLATE_RECT: (f32, f32, f32, f32) = (16.0, 36.0, 224.0, 135.0);

/// The same block declares `com_bg_tile_b.ddj` — the interior letter this pane
/// asks for, where the shell defaults to `_d`.
const BG_TILE: &str = "media://interface/ifcommon/bg_tile/com_bg_tile_b.ddj";

/// `GDR_ITEM_MALL_MYINFO_FACEFRAME` (`:348`): the ring the portrait sits in,
/// and the one piece of art this pane names for itself.
const FACE_FRAME_RECT: (f32, f32, f32, f32) = (16.0, 39.0, 64.0, 64.0);
/// The descriptor writes its separators doubled — `DDJ=STRING,
/// "interface\\mall\\mall_faceframe.ddj"` — which is resinfo's escaping, not
/// part of the path: [`art_path`] turns every backslash into a slash, so the
/// doubled form would resolve to `media://interface//mall//…` and miss.
const FACE_FRAME_DDJ: &str = r"interface\mall\mall_faceframe.ddj";
/// `GDR_ITEM_MALL_MYINFO_FACE` (`:329`, a `CIFStaticWithPictureClip`): the
/// head picture inside that ring.
const FACE_RECT: (f32, f32, f32, f32) = (23.0, 46.0, 48.0, 48.0);
/// `GDR_ITEM_MALL_MYINFO_NAME` (`:196`), `HAlign=1` — centred in its slot.
const NAME_RECT: (f32, f32, f32, f32) = (81.0, 47.0, 78.0, 14.0);
/// `GDR_ITEM_MALL_MYINFO_LEVEL` (`:177`), `HAlign=2` — flush right.
const LEVEL_RECT: (f32, f32, f32, f32) = (192.0, 47.0, 41.0, 14.0);
/// The level's own wording lives in the string table as a printf template
/// (`UIIT_STT_LV_LEVEL`, textuisystem L594 = `Lv %d`), so the pane prints what
/// the player's table says rather than a sentence of ours. Filled the way
/// `chat::model::format_restriction` fills `UIIT_STT_CANT_CHATTING`.
const LEVEL_KEY: &str = "UIIT_STT_LV_LEVEL";
const LEVEL_FALLBACK: &str = "Lv %d";

/// One counter's row: the label's rect and text key, and the value's rect.
/// Verbatim from `ifitemmallmyinfo.txt`, pane units, in the order the wire
/// carries them (`SilkUpdate { own, gift, point }`).
struct Row {
    label: (f32, f32, f32, f32),
    key: &'static str,
    fallback: &'static str,
    value: (f32, f32, f32, f32),
}

/// `:44`/`:158` own, `:25`/`:139` gift, `:6`/`:120` point. The fallbacks are
/// the English values the player's own table carries (L1166 / L3488 / L3490),
/// for scenes that load no textdata.
const ROWS: [Row; 3] = [
    Row {
        label: (82.0, 73.0, 33.0, 14.0),
        key: "UIIT_STT_SILKMALL_SILK",
        fallback: "Silk",
        value: (136.0, 74.0, 79.0, 14.0),
    },
    Row {
        label: (60.0, 98.0, 56.0, 14.0),
        key: "UIIT_CTL_SILK_INQUIRY_GIFT_SILK",
        fallback: "Bonus Silk",
        value: (136.0, 96.0, 79.0, 14.0),
    },
    Row {
        label: (26.0, 122.0, 90.0, 14.0),
        key: "UIIT_CTL_SILK_INQUIRY_POINT",
        fallback: "Point",
        value: (136.0, 120.0, 79.0, 14.0),
    },
];

/// Every block of this pane carries `FontIndex=0` (nine of nine, checked in
/// the file), which the ladder reads as 12 px.
const FONT_INDEX: usize = 0;

/// Spawn anchor (right/top, physical px). **Ours**: the mall window is not in
/// `wndpos.dat`'s persisted set, so the original's own position for it is not
/// in the data.
const WINDOW_RIGHT: f32 = 300.0;
const WINDOW_TOP: f32 = 120.0;

const CONTENT_W: f32 = PLATE_RECT.2;
const CONTENT_H: f32 = PLATE_RECT.3;

/// A pane rect rebased into the shell's content space: minus the plate's own
/// origin, and nothing else.
fn content_rect(rect: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    let (x, y, w, h) = rect;
    (x - PLATE_RECT.0, y - PLATE_RECT.1, w, h)
}

/// `UIIT_STT_LV_LEVEL`'s `%d` filled with the level. A table that dropped the
/// placeholder still has to show the number, so it is appended rather than
/// lost — same rule as the chat restriction readout.
fn level_text(template: &str, level: u8) -> String {
    if template.contains("%d") {
        template.replacen("%d", &level.to_string(), 1)
    } else {
        format!("{template} {level}")
    }
}

// --- Markers ----------------------------------------------------------------

#[derive(Component)]
pub struct MyInfoWindowRoot;

/// One of the three value statics, in wire order.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct MyInfoValue(pub usize);

/// The character's name static (`..._NAME`).
#[derive(Component)]
pub struct MyInfoName;

/// The character's level static (`..._LEVEL`).
#[derive(Component)]
pub struct MyInfoLevel;

/// The picture inside the face ring (`..._FACE`).
#[derive(Component)]
pub struct MyInfoFace;

// --- Spawning ---------------------------------------------------------------

/// Spawn the (initially hidden) My-Info pane.
pub fn spawn_my_info_window(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Ok(camera) = cam_query.single() else {
        warn!("item mall: no 2d camera to attach to");
        return;
    };
    let s = hud_scale();

    let window = game_window::spawn_game_window_styled(
        &mut commands,
        &asset_server,
        &fonts,
        camera,
        ui_strings.get_or("UIIT_STT_SILKMALL_MY_INFO", "My information"),
        (CONTENT_W, CONTENT_H),
        (WINDOW_RIGHT, WINDOW_TOP),
        s,
        GameWindowStyle {
            bg_tile: BG_TILE,
            ..GameWindowStyle::default()
        },
    );
    commands
        .entity(window.root)
        .insert((MyInfoWindowRoot, GlobalZIndex(55)))
        .entry::<Node>()
        .and_modify(|mut node| node.display = Display::None);
    commands
        .entity(window.expect_close_button())
        .observe(on_close_button);

    let font_px = font_index_px(FONT_INDEX);
    commands.entity(window.content).with_children(|content| {
        // identity row: the ring, the live head render inside it, then the
        // name and the level. The ring is the only art this pane names, and
        // it goes through the shared `art_path` conversion (#77) rather than a
        // fourth hand-written `media://interface/` prefix.
        content.spawn((
            ImageNode::new(asset_server.load::<Image>(art_path(FACE_FRAME_DDJ))),
            abs_node(content_rect(FACE_FRAME_RECT), s),
            Pickable::IGNORE,
        ));
        content.spawn((
            MyInfoFace,
            // filled by `update_my_info_portrait` once the mini-info panel's
            // offscreen head render exists; it is created in another plugin's
            // enter system, so the handle cannot be read at spawn time
            ImageNode::new(Handle::default()),
            abs_node(content_rect(FACE_RECT), s),
            Pickable::IGNORE,
        ));
        content.spawn((
            MyInfoName,
            Text::default(),
            text_font(&fonts, font_px, s),
            TextColor(Color::WHITE),
            TextLayout::justify(Justify::Center),
            abs_node(content_rect(NAME_RECT), s),
            Pickable::IGNORE,
        ));
        content.spawn((
            MyInfoLevel,
            Text::default(),
            text_font(&fonts, font_px, s),
            TextColor(Color::WHITE),
            TextLayout::justify(Justify::Right),
            abs_node(content_rect(LEVEL_RECT), s),
            Pickable::IGNORE,
        ));
        for (index, row) in ROWS.iter().enumerate() {
            // both the label and the number carry `HAlign=2` in the
            // descriptor, i.e. flush right in their own box
            content.spawn((
                Text::new(ui_strings.get_or(row.key, row.fallback)),
                text_font(&fonts, font_px, s),
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Right),
                abs_node(content_rect(row.label), s),
                Pickable::IGNORE,
            ));
            content.spawn((
                MyInfoValue(index),
                // empty until the server pushes: the descriptor's own value
                // statics carry `Text=""`, so nothing is invented here either
                Text::default(),
                text_font(&fonts, font_px, s),
                TextColor(Color::WHITE),
                TextLayout::justify(Justify::Right),
                abs_node(content_rect(row.value), s),
                Pickable::IGNORE,
            ));
        }
    });
}

fn text_font(fonts: &FontAssets, px: f32, s: f32) -> TextFont {
    TextFont {
        font: fonts.two.clone().into(),
        font_size: FontSize::Px(px * s),
        ..default()
    }
}

pub fn cleanup_my_info_window(
    mut commands: Commands,
    windows: Query<Entity, With<MyInfoWindowRoot>>,
) {
    for entity in windows.iter() {
        commands.entity(entity).despawn();
    }
}

// --- Behavior ---------------------------------------------------------------

fn on_close_button(_: On<Activate>, mut state: ResMut<ItemMallState>) {
    state.open = false;
}

/// Mirror [`ItemMallState`] onto the shell.
pub fn apply_my_info_visibility(
    state: Res<ItemMallState>,
    mut roots: Query<&mut Node, With<MyInfoWindowRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    for mut node in roots.iter_mut() {
        node.display = if state.open {
            Display::Flex
        } else {
            Display::None
        };
    }
}

/// Write the three counters into their statics. `SilkBalance` is a **net**
/// resource and may be absent in a HUD-only scene, and `None` until the first
/// `0x3153` — both leave the statics as the descriptor has them: empty.
pub fn update_my_info_values(
    balance: Option<Res<SilkBalance>>,
    mut values: Query<(&MyInfoValue, &mut Text)>,
) {
    let Some(balance) = balance else {
        return;
    };
    if !balance.is_changed() {
        return;
    }
    for (slot, mut text) in values.iter_mut() {
        let next = match (balance.0, slot.0) {
            (Some(counters), 0) => counters.own.to_string(),
            (Some(counters), 1) => counters.gift.to_string(),
            (Some(counters), 2) => counters.point.to_string(),
            _ => String::new(),
        };
        if text.0 != next {
            text.0 = next;
        }
    }
}

/// Write the character's name and level into their statics.
///
/// `PlayerVitals` is the same aggregate the mini-info panel and the character
/// window read (name from CHARACTER_DATA, level from it and `0x303D`), so the
/// three surfaces cannot disagree. Optional for the same reason the balance is:
/// a HUD-only scene may not own the resource, and a missing `Res` would fail
/// parameter validation rather than skip the system.
pub fn update_my_info_identity(
    vitals: Option<Res<PlayerVitals>>,
    strings: Option<Res<ClientUiStrings>>,
    mut names: Query<&mut Text, (With<MyInfoName>, Without<MyInfoLevel>)>,
    mut levels: Query<&mut Text, (With<MyInfoLevel>, Without<MyInfoName>)>,
) {
    let Some(vitals) = vitals else {
        return;
    };
    if !vitals.is_changed() {
        return;
    }
    for mut text in names.iter_mut() {
        if text.0 != vitals.name {
            text.0.clone_from(&vitals.name);
        }
    }
    let template = strings
        .as_deref()
        .map(|s| s.get_or(LEVEL_KEY, LEVEL_FALLBACK))
        .unwrap_or(LEVEL_FALLBACK);
    // level 0 is "nothing pushed yet", and the descriptor's static starts empty
    let next = if vitals.level == 0 {
        String::new()
    } else {
        level_text(template, vitals.level)
    };
    for mut text in levels.iter_mut() {
        if text.0 != next {
            text.0.clone_from(&next);
        }
    }
}

/// Point the face picture at the mini-info panel's offscreen head render.
///
/// That image is the player's own head, framed by its own camera
/// (`player_mini_info`, 96 px target) — the pane's `CIFStaticWithPictureClip`
/// slot gets the picture the client already renders, not a second rig and not
/// invented art. The resource is created in another plugin's enter system, so
/// this runs until it appears instead of reading it at spawn time.
pub fn update_my_info_portrait(
    target: Option<Res<PortraitTarget>>,
    mut faces: Query<&mut ImageNode, With<MyInfoFace>>,
) {
    let Some(target) = target else {
        return;
    };
    for mut face in faces.iter_mut() {
        if face.image != target.0 {
            face.image = target.0.clone();
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// The content box is the pane's own background plate
    /// (`ifitemmallmyinfo.txt:386`, `16,36,224,135`) — the interior is the
    /// data's, and the outer box is whatever our chrome adds to it.
    #[test]
    fn the_content_box_is_the_panes_own_background_plate() {
        assert_eq!((CONTENT_W, CONTENT_H), (PLATE_RECT.2, PLATE_RECT.3));
        // and the chrome is the only reason the window is bigger than the
        // original's frame (`:367`, 244x149)
        assert_eq!(
            game_window::outer_size((CONTENT_W, CONTENT_H)),
            (248.0, 187.0)
        );
    }

    /// Rebasing is one translation by the plate's origin, and **every** element
    /// — identity row included — has to land inside the content box. This is
    /// the check that catches a mis-copied rect, and the one that failed for
    /// the identity row while the box was still derived from the frame.
    #[test]
    fn every_element_lands_inside_the_content_box() {
        assert_eq!(
            content_rect((136.0, 74.0, 79.0, 14.0)),
            (120.0, 38.0, 79.0, 14.0)
        );
        let mut rects = vec![FACE_FRAME_RECT, FACE_RECT, NAME_RECT, LEVEL_RECT];
        for row in &ROWS {
            rects.push(row.label);
            rects.push(row.value);
        }
        for rect in rects {
            let (x, y, w, h) = content_rect(rect);
            assert!(x >= 0.0 && y >= 0.0, "{rect:?} rebases above the content");
            assert!(
                x + w <= CONTENT_W && y + h <= CONTENT_H,
                "{rect:?} leaves the content box ({CONTENT_W}x{CONTENT_H})"
            );
        }
    }

    /// The head picture sits inside its ring, which is what makes the ring a
    /// frame and not a plate behind the whole row.
    #[test]
    fn the_face_sits_inside_its_ring() {
        let (fx, fy, fw, fh) = FACE_RECT;
        let (rx, ry, rw, rh) = FACE_FRAME_RECT;
        assert!(fx > rx && fy > ry && fx + fw < rx + rw && fy + fh < ry + rh);
    }

    /// The level is the table's own `Lv %d`, filled — not a sentence of ours.
    /// The fallback branch matters too: a translation that drops the
    /// placeholder must still show the number.
    #[test]
    fn the_level_is_the_tables_own_template_filled() {
        assert_eq!(level_text("Lv %d", 40), "Lv 40");
        assert_eq!(level_text("Level", 40), "Level 40");
        // the slot is 41 units wide at 12 px, which is what "Lv 40" needs and
        // "Level 40" would not fit — the reason the data's own key is used
        assert_eq!(LEVEL_RECT.2, 41.0);
    }

    /// The rows are in wire order, so the slot index of a value static is the
    /// field of `SilkUpdate` it shows. A swapped pair here would show gift
    /// silk as own silk, which no later test could see.
    #[test]
    fn the_rows_follow_the_wire_order_and_carry_the_panes_keys() {
        assert_eq!(
            [ROWS[0].key, ROWS[1].key, ROWS[2].key],
            [
                "UIIT_STT_SILKMALL_SILK",
                "UIIT_CTL_SILK_INQUIRY_GIFT_SILK",
                "UIIT_CTL_SILK_INQUIRY_POINT",
            ]
        );
        // the labels descend in the order the pane lists them
        assert!(ROWS[0].value.1 < ROWS[1].value.1 && ROWS[1].value.1 < ROWS[2].value.1);
    }

    /// The pane declares `FontIndex=0` in all nine blocks, and the ladder reads
    /// that as 12 px. Pinned because the ladder is **not** ordered: a wrong
    /// index would silently pick a different size, not an obviously wrong one.
    #[test]
    fn the_font_size_is_the_descriptors_own_index() {
        assert_eq!(font_index_px(FONT_INDEX), 12.0);
    }
}

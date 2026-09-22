use bevy::prelude::*;
use bevy_asset_loader::prelude::AssetCollection;

use crate::assets::textdata::Textdata;

/// Assets for the intro v2 scene. Uses the same asset paths as the old
/// intro's collection, so the underlying assets are shared via the
/// asset server; only the collection resource is duplicated because the
/// old one lives in a private module.
#[derive(AssetCollection, Resource)]
// Test-only `Default`: the pregame `OnEnter` systems take this collection as a
// plain `Res<_>`, so a unit test that drives one of them has to have *a* value
// in the world even when the system never dereferences it (an empty handle
// resolves to nothing, which is what a test without an asset source wants).
#[cfg_attr(test, derive(Default))]
#[allow(dead_code)]
pub struct IntroV2Assets {
    // Windows
    // `login_window_europe.ddj`, not `login_window.ddj`: the login screen's
    // resinfo block names it directly —
    // `Media/resinfo/pstitle_europe.txt:581`, `GDR_STA_LOGINWINDOW:CIFStatic`
    // `ID=9` `Rect="0,0,288,140"`
    // `DDJ=STRING,"interface\\outer\\login_window_europe.ddj"`. Both files ship
    // (and `login_window_europe_02.ddj` besides), and both are 288x140 in their
    // DDS header, so this is a different *skin* of the same frame, not a
    // different size — the same `EUROPE_SYSTEM` reason the buttons below already
    // carry.
    #[asset(path = "media://interface/outer/login_window_europe.ddj")]
    pub login_window: Handle<Image>,

    // Logo
    #[asset(path = "media://interface/outer/logo.ddj")]
    pub logo: Handle<Image>,
    #[asset(path = "media://interface/outer/logo-big.ddj")]
    pub logo_big: Handle<Image>,
    // `GDR_STA_TITLE:CIFStatic` id 5, `Rect="47,110,184,36"`,
    // `interface\outer\text-connect.ddj` — `pstitle_europe.txt:657/662`. The
    // caption the original paints in the top left of the login screen.
    #[asset(path = "media://interface/outer/text-connect.ddj")]
    pub text_connect: Handle<Image>,
    // `GDR_STA_TITLE:CIFStatic` id 3, `Rect="47,110,368,36"`,
    // `interface\outer\text-characterselect.ddj` —
    // `pscharacterselect_europe.txt:972/976/980/981`. The lobby's caption.
    #[asset(path = "media://interface/outer/text-characterselect.ddj")]
    pub text_character_select: Handle<Image>,

    // Buttons — the `_europe` variants: `config/define.txt` defines
    // EUROPE_SYSTEM, and `resinfo/pscharacterselect.txt` names
    // `interface\outer\button_europe.ddj` / `info_europe.ddj` directly.
    #[asset(path = "media://interface/outer/button_europe.ddj")]
    pub button: Handle<Image>,
    #[asset(path = "media://interface/outer/button_europe_press.ddj")]
    pub button_press: Handle<Image>,
    #[asset(path = "media://interface/outer/button_europe_focus.ddj")]
    pub button_focus: Handle<Image>,
    // No `button_europe_disable.ddj` ships (the archive holds three
    // `*button_europe*` files and no `*europe*disable*` one), so the disabled
    // frame comes from the shared `button_disable.ddj`. Its DDS header reads
    // 91x40, i.e. the size the intro actually draws these buttons at
    // (`image_button(main_button_style(..), 91.0, 41.0)`); the europe art is
    // 92x40 and gets stretched to the same node either way.
    #[asset(path = "media://interface/outer/button_disable.ddj")]
    pub button_disable: Handle<Image>,
    // Exit is the one main button the original does *not* skin `_europe`: own
    // read of `pstitle_europe.txt:467`, `GDR_BTN_CANCEL` (`ID=12`,
    // `Text=UIO_CTL_EXIT`) carries `DDJ="interface\\outer\\button.ddj"` while
    // `GDR_BTN_OK` (`:486`, `UIO_CTL_CONNECT`) carries `button_europe.ddj`. The
    // asymmetry is in the data, so it gets its own three frames here; the
    // disabled frame is `button_disable.ddj`, which is the *same* file the
    // europe style borrows, and here it is the matching one by design (91x40,
    // like `button.ddj`).
    #[asset(path = "media://interface/outer/button.ddj")]
    pub exit_button: Handle<Image>,
    #[asset(path = "media://interface/outer/button_focus.ddj")]
    pub exit_button_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/button_press.ddj")]
    pub exit_button_press: Handle<Image>,
    #[asset(path = "media://interface/outer/list_button.ddj")]
    pub list_button: Handle<Image>,
    #[asset(path = "media://interface/outer/list_button_press.ddj")]
    pub list_button_press: Handle<Image>,
    #[asset(path = "media://interface/outer/list_button_focus.ddj")]
    pub list_button_focus: Handle<Image>,
    // The list button is the one button of this screen whose fourth frame
    // actually ships: `Media/interface/outer` holds `list_button.ddj`,
    // `_focus`, `_press` **and** `_disable` (48x24 each, checked in the DDS
    // header), unlike `button_europe*`, which has no `_disable`. So the
    // `ImageButtonStyle` for it can be complete instead of falling back to the
    // normal frame via `ui_v2`'s `warn_once`.
    #[asset(path = "media://interface/outer/list_button_disable.ddj")]
    pub list_button_disable: Handle<Image>,

    // Intro chrome bars — one pair per tree, see `chrome.rs`. All six arts
    // are 1600x172 ARGB1555 and md5-distinct, so none of them substitutes
    // for another.
    // pstitle_europe.txt:729/710 (title: splash, login, server select)
    #[asset(path = "media://interface/outer/blackbar_up_18_europe.ddj")]
    pub title_bar_up: Handle<Image>,
    #[asset(path = "media://interface/outer/blackbar_down_copyright_europe.ddj")]
    pub title_bar_down: Handle<Image>,
    // pscharacterselect_europe.txt:1010/991 (character list + region board)
    #[asset(path = "media://interface/outer/blackbar_up_europe.ddj")]
    pub blackbar_up: Handle<Image>,
    #[asset(path = "media://interface/outer/blackbar_down_europe.ddj")]
    pub blackbar_down: Handle<Image>,
    // The create screen has **two** trees, one per race, and they name
    // different bands: `pscharactercreate_europe.txt:253/234` (European) vs
    // `pscharactercreatechina.txt:253/234` (Chinese) — the two files are
    // line-aligned, so `GDR_STA_SCREENUP`/`SCREENDOWN` sit on the same lines in
    // both and only their `DDJ=` differs. `_europe` here is the
    // *race*, not the `EUROPE_SYSTEM` `#ifdef` — the two files differ in their
    // very first section header (`Section = CreateEurope` vs
    // `Section = CreateChina`) and in five more arts besides the bands.
    // The name states the *role* ("the ornamental band at the top of the
    // create screen"), not the colour, and the data keep a consistent pairing
    // convention for it: bare name = Chinese, `_europe` = the European
    // counterpart, so a caller swaps one name *part* instead of maintaining two
    // unrelated names. Of the 19 files in this PK2 whose name ends in `_europe`,
    // 15 have exactly that bare counterpart (`button`, `login_window`,
    // `customize_window`, `info`, `pstitle`, `pscharacterselect`, the four
    // `blackbar`s, …).
    // So only the Chinese pair is red and the European one is teal/blue-grey
    // with European scrollwork (both 1600x172 A1R5G5B5 in the DDS header,
    // mid-band RGB 153,28,0 vs 49,83,92, 94% of pixels differ) — that is
    // styling per race, not a misnamed file. Both pairs are therefore declared
    // here. See `chrome::create_bar_art`.
    #[asset(path = "media://interface/outer/redbar_up_europe.ddj")]
    pub redbar_up_europe: Handle<Image>,
    #[asset(path = "media://interface/outer/redbar_down_europe.ddj")]
    pub redbar_down_europe: Handle<Image>,
    #[asset(path = "media://interface/outer/redbar_up.ddj")]
    pub redbar_up_china: Handle<Image>,
    #[asset(path = "media://interface/outer/redbar_down.ddj")]
    pub redbar_down_china: Handle<Image>,

    // character_data
    #[asset(path = "media://server_dep/silkroad/textdata/characterdata.txt")]
    pub character_data: Handle<Textdata>,
    // gates the loading state on leveldata.txt so the exp percentage in the
    // character info box never races the textdata parse
    #[asset(path = "media://server_dep/silkroad/textdata/leveldata.txt")]
    pub level_data: Handle<Textdata>,
    // Same gate, same reason, for the UI string table: every caption of this
    // scene resolves a `UIO_*`/`UIIT_*` key through `ClientUiStrings` with an
    // English fallback, and the screens are built in `OnEnter(SceneState::IntroV2)`
    // — i.e. before a table loaded later would exist. With *this* PK2 the
    // fallbacks are character-identical to the table, so the race is invisible
    // here and would surface only as wrong text on a Korean (or any other)
    // language file. Naming the table in the loading state removes the race
    // instead of relying on that coincidence.
    #[asset(path = "media://server_dep/silkroad/textdata/textuisystem.txt")]
    pub ui_strings: Handle<Textdata>,

    // Server List
    #[asset(path = "media://interface/outer/server_window.ddj")]
    pub server_list_window: Handle<Image>,

    #[asset(path = "media://interface/outer/server_up.ddj")]
    pub server_list_slider_button_up: Handle<Image>,
    #[asset(path = "media://interface/outer/server_up_press.ddj")]
    pub server_list_slider_button_up_press: Handle<Image>,
    #[asset(path = "media://interface/outer/server_up_focus.ddj")]
    pub server_list_slider_button_up_focus: Handle<Image>,

    #[asset(path = "media://interface/outer/server_down.ddj")]
    pub server_list_slider_button_down: Handle<Image>,
    #[asset(path = "media://interface/outer/server_down_press.ddj")]
    pub server_list_slider_button_down_press: Handle<Image>,
    #[asset(path = "media://interface/outer/server_down_focus.ddj")]
    pub server_list_slider_button_down_focus: Handle<Image>,

    #[asset(path = "media://interface/outer/server_mov.ddj")]
    pub server_list_slider_button_mov: Handle<Image>,
    #[asset(path = "media://interface/outer/server_mov_press.ddj")]
    pub server_list_slider_button_mov_press: Handle<Image>,
    #[asset(path = "media://interface/outer/server_mov_focus.ddj")]
    pub server_list_slider_button_mov_focus: Handle<Image>,

    #[asset(path = "media://interface/outer/server_rollover.ddj")]
    pub server_list_item_hover: Handle<Image>,
    #[asset(path = "media://interface/outer/server_select.ddj")]
    pub server_list_item_select: Handle<Image>,

    // Character selection
    #[asset(path = "media://interface/outer/info_europe.ddj")]
    pub info_window: Handle<Image>,
    #[asset(path = "media://interface/outer/hp.ddj")]
    pub hp_bar: Handle<Image>,
    #[asset(path = "media://interface/outer/mp.ddj")]
    pub mp_bar: Handle<Image>,
    #[asset(path = "media://interface/outer/warning_delete.ddj")]
    pub warning_delete_window: Handle<Image>,
    /// `GDR_STA_WCREATE` (`pscharactercreate{china,_europe}.txt:25-43`), the
    /// create-confirm modal's 248x128 frame — the create screen's sibling of
    /// `warning_delete.ddj`, which char-select already uses.
    #[asset(path = "media://interface/outer/warning_create.ddj")]
    pub warning_create_window: Handle<Image>,
    #[asset(path = "media://interface/outer/warning_button.ddj")]
    pub warning_button: Handle<Image>,
    #[asset(path = "media://interface/outer/warning_button_focus.ddj")]
    pub warning_button_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/warning_button_press.ddj")]
    pub warning_button_press: Handle<Image>,

    // Character create chrome — the panels and buttons the screen's own
    // resinfo trees name. Both race trees are read: `pscharactercreatechina`
    // and `pscharactercreate_europe` diverge on exactly five assets, and
    // `customize_window` / `explain-window` are two of them (:139-157 and
    // :101-119 in either file).
    #[asset(path = "media://interface/outer/customize_window.ddj")]
    pub customize_window: Handle<Image>,
    #[asset(path = "media://interface/outer/customize_window_europe.ddj")]
    pub customize_window_europe: Handle<Image>,
    #[asset(path = "media://interface/outer/explain-window.ddj")]
    pub explain_window: Handle<Image>,
    #[asset(path = "media://interface/outer/explain-window_02.ddj")]
    pub explain_window_02: Handle<Image>,
    // `GDR_BTN_MALE` / `GDR_BTN_FEMALE` (:390-408 / :371-389): the selected
    // state is a TEXTURE swap (`man_on` <-> `man_off`), not a text colour —
    // the data gives both captions the same `255,249,212`.
    #[asset(path = "media://interface/outer/man_on.ddj")]
    pub man_on: Handle<Image>,
    #[asset(path = "media://interface/outer/man_on_focus.ddj")]
    pub man_on_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/man_on_press.ddj")]
    pub man_on_press: Handle<Image>,
    #[asset(path = "media://interface/outer/man_off.ddj")]
    pub man_off: Handle<Image>,
    #[asset(path = "media://interface/outer/man_off_focus.ddj")]
    pub man_off_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/man_off_press.ddj")]
    pub man_off_press: Handle<Image>,
    #[asset(path = "media://interface/outer/woman_on.ddj")]
    pub woman_on: Handle<Image>,
    #[asset(path = "media://interface/outer/woman_on_focus.ddj")]
    pub woman_on_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/woman_on_press.ddj")]
    pub woman_on_press: Handle<Image>,
    #[asset(path = "media://interface/outer/woman_off.ddj")]
    pub woman_off: Handle<Image>,
    #[asset(path = "media://interface/outer/woman_off_focus.ddj")]
    pub woman_off_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/woman_off_press.ddj")]
    pub woman_off_press: Handle<Image>,
    // `GDR_BTN_CHECK` (:409-427) — an exact-fit 75x25 texture, rather than the
    // 91x30 generic login button.
    #[asset(path = "media://interface/outer/overlap.ddj")]
    pub overlap: Handle<Image>,
    #[asset(path = "media://interface/outer/overlap_focus.ddj")]
    pub overlap_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/overlap_press.ddj")]
    pub overlap_press: Handle<Image>,

    // `Section = Slider` (`:685-744`) — ONE reusable template that serves all
    // five option rows: the thumb and the two arrows, each with its
    // `_focus`/`_press` variants.
    #[asset(path = "media://interface/outer/slider.ddj")]
    pub slider_thumb: Handle<Image>,
    #[asset(path = "media://interface/outer/slider_focus.ddj")]
    pub slider_thumb_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/slider_press.ddj")]
    pub slider_thumb_press: Handle<Image>,
    #[asset(path = "media://interface/outer/arrow_left.ddj")]
    pub slider_arrow_left: Handle<Image>,
    #[asset(path = "media://interface/outer/arrow_left_focus.ddj")]
    pub slider_arrow_left_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/arrow_left_press.ddj")]
    pub slider_arrow_left_press: Handle<Image>,
    #[asset(path = "media://interface/outer/arrow_right.ddj")]
    pub slider_arrow_right: Handle<Image>,
    #[asset(path = "media://interface/outer/arrow_right_focus.ddj")]
    pub slider_arrow_right_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/arrow_right_press.ddj")]
    pub slider_arrow_right_press: Handle<Image>,

    // Character create — `Section = Rotate` of
    // `resinfo/pscharactercreate{china,_europe}.txt` (both trees byte-identical
    // here, :120-138 for the window and :626-682 for the three buttons). Every
    // button ships `_focus`/`_press`, so they use the same three-state style as
    // the rest of the intro.
    #[asset(path = "media://interface/outer/rotate_window.ddj")]
    pub rotate_window: Handle<Image>,
    #[asset(path = "media://interface/outer/rotate_left.ddj")]
    pub rotate_left: Handle<Image>,
    #[asset(path = "media://interface/outer/rotate_left_focus.ddj")]
    pub rotate_left_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/rotate_left_press.ddj")]
    pub rotate_left_press: Handle<Image>,
    #[asset(path = "media://interface/outer/rotate_right.ddj")]
    pub rotate_right: Handle<Image>,
    #[asset(path = "media://interface/outer/rotate_right_focus.ddj")]
    pub rotate_right_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/rotate_right_press.ddj")]
    pub rotate_right_press: Handle<Image>,
    #[asset(path = "media://interface/outer/zoomin.ddj")]
    pub zoomin: Handle<Image>,
    #[asset(path = "media://interface/outer/zoomin_focus.ddj")]
    pub zoomin_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/zoomin_press.ddj")]
    pub zoomin_press: Handle<Image>,
    #[asset(path = "media://interface/outer/zoomout.ddj")]
    pub zoomout: Handle<Image>,
    #[asset(path = "media://interface/outer/zoomout_focus.ddj")]
    pub zoomout_focus: Handle<Image>,
    #[asset(path = "media://interface/outer/zoomout_press.ddj")]
    pub zoomout_press: Handle<Image>,

    // Join / loading overlay of the lobby — `Section = Loading` of
    // `resinfo/pscharacterselect_europe.txt`, four rect-bearing controls, all
    // four arts present in `Media.pk2`:
    //
    // | control | id | line | Rect | art (WxH) |
    // |---|---|---|---|---|
    // | `GDR_LOADING` | 22 | :364 | `0,0,1600,1200` | `loading_charactercustom_europe.ddj` 1024x768 |
    // | `GDR_LOADINGFRAME` | 23 | :345 | `241,973,1121,64` | `loading_form.ddj` 720x40 |
    // | `GDR_LOADINGG` (`CIFGauge`) | 24 | :326 | `268,985,1064,20` | `gauge_loading.ddj` 4x12 |
    // | `GDR_LOADING_STA` | 27 | :307 | `268,1025,252,35` | `nowloading.ddj` 144x20 |
    //
    // The gauge and the "now loading" strip are the reason the art sizes look
    // wrong next to the rects: `gauge_loading.ddj` is a 4x12 fill tile stretched
    // across its 1064px bar, and every one of the four is `Style=1`, i.e.
    // canvas-scaled from the 1600x1200 design space.
    #[asset(path = "media://interface/loading/loading_charactercustom_europe.ddj")]
    pub join_loading_background: Handle<Image>,
    #[asset(path = "media://interface/loading/loading_form.ddj")]
    pub join_loading_frame: Handle<Image>,
    #[asset(path = "media://interface/loading/gauge_loading.ddj")]
    pub join_loading_gauge: Handle<Image>,
    #[asset(path = "media://interface/loading/nowloading.ddj")]
    pub join_loading_label: Handle<Image>,

    // Deletion countdown window — `GDR_STA_REMAINTIME`
    // (`resinfo/pscharacterselect_europe.txt:687-691`) plus the two gauge
    // controls of `Section = Remain` (`:1071-1075` `GDR_REMAINGBOX`,
    // `:1090-1094` `GDR_REMAING`, a `CIFGauge`). Both gauge controls carry the
    // same `Rect="24,63,280,8"`, i.e. the fill art and the frame art are laid
    // over one another; `_up` is the overlay ("up" = upper layer), which is how
    // it is drawn in `character_select::spawn_deletion_countdown`.
    // All three arts are present in `Media.pk2`
    // (`interface/outer/delete_time_window.ddj`, `delete_time_gauge.ddj`,
    // `delete_time_gauge_up.ddj`).
    #[asset(path = "media://interface/outer/delete_time_window.ddj")]
    pub delete_time_window: Handle<Image>,
    #[asset(path = "media://interface/outer/delete_time_gauge.ddj")]
    pub delete_time_gauge: Handle<Image>,
    #[asset(path = "media://interface/outer/delete_time_gauge_up.ddj")]
    pub delete_time_gauge_up: Handle<Image>,

    /// Loaded but not drawn by the lobby, because the data says it should not
    /// be: no `resinfo` file references `loading_default.ddj`, while all four
    /// controls of the lobby's own `Section = Loading` name the arts above.
    /// `loading_default` is the *world* loading screen's fallback picture (the
    /// `loading_<region>.ddj` family, one per zone), so it is not a near-miss
    /// for the same art — it is a different screen's background. The handle
    /// stays loaded for the world loading screen that will need exactly this
    /// fallback.
    #[asset(path = "media://interface/loading/loading_default.ddj")]
    pub loading_background: Handle<Image>,

    // Captcha
    #[asset(path = "media://interface/outer/imagecode_window.ddj")]
    pub captcha_window: Handle<Image>,
    #[asset(path = "media://interface/ifcommon/com_mid_button.ddj")]
    pub captcha_confirm_button: Handle<Image>,
    #[asset(path = "media://interface/ifcommon/com_mid_button_focus.ddj")]
    pub captcha_confirm_button_focus: Handle<Image>,
    #[asset(path = "media://interface/ifcommon/com_mid_button_press.ddj")]
    pub captcha_confirm_button_press: Handle<Image>,

    // Sounds — **fallback only**, not the truth about these four sounds.
    //
    // `effectsound.txt` names each of them under a handle and adds the volume
    // the data wants (80), which a fixed path cannot carry; every playback
    // that can reach the table goes through `plugins::audio_events::play`
    // instead (the button click already does, `plugins/ui_v2/mod.rs`). These
    // fields survive because `AssetCollection` needs a literal at compile time
    // and `UiV2Plugin` runs from frame 1, before the textdata tables load.
    // The four paths are pinned against the table by
    // `plugins::audio_events::INTRO_FALLBACK_SOUNDS` so the two truths cannot
    // drift apart unnoticed.
    //
    // The paths are written **all-lowercase on purpose**, and that is not a
    // transcription slip: the shipped data disagrees with itself. The table row
    // is `ui\Error.wav` with a capital E (in both `resinfo/effectsound.txt` and
    // `server_dep/silkroad/textdata/effectsound.txt`), while the file the
    // archive actually ships is `Prim/snd/ui/error.wav` — capital P, lowercase
    // e. Neither spelling is "the" name. Two normalizations make both work and
    // are what these literals are written against: the row parser lowercases
    // (`assets::textdata::effectsound::EffectSound::path`) and `bevy_pk2`
    // indexes case-insensitively (`bevy_pk2/src/pk2/archive.rs:66`). So do not
    // "fix" `error.wav` to the table's `Error.wav`: that would match the column
    // and diverge from the parsed path these fields exist to mirror.
    #[asset(path = "data://prim/snd/ui/uibutton_a.wav")]
    pub sound_button_sound_a: Handle<AudioSource>,
    #[asset(path = "data://prim/snd/ui/uiwinopen.wav")]
    pub sound_window_open: Handle<AudioSource>,
    // **Deliberately loaded but not played anywhere in the pregame.** In the
    // original the window-close sound belongs to the `IFxxx` HUD window classes;
    // the pregame never plays it, which is why `server_select.rs` does not stack
    // `uiwinclose.wav` on its button click. The pregame's plates are `CIFStatic`
    // art, not closable windows, so wiring this up would invent a sound the
    // original does not play. The handle stays because the collection mirrors
    // the four UI sounds the intro's asset set declares; if a pregame window
    // ever *does* close, it needs a call site plus a reason, not just this field.
    #[asset(path = "data://prim/snd/ui/uiwinclose.wav")]
    pub sound_window_close: Handle<AudioSource>,
    #[asset(path = "data://prim/snd/ui/error.wav")]
    pub sound_error: Handle<AudioSource>,
}

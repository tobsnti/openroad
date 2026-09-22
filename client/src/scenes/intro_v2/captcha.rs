use std::io::Read;

use bevy::asset::RenderAssetUsages;
use bevy::input_focus::tab_navigation::TabGroup;
use bevy::input_focus::{FocusCause, InputFocus};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::text::{EditableText, EditableTextFilter, FontSourceTemplate};
use bevy::ui_widgets::Activate;
use image::EncodableLayout;
use libflate::zlib::Decoder;

use packets::login::{
    LoginCaptchaChallenge, LoginCaptchaConfirmRequest, LoginCaptchaConfirmResponse, WrongAttempt,
};
use packets::Packet;

use crate::assets::FontAssets;
use crate::net::connection::SilkroadConnection;
use crate::plugins::hud::modal_dialog::{modal_plate, modal_scrim};
use crate::plugins::net::gateway::GatewayConnection;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::textdata::ClientUiStrings;
use crate::plugins::ui_v2::style::ImageButtonStyle;
use crate::plugins::ui_v2::widgets::{image_button, label, text_input};

use super::assets::IntroV2Assets;
use super::chrome::InfoTextV2Update;
use super::{intro_font_px, IntroV2Ui};

/// Upper bound on the inflated bitmap we are willing to expand, so a hostile or
/// broken server cannot make us allocate arbitrarily. 64x the stock IBUV image,
/// which is 200x64 at 1bpp = 1600 bytes (`docs/net-login-gateway.md`).
const MAX_DECOMPRESSED_IMAGE_SIZE: usize = (200 * 64 / 8) * 64;

/// Expand 0x2322's payload into RGBA pixels.
///
/// The blob is a zlib stream of a 1-bit-per-pixel bitmap, LSB first within each
/// byte, white on opaque black. The inflated length is **derived** from the
/// packet's own `image_width`/`image_height` as `width * height / 8`, so a server
/// sending a differently sized image renders instead of crashing the client.
///
/// It is deliberately *not* taken from the packet's `unk_0x32c8` field: that
/// field is a constant `0x32C8` (13000) while the payloads inflate to exactly
/// 1600 bytes at 200x64, so it does not carry the size despite its old name.
///
/// Returns `None` for any payload whose dimensions or inflated length do not
/// agree — the modal then simply does not appear.
fn decode_captcha_pixels(challenge: &LoginCaptchaChallenge) -> Option<Vec<u8>> {
    let width = challenge.image_width as usize;
    let height = challenge.image_height as usize;

    if width == 0 || height == 0 {
        warn!("captcha: degenerate image size {width}x{height}");
        return None;
    }
    if !(width * height).is_multiple_of(8) {
        warn!("captcha: {width}x{height} is not a whole number of 1bpp bytes");
        return None;
    }
    let uncompressed = width * height / 8;
    if uncompressed > MAX_DECOMPRESSED_IMAGE_SIZE {
        warn!("captcha: refusing {uncompressed}-byte bitmap");
        return None;
    }

    let mut inflated = Vec::with_capacity(uncompressed);
    let decoder = match Decoder::new(challenge.image_data.as_bytes()) {
        Ok(decoder) => decoder,
        Err(e) => {
            warn!("captcha: not a zlib stream: {e}");
            return None;
        }
    };
    if let Err(e) = decoder.take(uncompressed as u64).read_to_end(&mut inflated) {
        warn!("captcha: failed to inflate image: {e}");
        return None;
    }
    if inflated.len() != uncompressed {
        warn!(
            "captcha: inflated {} bytes, header says {uncompressed}",
            inflated.len()
        );
        return None;
    }

    let mut pixel_data = Vec::with_capacity(width * height * 4);
    for byte in inflated {
        for bit in 0..8 {
            if byte & (1 << bit) > 0 {
                pixel_data.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
            } else {
                pixel_data.extend_from_slice(&[0, 0, 0, 0xff]);
            }
        }
    }

    Some(pixel_data)
}

/// textuisystem keys the original's `ifconfirmbox.txt` binds to this dialog's
/// three labels (`GDR_STA_CAPTION:78`, `GDR_STA_EXPLAIN:116`,
/// `GDR_BTN_ACCEPT:59`); the fallbacks mirror the shipped English rows.
const CAPTION_KEY: &str = "UIIT_PAG_GLOBAL_AUTHENTICATION";
const NOTICE_KEY: &str = "UIIT_STT_GLOBAL_AUTHENTICATION_NOTICE";
const CONFIRM_KEY: &str = "UIIS_CTL_CONFIRM";
/// The wrong-attempt line of this dialog:
/// `Media/server_dep/silkroad/textdata/textuisystem.txt:3970` carries
/// `UIIT_STT_GLOBAL_AUTHENTICATION_INPUT_ERROR` = "Image code entry has failed
/// %d out of %d times." — the row directly above the password twin
/// `UIIT_STT_GLOBAL_PASSWORD_INPUT_ERROR` (`:3971`) that `net.rs` uses.
///
/// **Fill order is the wire order, `(max_attempts, cur_attempts)`.** The
/// rejection is `02 | 06000000 | 01000000` on the wire, the struct's field order
/// is `max_attempts` then `cur_attempts` (`packets/src/login.rs`), and the
/// original client puts the *first* wire field in the *first* `%d`: its status
/// line reads **"Image code entry has failed 6 out of 1 times."** So the
/// sentence is odd, but it is the original's sentence, and the original is the
/// tie-breaker wherever we have no stated reason to depart.
///
/// The password twin in [`super::net`] *does* depart here on purpose and says
/// so in one place; that rationale is not restated for this dialog, so this one
/// follows the data instead of silently inheriting a deviation.
const ATTEMPTS_KEY: &str = "UIIT_STT_GLOBAL_AUTHENTICATION_INPUT_ERROR";

/// Native size of the captcha box art (`ifconfirmbox.txt`'s plate). The shared
/// shell owns the chrome, not this dialog's geometry.
const CAPTCHA_BOX_W: f32 = 400.0;
const CAPTCHA_BOX_H: f32 = 180.0;

/// Ink of this dialog's two coloured labels. resinfo `FontColor` is **ARGB**,
/// so the leading `255` is the alpha and the remaining three are the RGB:
///
/// * `GDR_STA_CAPTION` `FontColor=COLOR,"255,230,218,161"` (`ifconfirmbox.txt:71`)
/// * `GDR_BTN_ACCEPT`  `FontColor=COLOR,"255,255,249,211"` (`ifconfirmbox.txt:52`)
///
/// `Color::WHITE` is neither the data nor what the original draws: its dialog
/// paints the title glyphs in `rgb(230,218,161)` and the Confirm label in
/// `rgb(255,249,211)`, with no white in either (the explanation text below *is*
/// white, and stays white).
const CAPTION_COLOR: Color = Color::srgb_u8(230, 218, 161);
const CONFIRM_LABEL_COLOR: Color = Color::srgb_u8(255, 249, 211);

/// The decoded captcha image; its presence triggers the modal to spawn.
#[derive(Resource)]
pub struct CaptchaImageV2(pub Handle<Image>);

/// Root marker of the captcha modal.
#[derive(Component, Default, Clone)]
pub struct CaptchaModal;

/// Marker on the captcha code `EditableText`.
#[derive(Component, Default, Clone)]
pub struct CaptchaInput;

/// Marker on the modal's Confirm button, so the Enter path can activate the
/// very button the mouse activates instead of duplicating its observer.
#[derive(Component, Default, Clone)]
pub struct CaptchaConfirmButton;

pub fn on_captcha_challenge(
    mut events: MessageReader<LoginCaptchaChallenge>,
    mut commands: Commands,
    mut image_assets: ResMut<Assets<Image>>,
) {
    for event in events.read() {
        let Some(pixel_data) = decode_captcha_pixels(event) else {
            continue;
        };

        let img = Image::new_fill(
            Extent3d {
                width: event.image_width as u32,
                height: event.image_height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &pixel_data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );

        let image_handle = image_assets.add(img);

        commands.insert_resource(CaptchaImageV2(image_handle));
    }
}

fn confirm_button_style(assets: &IntroV2Assets) -> ImageButtonStyle {
    ImageButtonStyle {
        normal: assets.captcha_confirm_button.clone(),
        hover: assets.captcha_confirm_button_focus.clone(),
        press: assets.captcha_confirm_button_press.clone(),
        ..Default::default()
    }
}

fn captcha_modal(
    captcha_image: Handle<Image>,
    assets: &IntroV2Assets,
    fonts: &FontAssets,
    ui_strings: &ClientUiStrings,
) -> impl Scene {
    let caption = ui_strings
        .get_or(CAPTION_KEY, "Image code verification")
        .to_string();
    // The notice row is authored for the original's `CIFPML` rich-text control,
    // so it carries markup; `get_plain_or` reduces it (see `textdata::plain_text`).
    let notice = ui_strings.get_plain_or(
        NOTICE_KEY,
        "To prevent auto creation,\nplease enter the number/text as it appears.",
    );
    let confirm = ui_strings.get_or(CONFIRM_KEY, "Confirm").to_string();
    let window = assets.captcha_window.clone();
    let caption_font = fonts.nine.clone();
    let description_font = fonts.nine.clone();
    let input_font = fonts.nine.clone();
    let confirm_font = fonts.nine.clone();
    // One size for the whole box: its single resinfo host
    // `GDR_CMB_CONFIRMBOX` (`pstitle.txt:6`, section `Validate`) is
    // `FontIndex=0` -> 12 px on the ladder (`intro_font_px`), and the box has no
    // other authored control to read a second index from.
    let text_px = intro_font_px(0);
    // Confirm is a button, so it clicks: `resinfo/effectsound.txt` maps
    // `SND_BUTTON_CLICK` -> `ui\uibutton_a.wav` (`:52`, twin `uibutton_b.wav`
    // `:53`) while `SND_ERROR` -> `ui\Error.wav` sits at `:56`. A successful
    // captcha submit is not an error, so it plays the click cue. The
    // wrong-attempt reply keeps `sound_error` in
    // `on_captcha_confirm_response`, which is the event that is a failure.
    let confirm_sound = assets.sound_button_sound_a.clone();

    // The host entry `pstitle.txt:6` `GDR_CMB_CONFIRMBOX:CIFConfirmbox` (id 47,
    // section `Validate`) has a degenerate `Rect="0,0,1,1"`, so the original
    // positions this box from code and the data says nothing about where it goes.
    // Centring it over a dimmed screen is therefore an openroad convention, not a
    // reproduction; only the 400x180 box art and the child rects below come from
    // the original.
    bsn! {
        modal_scrim()
        CaptchaModal
        Name("Captcha Modal V2")
        // MODAL tab group, not `TabGroup::new(0)`. Bevy's rule is explicit
        // (`bevy_input_focus::tab_navigation::TabGroup`): a non-modal group
        // tabs through *all* non-modal groups, so with `new(0)` a Tab in the
        // code field walks straight out of the box and into the login form's
        // ID/PW/Connect/Exit behind the scrim — controls the original itself
        // greys out while this box is up (see
        // `login_form::lock_exit_while_captcha_is_open`). Modality of the tab
        // ring is the keyboard half of that same behaviour.
        TabGroup::modal()
        Children [
            (
                modal_plate(window, CAPTCHA_BOX_W, CAPTCHA_BOX_H)
                Children [
                    (
                        Text({caption})
                        TextFont { font: FontSourceTemplate::Handle({caption_font}), font_size: {FontSize::Px(text_px)} }
                        TextColor({CAPTION_COLOR})
                        // `HAlign=1` on `GDR_STA_CAPTION` (`ifconfirmbox.txt:73`)
                        TextLayout::justify(Justify::Center)
                        Node {
                            position_type: PositionType::Absolute,
                            align_self: AlignSelf::Center,
                            justify_content: JustifyContent::Center,
                            width: px(229),
                            height: px(15),
                            left: px(85),
                            top: px(18),
                        }
                    ),
                    (
                        Text({notice})
                        TextFont { font: FontSourceTemplate::Handle({description_font}), font_size: {FontSize::Px(text_px)} }
                        // `GDR_STA_EXPLAIN` really is white: `FontColor` is
                        // `255,255,255,255` (`ifconfirmbox.txt:109`), and the
                        // original draws it as RGB(255,255,255).
                        TextColor(Color::WHITE)
                        // `HAlign=1` on `GDR_STA_EXPLAIN` (`ifconfirmbox.txt:111`)
                        TextLayout::justify(Justify::Center)
                        Node {
                            position_type: PositionType::Absolute,
                            align_self: AlignSelf::Center,
                            justify_content: JustifyContent::Center,
                            width: px(336),
                            height: px(40),
                            left: px(32),
                            top: px(40),
                        }
                    ),
                    (
                        ImageNode { image: {captcha_image} }
                        Node {
                            position_type: PositionType::Absolute,
                            width: px(200),
                            height: px(64),
                            top: px(80),
                            left: px(33),
                        }
                    ),
                    (
                        text_input(input_font, 0)
                        CaptchaInput
                        EditableTextFilter::new(char::is_alphanumeric)
                        Node {
                            position_type: PositionType::Absolute,
                            align_self: AlignSelf::Center,
                            width: px(95),
                            height: px(15),
                            left: px(260),
                            top: px(88),
                        }
                    ),
                    (
                        image_button(confirm_button_style(assets), 55.0, 18.0)
                        CaptchaConfirmButton
                        Node {
                            position_type: PositionType::Absolute,
                            width: px(55),
                            height: px(18),
                            top: px(121),
                            left: px(257),
                        }
                        Children [
                            // label() starts transparent for the fade systems,
                            // which never run on this modal, so the colour has
                            // to be forced here.
                            (
                                label(&confirm, confirm_font, text_px)
                                TextColor({CONFIRM_LABEL_COLOR})
                            ),
                        ]
                        on(move |_activate: On<Activate>,
                            connection_query: Query<&SilkroadConnection, With<GatewayConnection>>,
                            input_query: Query<&EditableText, With<CaptchaInput>>,
                            options: Res<GameOptions>,
                            mut commands: Commands| {
                            let Ok(connection) = connection_query.single() else {
                                return;
                            };
                            let Ok(input) = input_query.single() else {
                                return;
                            };

                            let frame = Packet::from(LoginCaptchaConfirmRequest {
                                code: input.value().to_string(),
                            })
                            .into();

                            connection
                                .get_sender()
                                .send(frame)
                                .expect("failed to send LoginCaptchaConfirmRequest");

                            // The window deliberately stays up until the server
                            // answers: tearing it down on the *click* would
                            // leave a rejected code nothing to retype into
                            // while `on_captcha_confirm_response` counts "failed
                            // %d out of %d times" — a retry counter with no
                            // retry. `on_captcha_confirm_response` closes it on
                            // acceptance and clears the field on rejection.
                            if let Some(playback) = options.audio.fx_playback() {
                                commands.spawn((
                                    AudioPlayer::new(confirm_sound.clone()),
                                    playback,
                                ));
                            }
                        })
                    ),
                ]
            ),
        ]
    }
}

pub fn spawn_captcha(
    captcha: Res<CaptchaImageV2>,
    assets: Res<IntroV2Assets>,
    fonts: Res<FontAssets>,
    ui_strings: Res<ClientUiStrings>,
    mut commands: Commands,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Some(camera) = cam_query.iter().next() else {
        warn!("no 2d camera found");
        return;
    };

    commands
        .spawn_scene(captcha_modal(
            captcha.0.clone(),
            &assets,
            &fonts,
            &ui_strings,
        ))
        .insert((UiTargetCamera(camera), IntroV2Ui));
}

/// Fills the wrong-attempt row with the rejection's two counters.
///
/// Its own function so the order can be pinned by a test; inline in the system
/// a swapped pair is invisible. See [`ATTEMPTS_KEY`] for why the order is
/// `(max, cur)`.
fn wrong_attempt_line(template: &str, wrong_attempt: &WrongAttempt) -> String {
    super::fill_placeholders(
        template,
        &[wrong_attempt.max_attempts, wrong_attempt.cur_attempts],
    )
}

/// The answer to a submitted code. **This** is where the window closes.
///
/// Accepted (`wrong_attempt: None`) tears the modal down and drops the image
/// resource; rejected keeps the modal, states the attempt count, plays
/// `snd_error` and empties the field so the next code can simply be typed.
///
/// Unknown: whether a real gateway sends a **new** `0x2322` after a rejection.
/// If it does, the fresh image replaces this one through `spawn_captcha`'s
/// `resource_added` gate (which is why the resource is dropped on *acceptance*
/// only, not on every answer). If it does not, the user retypes the same image,
/// which is what the original's own retry counter implies. Both cases are
/// handled.
pub fn on_captcha_confirm_response(
    mut events: MessageReader<LoginCaptchaConfirmResponse>,
    mut info_text_writer: MessageWriter<InfoTextV2Update>,
    mut commands: Commands,
    assets: Res<IntroV2Assets>,
    options: Res<GameOptions>,
    ui_strings: Res<ClientUiStrings>,
    modal_query: Query<Entity, With<CaptchaModal>>,
    mut input_query: Query<(Entity, &mut EditableText), With<CaptchaInput>>,
    mut focus: ResMut<InputFocus>,
) {
    for event in events.read() {
        match &event.wrong_attempt {
            None => {
                commands.remove_resource::<CaptchaImageV2>();
                for modal in modal_query.iter() {
                    commands.entity(modal).despawn();
                }
            }
            Some(wrong_attempt) => {
                let template = ui_strings.get_plain_or(
                    ATTEMPTS_KEY,
                    "Image code entry has failed %d out of %d times.",
                );
                info_text_writer.write(InfoTextV2Update(wrong_attempt_line(
                    &template,
                    wrong_attempt,
                )));
                super::play_error_sound(&mut commands, &assets, &options);
                // Empty the field and put the caret back in it: the rejected
                // code is worthless, and the user should not have to select it
                // before retyping.
                if let Ok((entity, mut text)) = input_query.single_mut() {
                    text.clear();
                    focus.set(entity, FocusCause::Navigated);
                }
            }
        }
    }
}

/// Puts the caret in the code field the moment the modal exists.
///
/// `spawn_captcha` builds the modal through `spawn_scene`, which **defers**
/// entity creation, so the input entity does not exist yet inside that system —
/// the established answer in this tree is a follow-up system on `Added<...>`
/// (`hud/player_mini_info.rs:826 wire_cinfo_button`,
/// `hud/target_window.rs:436`). That is what this is.
///
/// **Deliberate affordance, not a reproduction.** The original's focus behaviour
/// on this dialog is unknown: the host block `GDR_CMB_CONFIRMBOX`
/// (`resinfo/pstitle.txt:6`) carries no default focus field. But the modal is
/// *the* only thing the screen accepts input for while it is up —
/// `lock_exit_while_captcha_is_open` and the in-flight Connect grey out
/// everything else — so the one field it has is the only sensible focus target.
/// The rejected case does the same (`on_captcha_confirm_response` re-focuses
/// the emptied field), so the *first* code does not have to be clicked into
/// either.
pub fn focus_captcha_input(
    fresh: Query<Entity, Added<CaptchaInput>>,
    mut focus: ResMut<InputFocus>,
) {
    if let Some(entity) = fresh.iter().next() {
        focus.set(entity, FocusCause::Navigated);
    }
}

/// Enter in the code field confirms the code.
///
/// Same shape and same rationale as [`super::login_form::submit_on_enter`] one
/// screen earlier — that pattern extended to this modal, not a second
/// mechanism: `bevy_ui_widgets`' button already handles Enter when the *button*
/// holds focus, so the only gap is Enter while the caret is in the edit row,
/// which is where a code is actually typed. It goes through `Activate` on the
/// real Confirm button so the send, the sound and the "window stays up until
/// the server answers" rule all stay in the one observer the mouse uses.
///
/// The guard is the focus itself: only a caret sitting in `CaptchaInput`
/// submits, so Enter on some other focused widget still does that widget's own
/// thing.
pub fn confirm_captcha_on_enter(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    input_query: Query<(), With<CaptchaInput>>,
    confirm_query: Query<Entity, With<CaptchaConfirmButton>>,
    mut commands: Commands,
) {
    if !keys.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]) {
        return;
    }
    let Some(focused) = focus.get() else {
        return;
    };
    if input_query.get(focused).is_err() {
        return;
    }
    if let Ok(confirm) = confirm_query.single() {
        commands.trigger(Activate { entity: confirm });
    }
}

/// `OnExit(LoginForm)`: the captcha window is a root of its own on the 2d
/// camera, so nothing in the login form's own teardown reaches it. Leaving the
/// form with a captcha still up (a scene restart, or an accepted login racing
/// the state change) would leave a modal scrim over the next screen.
pub fn despawn_captcha_modal(query: Query<Entity, With<CaptchaModal>>, mut commands: Commands) {
    for entity in query.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<CaptchaImageV2>();
}

#[cfg(test)]
mod test {
    use std::io::Write;

    use libflate::zlib::Encoder;

    use super::*;
    use crate::assets::textdata::uisystem::UiSystemText;
    use crate::plugins::textdata::plain_text;

    // ---- Focus and Enter inside the modal ---------------------------------
    //
    // Both systems are driven with plain marker entities: the geometry comes
    // from `captcha_modal`, but the *behaviour* under test is only "which
    // entity holds focus" and "does the real Confirm button get `Activate`",
    // and a `bsn!` scene would drag the whole asset/font stack into a headless
    // fixture for nothing.

    /// Records every `Activate` the systems fire, so the assertion does not
    /// depend on Bevy's message buffer surviving an `app.update()`.
    #[derive(Resource, Default)]
    struct Activations(Vec<Entity>);

    fn enter_app() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<InputFocus>()
            .init_resource::<Activations>()
            .add_systems(Update, (focus_captcha_input, confirm_captcha_on_enter))
            .add_observer(|activate: On<Activate>, mut seen: ResMut<Activations>| {
                seen.0.push(activate.entity);
            });
        let input = app.world_mut().spawn(CaptchaInput).id();
        let confirm = app.world_mut().spawn(CaptchaConfirmButton).id();
        (app, input, confirm)
    }

    fn press_enter(app: &mut App) {
        // `reset` before `press`: without an `InputPlugin` nothing clears the
        // just-pressed set between frames (same reasoning as `chat/input.rs`).
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .reset(KeyCode::Enter);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();
    }

    /// The code field takes focus the moment it exists, so the first code can be
    /// typed without clicking into the field.
    #[test]
    fn the_code_field_takes_focus_as_soon_as_it_exists() {
        let (mut app, input, _confirm) = enter_app();
        assert_eq!(app.world().resource::<InputFocus>().get(), None);

        app.update();

        assert_eq!(app.world().resource::<InputFocus>().get(), Some(input));
    }

    /// Enter in the code field activates the *real* Confirm button, so the
    /// send, the sound and "the window stays up until the server answers" all
    /// stay in the one observer the mouse uses.
    #[test]
    fn enter_in_the_code_field_activates_the_confirm_button() {
        let (mut app, _input, confirm) = enter_app();
        app.update(); // focus lands in the field

        press_enter(&mut app);

        assert_eq!(app.world().resource::<Activations>().0, vec![confirm]);
    }

    /// Negative control: the same key press with the focus somewhere else must
    /// fire nothing. Without the focus guard in `confirm_captcha_on_enter` this
    /// test goes red, which is what makes the green one above a proof instead
    /// of a coincidence.
    #[test]
    fn enter_outside_the_code_field_confirms_nothing() {
        let (mut app, _input, _confirm) = enter_app();
        app.update();
        let elsewhere = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(elsewhere, FocusCause::Navigated);

        press_enter(&mut app);

        assert!(app.world().resource::<Activations>().0.is_empty());
    }

    /// And no key press at all must not confirm either — the guard that would
    /// otherwise be invisible if `any_just_pressed` were ever dropped.
    #[test]
    fn a_frame_without_enter_confirms_nothing() {
        let (mut app, _input, _confirm) = enter_app();
        app.update();
        app.update();

        assert!(app.world().resource::<Activations>().0.is_empty());
    }

    /// A 0x2322 payload in the stock shape the gateway sends: a zlib stream of
    /// `width * height / 8` bytes at 1bpp. `unk_0x32c8` carries its usual
    /// constant, so a decoder that still trusted it would fail here.
    fn challenge(width: u16, height: u16, bitmap: &[u8]) -> LoginCaptchaChallenge {
        let mut encoder = Encoder::new(Vec::new()).expect("zlib encoder");
        encoder.write_all(bitmap).expect("deflate");
        let image_data = encoder.finish().into_result().expect("zlib stream");

        LoginCaptchaChallenge {
            // Header values as the gateway sends them: flag 0, remain =
            // everything after it.
            image_flag: 0,
            image_remain: (image_data.len() + 8) as u16,
            image_compressed: image_data.len() as u16,
            // The observed constant, which the decoder must ignore.
            unk_0x32c8: 0x32C8,
            image_width: width,
            image_height: height,
            image_data,
        }
    }

    #[test]
    fn decode_captcha_pixels_expands_the_stock_200x64_image() {
        // 1600 bytes: first byte 0b0000_0001 -> only pixel 0 white, rest black.
        let mut bitmap = vec![0u8; 200 * 64 / 8];
        bitmap[0] = 0x01;

        let pixels =
            decode_captcha_pixels(&challenge(200, 64, &bitmap)).expect("stock captcha decodes");

        assert_eq!(pixels.len(), 200 * 64 * 4);
        assert_eq!(&pixels[0..4], &[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(&pixels[4..8], &[0, 0, 0, 0xff]);
    }

    #[test]
    fn decode_captcha_pixels_follows_the_wire_size_not_the_stock_size() {
        let bitmap = vec![0xffu8; 8]; // 8 bytes = 64 pixels = 8x8
        let pixels =
            decode_captcha_pixels(&challenge(8, 8, &bitmap)).expect("non-stock captcha decodes");

        assert_eq!(pixels.len(), 8 * 8 * 4);
        assert!(pixels
            .chunks(4)
            .all(|p| p == [0xff, 0xff, 0xff, 0xff].as_slice()));
    }

    #[test]
    fn decode_captcha_pixels_rejects_inconsistent_or_corrupt_payloads() {
        let bitmap = vec![0u8; 200 * 64 / 8];

        // degenerate dimensions
        assert!(decode_captcha_pixels(&challenge(0, 64, &bitmap)).is_none());
        // dimensions that are not a whole number of 1bpp bytes
        assert!(decode_captcha_pixels(&challenge(3, 1, &bitmap)).is_none());
        // truncated / non-zlib blob
        let mut corrupt = challenge(200, 64, &bitmap);
        corrupt.image_data = vec![0xde, 0xad, 0xbe, 0xef];
        assert!(decode_captcha_pixels(&corrupt).is_none());
        // a stream that inflates to fewer bytes than the dimensions require
        assert!(decode_captcha_pixels(&challenge(200, 64, &bitmap[..800])).is_none());
    }

    /// Rows :11, :3968 and :3969 verbatim from the user's
    /// `media://server_dep/silkroad/textdata/textuisystem.txt` — 10 tab-separated
    /// columns with the English text last.
    const TEXTUISYSTEM_ROWS: &str = concat!(
        "1\tUIIS_CTL_CONFIRM\t\t\t\t\t\t\t\tConfirm\r\n",
        "1\tUIIT_PAG_GLOBAL_AUTHENTICATION\t\t\t\t\t\t\t\tImage code verification\r\n",
        "1\tUIIT_STT_GLOBAL_AUTHENTICATION_NOTICE\t\t\t\t\t\t\t\t",
        "<sml2>To prevent auto creation,<br>please enter the number/text as it appears.</sml2>\r\n",
    );

    #[test]
    fn captcha_keys_resolve_against_the_shipped_string_table() {
        let strings = UiSystemText::parse(TEXTUISYSTEM_ROWS);

        assert_eq!(strings.get(CAPTION_KEY), Some("Image code verification"));
        assert_eq!(strings.get(CONFIRM_KEY), Some("Confirm"));
        assert_eq!(
            plain_text(strings.get(NOTICE_KEY).expect("notice row")),
            "To prevent auto creation,\nplease enter the number/text as it appears."
        );
    }

    #[test]
    fn plain_text_keeps_authored_breaks_and_drops_tags() {
        assert_eq!(
            plain_text("<font color=\"255,0,0,0\">red<br/>line</font>"),
            "red\nline"
        );
        assert_eq!(plain_text("no markup"), "no markup");
        assert_eq!(plain_text("unterminated <tag"), "unterminated <tag");
    }

    // ---- The wrong-attempt status line ------------------------------------

    /// The original: wire `02 | 06000000 | 01000000` makes it print "failed
    /// **6 out of 1** times". The first wire field goes in the first `%d`.
    #[test]
    fn the_wrong_attempt_line_prints_the_first_wire_field_first() {
        let rejection = WrongAttempt {
            max_attempts: 6,
            cur_attempts: 1,
        };

        assert_eq!(
            wrong_attempt_line(
                "Image code entry has failed %d out of %d times.",
                &rejection
            ),
            "Image code entry has failed 6 out of 1 times."
        );
    }

    /// Negative control for the test above: 6 and 1 read the same in either
    /// order only if the two fields are equal, so a second rejection with two
    /// *different* pairs proves the swap is not accidentally symmetric.
    #[test]
    fn the_wrong_attempt_line_is_not_the_reversed_order() {
        let rejection = WrongAttempt {
            max_attempts: 6,
            cur_attempts: 1,
        };

        assert_ne!(
            wrong_attempt_line("%d out of %d", &rejection),
            "1 out of 6",
            "this is the order that stood here and that the original contradicts"
        );
        assert_eq!(wrong_attempt_line("%d out of %d", &rejection), "6 out of 1");
    }

    /// The two colours the data names, so a future "tidy the colours" pass
    /// cannot quietly put `Color::WHITE` back: resinfo `FontColor` is ARGB, and
    /// the original draws both of them.
    #[test]
    fn the_dialog_ink_matches_ifconfirmbox() {
        // GDR_STA_CAPTION FontColor="255,230,218,161" (ifconfirmbox.txt:71)
        assert_eq!(CAPTION_COLOR, Color::srgb_u8(230, 218, 161));
        assert_ne!(CAPTION_COLOR, Color::WHITE);
        // GDR_BTN_ACCEPT FontColor="255,255,249,211" (ifconfirmbox.txt:52)
        assert_eq!(CONFIRM_LABEL_COLOR, Color::srgb_u8(255, 249, 211));
        assert_ne!(CONFIRM_LABEL_COLOR, Color::WHITE);
    }
}

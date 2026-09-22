use std::path::Path;

use bevy::prelude::*;
use bevy_asset_loader::prelude::{ConfigureLoadingState, LoadingStateAppExt, LoadingStateConfig};
use bevy_tweening::TweenAnim;

use crate::assets::char_select_scene::CharSelectScene;
use crate::assets::intro_scene::{IntroOption, IntroScene, DEFAULT_INTRO_BGM};
use crate::assets::textdata::decode::decode_textdata;
use crate::plugins::assets::sro::MediaArchive;
use crate::plugins::camera::{
    despawn_cinematic_camera, disable_camera, enable_camera, spawn_cinematic_camera,
    CinematicCamera, CinematicCamera2,
};
use crate::plugins::config::ClientConfig;
use crate::plugins::map::terrain::Terrain;
use crate::plugins::net::gateway::shard_list::ShardList;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::ui_v2::UiV2Plugin;
use crate::plugins::world_origin::{set_world_origin, WorldOrigin};
use crate::scenes::SceneState;

use assets::IntroV2Assets;
use scene_data::{
    ActiveCharSelectSceneV2, ActiveIntroSceneV2, DesiredCharSelectSceneV2, DesiredIntroSceneV2,
};

pub mod assets;
pub mod captcha;
pub mod character_create;
pub mod character_select;
pub mod chrome;
pub mod dev_fast_login;
pub mod fade;
pub mod login_form;
pub mod model;
pub mod net;
pub mod race_catalog;
pub mod race_stage;
pub mod region_select;
pub mod scene_data;
pub mod server_select;
pub mod splash;

/// Sub-state machine of [`SceneState::IntroV2`]. Created when the scene is
/// entered and removed when it is left.
#[derive(SubStates, Reflect, Debug, Hash, Eq, PartialEq, Default, Copy, Clone)]
#[source(SceneState = SceneState::IntroV2)]
pub enum IntroV2State {
    #[default]
    Loading,
    Splash,
    LoginForm,
    ServerSelection,
    CharacterList,
    /// The original "Region Select" board between the list and creation
    /// (race plates over the stage, resinfo/pscharacterselect.txt §Select).
    RegionSelect,
    /// Character creation. **Deliberately a sub-state of [`SceneState::IntroV2`],
    /// not a `SceneState` of its own** (settled for #645).
    ///
    /// In the original, create *is* its own screen — its own resinfo tree
    /// `pscharactercreate{china,_europe}.txt` with its own red chrome
    /// (`docs/re/ui/scene-intro-character-create.md` §3, honoured by
    /// [`chrome::bar_tree`]). But a screen is not a scene: that tree draws
    /// `GDR_CWND_CHARACTER 0,0,1600,1200` over the *same* char-select stage, so
    /// creation shares this scene's terrain, world origin, cinematic camera and
    /// live agent connection (`character_select::disconnect_from_agent_server`
    /// is held across the whole sub-flow on purpose). A separate `SceneState`
    /// tears all four down and rebuilds them, which changes behaviour rather
    /// than fixing it — the same call #372 made for the world scenes.
    ///
    /// The RE data pushes the same way: `docs/re/ui/scene-intro-region-select.md`
    /// §6 names "our code models region select as a separate scene state" the
    /// *substantive defect* of that module. The direction of travel here is
    /// fewer states over one stage, not more.
    CharacterCreate,
}

/// The original's five text slots, by resinfo `FontIndex`: 9/8/12/11/15 pt at
/// 96 dpi, the order the client constructs them in (confirmed by the user's own
/// `event/event_interface.txt:2`).
const FONT_INDEX_PX: [f32; 5] = [12.0, 11.0, 16.0, 15.0, 20.0];

/// `FontSize::Px` of an intro control with resinfo `FontIndex` `font_index`.
/// Unscaled on purpose: the intro draws its rects at the art's native size, so
/// its text carries the same 1:1 factor as the box it sits in.
pub(crate) const fn intro_font_px(font_index: usize) -> f32 {
    let last = FONT_INDEX_PX.len() - 1;
    FONT_INDEX_PX[if font_index > last { last } else { font_index }]
}

/// Marker for every UI root spawned by the intro v2 scene, used for cleanup.
#[derive(Component, Default, Clone)]
pub struct IntroV2Ui;

/// Marker for the looping intro background music.
#[derive(Component, Default, Clone)]
pub struct BackgroundMusicV2;

pub struct IntroV2ScenePlugin;

impl Plugin for IntroV2ScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiV2Plugin)
            .register_type::<IntroV2State>()
            .add_sub_state::<IntroV2State>()
            .add_message::<fade::FadeToBlack>()
            .add_message::<chrome::InfoTextV2Update>()
            // FontAssets is loaded by the scene manager (used across all
            // scenes); only the v2 collection needs registering here.
            .configure_loading_state(
                LoadingStateConfig::new(SceneState::Loading).load_collection::<IntroV2Assets>(),
            )
            .init_resource::<server_select::SelectedShardV2>()
            .init_resource::<server_select::ShardListScroll>()
            // What the create screen's camera frames on: measured off the body
            // that is actually on stage (`character_create::measure_create_figure`).
            .init_resource::<character_create::CreateFigureMetrics>()
            .add_systems(
                OnEnter(SceneState::IntroV2),
                (
                    init_scene_data,
                    spawn_chrome,
                    spawn_cinematic_camera::<CinematicCamera>,
                    enable_camera::<CinematicCamera>,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                load_scene_data.run_if(in_state(IntroV2State::Loading)),
            )
            .add_systems(
                OnEnter(IntroV2State::Splash),
                (start_camera_animation, start_background_audio),
            )
            .add_systems(
                PreUpdate,
                apply_background_music_options
                    .run_if(crate::plugins::settings::live::options_changed),
            )
            .add_systems(
                Update,
                splash::on_splash_click.run_if(in_state(IntroV2State::Splash)),
            )
            .add_systems(
                OnEnter(IntroV2State::LoginForm),
                (
                    fade::show_screen::<login_form::LoginFormRoot>,
                    fade::hide_screen::<splash::SplashRoot>,
                    net::reenable_connect_button,
                    // A successful login despawns the gateway connection; coming
                    // back (restart from the Esc menu, Cancel from character
                    // selection) needs a new one. `init_gateway_service` only
                    // starts the async connect here; `poll_gateway_connection`
                    // requests the shard list once it is established, which
                    // repopulates the (otherwise empty) server window. Gated:
                    // that poller ships with `GatewayPlugin`, so connecting with
                    // networking disabled would hang on `Connecting` forever.
                    crate::plugins::net::gateway::systems::init_gateway_service
                        .run_if(crate::plugins::net::plugin::networking_enabled),
                    // Show a connect failure if one is already pending on entry.
                    net::surface_gateway_error,
                    login_form::focus_id_input,
                    // The Tab ring only exists while this screen is on show —
                    // `hide_screen` leaves the tree alive and Bevy collects Tab
                    // targets structurally, see `login_form::open_tab_ring`.
                    login_form::open_tab_ring,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    login_form::submit_on_enter,
                    // The original greys Connect *and* Exit while the captcha
                    // window is up.
                    login_form::lock_exit_while_captcha_is_open,
                )
                    .run_if(in_state(IntroV2State::LoginForm)),
            )
            .add_systems(
                OnExit(IntroV2State::LoginForm),
                (
                    fade::hide_screen::<login_form::LoginFormRoot>,
                    login_form::close_tab_ring,
                    captcha::despawn_captcha_modal,
                ),
            )
            .add_systems(
                OnEnter(IntroV2State::CharacterList),
                (
                    character_select::set_origin_to_char_select,
                    disable_camera::<CinematicCamera>,
                    spawn_cinematic_camera::<CinematicCamera2>,
                    enable_camera::<CinematicCamera2>,
                    character_select::enable_mesh_picking_camera,
                    character_select::reset_join_state,
                    character_select::start_camera_animation,
                    character_select::request_character_list,
                    character_select::spawn_control_buttons,
                    fade::show_screen::<character_select::CharSelectControls>,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    character_select::on_char_selection_action_response,
                    character_select::tag_pickable_meshes,
                    character_select::update_hover_highlight.run_if(not(resource_exists::<
                        character_select::SelectedCharacterV2,
                    >)),
                    character_select::clear_highlights
                        .run_if(resource_added::<character_select::SelectedCharacterV2>),
                    character_select::spawn_selection_ui
                        .run_if(resource_added::<character_select::SelectedCharacterV2>),
                    // the respawned Create/Cancel row starts transparent, so the
                    // fade-in is chained right after the teardown
                    (
                        character_select::on_deselect,
                        fade::show_screen::<character_select::CharSelectControls>,
                    )
                        .chain()
                        .run_if(resource_removed::<character_select::SelectedCharacterV2>),
                    character_select::on_character_delete_response,
                    character_select::on_character_join_response,
                )
                    .run_if(in_state(IntroV2State::CharacterList)),
            )
            // Leaving the list has two destinations, and only one of them is
            // "off the stage". Going deeper into the pregame flow (race board,
            // creation) stays on the very same 3D stage, so the stage and its
            // camera survive that exit; only leaving for the shard list takes
            // them down. `EnteringCharacterCreate` is the marker the Create
            // button already sets for the connection (see
            // `character_select::disconnect_from_agent_server`).
            .add_systems(
                OnExit(IntroV2State::CharacterList),
                (
                    character_select::despawn_controls,
                    character_select::despawn_selection_ui,
                    character_select::disconnect_from_agent_server,
                )
                    .chain(),
            )
            .add_systems(
                OnExit(IntroV2State::CharacterList),
                (
                    character_select::despawn_characters,
                    despawn_cinematic_camera::<CinematicCamera2>,
                    enable_camera::<CinematicCamera>,
                    // the world origin moved to the char-select anchor on enter, so
                    // the re-enabled intro camera (and its tween targets) must be
                    // rebuilt against the intro anchor
                    set_origin_to_intro,
                    start_camera_animation,
                )
                    .chain()
                    .run_if(not(resource_exists::<
                        character_create::EnteringCharacterCreate,
                    >)),
            )
            // The region-select board sits between the list and creation: race
            // plates over the idols on the stage; picking one flies the camera
            // to the chosen figure, shows the race loading art and enters
            // CharacterCreate. The agent connection is kept alive across the
            // whole sub-flow (see `disconnect_from_agent_server`) so
            // Create/CheckName can use it.
            .add_systems(
                OnEnter(IntroV2State::RegionSelect),
                (
                    character_select::set_origin_to_char_select,
                    disable_camera::<CinematicCamera>,
                    spawn_cinematic_camera::<CinematicCamera2>,
                    enable_camera::<CinematicCamera2>,
                    race_stage::fly_camera_to_race_board,
                    // The props the camera is flying *to* — without them the
                    // pose frames empty sky.
                    race_stage::spawn_race_board_props,
                    region_select::enter_region_select,
                )
                    .chain(),
            )
            // The race plates are a hover state on the idols, not furniture:
            // the original picks the idol under the cursor, ramps that plate's
            // alpha in, hangs it on the idol's projected point and confirms on
            // a left click. Chained because the click has to see this frame's
            // hover, not last frame's.
            .add_systems(
                Update,
                (
                    region_select::update_region_hover,
                    region_select::update_region_plates,
                    region_select::confirm_hovered_region,
                )
                    .chain()
                    .run_if(in_state(IntroV2State::RegionSelect)),
            )
            // Cancel on the board flies the camera home; the state follows
            // when the flight lands.
            .add_systems(
                Update,
                race_stage::tick_return_flight
                    .run_if(in_state(IntroV2State::RegionSelect))
                    .run_if(resource_exists::<race_stage::RaceBoardReturn>),
            )
            // After the click, in the original's order: the camera move to the
            // chosen figure, then the race loading art with its gauge at zero,
            // then `CharacterCreate` once the creation screen's assets loaded.
            .add_systems(
                Update,
                region_select::tick_region_confirm
                    .run_if(in_state(IntroV2State::RegionSelect))
                    .run_if(resource_exists::<region_select::RegionConfirm>),
            )
            .add_systems(
                OnExit(IntroV2State::RegionSelect),
                (
                    race_stage::clear_race_board_flight,
                    region_select::clear_region_confirm,
                    race_stage::despawn_race_board_props,
                    region_select::despawn_region_select,
                ),
            )
            // Board -> creation is a move on one stage, so the camera survives
            // it; only Cancel back to the list takes it down.
            .add_systems(
                OnExit(IntroV2State::RegionSelect),
                despawn_cinematic_camera::<CinematicCamera2>.run_if(not(resource_exists::<
                    character_create::EnteringCharacterCreate,
                >)),
            )
            .add_systems(
                OnEnter(IntroV2State::CharacterCreate),
                (
                    // Not the char-select anchor: creation stands in its own
                    // world region, one per race.
                    character_create::set_origin_to_create_stage,
                    disable_camera::<CinematicCamera>,
                    spawn_cinematic_camera::<CinematicCamera2>,
                    enable_camera::<CinematicCamera2>,
                    character_create::set_create_camera_pose,
                    // The figure renders in its own pass, after the UI, so it
                    // stands in front of the screen's bands like the original's.
                    character_create::spawn_figure_overlay_camera,
                    character_create::enter_character_create,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    character_create::update_preview
                        .run_if(resource_exists_and_changed::<model::CharCreateSelection>),
                    character_create::highlight_selection_buttons
                        .run_if(resource_exists_and_changed::<model::CharCreateSelection>),
                    // The rows carry no value text; the Explain box is the
                    // readout, so it re-reads on either a selection change or
                    // a focus change.
                    character_create::update_explain_panel
                        .run_if(resource_exists::<character_create::CharCreateFocus>),
                    character_create::update_slider_thumbs
                        .run_if(resource_exists_and_changed::<model::CharCreateSelection>),
                    // `Section = Rotate`: yaw + zoom are applied whenever the
                    // view state changes (and once on enter, via the added
                    // resource), so the preview keeps them across re-spawns.
                    character_create::apply_preview_view.run_if(
                        resource_exists_and_changed::<character_create::CharCreatePreviewView>,
                    ),
                    character_create::on_check_name_response,
                    character_create::on_character_create_response,
                    region_select::tick_loading_cut,
                    // The overlay pass: it follows the create camera, owns the
                    // figure's meshes, and steps aside for the confirm modal.
                    character_create::sync_figure_overlay_camera,
                    character_create::tag_figure_overlay_meshes,
                    character_create::hide_figure_overlay_behind_modal,
                    // The frame is solved against the body actually on stage,
                    // so the body is sized first and the camera set after.
                    // Running every frame is what makes a window resize
                    // re-solve it.
                    (
                        character_create::measure_create_figure,
                        character_create::frame_create_camera,
                    )
                        .chain(),
                )
                    .run_if(in_state(IntroV2State::CharacterCreate)),
            )
            .add_systems(
                OnExit(IntroV2State::CharacterCreate),
                (
                    character_create::despawn_character_create,
                    character_create::despawn_figure_overlay_camera,
                    region_select::despawn_loading_cut,
                    despawn_cinematic_camera::<CinematicCamera2>,
                ),
            )
            .add_systems(
                OnEnter(IntroV2State::ServerSelection),
                fade::show_screen::<server_select::ServerSelectRoot>,
            )
            .add_systems(
                OnExit(IntroV2State::ServerSelection),
                fade::hide_screen::<server_select::ServerSelectRoot>,
            )
            .add_systems(
                Update,
                server_select::update_shard_rows
                    .run_if(in_state(SceneState::IntroV2))
                    .run_if(server_select::shard_rows_need_refresh),
            )
            .add_systems(
                Update,
                (
                    server_select::update_shard_row_visuals
                        .run_if(in_state(IntroV2State::ServerSelection)),
                    // the slider's missing half: wheel input, the thumb that
                    // tracks the offset, and Select's disabled state
                    server_select::scroll_shard_list_with_wheel
                        .run_if(in_state(IntroV2State::ServerSelection)),
                    server_select::update_slider_thumb
                        .run_if(in_state(IntroV2State::ServerSelection)),
                    server_select::update_select_button_enabled
                        .run_if(in_state(IntroV2State::ServerSelection)),
                    server_select::update_shard_name_text
                        .run_if(in_state(SceneState::IntroV2))
                        .run_if(resource_exists::<ShardList>),
                    // Displayed server and *selected* server used to drift
                    // apart, and Connect's "select a server first" then read as
                    // a lie about what the row shows.
                    server_select::commit_remembered_shard
                        .run_if(in_state(SceneState::IntroV2))
                        .run_if(resource_exists::<ShardList>),
                ),
            )
            .add_systems(
                Update,
                (
                    chrome::update_info_text,
                    // the bars are spawned once for the scene, so the art
                    // follows the state (#371: create declares RED bars)
                    chrome::update_chrome_art,
                    fade::on_fade_to_black,
                    net::on_gateway_login_response,
                    net::on_agent_login_response,
                    captcha::on_captcha_challenge,
                    captcha::on_captcha_confirm_response,
                    // The modal is the only thing the screen takes input for
                    // while it is up: caret into its one field, Enter confirms.
                    captcha::focus_captcha_input,
                    captcha::confirm_captcha_on_enter,
                )
                    .run_if(in_state(SceneState::IntroV2)),
            )
            .add_systems(
                Update,
                captcha::spawn_captcha
                    .run_if(in_state(SceneState::IntroV2))
                    .run_if(resource_added::<captcha::CaptchaImageV2>),
            )
            // The gateway connect runs off-thread, so a failure can land after
            // the login form is shown — surface it as info text when it does.
            .add_systems(
                Update,
                net::surface_gateway_error
                    .run_if(in_state(SceneState::IntroV2))
                    .run_if(
                        resource_changed::<crate::plugins::net::gateway::GatewayConnectionStatus>,
                    ),
            )
            .add_systems(
                Update,
                fade::on_fade_timer_finished
                    .run_if(in_state(SceneState::IntroV2))
                    .run_if(resource_exists::<fade::FadeToBlackTimer>),
            )
            // Optional developer fast-login (config `dev_fast_login.enabled`):
            // drives the same packet flow as the manual UI, from config instead
            // of the widgets. Its one-shot guards are cleared on scene entry so
            // a second visit is not wedged by the first one's state.
            .init_resource::<dev_fast_login::FastLoginProgress>()
            .add_systems(
                OnEnter(SceneState::IntroV2),
                dev_fast_login::reset_progress.run_if(dev_fast_login::enabled),
            )
            .add_systems(
                Update,
                dev_fast_login::skip_splash
                    .run_if(in_state(IntroV2State::Splash))
                    .run_if(dev_fast_login::enabled),
            )
            .add_systems(
                Update,
                dev_fast_login::send_login
                    .run_if(in_state(IntroV2State::LoginForm))
                    .run_if(dev_fast_login::enabled),
            )
            .add_systems(
                Update,
                dev_fast_login::answer_captcha
                    .run_if(in_state(SceneState::IntroV2))
                    .run_if(dev_fast_login::enabled),
            )
            .add_systems(
                Update,
                dev_fast_login::join_first_character
                    .run_if(in_state(IntroV2State::CharacterList))
                    .run_if(dev_fast_login::enabled),
            )
            // Dev hook (`OPENROAD_INTRO_JUMP`), inert unless the env var is
            // set: the race board and the creation screen are otherwise only
            // reachable by hand.
            .add_systems(
                Update,
                dev_fast_login::jump_to_requested_screen
                    .run_if(in_state(IntroV2State::CharacterList))
                    .run_if(dev_fast_login::enabled),
            )
            .add_systems(
                OnExit(SceneState::IntroV2),
                (disable_camera::<CinematicCamera>, cleanup),
            );
    }
}

/// Grabs the UI camera (spawned by the old, always-registered `UiPlugin`) so
/// v2 roots render on the same 2d camera as the rest of the UI.
fn ui_camera(cam_query: &Query<Entity, With<Camera2d>>) -> Option<Entity> {
    let camera = cam_query.iter().next();
    if camera.is_none() {
        warn!("no 2d camera found");
    }
    camera
}

fn spawn_chrome(
    mut commands: Commands,
    assets: Res<IntroV2Assets>,
    fonts: Res<crate::assets::FontAssets>,
    ui_strings: Res<crate::plugins::textdata::ClientUiStrings>,
    cam_query: Query<Entity, With<Camera2d>>,
) {
    let Some(camera) = ui_camera(&cam_query) else {
        return;
    };

    commands
        .spawn_scene(fade::fade_screen())
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(chrome::header(&assets))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(chrome::footer(&assets))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(chrome::info_text(&fonts))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(splash::splash_logo(&assets))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(login_form::login_form(&assets, &fonts, &ui_strings))
        .insert((UiTargetCamera(camera), IntroV2Ui));
    commands
        .spawn_scene(server_select::server_window(&assets, &fonts, &ui_strings))
        .insert((UiTargetCamera(camera), IntroV2Ui));
}

/// The stock client's own startup options, which name the intro script
/// (`IntroName`) and its track (`IntroBGM`). Archive-relative, inside Media.pk2.
const OPTION_TXT: &str = "config/option.txt";

/// Resolve the cutscene the intro plays, preferring the user's own data over
/// anything we could ship.
///
/// Idea: the camera path is SRO data, so no `.intro` is committed and a release
/// download has none — but the script it is transcribed from sits in the user's
/// own `Media.pk2`, and `IntroScene::from_camera_script` is already the whole
/// converter. So we read it at startup instead of requiring a hand-run tool
/// (#569). Three sources, in order of how explicit the user was:
///
/// 1. `assets/intros/<name>.intro` on disk — a hand-authored or pre-converted
///    path wins, which is what keeps `make cutscene convert` meaningful.
/// 2. the camera script in Media.pk2, named either by `scenes.intro_location`
///    or, failing that, by the client's own `config/option.txt`.
/// 3. neither — fall through to the failing asset load so `load_scene_data`
///    reports it with something actionable.
fn init_scene_data(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    config: Res<ClientConfig>,
    media: Option<Res<MediaArchive>>,
    mut intro_scenes: ResMut<Assets<IntroScene>>,
) {
    let char_select_scene: Handle<CharSelectScene> = asset_server.load(format!(
        "char_selects/{}.selection",
        config.scenes.char_select_location
    ));
    commands.insert_resource(DesiredCharSelectSceneV2(char_select_scene));

    // What the user's own client would play, when we can read it.
    let option = media.as_ref().and_then(|media| {
        let bytes = media.0.read_file_bytes(Path::new(OPTION_TXT))?;
        IntroOption::from_option_txt(&decode_textdata(&bytes))
    });

    let configured = config.scenes.intro_location.trim();
    let name = if configured.is_empty() {
        option.as_ref().map(IntroOption::name).unwrap_or("intro")
    } else {
        configured
    };

    // 1. An `.intro` on disk is an explicit override.
    let override_path = Path::new("assets/intros").join(format!("{name}.intro"));
    if override_path.is_file() {
        info!("intro cutscene: {}", override_path.display());
        let handle = asset_server.load(format!("intros/{name}.intro"));
        commands.insert_resource(DesiredIntroSceneV2(handle));
        return;
    }

    // 2. Derive it from the user's own archive. `scenes.intro_location` names a
    // script directly; option.txt's own path is the fallback, which is also what
    // covers a configured name that is not in this archive.
    let candidates = [
        (!configured.is_empty()).then(|| format!("script/intro/{configured}.txt")),
        option.as_ref().map(|o| o.script.clone()),
    ];
    let music = option
        .as_ref()
        .map(|o| o.music.as_str())
        .unwrap_or(DEFAULT_INTRO_BGM);
    for script in candidates.iter().flatten() {
        let Some(media) = media.as_ref() else { break };
        let Some(bytes) = media.0.read_file_bytes(Path::new(script)) else {
            continue;
        };
        match IntroScene::from_camera_script(name, music, &decode_textdata(&bytes)) {
            Ok(scene) => {
                info!("intro cutscene: {script} from Media.pk2 (music {music})");
                commands.insert_resource(DesiredIntroSceneV2(intro_scenes.add(scene)));
                return;
            }
            // A script that is present but unreadable is worth saying out loud;
            // the loop still tries the next candidate.
            Err(err) => error!("cannot convert {script}: {err}"),
        }
    }

    // 3. Nothing resolved. The failing load keeps `load_scene_data`'s reporting
    // in one place rather than duplicating the message here.
    commands.insert_resource(DesiredIntroSceneV2(
        asset_server.load(format!("intros/{name}.intro")),
    ));
}

fn load_scene_data(
    desired_intro_scene: Res<DesiredIntroSceneV2>,
    desired_select_scene: Res<DesiredCharSelectSceneV2>,
    intro_scene_assets: Res<Assets<IntroScene>>,
    select_scene_assets: Res<Assets<CharSelectScene>>,
    mut commands: Commands,
    mut next_state: ResMut<NextState<IntroV2State>>,
    mut origin: ResMut<WorldOrigin>,
    mut terrain: Query<&mut Transform, With<Terrain>>,
    asset_server: Res<AssetServer>,
    mut reported: Local<bool>,
) {
    let Some(intro_scene) = intro_scene_assets.get(&desired_intro_scene.0) else {
        // Reaching here means every source in `init_scene_data` came up empty:
        // no `.intro` on disk, and no camera script we could read from the
        // user's own archive. That is a real misconfiguration rather than the
        // normal state of a fresh checkout, so say which of the two to fix.
        if !*reported && asset_server.load_state(&desired_intro_scene.0).is_failed() {
            *reported = true;
            let name = desired_intro_scene
                .0
                .path()
                .and_then(|p| {
                    p.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "<name>".to_string());
            error!(
                "no intro cutscene for `{name}`. Camera paths are the original's data, so \
                 openroad ships none and reads yours instead: it looks for \
                 `script/intro/{name}.txt` in your Media.pk2, falling back to whatever \
                 `config/option.txt` names. Neither was readable.\n  \
                 - check `scenes.intro_location` in config.yaml names a script your \
                 Media.pk2 actually has (leave it empty to follow your own option.txt)\n  \
                 - or supply a converted path at `assets/intros/{name}.intro`"
            );
        }
        return;
    };
    let Some(select_scene) = select_scene_assets.get(&desired_select_scene.0) else {
        return;
    };

    // Anchor the floating world origin on the cinematic's flight path so the
    // camera and the harbor terrain live at small render-space coordinates
    // (see `world_origin`). Must happen before the camera tween is built.
    if let Some(anchor) = intro_scene.start_anchor() {
        set_world_origin(anchor, &mut origin, &mut terrain);
    }

    commands.insert_resource(ActiveIntroSceneV2(intro_scene.clone()));
    commands.insert_resource(ActiveCharSelectSceneV2(select_scene.clone()));
    next_state.set(IntroV2State::Splash);
}

/// Re-anchor the floating world origin on the intro cinematic (used when
/// coming back from character selection, whose enter moved the origin).
fn set_origin_to_intro(
    intro_scene_data: Res<ActiveIntroSceneV2>,
    mut origin: ResMut<WorldOrigin>,
    mut terrain: Query<&mut Transform, With<Terrain>>,
) {
    if let Some(anchor) = intro_scene_data.0.start_anchor() {
        set_world_origin(anchor, &mut origin, &mut terrain);
    }
}

fn start_camera_animation(
    mut cam_query: Query<Entity, With<CinematicCamera>>,
    intro_scene_data: Res<ActiveIntroSceneV2>,
    origin: Res<WorldOrigin>,
    mut commands: Commands,
) {
    let Ok(entity) = cam_query.single_mut() else {
        return;
    };

    commands
        .entity(entity)
        .insert(TweenAnim::new(intro_scene_data.0.get_camera_anim(origin.0)));
}

/// Despawns everything the intro v2 scene created when it is left. The
/// character-selection cleanup (previews, agent connection, camera 2) runs
/// via `OnExit(IntroV2State::CharacterList)` when the sub-state is removed.
fn cleanup(
    mut commands: Commands,
    ui_query: Query<Entity, With<IntroV2Ui>>,
    music_query: Query<Entity, With<BackgroundMusicV2>>,
) {
    for entity in ui_query.iter().chain(music_query.iter()) {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<fade::FadeToBlackTimer>();
    commands.remove_resource::<captcha::CaptchaImageV2>();
}

fn start_background_audio(
    intro_scene_data: Res<ActiveIntroSceneV2>,
    asset_server: Res<AssetServer>,
    options: Res<GameOptions>,
    mut commands: Commands,
    query: Query<Entity, With<BackgroundMusicV2>>,
) {
    if !query.is_empty() {
        return;
    }

    // Spawned even when BGM is off — muted is a *paused sink*, not a missing
    // entity, so turning BGM on mid-scene starts the track instead of doing
    // nothing until the next scene load (#647).
    commands.spawn((
        AudioPlayer::new(asset_server.load(intro_scene_data.0.music())),
        options.audio.bgm_playback_settings(),
        BackgroundMusicV2,
        Name::from("Background Music V2"),
    ));
}

/// Turns the **literal** two-character `\\n` that textdata rows carry into a
/// real newline; a Bevy `Text` node renders the backslash literally.
/// Deliberately not folded into `ClientUiStrings::get_or`: that accessor is used
/// all over the client, and some rows carry a literal backslash as data.
pub(super) fn unescape_newlines(text: &str) -> String {
    text.replace("\\n", "\n")
}

/// Replaces the `%d` placeholders of a textdata string, in order, with
/// `values`. Extra placeholders are left as-is rather than dropped, so a
/// template with more fields than we can fill stays recognisable.
pub(super) fn fill_placeholders(template: &str, values: &[u32]) -> String {
    let mut out = String::with_capacity(template.len() + 8);
    let mut rest = template;
    let mut values = values.iter();
    while let Some(at) = rest.find("%d") {
        out.push_str(&rest[..at]);
        match values.next() {
            Some(v) => out.push_str(&v.to_string()),
            None => out.push_str("%d"),
        }
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

/// Plays the original's `snd_error` once, honouring the audio options. Shared
/// by every pregame refusal (login, captcha retry, lobby actions).
pub(super) fn play_error_sound(
    commands: &mut Commands,
    assets: &IntroV2Assets,
    options: &GameOptions,
) {
    if let Some(playback) = options.audio.fx_playback() {
        commands.spawn((AudioPlayer::new(assets.sound_error.clone()), playback));
    }
}

/// Applies the audio options to the playing background music
/// ([`crate::plugins::settings::live`]).
///
/// The volume slider and the BGM checkbox take effect on the track that is
/// already playing; sound effects need no apply system because every one-shot
/// reads `fx_playback()` at the moment it is spawned.
fn apply_background_music_options(
    options: Res<GameOptions>,
    mut sinks: Query<&mut AudioSink, With<BackgroundMusicV2>>,
) {
    for mut sink in sinks.iter_mut() {
        sink.set_volume(options.audio.bgm_gain());
        if options.audio.bgm_enabled {
            sink.play();
        } else {
            sink.pause();
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn the_intro_text_sizes_are_the_unscaled_font_ladder() {
        assert_eq!(intro_font_px(0), 12.0);
        assert_eq!(intro_font_px(2), 16.0, "every caption/button on the intro");
        assert_eq!(intro_font_px(4), 20.0);
        // a data index the binary has no slot for must not panic
        assert_eq!(intro_font_px(7), intro_font_px(4));
    }

    #[test]
    fn a_literal_backslash_n_becomes_a_line_break() {
        assert_eq!(unescape_newlines("a\\nb"), "a\nb");
        assert_eq!(unescape_newlines("plain"), "plain");
    }

    #[test]
    fn placeholders_are_filled_in_order_and_extras_survive() {
        assert_eq!(
            fill_placeholders("failed %d out of %d", &[1, 6]),
            "failed 1 out of 6"
        );
        assert_eq!(fill_placeholders("%d and %d", &[3]), "3 and %d");
    }

    /// #645, ownership half: character creation lives *inside* the intro scene.
    /// The state graph is what makes creation share the char-select stage,
    /// world origin, cinematic camera and agent connection — promoting
    /// `CharacterCreate` to its own `SceneState` would tear all four down on
    /// entry. Pinned here so the promotion cannot happen silently; the
    /// rationale and its citations sit on the variant itself.
    #[test]
    fn character_create_is_a_sub_state_of_the_intro_scene() {
        let mut app = App::new();
        app.add_plugins(bevy::state::app::StatesPlugin)
            .init_state::<SceneState>()
            .add_sub_state::<IntroV2State>();

        // outside the intro scene the whole create sub-flow does not exist
        app.update();
        assert!(app.world().get_resource::<State<IntroV2State>>().is_none());

        app.world_mut()
            .resource_mut::<NextState<SceneState>>()
            .set(SceneState::IntroV2);
        app.update();
        assert!(app.world().get_resource::<State<IntroV2State>>().is_some());

        // ... and CharacterCreate is reachable without leaving SceneState::IntroV2
        app.world_mut()
            .resource_mut::<NextState<IntroV2State>>()
            .set(IntroV2State::CharacterCreate);
        app.update();
        assert_eq!(
            *app.world().resource::<State<IntroV2State>>().get(),
            IntroV2State::CharacterCreate
        );
        assert_eq!(
            *app.world().resource::<State<SceneState>>().get(),
            SceneState::IntroV2
        );

        // leaving the intro scene removes creation with it
        app.world_mut()
            .resource_mut::<NextState<SceneState>>()
            .set(SceneState::Loading);
        app.update();
        assert!(app.world().get_resource::<State<IntroV2State>>().is_none());
    }
}

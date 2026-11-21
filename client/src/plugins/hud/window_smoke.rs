//! Headless smoke test: every HUD window opens and closes without a panic.
//!
//! `make ci` never starts the app, so two crash classes reach a player
//! unseen: a system taking a resource nobody registered (Bevy's parameter
//! validation panics the schedule) and a spawn whose bundle carries one
//! component twice (H-6, "Bundle … has duplicate components"). Both fire only
//! when the systems really run. So this builds the real [`HudPlugin`] on
//! `MinimalPlugins` — no window, no renderer, no PK2 — beside what the game
//! registers outside the HUD before any data arrives, enters `GameWorld`, and
//! opens each window the way the game does: mostly through the `open` flag its
//! own toggle flips.
//!
//! Everything [`hud_app`] adds must be something the game always has. A window
//! that needs more than that is the finding, not a gap in this fixture.

use bevy::ecs::entity::EntityHashMap;
use bevy::ecs::error::{BevyError, ErrorContext, FallbackErrorHandler, Severity};
use bevy::input::mouse::MouseButtonInput;
use bevy::input::{ButtonState, InputPlugin};
use bevy::input_focus::InputFocus;
use bevy::picking::backend::HitData;
use bevy::picking::hover::HoverMap;
use bevy::picking::pointer::PointerId;
use bevy::prelude::*;
use bevy::scene::ScenePlugin;
use bevy::state::app::StatesPlugin;
use bevy::window::PrimaryWindow;

use super::context_menu::{ContextMenuOwner, ContextMenuRoot};
use super::death::PlayerDeath;
use super::quest_reward::QuestRewardState;
use super::quest_reward_confirm::QuestRewardConfirmState;
use super::HudPlugin;
use crate::assets::bsr::resource::SroResource;
use crate::assets::dof::JMXVDOF;
use crate::assets::resinfo::item_rare::ItemRareTable;
use crate::assets::textdata::Textdata;
use crate::assets::twodt::JMXV2DT;
use crate::assets::FontAssets;
use crate::plugins::combat::{AttackOrder, DamagePopup, LocalAttackLanded, PickupOrder};
use crate::plugins::config::ClientConfig;
use crate::plugins::cos::{pet::PetCommand, ActiveCosList, CosCommand, RiderState};
use crate::plugins::cursor::interactions::entity_select::{HoveredEntity, SelectedEntity};
use crate::plugins::cursor::interactions::npcs::TalkStarted;
use crate::plugins::gm::SlashCommand;
use crate::plugins::net::entities::RemoteEntity;
use crate::plugins::net::plugin::NetworkCorePlugin;
use crate::plugins::settings::options::GameOptions;
use crate::plugins::skills::book::{SkillBook, SkillGroupIndex};
use crate::plugins::skills::cast::{CastRequest, SkillCooldowns};
use crate::plugins::skills::EquippedWeapon;
use crate::plugins::textdata::TextdataPlugin;
use crate::plugins::world_origin::WorldOrigin;
use crate::scenes::SceneState;

/// Panics with the failing system named and the inner panic attached. Bevy's
/// own handler resumes a system's panic as it came, which names no system (a
/// spawn whose commands panic, H-6, stays anonymous) and, from a worker thread,
/// prints where the test harness does not capture it. Below `Panic` the game
/// only logs; staying quiet there also keeps these tests out of `game_scene`'s
/// capture of the `log` crate.
fn panic_naming_the_system(mut error: BevyError, ctx: ErrorContext) {
    if error.severity() != Severity::Panic {
        return;
    }
    let inner = error.take_payload().and_then(|payload| {
        payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
    });
    panic!("{ctx}: {error}{}", inner.unwrap_or_default());
}

/// The HUD as the game runs it, one frame into `GameWorld`.
fn hud_app() -> App {
    let mut config = ClientConfig::from_file(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../config.example")
            .to_str()
            .expect("the example path is utf-8"),
    )
    .expect("config.example.yaml matches ClientConfig");
    // `hud::scale`'s test asserts on the process-global HUD scale. An invalid
    // value is refused by `set_hud_scale`, so these tests never write it.
    config.hud.hud_scale = 0.0;

    let mut app = App::new();
    // the DefaultPlugins members the HUD reaches, minus window and renderer
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        StatesPlugin,
        InputPlugin,
        ScenePlugin,
    ));
    app.insert_resource(FallbackErrorHandler(panic_naming_the_system))
        // ImagePlugin, MeshPlugin, TextPlugin and SroAssetStructsPlugin
        .init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<Font>()
        .init_asset::<JMXV2DT>()
        .init_asset::<JMXVDOF>()
        .init_asset::<SroResource>()
        .init_asset::<Textdata>()
        .init_asset::<ItemRareTable>()
        // InputDispatchPlugin
        .init_resource::<InputFocus>()
        // PickingPlugin
        .init_resource::<HoverMap>()
        // main.rs inserts the config before any plugin is built
        .insert_resource(config)
        // SceneManagerPlugin; the fonts are its `Loading` collection
        .init_state::<SceneState>()
        .init_resource::<WorldOrigin>()
        .insert_resource(FontAssets {
            one: default(),
            two: default(),
            three: default(),
            nine: default(),
        })
        // SettingsPlugin, CursorPlugin, GmPlugin
        .init_resource::<GameOptions>()
        .init_resource::<HoveredEntity>()
        .init_resource::<SelectedEntity>()
        .add_message::<TalkStarted>()
        .add_message::<SlashCommand>()
        // SkillsPlugin and CombatPlugin
        .init_resource::<SkillBook>()
        .init_resource::<SkillGroupIndex>()
        .init_resource::<EquippedWeapon>()
        .init_resource::<SkillCooldowns>()
        .add_message::<CastRequest>()
        .add_message::<AttackOrder>()
        .add_message::<PickupOrder>()
        .add_message::<DamagePopup>()
        .add_message::<LocalAttackLanded>()
        // cos::CosPlugin
        .init_resource::<ActiveCosList>()
        .init_resource::<RiderState>()
        .add_message::<CosCommand>()
        .add_message::<PetCommand>()
        // Both run headless already (the netcheck harness and the tables'
        // empty defaults), so they come in whole rather than copied out.
        .add_plugins((NetworkCorePlugin, TextdataPlugin, HudPlugin));
    // CameraPlugin's persistent UI camera, which every window attaches to
    app.world_mut().spawn(Camera2d);
    app.world_mut()
        .resource_mut::<NextState<SceneState>>()
        .set(SceneState::GameWorld);
    app.update();
    assert!(
        shown_roots(&mut app) > 0,
        "entering GameWorld spawned no HUD at all"
    );
    app
}

/// UI roots that are laid out: what a player sees as open surfaces.
fn shown_roots(app: &mut App) -> usize {
    let mut roots = app.world_mut().query_filtered::<&Node, Without<ChildOf>>();
    roots
        .iter(app.world())
        .filter(|node| node.display != Display::None)
        .count()
}

/// Open a window, give it three frames, close it, give it one more. It must
/// show something while open, or the test opened nothing.
fn open_and_close(window: &str, set_open: impl Fn(&mut World, bool)) {
    let mut app = hud_app();
    let closed = shown_roots(&mut app);
    set_open(app.world_mut(), true);
    for _ in 0..3 {
        app.update();
    }
    assert!(
        shown_roots(&mut app) > closed,
        "opening `{window}` showed nothing"
    );
    set_open(app.world_mut(), false);
    app.update();
}

/// One test per window, named after it, opened through its state's `open`.
macro_rules! open_flag {
    ($($window:ident: $state:ty,)*) => {$(
        #[test]
        fn $window() {
            open_and_close(stringify!($window), |world, open| {
                world.resource_mut::<$state>().open = open;
            });
        }
    )*};
}

// Not here, though they carry `open` too: the enchant box and the world map
// draw only from PK2 data (their 2dt descriptor, `worldmap_mapinfo`), and the
// storage gold box closes itself outside a storage session.
open_flag! {
    academy: super::academy::AcademyWindowState,
    academy_appraisal: super::academy_appraisal::AcademyAppraisalState,
    action: super::action::ActionWindowState,
    alchemy: super::alchemy::model::AlchemyState,
    alchemy_grant: super::alchemy::grant::GrantState,
    appearance_change: super::appearance_change::AppearanceChangeState,
    auto_potion: super::autopotion::model::AutoPotionState,
    character_info: super::character_info::model::CharacterInfoState,
    collection: super::collection::CollectionWindowState,
    community: super::community::model::CommunityState,
    cos: super::cos::model::CosWindowState,
    game_guide: super::game_guide::GameGuideWindowState,
    inventory: super::inventory::model::InventoryState,
    item_mall: super::item_mall::ItemMallState,
    item_mall_shop: super::item_mall::shop::MallShopState,
    party: super::party::model::PartyWindowState,
    party_mode: super::party::mode_modal::PartyModeState,
    party_matching: super::party_matching::model::PartyMatchState,
    quest: super::quest::QuestWindowState,
    quest_detail: super::quest_detail::QuestDetailState,
    skill_window: super::skill_window::model::SkillWindowState,
    stall: super::stall::model::StallState,
    stall_network: super::stall_network::StallNetworkState,
}

/// The death box, through the calls the death packets make.
#[test]
fn death() {
    open_and_close("death", |world, open| {
        let mut death = world.resource_mut::<PlayerDeath>();
        if open {
            death.set_dead();
        } else {
            death.set_alive();
        }
    });
}

/// The reward picker opens on a quest id, as the offline preview opens it —
/// there is no reward opcode yet. Any id: the tables are empty.
#[test]
fn quest_reward() {
    open_and_close("quest_reward", |world, open| {
        world.resource_mut::<QuestRewardState>().quest = open.then_some(1);
    });
}

/// The confirm box belongs to the picker and only opens over it.
#[test]
fn quest_reward_confirm() {
    open_and_close("quest_reward_confirm", |world, open| {
        world.resource_mut::<QuestRewardState>().quest = open.then_some(1);
        world.resource_mut::<QuestRewardConfirmState>().open = open;
    });
}

/// A right-click that did not drag opens the target menu on a selected player
/// and first closes whatever popup was open. Over the UI it does neither: the
/// click belongs to the window under it, as a potion's right-click in the
/// inventory does, or a party slot's, which opens the party's own menu (K32).
/// So every case starts with the quick-party board's menu open, as the press
/// on its slot opens it.
#[test]
fn a_right_click_on_the_ui_leaves_the_popups_to_the_ui() {
    for (over_ui, selected, want) in [
        (true, false, (1, ContextMenuOwner::QuickParty)),
        (true, true, (1, ContextMenuOwner::QuickParty)),
        (false, true, (1, ContextMenuOwner::Target)),
    ] {
        let mut app = hud_app();
        let world = app.world_mut();
        world.spawn(ContextMenuRoot);
        *world.resource_mut::<ContextMenuOwner>() = ContextMenuOwner::QuickParty;
        if selected {
            let player = world.spawn(RemoteEntity::Player).id();
            world.resource_mut::<SelectedEntity>().0 = Some(player);
        }
        let mut window = Window::default();
        window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        let window = world.spawn((window, PrimaryWindow)).id();
        if over_ui {
            let node = world.spawn(Node::default()).id();
            let mut hits = EntityHashMap::default();
            hits.insert(node, HitData::new(window, 0.0, None, None));
            world
                .resource_mut::<HoverMap>()
                .insert(PointerId::Mouse, hits);
        }
        for state in [ButtonState::Pressed, ButtonState::Released] {
            app.world_mut().write_message(MouseButtonInput {
                button: MouseButton::Right,
                state,
                window,
            });
            app.update();
        }
        let mut roots = app.world_mut().query::<&ContextMenuRoot>();
        let open = roots.iter(app.world()).count();
        assert_eq!(
            (open, *app.world().resource::<ContextMenuOwner>()),
            want,
            "over the UI: {over_ui}, a player selected: {selected}"
        );
    }
}

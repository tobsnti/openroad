//! Takes closed HUD windows out of Bevy's per-frame UI passes.
//!
//! Idea: a closed window is a UI root with `Display::None`. Taffy skips its
//! layout, but Bevy 0.19 still walks every node under every root each frame —
//! child sync and geometry write-back in `ui_layout_system`, then stacking,
//! clipping and render extraction. With the dozen big windows closed that was
//! ~1,900 of ~2,800 UI nodes, measured at ~1 ms of main-thread time per frame
//! (BRP A/B at the Jangan West waterfall, 2026-10-05).
//!
//! So when a root switches to `Display::None`, its children move under a
//! plain [`ParkedUiHolder`] entity (no `Node`, so Bevy's UI tree walks stop
//! there) that is itself a child of the root; when the root is shown again,
//! the children move back in front of anything added meanwhile and the holder
//! goes. This runs in `PostUpdate` before `UiSystems::Prepare`, so a window
//! opened this frame is laid out this frame.
//!
//! Why reparent rather than `Disabled`: disabled entities vanish from *every*
//! query, so a window's own refresh — which typically runs in the very frame
//! it opens, keyed on a state change — would hit still-disabled cells, spend
//! its trigger and leave the window stale. Parked content stays an ordinary
//! entity for our systems; only the UI tree no longer reaches it, and walks
//! over all descendants (`iter_descendants`) still pass through the holder.
//!
//! Reattaching flags every `Children` list in the content as changed: Bevy
//! syncs child links into Taffy only for nodes it reaches, so content built
//! or rebuilt while parked would otherwise reopen with its inner nodes
//! unlinked — the window frame showing, everything inside it missing.
//!
//! The holder is `Visibility::Hidden`: parked content keeps the `ComputedNode`
//! of its last layout (layout never runs on it again until it is reattached),
//! so without the hidden parent a window closed this frame would keep drawing.

use bevy::camera::visibility::VisibilitySystems;
use bevy::prelude::*;
use bevy::ui::UiSystems;

pub struct UiParkingPlugin;

impl Plugin for UiParkingPlugin {
    fn build(&self, app: &mut App) {
        // before visibility propagation too, so reopened content is visible
        // in the frame its window opens
        app.add_systems(
            PostUpdate,
            park_closed_ui_roots
                .before(UiSystems::Prepare)
                .before(VisibilitySystems::VisibilityPropagate),
        );
    }
}

/// On a closed UI root: the entity holding its parked children.
#[derive(Component)]
pub struct ParkedUiContent(pub Entity);

/// The non-UI parent of a closed root's parked children.
#[derive(Component)]
pub struct ParkedUiHolder;

/// Parks the children of UI roots that became `Display::None` and reattaches
/// them when the root is shown again (see the module docs).
pub fn park_closed_ui_roots(
    mut commands: Commands,
    roots: Query<(Entity, &Node, Option<&ParkedUiContent>), (Without<ChildOf>, Changed<Node>)>,
    holders: Query<(), With<ParkedUiHolder>>,
    mut children: Query<&mut Children>,
    mut stack: Local<Vec<Entity>>,
) {
    for (root, node, parked) in &roots {
        // a holder that is gone (the root's children were rebuilt while it
        // was closed) no longer counts
        let holder = parked
            .map(|p| p.0)
            .filter(|&holder| holders.contains(holder));

        if node.display == Display::None {
            // everything directly under the root except the holder itself
            let content: Vec<Entity> = children
                .get(root)
                .map(|c| c.iter().filter(|&child| Some(child) != holder).collect())
                .unwrap_or_default();
            if content.is_empty() {
                continue;
            }
            let holder = holder.unwrap_or_else(|| {
                let holder = commands
                    .spawn((ParkedUiHolder, Visibility::Hidden, Name::new("parked UI")))
                    .id();
                commands
                    .entity(root)
                    .add_child(holder)
                    .insert(ParkedUiContent(holder));
                holder
            });
            commands.entity(holder).add_children(&content);
        } else if parked.is_some() {
            commands.entity(root).remove::<ParkedUiContent>();
            let Some(holder) = holder else {
                continue;
            };
            let content: Vec<Entity> = children
                .get(holder)
                .map(|c| c.iter().collect())
                .unwrap_or_default();
            // Bevy syncs a node's children into Taffy only when it reaches
            // the node with its `Children` changed (or a child just added).
            // Parked content was unreachable: built while closed — the HUD
            // spawns its windows closed — or refreshed while closed, its
            // child links never reached Taffy. Flag every list in the
            // subtree so the next layout re-links all of it.
            stack.clear();
            stack.extend_from_slice(&content);
            while let Some(entity) = stack.pop() {
                if let Ok(mut list) = children.get_mut(entity) {
                    list.set_changed();
                    stack.extend(list.iter());
                }
            }
            // the parked children were the root's first ones
            commands.entity(root).insert_children(0, &content);
            commands.entity(holder).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn children_of(world: &World, entity: Entity) -> Vec<Entity> {
        world
            .get::<Children>(entity)
            .map(|c| c.iter().collect())
            .unwrap_or_default()
    }

    #[test]
    fn closing_parks_children_and_opening_restores_their_order() {
        let mut world = World::new();
        let root = world.spawn(Node::default()).id();
        let a = world.spawn((Node::default(), ChildOf(root))).id();
        let b = world.spawn((Node::default(), ChildOf(root))).id();

        // open: nothing to do
        world.run_system_once(park_closed_ui_roots).unwrap();
        assert_eq!(children_of(&world, root), [a, b]);

        world.get_mut::<Node>(root).unwrap().display = Display::None;
        world.run_system_once(park_closed_ui_roots).unwrap();
        let holder = world.get::<ParkedUiContent>(root).expect("parked").0;
        assert_eq!(children_of(&world, root), [holder]);
        assert_eq!(children_of(&world, holder), [a, b]);
        assert_eq!(world.get::<Visibility>(holder), Some(&Visibility::Hidden));

        // a child added while closed is parked too, after the others
        let c = world.spawn((Node::default(), ChildOf(root))).id();
        world.get_mut::<Node>(root).unwrap().left = Val::Px(1.0);
        world.run_system_once(park_closed_ui_roots).unwrap();
        assert_eq!(children_of(&world, holder), [a, b, c]);

        world.get_mut::<Node>(root).unwrap().display = Display::Flex;
        world.run_system_once(park_closed_ui_roots).unwrap();
        assert_eq!(children_of(&world, root), [a, b, c]);
        assert!(world.get::<ParkedUiContent>(root).is_none());
        assert!(world.get_entity(holder).is_err(), "holder despawned");
    }

    /// Close and reopen a window through Bevy's own UI propagation, layout and
    /// visibility systems: the reopened content must be laid out, targeted
    /// at the camera and visible again.
    #[test]
    fn reopened_content_is_laid_out_and_visible_again() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo, Viewport};
        use bevy::ui::{ComputedNode, ComputedUiTargetCamera};

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            bevy::asset::AssetPlugin::default(),
            bevy::image::ImagePlugin::default(),
            bevy::transform::TransformPlugin,
            bevy::camera::visibility::VisibilityPlugin,
            bevy::text::TextPlugin,
            bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            },
            bevy::input::InputPlugin,
            bevy::picking::DefaultPickingPlugins,
            bevy::ui::UiPlugin,
            UiParkingPlugin,
        ));
        app.init_asset::<bevy::image::TextureAtlasLayout>()
            .init_asset::<Mesh>()
            .init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
        app.world_mut().spawn((
            Camera2d,
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 600),
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                viewport: Some(Viewport {
                    physical_size: UVec2::new(800, 600),
                    ..default()
                }),
                ..default()
            },
        ));

        let world = app.world_mut();
        // spawned closed, like the HUD windows: the content is parked before
        // it was ever laid out
        let root = world
            .spawn(Node {
                display: Display::None,
                width: Val::Px(400.0),
                height: Val::Px(300.0),
                ..default()
            })
            .id();
        let icon_node = |width: f32| Node {
            width: Val::Px(width),
            height: Val::Px(10.0),
            ..default()
        };
        let panel = world
            .spawn((
                Node {
                    width: Val::Px(100.0),
                    height: Val::Px(50.0),
                    ..default()
                },
                ChildOf(root),
            ))
            .id();
        let icon = world.spawn((icon_node(20.0), ChildOf(panel))).id();

        let check = |app: &App, label: &str, icons: &[(Entity, f32)]| {
            let world = app.world();
            for entity in std::iter::once(panel).chain(icons.iter().map(|i| i.0)) {
                assert!(
                    world.get::<ComputedUiTargetCamera>(entity).is_some(),
                    "{label}: {entity} lost its UI camera"
                );
                assert_eq!(
                    world.get::<InheritedVisibility>(entity),
                    Some(&InheritedVisibility::VISIBLE),
                    "{label}: {entity} not visible"
                );
            }
            for &(entity, width) in icons {
                assert_eq!(
                    world.get::<ComputedNode>(entity).unwrap().size(),
                    Vec2::new(width, 10.0),
                    "{label}: {entity} not laid out"
                );
            }
        };

        app.update();
        app.update();
        assert!(app.world().get::<ParkedUiContent>(root).is_some());

        // a window refresh while closed adds a cell deep inside the content
        let late = app
            .world_mut()
            .spawn((icon_node(30.0), ChildOf(panel)))
            .id();
        app.update();

        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::Flex;
        app.update();
        check(&app, "first open", &[(icon, 20.0), (late, 30.0)]);

        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::None;
        app.update();
        app.update();
        app.world_mut().get_mut::<Node>(root).unwrap().display = Display::Flex;
        app.update();
        check(&app, "reopened", &[(icon, 20.0), (late, 30.0)]);
    }

    #[test]
    fn despawning_a_closed_root_takes_its_parked_content_along() {
        let mut world = World::new();
        let root = world
            .spawn(Node {
                display: Display::None,
                ..default()
            })
            .id();
        let child = world.spawn((Node::default(), ChildOf(root))).id();
        world.run_system_once(park_closed_ui_roots).unwrap();
        world.entity_mut(root).despawn();
        assert!(world.get_entity(child).is_err());
    }
}

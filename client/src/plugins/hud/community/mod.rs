pub mod friend;
pub mod guild;
pub mod letter;
pub mod letter_sub;
pub mod model;
pub mod notice_write;
pub mod ui;

use bevy::prelude::*;

/// Self-registration for the community window (#558). The HUD registry holds one line per
/// window, so two windows landing in the same lap no longer collide on it.
pub struct CommunityPlugin;

impl Plugin for CommunityPlugin {
    fn build(&self, app: &mut App) {
        use crate::scenes::SceneState;

        app.init_resource::<model::CommunityState>()
            .init_resource::<guild::GuildNoticeOpen>()
            .init_resource::<guild::GuildRelationsState>()
            .init_resource::<guild::GuildRosterSelection>()
            .init_resource::<notice_write::GuildNoticeWrite>()
            .init_resource::<friend::FriendSelection>()
            .add_systems(OnEnter(SceneState::GameWorld), ui::spawn_community_window)
            .add_systems(
                OnExit(SceneState::GameWorld),
                (
                    ui::cleanup_community_window,
                    notice_write::cleanup_guild_notice_write,
                ),
            )
            .add_systems(
                Update,
                letter_sub::apply_letter_sub_window.run_if(
                    in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting)),
                ),
            )
            .add_systems(
                Update,
                (
                    guild::update_guild_info,
                    guild::update_guild_roster,
                    guild::update_guild_notice,
                    guild::apply_guild_relations_tab,
                    guild::update_alliance_pane,
                    notice_write::apply_guild_notice_write,
                    friend::update_friend_list,
                )
                    .run_if(
                        in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting)),
                    ),
            )
            .add_systems(
                Update,
                (
                    model::toggle_community_window,
                    ui::apply_community_visibility,
                )
                    .chain()
                    .run_if(
                        in_state(SceneState::GameWorld).or_else(in_state(SceneState::UiTesting)),
                    ),
            );
    }
}

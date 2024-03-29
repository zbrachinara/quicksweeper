use bevy::{prelude::*, utils::Uuid};

mod client_systems;
mod components;
mod impl_v2;
mod protocol;
pub mod puppet;
mod server_systems;
mod states;

use iyes_loopless::prelude::*;
use server_systems::*;

use crate::{
    area_attack::{components::FreezeTimer, protocol::AreaAttackUpdate},
    main_menu::{Menu, ToGame},
    server::{GameMarker, LocalEvent},
};

pub use impl_v2::IAreaAttack;

use self::{components::RevealTile, protocol::AreaAttackRequest, states::AreaAttack};

pub const AREA_ATTACK_MARKER: GameMarker = GameMarker(
    match Uuid::try_parse("040784a0-e905-44a9-b698-14a71a29b3fd") {
        Ok(val) => val,
        Err(_) => unreachable!(),
    },
);

#[derive(Component)]
pub struct AreaAttackServer;

impl Plugin for AreaAttackServer {
    fn build(&self, app: &mut App) {
        app.add_event::<LocalEvent<AreaAttackRequest>>()
            .add_event::<RevealTile>()
            .add_system_set(
                ConditionSet::new()
                    .run_not_in_state(Menu::Loading)
                    .with_system(broadcast_positions)
                    .with_system(create_game)
                    .with_system(reveal_tiles)
                    .with_system(unmark_init_access)
                    .with_system(prepare_player)
                    .with_system(unfreeze_players)
                    .with_system(send_tiles)
                    .with_system(net_events)
                    .with_system(initial_transition)
                    .with_system(stage_transitions)
                    .with_system(update_selecting_tile)
                    .with_system(update_tile_playing)
                    .into(),
            );
    }
}

pub struct AreaAttackClient;

impl Plugin for AreaAttackClient {
    fn build(&self, app: &mut App) {
        use AreaAttack::*;
        app.add_loopless_state(Inactive)
            .init_resource::<FreezeTimer>()
            .add_event::<AreaAttackUpdate>()
            .add_system(|mut commands: Commands, mut ev: EventReader<ToGame>| {
                if ev.iter().any(|e| **e == AREA_ATTACK_MARKER) {
                    // transition from menu into game
                    commands.insert_resource(NextState(Selecting))
                }
            })
            .add_system(client_systems::begin_game.run_in_state(Selecting))
            .add_exit_system(Menu::Loading, client_systems::create_freeze_timer)
            .add_system_set(
                ConditionSet::new()
                    .run_not_in_state(Inactive)
                    .with_system(client_systems::send_position)
                    .with_system(client_systems::request_reveal)
                    .with_system(client_systems::draw_tiles)
                    .with_system(client_systems::freeze_timer)
                    // Systems for receiving network events
                    .with_system(client_systems::listen_net)
                    .with_system(client_systems::reset_field)
                    .with_system(client_systems::player_update)
                    .with_system(client_systems::self_update)
                    .with_system(client_systems::puppet_control)
                    .with_system(client_systems::state_transitions)
                    .into(),
            );
    }
}

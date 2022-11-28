#![warn(clippy::all)]
#![allow(clippy::type_complexity)] // this lint marks type signatures of queries as too long, which is unnecessary

mod area_attack;
mod common;
mod cursor;
mod load;
mod main_menu;
mod multiplayer;
mod server;
mod singleplayer;
mod state;
mod registry;

use bevy::prelude::*;

use bevy_egui::EguiPlugin;
use main_menu::MainMenuPlugin;
use registry::GameRegistry;
pub use singleplayer::SingleplayerState;

fn main() {
    App::new()
        .init_resource::<GameRegistry>()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            window: WindowDescriptor {
                title: "Quicksweeper".to_string(),
                ..default()
            },
            ..default()
        }))
        .add_plugin(EguiPlugin)
        .add_plugin(MainMenuPlugin)
        .add_plugin(singleplayer::SingleplayerMode)
        .add_plugin(multiplayer::MultiplayerMode)
        .add_plugin(common::QuicksweeperTypes)
        .add_plugin(load::LoadPlugin)
        .add_plugin(singleplayer::minefield::MinefieldPlugin)
        .add_plugin(server::ServerPlugin)
        .run();
}

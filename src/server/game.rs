//! ## How to create a quicksweeper game
//!
//! A quicksweeper gamemode is an entity that derives from the [GameBundle] bundle. When connections
//! are requested to it, the players will become children of the game, and the game will be given
//! management of their connections. Unfortunately, a gamemode right now is given trust over the
//! entire world, so caution should be exercised when modifying entities.
//!

use bevy::{prelude::*, utils::Uuid};

use serde::{Deserialize, Serialize};

use crate::registry::GameRegistry;

use super::{
    protocol::{ActiveGame, ClientData, ClientMessage, ServerData, ServerMessage},
    sockets::ConnectionInfo,
    IngameEvent,
};

#[derive(Component, Serialize, Deserialize, Debug, Clone)]
pub struct GameDescriptor {
    pub name: String,
    pub description: String,
}

#[derive(Component, Deref, DerefMut, Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GameMarker(pub Uuid);

#[derive(Bundle)]
pub struct GameBundle {
    pub marker: GameMarker,
}

pub fn server_messages(
    mut commands: Commands,
    mut incoming: ResMut<Events<ClientMessage>>,
    mut outgoing: EventWriter<ServerMessage>,
    mut game_events: EventWriter<IngameEvent>,
    active_games: Query<(Entity, &GameMarker, &Children)>,
    q_players: Query<&ConnectionInfo>,
    registry: Res<GameRegistry>,
) {
    let mut translate = |incoming: ClientMessage| {
        let data = match incoming.data {
            ClientData::Games => ServerData::ActiveGames(
                active_games
                    .iter()
                    .map(|(id, &marker, player_ids)| {
                        let players = player_ids
                            .iter()
                            .map(|&ent| q_players.get(ent).unwrap().username.clone())
                            .collect();
                        ActiveGame {
                            marker,
                            id, 
                            players,
                        }
                    })
                    .collect(),
            ),
            ClientData::Create { game, data } => {
                let game_id = commands.spawn((game,)).add_child(incoming.sender).id();
                game_events.send(IngameEvent::Create {
                    client: incoming.sender,
                    game: game_id,
                    kind: game,
                    data,
                });
                ServerData::Confirmed
            }
            ClientData::Join { game } => {
                if let Some(mut ent) = commands.get_entity(game) {
                    ent.add_child(incoming.sender);
                    ServerData::Confirmed
                } else {
                    ServerData::Malformed
                }
            }
            _ => ServerData::Malformed, // reject unimplemented requests for now
        };

        ServerMessage {
            receiver: incoming.sender,
            data,
        }
    };

    for incoming in incoming.drain() {
        outgoing.send(translate(incoming))
    }
}

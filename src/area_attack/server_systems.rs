use bevy::{hierarchy::HierarchyEvent, prelude::*};
use itertools::Itertools;
use strum::IntoEnumIterator;

use crate::{
    common::Position,
    load::Field,
    server::{
        Access, Connection, ConnectionInfo, ConnectionSwitch, GameMarker, IngameEvent, LocalEvent,
        MessageSocket,
    },
    singleplayer::minefield::{FieldShape, Minefield},
};

use super::{
    components::{
        AreaAttackBundle, ClientTile, Frozen, InitialSelections, PlayerBundle, PlayerColor,
        RevealTile, SendTile, ServerTile,
    },
    protocol::{AreaAttackRequest, AreaAttackUpdate},
    states::AreaAttackState,
    AreaAttackServer, AREA_ATTACK_MARKER,
};

#[derive(Component)]
pub struct Host;

pub fn create_game(
    mut commands: Commands,
    new_games: Query<(Entity, &GameMarker, &Children), Added<GameMarker>>,
    field_templates: Res<Assets<FieldShape>>,
    template_handles: Res<Field>,
) {
    for (game, kind, children) in new_games.iter() {
        if *kind != AREA_ATTACK_MARKER {
            continue;
        }

        // initialize game variables
        let template = template_handles.take_one(&mut rand::thread_rng()).clone();
        let bundle = AreaAttackBundle::new(&mut commands, template, &field_templates);
        commands.entity(game).insert(bundle);

        // there should only be one player right now, mark it as host
        let &player = children.iter().exactly_one().unwrap();
        commands.entity(player).insert(Host);
    }
}

pub fn unmark_init_access(mut access: Query<&mut Access, Added<AreaAttackServer>>) {
    access.for_each_mut(|mut access| {
        *access = Access::Open;
    })
}

// transcribes incoming messages to area attack events
pub fn net_events(
    mut network: EventReader<IngameEvent>,
    mut local: EventWriter<LocalEvent<AreaAttackRequest>>,
    games: Query<Entity, With<AreaAttackServer>>,
) {
    for ige @ IngameEvent { game, .. } in network.iter() {
        if games.contains(*game) {
            if let Ok(msg) = ige.transcribe() {
                local.send(msg);
            }
        }
    }
}

pub fn selection_transition(
    mut ev: EventReader<LocalEvent<AreaAttackRequest>>,
    mut games: Query<(&mut AreaAttackState, &InitialSelections, &Minefield)>,
    mut tiles: Query<&mut ServerTile>,
    maybe_host: Query<(), With<Host>>,
    mut connections: Query<&mut Connection>,
    mut request_tile: EventWriter<RevealTile>,
) {
    for ev in ev.iter() {
        if !matches!(ev.data, AreaAttackRequest::StartGame) {
            continue;
        }
        if let Ok((mut state, selections, field)) = games.get_mut(ev.game) {
            if maybe_host.get(ev.player).is_ok() && matches!(*state, AreaAttackState::Selecting) {
                *state = AreaAttackState::Stage1;

                let mut ignore = Vec::with_capacity(selections.len() * 16);

                // generate minefield while ignoring selected tiles
                for selection in selections.values() {
                    ignore.extend(selection.local_group());
                }

                field
                    .choose_multiple(&ignore, &mut rand::thread_rng())
                    .into_iter()
                    .for_each(|(_, tile_id)| {
                        *tiles.get_mut(tile_id).unwrap() = ServerTile::Mine;
                    });

                for (owner, selection) in selections.iter() {
                    request_tile.send(RevealTile {
                        position: *selection,
                        player: *owner,
                        game: ev.game,
                    })
                }

                connections.for_each_mut(|mut conn| {
                    conn.send_message(AreaAttackUpdate::Transition(AreaAttackState::Stage1));
                });
                println!("Transitioned game to stage 1")
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn reveal_tiles(
    mut requested: EventReader<RevealTile>,
    games: Query<&Minefield>,
    mut tiles: Query<&mut ServerTile>,
    mut send: EventWriter<SendTile>,
    time: Res<Time>,
    mut freeze: Query<&mut Frozen>,
    // swtich between request buffers for each iteration
    mut request_buffer: Local<Vec<RevealTile>>,
    mut request_buffer2: Local<Vec<RevealTile>>,
) {
    std::mem::swap(&mut *request_buffer, &mut *request_buffer2);
    request_buffer.extend(requested.iter());

    for RevealTile {
        position,
        player,
        game,
    } in request_buffer2.drain(..)
    {
        if let Ok(field) = games.get(game) {
            let mut frozen = freeze.get_mut(player).unwrap();
            if frozen.is_some() {
                continue;
            }
            let mut tile = tiles.get_mut(field[&position]).unwrap();
            match *tile {
                ServerTile::Empty => {
                    *tile = ServerTile::Owned { player };
                    let mine_count = field // TODO: Extract to function
                        .iter_neighbors(position)
                        .filter_map(|ent| tiles.get(ent).ok())
                        .filter(|tile| matches!(tile, ServerTile::Mine | ServerTile::HardMine))
                        .count() as u8;

                    if mine_count == 0 {
                        request_buffer.extend(field.iter_neighbor_positions(position).map(
                            |position| RevealTile {
                                position,
                                player,
                                game,
                            },
                        ))
                    }

                    send.send(SendTile {
                        tile: ServerTile::Owned { player },
                        position,
                        game,
                    })
                }
                ServerTile::Mine => {
                    *tile = ServerTile::HardMine; // TODO Freeze player
                    **frozen = Some(time.elapsed());
                    send.send(SendTile {
                        tile: ServerTile::HardMine,
                        position,
                        game,
                    });
                }
                ServerTile::HardMine | ServerTile::Owned { .. } => {
                    // do nothing
                }
            }
        }
    }
}

pub fn unfreeze_players(time: Res<Time>, mut freeze: Query<&mut Frozen>) {
    let instant = time.elapsed();
    for mut f in freeze.iter_mut() {
        if let Some(start) = **f {
            if (instant - start).as_secs() > 5 {
                **f = None;
            }
        }
    }
}

pub fn send_tiles(
    mut new_tiles: EventReader<SendTile>,
    tiles: Query<&ServerTile>,
    games: Query<(&Minefield, &Children)>,
    mut players: Query<(Entity, &mut Connection)>,
) {
    for SendTile {
        tile,
        position,
        game,
    } in new_tiles.iter()
    {
        if let Ok((minefield, children)) = games.get(*game) {
            let peers = players.iter_mut().filter(|(id, _)| children.contains(id));

            match tile {
                ServerTile::Empty | ServerTile::Mine => {
                    for (_, mut connection) in peers {
                        connection.send_message(AreaAttackUpdate::TileChanged {
                            position: *position,
                            to: ClientTile::Unknown,
                        });
                    }
                }
                ServerTile::Owned { player: owner } => {
                    let mine_count = minefield
                        .iter_neighbors(*position)
                        .filter_map(|ent| tiles.get(ent).ok())
                        .filter(|tile| matches!(tile, ServerTile::Mine | ServerTile::HardMine))
                        .count() as u8;

                    for (player_id, mut connection) in peers {
                        connection.send_message(AreaAttackUpdate::TileChanged {
                            position: *position,
                            to: ClientTile::Owned {
                                player: *owner,
                                num_neighbors: if player_id == *owner { mine_count } else { 0 },
                            },
                        });
                    }
                }
                ServerTile::HardMine => {
                    for (_, mut connection) in peers {
                        connection.send_message(AreaAttackUpdate::TileChanged {
                            position: *position,
                            to: ClientTile::HardMine,
                        });
                    }
                }
            }
        }
    }
}

pub fn broadcast_positions(
    mut requests: EventReader<LocalEvent<AreaAttackRequest>>,
    mut connections: Query<(&mut Position, &mut Connection)>,
    q_game: Query<&Children, With<AreaAttackServer>>,
) {
    for LocalEvent { player, game, data } in requests.iter() {
        if let AreaAttackRequest::Position(pos) = data {
            let peers = q_game.get(*game).unwrap();
            peers
                .iter()
                .filter(|&&e| *player != e)
                .for_each(|&conn_id| {
                    let (mut player_pos, mut sock) = connections.get_mut(conn_id).unwrap();
                    *player_pos = *pos;
                    sock.send_message(AreaAttackUpdate::Reposition {
                        id: *player,
                        position: *pos,
                    });
                })
        }
    }
}

pub fn update_selecting_tile(
    mut requests: EventReader<LocalEvent<AreaAttackRequest>>,
    mut games: Query<(&AreaAttackState, &mut InitialSelections, &Children)>,
    mut players: Query<(Entity, &mut Connection)>,
) {
    'event_loop: for LocalEvent { player, game, data } in requests.iter() {
        if let AreaAttackRequest::Reveal(requested) = data {
            let Ok((AreaAttackState::Selecting, mut selections, children)) = games.get_mut(*game) else { continue; };

            for selection in selections
                .iter()
                .filter_map(|(owner, pos)| (owner != player).then_some(pos))
            {
                if selection.distance(requested) < 10.0 {
                    // TODO Notify client or have client indicate this itself
                    continue 'event_loop;
                }
            }

            if let Some(previous_position) = selections.remove(player) {
                for mut conn in players
                    .iter_mut()
                    .filter_map(|(e, it)| (children.contains(&e)).then_some(it))
                {
                    conn.send_message(AreaAttackUpdate::TileChanged {
                        position: previous_position,
                        to: ClientTile::Unknown,
                    });
                }
            }
            for mut conn in players
                .iter_mut()
                .filter_map(|(e, it)| (children.contains(&e)).then_some(it))
            {
                conn.send_message(AreaAttackUpdate::TileChanged {
                    position: *requested,
                    to: ClientTile::Owned {
                        player: *player,
                        num_neighbors: 0,
                    },
                });
            }
            selections.insert(*player, *requested);
        }
    }
}

pub fn update_stage1_tile(
    mut requests: EventReader<LocalEvent<AreaAttackRequest>>,
    mut games: Query<&AreaAttackState>,
    mut reveal: EventWriter<RevealTile>,
) {
    for LocalEvent { player, game, data } in requests.iter() {
        let AreaAttackRequest::Reveal(pos) = data else {continue;};
        let Ok(AreaAttackState::Stage1) = games.get_mut(*game) else { continue; };

        reveal.send(RevealTile {
            position: *pos,
            player: *player,
            game: *game,
        })
    }
}

pub fn prepare_player(
    mut commands: Commands,
    mut ev: EventReader<ConnectionSwitch>,
    mut games: Query<(&Children, &Minefield, &FieldShape, &mut Access), With<AreaAttackServer>>,
    players: Query<(&ConnectionInfo, &mut PlayerColor, &Position)>,
    partial_connection_info: Query<&ConnectionInfo>,
    mut connections: Query<&mut Connection>,
) {
    for ev in ev.iter() {
        let ConnectionSwitch(HierarchyEvent::ChildAdded { child: player, parent: game }) = ev else {
            // TODO add ChildMoved variant as well
            continue;
        };
        let Ok((peers, minefield, shape, mut access)) = games.get_mut(*game) else { continue; };
        let Ok(mut this_connection) = connections.get_mut(*player) else {continue; };

        let peers = peers.iter().filter(|e| **e != *player).collect_vec();

        // guard if there are too many players
        if peers.len() >= 4 {
            *access = Access::Full // TODO: Reset when connection drops
        }

        let mut taken_colors = Vec::new();

        // send the selected board
        let _ = this_connection.send_message(AreaAttackUpdate::FieldShape(shape.clone()));

        // send list of players and player properties
        for &&peer_id in peers.iter() {
            if peer_id == *player {
                continue;
            }
            let (ConnectionInfo { username }, &color, &position) = players.get(peer_id).unwrap();
            taken_colors.push(color);
            let _ = this_connection.send_message(AreaAttackUpdate::PlayerProperties {
                id: peer_id,
                username: username.clone(),
                color,
                position,
            });
        }

        let assigned_color = PlayerColor::iter()
            .find(|co| !taken_colors.contains(co))
            .unwrap();
        let assigned_position = shape
            .center()
            .unwrap_or_else(|| minefield.iter_positions().next().unwrap());

        let this_username = &partial_connection_info.get(*player).unwrap().username;
        // send this player's properties to peers
        for &peer_id in peers {
            connections.get_mut(peer_id).unwrap().send_message(
                AreaAttackUpdate::PlayerProperties {
                    id: *player,
                    username: this_username.clone(),
                    color: assigned_color,
                    position: assigned_position,
                },
            );
        }

        // reobtain this connection to respect the lifetime of the query (since the previous loop
        // reuses this query)
        let mut this_connection = connections.get_mut(*player).unwrap();

        // initialize some player properties
        commands.entity(*player).insert(PlayerBundle {
            color: assigned_color,
            position: assigned_position,
            frozen: Frozen::default(),
        });
        let _ = this_connection.send_message(AreaAttackUpdate::SelfChange {
            color: assigned_color,
            position: assigned_position,
        });
    }
}

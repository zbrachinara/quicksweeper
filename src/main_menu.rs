use std::net::TcpStream;

use bevy::prelude::*;
use bevy_egui::EguiContext;
use egui::{Color32, InnerResponse, Key, RichText, TextEdit, Ui};
use iyes_loopless::{
    prelude::{AppLooplessStateExt, IntoConditionalSystem},
    state::{CurrentState, NextState},
};
use tungstenite::{handshake::client::Response, ClientHandshake, HandshakeError, WebSocket};

use crate::{
    cursor::Bindings,
    registry::{GameRegistry, REGISTRY},
    server::{
        ActiveGame, ClientMessage, CommonConnection as Connection, GameMarker, Greeting,
        ServerMessage,
    },
    Singleplayer,
};

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Menu {
    Loading,
    MainMenu,
    ServerSelect,
    GameSelect,

    // active/pause states
    Ingame,
    Pause,
}

type ClientResult =
    Result<(WebSocket<TcpStream>, Response), HandshakeError<ClientHandshake<TcpStream>>>;

#[derive(Resource, Default)]
struct MenuFields {
    remote_addr: String,
    username: String,
    remote_select_err: &'static str,
    #[cfg(target_arch = "wasm32")]
    trying_connection: Option<Connection>,
    #[cfg(not(target_arch = "wasm32"))]
    trying_connection: Option<ClientResult>,
}

pub fn standard_window<F, R>(
    ctx: &mut EguiContext,
    add_contents: F,
) -> Option<InnerResponse<Option<R>>>
where
    F: FnOnce(&mut Ui) -> R,
{
    egui::Window::new("")
        .resizable(false)
        .collapsible(false)
        .title_bar(false)
        .show(ctx.ctx_mut(), add_contents)
}

fn run_main_menu(mut commands: Commands, mut ctx: ResMut<EguiContext>) {
    standard_window(&mut ctx, |ui| {
        ui.vertical_centered(|ui| {
            let initial_height = ui.available_height();
            ui.label(
                RichText::new("Quicksweeper")
                    .size(32.0)
                    .color(Color32::GOLD),
            );
            if ui.button("Singleplayer mode").clicked() {
                commands.insert_resource(NextState(Singleplayer::PreGame));
                commands.insert_resource(NextState(Menu::Ingame));
            }
            if ui.button("Connect to server").clicked() {
                commands.insert_resource(NextState(Menu::ServerSelect))
            }
            let height = initial_height - ui.available_height();
            ui.set_max_height(height)
        });
    });
}

fn server_select_menu(
    mut commands: Commands,
    mut ctx: ResMut<EguiContext>,
    mut fields: Local<MenuFields>,
) {
    standard_window(&mut ctx, |ui| {
        let (focus_lost, focus_gained) = ui
            .vertical_centered(|ui| {
                ui.horizontal(|ui| {
                    if ui.button("back").clicked() {
                        commands.insert_resource(NextState(Menu::MainMenu))
                    }
                    ui.colored_label(Color32::RED, fields.remote_select_err);
                });

                let r1 = ui
                    .horizontal(|ui| {
                        ui.label("Server address:");
                        ui.add_enabled(
                            fields.trying_connection.is_none(),
                            TextEdit::singleline(&mut fields.remote_addr),
                        )
                    })
                    .inner;

                let r2 = ui
                    .horizontal(|ui| {
                        ui.label("Username:");
                        ui.add_enabled(
                            fields.trying_connection.is_none(),
                            TextEdit::singleline(&mut fields.username),
                        )
                    })
                    .inner;

                (
                    r1.lost_focus() || r2.lost_focus(),
                    r1.gained_focus() || r2.gained_focus(),
                )
            })
            .inner;

        if fields.trying_connection.is_some() {
            if let Some(mut socket) = {
                #[cfg(target_arch = "wasm32")]
                {
                    if fields.trying_connection.as_ref().unwrap().is_ready() {
                        std::mem::take(&mut fields.trying_connection)
                    } else {
                        None
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let maybe_handshake = std::mem::take(&mut fields.trying_connection).unwrap();
                    match maybe_handshake {
                        Ok((socket, _)) => Some(Connection::new_pc(socket)),
                        Err(HandshakeError::Interrupted(handshake)) => {
                            fields.trying_connection = Some(handshake.handshake());
                            None
                        }
                        Err(e) => {
                            eprintln!("{e}");
                            fields.remote_select_err = "Failed to perform handshake with server";
                            None
                        }
                    }
                }
            } {
                socket
                    .try_send(Greeting {
                        username: fields.username.clone(),
                    })
                    .map_err(|_| {
                        fields.remote_select_err = "Failure in initializing connection";
                    })?;

                socket.try_send(ClientMessage::Games).map_err(|_| {
                    fields.remote_select_err = "Could not retrieve games on the server"
                })?;

                commands.insert_resource(socket);
                commands.insert_resource(NextState(Menu::GameSelect));
            }
        }
        // execute requests to connect to server
        else if focus_lost && ui.input().key_pressed(Key::Enter) {
            let addr = format!("ws://{}/", fields.remote_addr);
            #[cfg(target_arch = "wasm32")]
            {
                fields.trying_connection = Some(Connection::new_web(&addr))
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let stream = TcpStream::connect(&fields.remote_addr).map_err(|_| {
                    fields.remote_select_err = "Could not find that address";
                })?;

                stream.set_nonblocking(true).map_err(|_| {
                    fields.remote_select_err = "Unable to set nonblocking connection mode";
                })?;

                fields.trying_connection = Some(tungstenite::client(addr, stream));
            }
        } else if focus_gained {
            fields.remote_select_err = "";
        }

        Result::<_, ()>::Ok(())
    });
}

/// Event issued when a multiplayer game has been selected. The corresponding game's client
/// implementation should then pick up this event and
#[derive(Deref)]
pub struct ToGame(pub GameMarker);

fn game_select_menu(
    mut commands: Commands,
    mut ctx: ResMut<EguiContext>,
    mut games: Local<Vec<ActiveGame>>,
    mut selected_gamemode: Local<(Option<String>, Option<GameMarker>)>,
    mut socket: ResMut<Connection>,
    mut start_game: EventWriter<ToGame>,
) {
    struct GameSelectResponse {
        go_back: egui::Response,
        reload: egui::Response,
        create: Option<GameMarker>,
        join_game: Option<(u64, GameMarker)>,
    }

    if let Some(Ok(ServerMessage::ActiveGames(v))) = socket.recv_message() {
        *games = v;
    }

    let response = standard_window(&mut ctx, |ui| {
        egui::Grid::new("window").show(ui, |ui| {
            // header
            let go_back = ui.button("⏴back");
            ui.label("Game select");
            let reload = ui.button("reload🔁");
            ui.end_row();

            let mut join_game = None;
            for game in games.iter() {
                if let Some(descriptor) = REGISTRY.get(&game.marker) {
                    ui.vertical(|ui| {
                        ui.label(&descriptor.name);
                        ui.label(&descriptor.description);
                    });

                    ui.label(""); // empty

                    if ui.button("Join").clicked() {
                        join_game = Some((game.id, game.marker))
                    }

                    ui.end_row();
                } else {
                    ui.label("Unsuppored game type");
                }
            }

            egui::ComboBox::from_label("gamemode")
                .selected_text(
                    selected_gamemode
                        .0
                        .as_deref()
                        .unwrap_or("<Choose gamemode>"),
                )
                .show_ui(ui, |ui| {
                    for (&marker, descriptor) in REGISTRY.iter() {
                        ui.selectable_value(
                            &mut *selected_gamemode,
                            (Some(descriptor.name.clone()), Some(marker)),
                            &descriptor.name,
                        );
                    }
                });
            let create = ui.button("+create").clicked();
            ui.end_row();

            GameSelectResponse {
                go_back,
                reload,
                create: create.then_some(selected_gamemode.1).flatten(),
                join_game,
            }
        })
    })
    .unwrap()
    .inner
    .unwrap()
    .inner;

    if response.go_back.clicked() {
        commands.insert_resource(NextState(Menu::MainMenu));
    } else if response.reload.clicked() {
        socket.send_logged(ClientMessage::Games);
    } else if let Some(mode) = response.create {
        socket.send_logged(ClientMessage::Create {
            game: mode,
            args: Vec::new(),
        });
        start_game.send(ToGame(mode));
        commands.insert_resource(NextState(Menu::Ingame));
    } else if let Some((game, marker)) = response.join_game {
        socket.send_logged(ClientMessage::Join { game });
        start_game.send(ToGame(marker));
        commands.insert_resource(NextState(Menu::Ingame));
    }
}

fn poll_connection(connection: Option<ResMut<Connection>>) {
    if let Some(mut connection) = connection {
        connection.repetition();
    }
}

fn pause(
    mut commands: Commands,
    state: Res<CurrentState<Menu>>,
    keybinds: Res<Bindings>,
    input: Res<Input<KeyCode>>,
) {
    if input.just_pressed(keybinds.pause) {
        if let Some(next) = match state.0 {
            Menu::Ingame => Some(Menu::Pause),
            Menu::Pause => Some(Menu::Ingame),
            _ => None,
        } {
            commands.insert_resource(NextState(dbg!(next)))
        }
    }
}

pub fn gltf_diagnostics(
    meshes: Query<(&Handle<Mesh>, &Name, &Parent)>,
    mut detected_meshes: Local<bool>,
) {
    if !*detected_meshes {
        for m in &meshes {
            *detected_meshes = true;
            println!("mesh: {m:?}")
        }
    }
}

pub struct MainMenuPlugin;

impl Plugin for MainMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_loopless_state(Menu::Loading)
            .add_event::<ToGame>()
            .init_resource::<MenuFields>()
            .add_system(poll_connection)
            .add_system(run_main_menu.run_in_state(Menu::MainMenu))
            .add_system(server_select_menu.run_in_state(Menu::ServerSelect))
            .add_system(game_select_menu.run_in_state(Menu::GameSelect))
            .add_system(pause)
            .add_enter_system(Menu::MainMenu, |mut commands: Commands| {
                commands.remove_resource::<Connection>()
            });
    }
}

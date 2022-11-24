use std::net::{TcpListener, TcpStream};

use bevy::prelude::*;
use tungstenite::WebSocket;

#[derive(Resource, DerefMut, Deref)]
pub struct OpenPort(TcpListener);

impl OpenPort {
    pub fn generate() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener
            .set_nonblocking(true)
            .expect("could not start server in nonblocking mode");
        println!("server bound to: {}", listener.local_addr().unwrap());
        Self(listener)
    }
}

#[derive(Component, Deref, DerefMut)]
pub struct Connection(WebSocket<TcpStream>);

pub fn receive_client(listener: Res<OpenPort>, mut commands: Commands) {
    if let Ok((client, _)) = listener.accept() {
        match tungstenite::accept(client) {
            Ok(socket) => {
                commands.spawn((Connection(socket),));
            }
            Err(msg) => eprintln!("Connection failed for reason: {msg:?}"),
        };
    }
}

pub fn listen_clients(mut clients: Query<(Entity, &mut Connection)>) {
    for (id, mut client) in clients.iter_mut() {
        if let Ok(msg) = client.read_message() {
            println!("From {id:?} received message {msg}")
        }
    }
}

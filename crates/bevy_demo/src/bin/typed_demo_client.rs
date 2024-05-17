
use bevy::prelude::*;
use bevy_demo::*;
use nifty_net_bevy::prelude::*;

const SERVER_ADDR: &'static str = "127.0.0.1:3000";


fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::log::LogPlugin::default());

    app.add_plugins(NetworkingPlugin);
    app.add_plugins(typed_plugin());

    app.add_systems(Startup, setup);
    app.add_systems(Update, (
        send_pings,
        receive_pongs,
    ));


    app.run();
}

fn setup(
    mut commands: Commands,
) {
    let mut socket = NetSocket::new(
        "0.0.0.0:0".parse().unwrap(),
        NetSocketConfig {
            socket_config: Config::default(),
            accept_incoming: false,
        },
    ).unwrap();

    socket.open_connection(SERVER_ADDR.parse().unwrap());

    commands.spawn((
        TypedSocket,
        socket,
    ));
}

fn send_pings(
    mut connected_r: EventReader<Connected>,
    mut messages: ResMut<TypedMessages<Ping>>,
) {
    for &Connected { connection_entity, connection_addr, ..} in connected_r.read() {
        info!("connected to {}", connection_addr);

        for i in 1..=5 {
            messages.send(Connections::One(connection_entity), true, &Ping {
                message: format!("Hello Server {}", i),
            });
        }
    }
}

fn receive_pongs(
    pongs: Res<TypedMessages<Pong>>,
) {
    for (_, Pong { message }) in pongs.iter() {
        info!("got a pong \"{}\"", message);
    }
}

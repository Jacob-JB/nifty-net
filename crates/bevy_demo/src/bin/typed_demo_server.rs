
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
        log_connections,
        receive_pings,
    ));

    app.run();
}

fn setup(
    mut commands: Commands,
) {
    commands.spawn(NetSocket::new(
        SERVER_ADDR.parse().unwrap(),
        NetSocketConfig {
            socket_config: Config::default(),
            accept_incoming: true,
        },
    ).unwrap());
}

fn log_connections(
    mut connected_r: EventReader<Connected>,
    mut disconnected_r: EventReader<Disconnected>,
) {
    for connected in connected_r.read() {
        info!("{} connected", connected.connection_addr);
    }

    for disconnected in disconnected_r.read() {
        info!("{} disconnected", disconnected.connection_addr);
    }
}

fn receive_pings(
    pings: Res<TypedMessages<Ping>>,
    mut pongs: ResMut<TypedMessages<Pong>>,
) {
    for (connection_entity, Ping { message }) in pings.iter() {
        info!("got a ping from {:?} \"{}\"", connection_entity, message);

        pongs.send(Connections::One(connection_entity), true, &Pong {
            message: format!("response to {}", message),
        });
    }
}

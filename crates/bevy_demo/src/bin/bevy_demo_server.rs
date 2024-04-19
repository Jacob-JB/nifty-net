
use bevy::prelude::*;
use nifty_net_bevy::prelude::*;

const SERVER_ADDR: &'static str = "127.0.0.1:3000";

fn main() {
    let mut app = App::new();

    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::log::LogPlugin::default());

    app.add_plugins(NetworkingPlugin);

    app.add_systems(Startup, setup);
    app.add_systems(Update, (
        read_messages,
        log_connections,
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

fn read_messages(
    mut client_q: Query<&mut Connection>,
) {
    for mut client in client_q.iter_mut() {
        let addr = client.address();
        for message in client.drain_messages() {
            info!("got data from {} {:?}", addr, message);
        }
    }
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

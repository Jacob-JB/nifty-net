
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
        send_data,
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

    commands.spawn(socket);
}

fn send_data(
    mut connected_r: EventReader<Connected>,
    mut connection_q: Query<&mut Connection>,
) {
    for &Connected { connection_entity, connection_addr, ..} in connected_r.read() {
        info!("connected to {}", connection_addr);

        let mut connection = connection_q.get_mut(connection_entity).unwrap();

        for i in 1..=5 {
            connection.send(true, vec![i; 30].into_boxed_slice());
        }
    }
}

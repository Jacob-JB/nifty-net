
use std::{collections::VecDeque, net::SocketAddr};

use bevy::{prelude::*, utils::HashMap};
use nifty_net::prelude::*;



/// sockets are updated in this set in [PreUpdate]
#[derive(Hash, Debug, PartialEq, Eq, Clone, SystemSet)]
pub struct UpdateSockets;


/// provides basic functionality for [NetSocket]s,
/// have them make an break [Connection]s
/// and send and receive messages
pub struct NetworkingPlugin;

impl Plugin for NetworkingPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<Connected>();
        app.add_event::<Disconnected>();
        app.add_event::<FailedConnection>();

        app.add_systems(PreUpdate, update_sockets.in_set(UpdateSockets));
    }
}


/// configuration for a [NetSocket] component
///
/// contains the [Config] for the underlying [Socket],
/// as well as it's own config
#[derive(Clone)]
pub struct NetSocketConfig {
    /// configuration for the underlying [Socket]
    pub socket_config: Config,
    /// when `true` the socket will accept incoming connections, else it will deny them
    pub accept_incoming: bool,
}

/// a wrapper around a [Socket]
///
/// create one and insert it into the world to open on a port
#[derive(Component)]
pub struct NetSocket {
    /// the wrapped socket
    socket: Socket,
    /// the local address of the socket
    addr: SocketAddr,
    /// whether to accept all incoming connections
    accept_incoming: bool,
    /// map of connected addresses to child connection entities
    connections: HashMap<SocketAddr, Entity>,
    /// queue of addresses to connect to
    connect_queue: VecDeque<SocketAddr>,
}

/// represents a connection on it's parent entity [NetSocket]
///
/// created whenever a connection is made and can be used to get the address of the connection
/// and send and receive messages
///
/// don't delete this entity yourself,
/// if you want to disconnect see [disconnect](Connection::disconnect)
#[derive(Component)]
pub struct Connection {
    /// the peer address of the connection
    addr: SocketAddr,
    /// messages that have been received and not read yet
    receive_queue: VecDeque<Box<[u8]>>,
    /// messages that have been sent and need to be pushed to the [NetSocket]
    ///
    /// the `bool` is if the message is reliable and the `Box` is the data
    send_queue: VecDeque<(bool, Box<[u8]>)>,
    /// marker to disconnect this connection
    disconnect: bool,
}

/// event fired when a new [Connection] is made on a [NetSocket]
#[derive(Event)]
pub struct Connected {
    /// the entity of the [NetSocket]
    pub socket_entity: Entity,
    /// the address of the socket
    pub socket_addr: SocketAddr,
    /// the entity of the [Connection]
    pub connection_entity: Entity,
    /// the address of the connection
    pub connection_addr: SocketAddr,
}

/// event fired when a [Connection] on a [NetSocket] is removed
#[derive(Event)]
pub struct Disconnected {
    /// the entity of the [NetSocket]
    pub socket_entity: Entity,
    /// the address of the socket
    pub socket_addr: SocketAddr,
    /// the entity the [Connection] had, but was despawned
    pub connection_entity: Entity,
    /// the address of the connection
    pub connection_addr: SocketAddr,
}

/// event fired when a [NetSocket] closed a connection before it was established
///
/// this happens when it opens a connection but never gets a response
#[derive(Event)]
pub struct FailedConnection {
    pub addr: SocketAddr,
}


impl NetSocket {
    /// binds to an address, returns a [NetSocket] if successful
    pub fn new(addr: SocketAddr, config: NetSocketConfig) -> Result<Self, std::io::Error> {
        Ok(NetSocket {
            socket: Socket::bind(addr, config.socket_config.clone())?,
            addr,
            accept_incoming: config.accept_incoming,
            connections: HashMap::new(),
            connect_queue: VecDeque::new(),
        })
    }

    /// sets whether the socket should accept incoming connections
    pub fn set_accept_incoming(&mut self, accept_incoming: bool) {
        self.accept_incoming = accept_incoming;
    }

    /// returns the local address of the socket
    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    /// makes the socket connect to an address in the next update
    ///
    /// will fire a warning if already connected to that address
    pub fn open_connection(&mut self, addr: SocketAddr) {
        self.connect_queue.push_back(addr);
    }
}

impl Connection {
    fn new(addr: SocketAddr) -> Self {
        Connection {
            addr,
            receive_queue: VecDeque::new(),
            send_queue: VecDeque::new(),
            disconnect: false,
        }
    }

    /// gets the peer address of the connection
    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    /// drains the receive message queue
    ///
    /// if you don't continuously call this messages will fill up forever causing a memory leak
    pub fn drain_messages(&mut self) -> impl Iterator<Item = Box<[u8]>> + '_ {
        self.receive_queue.drain(..)
    }

    /// send a message through the connection
    pub fn send(&mut self, reliable: bool, data: Box<[u8]>) {
        self.send_queue.push_back((reliable, data));
    }

    /// disconnect the connection in the next update
    pub fn disconnect(&mut self) {
        self.disconnect = true;
    }
}


fn update_sockets(
    mut commands: Commands,
    mut socket_q: Query<(Entity, &mut NetSocket, Option<&Children>)>,
    mut connection_q: Query<&mut Connection>,
    mut connected_w: EventWriter<Connected>,
    mut disconnected_w: EventWriter<Disconnected>,
    mut failed_connection_w: EventWriter<FailedConnection>,
    time: Res<Time>,
) {
    for (socket_entity, mut socket, socket_children) in socket_q.iter_mut() {
        // needed for borrow checker and change detection gets triggered anyway
        let socket = socket.as_mut();


        for addr in socket.connect_queue.drain(..) {
            if let Err(()) = socket.socket.open_connection(time.elapsed(), addr) {
                warn!("tried to connect to {} on {:?} {} but already connected", addr, socket_entity, socket.addr);
            }
        }


        if let Some(socket_children) = socket_children {
            for &connection_entity in socket_children.iter() {
                let Ok(mut connection) = connection_q.get_mut(connection_entity) else {
                    // only for connection children
                    continue;
                };

                let addr = connection.addr;

                for (reliable, data) in connection.send_queue.drain(..) {
                    if let Err(()) = socket.socket.send(addr, reliable, data) {
                        error!("tried to send a message to {} on {:?} {} but the connection didn't exist", addr, socket_entity, socket.addr);
                    }
                }

                if connection.disconnect {
                    if let Err(()) = socket.socket.close_connection(addr) {
                        error!("tried to close connection {} on {:?} {} but the connection didn't exist", addr, socket_entity, socket.addr);
                    }
                }
            }
        }


        // keep connections added this tick here until they can be flushed to the ecs between updates
        let mut new_connections = HashMap::new();

        socket.socket.update(time.elapsed(), |event| {
            match event {
                SocketEvent::Error(err) => {
                    error!("Socket Error: {:?}", err);
                },

                SocketEvent::ConnectionRequest { accept_connection, .. } => {
                    *accept_connection = socket.accept_incoming;
                },

                SocketEvent::NewConnection { addr } => {
                    let connection_entity = commands.spawn_empty().set_parent(socket_entity).id();

                    socket.connections.insert(addr, connection_entity);

                    new_connections.insert(connection_entity, Connection::new(addr));

                    connected_w.send(Connected {
                        socket_entity,
                        socket_addr: socket.addr,
                        connection_entity,
                        connection_addr: addr,
                    });
                },

                SocketEvent::ClosedConnection { addr } => {
                    let Some(connection_entity) = socket.connections.remove(&addr) else {
                        failed_connection_w.send(FailedConnection { addr });
                        return;
                    };

                    let Some(entity_commands) = commands.get_entity(connection_entity) else {
                        error!("tried to remove a connectio entity {:?} but it didn' exist", connection_entity);
                        return;
                    };

                    entity_commands.despawn_recursive();

                    disconnected_w.send(Disconnected {
                        socket_entity,
                        socket_addr: socket.addr,
                        connection_entity,
                        connection_addr: addr,
                    });
                },

                SocketEvent::Received { addr, data } => {
                    let Some(&connection_entity) = socket.connections.get(&addr) else {
                        error!("tried to receive data from {} but it wasn't connected", addr);
                        return;
                    };

                    if let Ok(mut connection) = connection_q.get_mut(connection_entity) {
                        connection.receive_queue.push_back(data);

                    } else if let Some(connection) = new_connections.get_mut(&connection_entity) {
                        connection.receive_queue.push_back(data);

                    } else {
                        error!("tried to receive data from {} into connection entity {:?} but couldn't find it", addr, connection_entity);
                    }
                },
            }
        });


        for (connection_entity, connection) in new_connections {
            commands.entity(connection_entity).insert(connection);
        }
    }
}

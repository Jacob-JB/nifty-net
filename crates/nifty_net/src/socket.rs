use std::{
    io::ErrorKind, net::{SocketAddr, UdpSocket}, time::Duration
};

use crate::{connection::{Connection, Connections}, packet::{Handshake, Packet}, Config, Error};


const RECV_BUFFER_SIZE: usize = u16::MAX as usize;

pub struct Socket {
    config: Config,
    udp_socket: UdpSocket,
    /// cached to not have constant reallocation
    receive_buffer: Option<Box<[u8; RECV_BUFFER_SIZE]>>,
    connections: Connections,
}

pub enum SocketEvent<'a> {
    Received {
        addr: SocketAddr,
        data: Box<[u8]>,
    },
    NewConnection {
        addr: SocketAddr,
    },
    ConnectionRequest {
        addr: SocketAddr,
        accept_connection: &'a mut bool,
    },
    ClosedConnection {
        addr: SocketAddr,
    },
    Error(Error),
}



impl Socket {
    /// binds to a port and creates a new socket
    pub fn bind(addr: SocketAddr, config: Config) -> Result<Self, std::io::Error> {
        let udp_socket = UdpSocket::bind(addr)?;

        udp_socket.set_nonblocking(true)?;

        Ok(Socket {
            config,
            udp_socket,
            receive_buffer: None,
            connections: Connections::new(),
        })
    }

    /// receives packets and updates internal state
    ///
    /// pass in a closure to handle events produced by the socket
    pub fn update(&mut self, time: Duration, mut event_handler: impl FnMut(SocketEvent)) {

        // update individual connections
        let mut connections_to_drop = Vec::new();

        for connection in self.connections.iter_mut() {
            if let Err(err) = connection.update(time, &self.config, &self.udp_socket) {
                event_handler(SocketEvent::Error(err));
            }

            if connection.should_drop() {
                connections_to_drop.push(connection.address());
            }

            if connection.just_connected() {
                event_handler(SocketEvent::NewConnection { addr: connection.address() })
            }
        }

        for addr in connections_to_drop {
            self.connections.remove_connection(addr);
            event_handler(SocketEvent::ClosedConnection { addr });
        }


        // receive and process messages from the `UdpSocket`

        // remove for ownership, reinitialize if it was dropped due to an error
        let mut receive_buffer = self.receive_buffer.take().unwrap_or_else(|| [0; RECV_BUFFER_SIZE].into());

        loop {
            let event = self.udp_socket.recv_from(receive_buffer.as_mut());

            match event {

                // received a packet
                Ok((received_bytes, addr)) => {

                    let bytes = receive_buffer.get(0..received_bytes).unwrap();
                    // handle in case of handshake
                    if let Some(handshake) = Handshake::deserialize_handshake(bytes) {
                        if handshake.protocol_id != self.config.protocol_id {
                            // ignore wrong protocol id's
                            continue;
                        }

                        if self.connections.get_connection(addr).is_some() {
                            // ignore duplicate handshakes
                            continue;
                        }

                        let mut accept_connection = false;
                        event_handler(SocketEvent::ConnectionRequest {
                            addr,
                            accept_connection: &mut accept_connection,
                        });

                        if accept_connection {
                            // unwrap is safe, connection doesn't exist
                            self.connections.new_connection(Connection::new(time, addr, false)).unwrap();
                        }

                        continue;
                    }

                    let Some(connection) = self.connections.get_connection_mut(addr) else {
                        // message is from an address without a connection
                        continue;
                    };

                    // parse the packet
                    let Some(packet) = Packet::deserialize(bytes) else {
                        event_handler(SocketEvent::Error(Error::MalformedPacket { addr }));
                        continue;
                    };

                    // handle the packet with the connection
                    if let Err(()) = connection.receive(time, &self.config, packet) {
                        event_handler(SocketEvent::Error(Error::MalformedPacket { addr }));
                    }
                },

                // some other event
                Err(err) => match err.kind() {

                    // nothing in queue
                    ErrorKind::WouldBlock => break,

                    // errors to ignore
                    ErrorKind::ConnectionReset |
                    ErrorKind::ConnectionRefused |
                    ErrorKind::ConnectionAborted => (),

                    // unhandled
                    _ => {
                        event_handler(SocketEvent::Error(Error::IoError(err)));
                        break;
                    },
                }
            }
        }

        // put allocated buffer back
        self.receive_buffer = Some(receive_buffer);


        // flush complete messages
        for connection in self.connections.iter_mut() {
            let addr = connection.address();
            connection.flush_messages(time, |data| {
                event_handler(SocketEvent::Received { addr, data });
            });
        }

    }

    /// opens a new connection with an address
    ///
    /// fails if there is already a connection to that address
    pub fn open_connection(&mut self, time: Duration, addr: SocketAddr) -> Result<(), ()> {
        let Ok(_) = self.connections.new_connection(Connection::new(time, addr, true)) else {
            return Err(());
        };

        Ok(())
    }

    /// sends a message to an address
    ///
    /// fails if there is no connection with that address, see [open_connection](Socket::open_connection)
    pub fn send(&mut self, addr: SocketAddr, reliable: bool, data: Box<[u8]>) -> Result<(), ()> {
        let Some(connection) = self.connections.get_connection_mut(addr) else {
            return Err(());
        };

        connection.send(reliable, data);

        Ok(())
    }

    /// returns the number of messages that have not yet been delivered to some address
    ///
    /// fails if there is no connection to that address
    pub fn messages_in_transit(&self, addr: SocketAddr) -> Result<usize, ()> {
        self.connections.get_connection(addr).map(|connection| connection.in_transit()).ok_or(())
    }

    /// drops the connection with an address
    ///
    /// returns `Err` if the connection didn't exist
    pub fn close_connection(&mut self, addr: SocketAddr) -> Result<(), ()> {
        if let Some(connection) = self.connections.get_connection_mut(addr) {
            connection.drop();
            Ok(())
        } else {
            Err(())
        }
    }
}


use std::time::Instant;

use nifty_net::prelude::*;


fn main() {
    let mut socket = Socket::bind(
        "0.0.0.0:3000".parse().unwrap(),
        Config::default(),
    ).expect("failed to bind address");

    let start_time = Instant::now();

    loop {
        socket.update(start_time.elapsed(), |event| {
            match event {
                SocketEvent::Error(err) => panic!("{:?}", err),

                SocketEvent::NewConnection { addr, accept_connection } => {
                    println!("new connection from {}", addr);
                    *accept_connection = true;
                },

                SocketEvent::Received { addr, data } => {
                    println!("received data from {} {:?}", addr, data);
                },

                SocketEvent::ClosedConnection { addr } => {
                    println!("connection closed {}", addr);
                }
            }
        });
    }
}

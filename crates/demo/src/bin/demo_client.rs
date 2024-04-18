
use std::time::Instant;

use nifty_net::prelude::*;


const SERVER_ADDR: &'static str = "127.0.0.1:3001";


fn main() {
    let mut socket = Socket::bind(
        "0.0.0.0:0".parse().unwrap(),
        Config {
            mtu: 20,
            ..Default::default()
        },
    ).expect("failed to bind address");

    let start_time = Instant::now();


    let server_addr = SERVER_ADDR.parse().unwrap();
    socket.open_connection(start_time.elapsed(), server_addr).unwrap();

    for i in 1..=5 {
        socket.send(server_addr, true, vec![i; 30].into_boxed_slice()).unwrap();
    }


    let mut closed = false;

    loop {
        socket.update(start_time.elapsed(), |event| {
            match event {
                SocketEvent::Error(err) => {
                    println!("socket error {:?}", err);
                },

                SocketEvent::NewConnection { addr } => {
                    println!("new connection with {}", addr);
                },

                SocketEvent::ConnectionRequest { .. } => (),

                SocketEvent::Received { addr, data } => {
                    println!("received data from {} {:?}", addr, data);
                },

                SocketEvent::ClosedConnection { addr } => {
                    println!("connection closed {}", addr);
                    closed = true;
                }
            }
        });

        if closed {
            return;
        }

        if socket.messages_in_transit(server_addr).unwrap() == 0 {
            socket.close_connection(server_addr).unwrap();
            println!("all messages received, closing");
        }
    }
}


use std::time::{Duration, Instant};

use nifty_net::prelude::*;


const SERVER_ADDR: &'static str = "127.0.0.1:3001";
const PING_DELAY: Duration = Duration::from_millis(2000);


fn main() {
    let mut socket = Socket::bind(
        "0.0.0.0:0".parse().unwrap(),
        Config {
            mtu: 20,
            ..Default::default()
        },
    ).expect("failed to bind address");

    let start_time = Instant::now();

    socket.open_connection(start_time.elapsed(), SERVER_ADDR.parse().unwrap()).unwrap();


    let mut last_ping = Instant::now();
    let mut counter = 0;

    let mut closed = false;

    loop {
        socket.update(start_time.elapsed(), |event| {
            match event {
                SocketEvent::Error(err) => panic!("{:?}", err),

                SocketEvent::NewConnection { .. } => (),

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

        if last_ping.elapsed() >= PING_DELAY {
            last_ping = Instant::now();
            println!("ping {}", counter);
            socket.send(SERVER_ADDR.parse().unwrap(), true, vec![counter; 20].into_boxed_slice()).unwrap();
            counter += 1;
        }
    }
}

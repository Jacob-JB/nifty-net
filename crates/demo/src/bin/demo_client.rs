
use std::time::{Duration, Instant};

use nifty_net::prelude::*;


const RELAY_ADDR: &'static str = "127.0.0.1:3001";
const PING_DELAY: Duration = Duration::from_millis(2000);


fn main() {
    let mut socket = Socket::bind(
        "0.0.0.0:0".parse().unwrap(),
        Config {
            mtu: 20,
            ..Default::default()
        },
    ).expect("failed to bind address");

    let server_addr = RELAY_ADDR.parse().unwrap();
    socket.open_connection(server_addr).unwrap();

    let start_time = Instant::now();

    let mut last_ping = Instant::now();
    let mut counter = 0;

    loop {
        socket.update(start_time.elapsed(), |event| {
            match event {
                SocketEvent::Error(err) => panic!("{:?}", err),

                SocketEvent::NewConnection { .. } => (),

                SocketEvent::Received { addr, data } => {
                    println!("received data from {} {:?}", addr, data);
                }
            }
        });

        if last_ping.elapsed() >= PING_DELAY {
            last_ping = Instant::now();
            println!("ping {}", counter);
            socket.send(server_addr, true, vec![counter; 20].into_boxed_slice()).unwrap();
            counter += 1;
        }
    }
}

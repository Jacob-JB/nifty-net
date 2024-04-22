use std::{collections::HashMap, net::{SocketAddr, UdpSocket}, time::{Duration, Instant}};

use rand::Rng;

const SERVER_ADDR: &'static str = "127.0.0.1:3000";
const RELAY_ADDR: &'static str = "127.0.0.1:3001";

/// change this to adjust packet loss
fn drop_packet() -> bool {
    // lots of packet loss
    rand::thread_rng().gen_bool(0.25)
}

/// change this to adjust rtt
fn delay_packet() -> Duration {
    // lots of delay
    Duration::from_millis(rand::thread_rng().gen_range(100..=300))
}

fn main() {
    let mut server_socket = Socket::new(RELAY_ADDR);
    let mut client_sockets = HashMap::new();

    let start_time = Instant::now();

    let mut buffer = vec![0; u32::MAX as usize].into_boxed_slice();

    loop {
        while let Ok((received, addr)) = server_socket.socket.recv_from(&mut buffer) {
            let bytes = buffer[0..received].into();

            let client_socket = client_sockets.entry(addr).or_insert_with(|| {
                let socket = Socket::new("0.0.0.0:0");
                println!("relaying {} -> {}", addr, socket.socket.local_addr().unwrap());
                socket
            });

            if drop_packet() {
                continue;
            }

            client_socket.queue(
                start_time.elapsed() + delay_packet(),
                SERVER_ADDR.parse().unwrap(),
                bytes,
            );
        }

        server_socket.flush(start_time.elapsed());


        for (&client_addr, client_socket) in client_sockets.iter_mut() {
            while let Ok((received, _)) = client_socket.socket.recv_from(&mut buffer) {
                let bytes = buffer[0..received].into();

                if drop_packet() {
                    continue;
                }

                server_socket.queue(
                    start_time.elapsed() + delay_packet(),
                    client_addr,
                    bytes,
                );
            }

            client_socket.flush(start_time.elapsed());
        }
    }

}


struct Socket {
    socket: UdpSocket,
    queue: Vec<(Duration, SocketAddr, Box<[u8]>)>,
}

impl Socket {
    fn new(addr: &str) -> Self {
        let socket = UdpSocket::bind(addr).unwrap();
        socket.set_nonblocking(true).unwrap();

        Socket {
            socket,
            queue: Vec::new(),
        }
    }

    fn queue(&mut self, send_time: Duration, addr: SocketAddr, bytes: Box<[u8]>) {
        self.queue.push((send_time, addr, bytes));
    }

    fn flush(&mut self, current_time: Duration) {
        self.queue.retain(|(send_time, addr, bytes)| {
            if *send_time <= current_time {
                self.socket.send_to(bytes, addr).unwrap();
                false
            } else {
                true
            }
        })
    }
}

use std::{collections::{hash_map::Entry, HashMap, VecDeque}, net::{SocketAddr, UdpSocket}, time::Duration};

use crate::{
    message::*,
    packet::*,
    metrics::*,
    Config,
    Error,
};




pub struct Connection {
    addr: SocketAddr,

    /// this will be some whilst trying to establish a connection
    ///
    /// gets set to `None` when a heartbeat is received
    ///
    /// contains the time the last heartbeat was sent at
    last_handshake: Option<Option<Duration>>,

    last_heartbeat: Duration,
    /// a queue of heartbeats to respond to
    heartbeat_responses: Vec<Heartbeat>,
    rtt_samples: VecDeque<Duration>,
    /// recalculated when `rtt_samples` changes
    cached_rtt: Option<Duration>,
    last_keep_alive: Duration,

    next_fragmentation_id: u16,
    send_messages: Vec<SendMessage>,

    receive_messages: Vec<ReceiveMessage>,
    /// acknowledgements to send
    acknowledgements: Vec<Acknowledgement>,
    reliable_blacklist: Vec<(Duration, u16)>,

    /// when set to true the connection will continue to function
    /// but be removed at the end of the next update
    drop_connection: bool,
    /// set to true to signal that a connection socket event needs to be fired
    just_connected: bool,

    // metrics
    sent_packets: u64,
    sent_bytes: u64,
    reliable_message_count: u64,
    unreliable_message_count: u64,
}

pub struct Connections {
    connections: HashMap<SocketAddr, Connection>,
}

struct PacketGrouper<'a> {
    addr: SocketAddr,
    socket: &'a UdpSocket,
    mtu: u16,
    current_packet: Packet,
    sent_packets: &'a mut u64,
    sent_bytes: &'a mut u64,
}


impl Connections {
    pub fn new() -> Self {
        Connections {
            connections: HashMap::new(),
        }
    }

    /// tries to create a new connection
    ///
    /// will return [Ok] if the connection didn't exist with a mutable reference to the [Connection]
    ///
    /// will return [Err] if there is already a connection with that address
    pub fn new_connection(&mut self, connection: Connection) -> Result<&mut Connection, ()> {
        let entry: Entry<'_, SocketAddr, Connection> = self.connections.entry(connection.addr);

        match entry {
            Entry::Occupied(_) => Err(()),
            Entry::Vacant(entry) => Ok(entry.insert(connection)),
        }
    }

    pub fn get_connection(&self, addr: SocketAddr) -> Option<&Connection> {
        self.connections.get(&addr)
    }

    pub fn get_connection_mut(&mut self, addr: SocketAddr) -> Option<&mut Connection> {
        self.connections.get_mut(&addr)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Connection> + '_ {
        self.connections.values_mut()
    }

    pub fn remove_connection(&mut self, addr: SocketAddr) {
        self.connections.remove(&addr);
    }
}


impl Connection {
    /// creates a new connection at some `time` to some `addr`
    ///
    /// `opening_party` should be true if this socket is the one responsible for creating the connection,
    /// meaning it has to wait before knowing that the connection is established
    pub fn new(time: Duration, addr: SocketAddr, opening_party: bool) -> Self {
        Connection {
            addr,

            last_handshake: if opening_party {
                Some(None)
            } else {
                None
            },

            last_heartbeat: Duration::ZERO,
            heartbeat_responses: Vec::new(),
            rtt_samples: VecDeque::new(),
            cached_rtt: None,
            last_keep_alive: time,

            next_fragmentation_id: 0,
            send_messages: Vec::new(),

            receive_messages: Vec::new(),
            acknowledgements: Vec::new(),
            reliable_blacklist: Vec::new(),

            drop_connection: false,
            just_connected: !opening_party,

            sent_packets: 0,
            sent_bytes: 0,
            reliable_message_count: 0,
            unreliable_message_count: 0,
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    pub fn send(&mut self, reliable: bool, data: Box<[u8]>) {
        let fragmentation_id = self.next_fragmentation_id;
        self.next_fragmentation_id = self.next_fragmentation_id.wrapping_add(1);

        self.send_messages.push(SendMessage::new(reliable, fragmentation_id, data));

        if reliable {
            self.reliable_message_count += 1;
        } else {
            self.unreliable_message_count += 1;
        }
    }

    pub fn update(&mut self, time: Duration, config: &Config, socket: &UdpSocket) -> Result<(), Error> {

        // timeout connection
        if self.last_keep_alive + config.timeout_delay < time {
            self.drop_connection = true;
        }

        // pause normal logic until a connection has been established
        if let Some(last_handshake) = self.last_handshake.as_mut() {
            if if let Some(last_handshake) = last_handshake {
                // time to send another handshake
                *last_handshake + config.heartbeat_interval <= time
            } else {
                // never sent a handshake
                true
            } {
                *last_handshake = Some(time);

                let sent_bytes = Handshake {
                    protocol_id: config.protocol_id,
                }.send(self.addr, socket).map_err(|err| Error::IoError(err))?;

                // update metrics
                self.sent_packets += 1;
                self.sent_bytes += sent_bytes as u64;
            }

            return Ok(());
        }

        let mut grouper = PacketGrouper::new(self.addr, socket, config.mtu, &mut self.sent_packets, &mut self.sent_bytes);

        let resend_delay = self.cached_rtt.map(|rtt| Duration::from_secs_f32(
            rtt.as_secs_f32() * config.reliable_resend_threshold
        ));

        // send message fragments
        for message in self.send_messages.iter_mut() {

            // decide whether to send fragments
            let send_fragments = 'b: {
                let Some(last_sent) = message.reliable() else {
                    // unreliable, always send
                    break 'b true;
                };

                let Some(last_sent) = last_sent else {
                    // reliable but have never sent
                    break 'b true;
                };

                let Some(resend_delay) = resend_delay else {
                    // have sent once but no rtt calculated, wait for rtt
                    break 'b false;
                };

                if *last_sent + resend_delay <= time {
                    // send if resend threshold has been reached
                    break 'b true;
                }

                false
            };

            if !send_fragments {
                continue;
            }

            let mut deliverd_intervals = message.get_deliverd_intervals();

            loop {
                let available_space = grouper.space_left();

                let Some(blob) = message.create_blob(&mut deliverd_intervals, available_space) else {
                    // no more blobs to send
                    break;
                };

                let Some(blob) = blob else {
                    // not enough space for a blob
                    grouper.create_space()?;
                    continue;
                };

                grouper.push(blob);
            }

            if let Some(last_sent) = message.reliable() {
                // if reliable, mark now as the last sent time
                *last_sent = Some(time);
            } else {
                // if unreliable assume that the packets were delivered
                message.set_delivered_intervals(deliverd_intervals);
            }
        }
        self.send_messages.retain(|message| !message.delivered());


        // send heartbeats
        if self.last_heartbeat + config.heartbeat_interval <= time {
            self.last_heartbeat = time;

            let blob = Blob::Heartbeat(Heartbeat::new(time));
            grouper.ensure_space(blob.size())?;
            grouper.push(blob);
        }


        // send heartbeat responses
        for heartbeat in self.heartbeat_responses.drain(..) {
            let blob = Blob::HeartbeatResponse(heartbeat);
            grouper.ensure_space(blob.size())?;
            grouper.push(blob);
        }


        // send acknowledgements
        for ack in self.acknowledgements.drain(..) {
            let blob = Blob::Acknowledgement(ack);
            grouper.ensure_space(blob.size())?;
            grouper.push(blob);
        }


        // send disconnect message if just decided to drop
        if self.drop_connection {
            let blob = Blob::Disconnect;
            grouper.ensure_space(blob.size())?;
            grouper.push(blob);
        }


        grouper.send_remaining()?;


        // drop incomplete unreliable messages
        if let Some(rtt) = self.round_trip_time() {
            let drop_delay = Duration::from_secs_f32(
                rtt.as_secs_f32() * config.unreliable_drop_threshhold
            );

            self.receive_messages.retain(|message| {
                message.is_reliable() || // message is reliable, never drop
                message.last_received_time() + drop_delay > time // drop threshold is in the future, don't drop yet
            });
        }


        // trim reliable message blacklist
        if let Some(rtt) = self.round_trip_time() {
            let trim_delay = Duration::from_secs_f32(
                rtt.as_secs_f32() * config.reliable_message_blacklist_memory
            );

            self.trim_blacklist(time.saturating_sub(trim_delay));
        }


        Ok(())
    }

    /// processes a [Packet]
    ///
    /// fails if the packet had malformed data
    pub fn receive(&mut self, time: Duration, config: &Config, packet: Packet) -> Result<(), ()> {
        self.last_keep_alive = time;

        for blob in packet.into_iter() {
            match blob {
                Blob::Fragment(fragment) => {
                    let ack = fragment.acknowledgement();

                    // ignore blacklisted reliable ids
                    if !(fragment.send_ack && self.is_blacklisted(fragment.fragmentation_id)) {
                        if let Some(message) = self.receive_messages.iter_mut().find(
                            |message| message.fragmentation_id() == fragment.fragmentation_id
                        ) {
                            message.add_fragment(time, fragment)?;
                        } else {
                            self.receive_messages.push(ReceiveMessage::new(time, fragment)?);
                        }
                    }

                    if let Some(ack) = ack {
                        self.acknowledgements.push(ack);
                    }
                },

                Blob::Heartbeat(heartbeat) => {
                    if self.last_handshake.is_some() {
                        self.just_connected = true;
                        self.last_handshake = None;
                    }

                    self.heartbeat_responses.push(heartbeat);
                },

                Blob::HeartbeatResponse(heartbeat) => {
                    let rtt = time.saturating_sub(heartbeat.time());

                    self.rtt_samples.push_back(rtt);

                    while self.rtt_samples.len() > config.rtt_memory {
                        self.rtt_samples.pop_front();
                    }

                    self.cached_rtt = self.rtt_samples.iter().sum::<Duration>().checked_div(self.rtt_samples.len() as u32);
                },

                Blob::Acknowledgement(ack) => {
                    if let Some(message) = self.send_messages.iter_mut().find(
                        |message| message.fragmentation_id() == ack.fragmentation_id
                    ) {
                        message.set_delivered(ack.start as usize .. (ack.start as usize + ack.len as usize))?;
                    }
                },

                Blob::Disconnect => {
                    self.drop_connection = true;
                },
            }
        }

        Ok(())
    }

    /// flushes any complete messages, returning them
    pub fn flush_messages(&mut self, time: Duration, mut flush: impl FnMut(Box<[u8]>)) {
        let mut i = 0;
        while let Some(message) = self.receive_messages.get(i) {
            if message.complete() {
                if message.is_reliable() {
                    self.blacklist_id(time, message.fragmentation_id());
                }

                flush(self.receive_messages.remove(i).data());
            } else {
                i += 1;
            }
        }
    }

    /// gets the round trip time
    ///
    /// takes an average from the last few samples collected from heartbeats.
    /// the number of samples is configured in the config
    ///
    /// returns `None` if there are no samples to calculate from
    ///
    /// rtt includes processing delay of the opposite connection
    pub fn round_trip_time(&self) -> Option<Duration> {
        self.cached_rtt
    }


    fn blacklist_id(&mut self, time: Duration, id: u16) {
        self.reliable_blacklist.push((time, id));
    }

    fn is_blacklisted(&self, id: u16) -> bool {
        self.reliable_blacklist.iter().any(|(_, blacklisted_id)| *blacklisted_id == id)
    }

    fn trim_blacklist(&mut self, earliest: Duration) {
        self.reliable_blacklist.retain(|(time, _)| *time >= earliest);
    }

    /// returns the number of messages that haven't been delivered yet
    pub fn in_transit(&self) -> usize {
        self.send_messages.len()
    }

    pub fn drop(&mut self) {
        self.drop_connection = true;
    }

    pub fn should_drop(&self) -> bool {
        self.drop_connection
    }

    pub fn just_connected(&mut self) -> bool {
        if self.just_connected {
            self.just_connected = false;
            true
        } else {
            false
        }
    }

    pub fn metrics(&self) -> ConnectionMetrics {
        ConnectionMetrics {
            sent_packets: self.sent_packets,
            sent_bytes: self.sent_bytes,
            rtt: self.cached_rtt,
            unreliable_message_count: self.unreliable_message_count,
            reliable_message_count: self.reliable_message_count,
        }
    }
}

impl<'a> PacketGrouper<'a> {
    fn new(addr: SocketAddr, socket: &'a UdpSocket, mtu: u16, sent_packets: &'a mut u64, sent_bytes: &'a mut u64) -> Self {
        PacketGrouper {
            addr,
            socket,
            mtu,
            current_packet: Packet::new(),
            sent_packets,
            sent_bytes,
        }
    }

    fn space_left(&self) -> u16 {
        self.current_packet.space_left(self.mtu)
    }

    /// adds a blob to the current packet
    ///
    /// does not check agains mtu
    fn push(&mut self, blob: Blob) {
        self.current_packet.push(blob);
    }

    /// either garuntees that there is enough space for a blob, or errors
    fn ensure_space(&mut self, space_needed: u16) -> Result<(), Error> {
        if self.space_left() < space_needed {
            self.create_space()?;
        }

        if self.current_packet.space_left(self.mtu) < space_needed {
            return Err(Error::MtuTooSmall);
        }

        Ok(())
    }

    /// get more space by sending the current packet
    ///
    /// errors if the packet is empty and no space can be created
    fn create_space(&mut self) -> Result<(), Error> {
        if self.current_packet.blob_count() == 0 {
            return Err(Error::MtuTooSmall);
        }

        let sent_bytes = self.current_packet.send(self.addr, &self.socket).map_err(|err| Error::IoError(err))?;
        self.current_packet = Packet::new();

        *self.sent_packets += 1;
        *self.sent_bytes += sent_bytes as u64;

        Ok(())
    }

    fn send_remaining(self) -> Result<(), Error> {
        if self.current_packet.blob_count() > 0 {
            if let Err(err) = self.current_packet.send(self.addr, &self.socket) {
                return Err(Error::IoError(err));
            }
        }

        Ok(())
    }
}

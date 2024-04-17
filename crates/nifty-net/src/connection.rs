use std::{collections::{hash_map::Entry, HashMap, VecDeque}, net::{SocketAddr, UdpSocket}, time::Duration};

use crate::{message::{ReceiveMessage, SendMessage}, packet::{Acknowledgement, Blob, Heartbeat, Packet}, Config, Error};




pub struct Connection {
    addr: SocketAddr,

    last_heartbeat: Duration,
    /// a queue of heartbeats to respond to
    heartbeat_responses: Vec<Heartbeat>,
    rtt_samples: VecDeque<Duration>,
    /// recalculated when `rtt_samples` changes
    cached_rtt: Option<Duration>,

    next_fragmentation_id: u16,
    send_messages: Vec<SendMessage>,

    receive_messages: Vec<ReceiveMessage>,
    /// acknowledgements to send
    acknowledgements: Vec<Acknowledgement>,
    reliable_blacklist: Vec<(Duration, u16)>,
}

pub struct Connections {
    connections: HashMap<SocketAddr, Connection>,
}

struct PacketGrouper<'a> {
    addr: SocketAddr,
    socket: &'a UdpSocket,
    mtu: u16,
    current_packet: Packet,
}


impl Connections {
    pub fn new() -> Self {
        Connections {
            connections: HashMap::new(),
        }
    }

    /// tries to create a new connection
    ///
    /// will return [Some] if the socket didn't exist with a mutable reference to the [Connection]
    ///
    /// will return [None] if the socket existed
    pub fn new_connection(&mut self, addr: SocketAddr) -> Option<&mut Connection> {
        let entry = self.connections.entry(addr);

        match entry {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => Some(entry.insert(
                Connection::new(addr)
            )),
        }
    }

    pub fn get_connection_mut(&mut self, addr: SocketAddr) -> Option<&mut Connection> {
        self.connections.get_mut(&addr)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Connection> + '_ {
        self.connections.values_mut()
    }
}


impl Connection {
    pub fn new(addr: SocketAddr) -> Self {
        Connection {
            addr,

            last_heartbeat: Duration::ZERO,
            heartbeat_responses: Vec::new(),
            rtt_samples: VecDeque::new(),
            cached_rtt: None,

            next_fragmentation_id: 0,
            send_messages: Vec::new(),

            receive_messages: Vec::new(),
            acknowledgements: Vec::new(),
            reliable_blacklist: Vec::new(),
        }
    }

    pub fn address(&self) -> SocketAddr {
        self.addr
    }

    pub fn send(&mut self, reliable: bool, data: Box<[u8]>) {
        let fragmentation_id = self.next_fragmentation_id;
        self.next_fragmentation_id = self.next_fragmentation_id.wrapping_add(1);

        self.send_messages.push(SendMessage::new(reliable, fragmentation_id, data));
    }

    pub fn update(&mut self, time: Duration, config: &Config, socket: &UdpSocket) -> Result<(), Error> {
        let mut grouper = PacketGrouper::new(self.addr, socket, config.mtu);

        let resend_delay = self.round_trip_time().map(|rtt| Duration::from_secs_f32(
            rtt.as_secs_f32() * config.reliable_resend_threshold
        ));

        // send message fragments
        for message in self.send_messages.iter_mut() {
            let send_blobs = 'b: {
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

                // send if resend threshold has been reached
                *last_sent + resend_delay <= time
            };

            if !send_blobs {
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
                }
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
}

impl<'a> PacketGrouper<'a> {
    fn new(addr: SocketAddr, socket: &'a UdpSocket, mtu: u16) -> Self {
        PacketGrouper {
            addr,
            socket,
            mtu,
            current_packet: Packet::new(),
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

        self.current_packet.send(self.addr, &self.socket).map_err(|err| Error::IoError(err))?;
        self.current_packet = Packet::new();

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

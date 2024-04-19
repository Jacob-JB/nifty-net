
pub mod socket;
pub(crate) mod connection;
pub(crate) mod packet;
pub(crate) mod message;

pub mod prelude {
    pub use crate::socket::{Socket, SocketEvent};
    pub use crate::Config;
}

#[derive(Clone)]
pub struct Config {
    /// when a handshake is received, connections will only be established with matching protocol id's
    pub protocol_id: u64,
    /// the maximum transmission unit for packets
    ///
    /// messages larger than this will be fragmented
    /// and smaller messages will be grouped together up to this size
    pub mtu: u16,
    /// the interval to send heartbeat messages at
    ///
    /// heartbeats are used to keep the connection alive and estimate rtt
    pub heartbeat_interval: std::time::Duration,
    /// the interval to send handshakes at
    ///
    /// handshake requests might be dropped,
    /// this is how long to wait before sending another until connected
    pub handshake_interval: std::time::Duration,
    /// how many round trip time samples to keep to calculate an average from
    pub rtt_memory: usize,
    /// what multiple of the round trip time to wait before resending unacknowledged fragments
    ///
    /// the lower this is the more likely an unecessary resend occurs
    ///
    /// on average an acknowledgement will come back after one rtt,
    /// so values close to or less than one will cause significantly
    /// increased bandwidth usage for not much benefit
    pub reliable_resend_threshold: f32,
    /// what multiple of the round trip time to wait before dropping incomplete unreliable messages
    ///
    /// if unreliable messages get fragmented and not all of the message is received
    /// then the incomplete message will sit in memory until this threshold is reached
    pub unreliable_drop_threshhold: f32,
    /// what multiple of the round trip time to wait before forgetting the id of a reliable message
    ///
    /// when reliable message fragments get retransmitted because the ack wasn't received,
    /// they could be received twice after the message has been completed.
    /// in order to recognise that it is not a new message we need to keep a blacklist
    /// of received messages. this option controls how long to wait before forgetting those ids.
    /// if it is too low, then reliable message fragments won't be ignored and will be received twice at best
    /// and at worst be a memory leak as it waits forever for other fragments to complete it
    pub reliable_message_blacklist_memory: f32,
    /// how long to wait before dropping a connection because no packets were received
    pub timeout_delay: std::time::Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            protocol_id: 0,
            mtu: 1500,
            heartbeat_interval: std::time::Duration::from_millis(500),
            handshake_interval: std::time::Duration::from_millis(100),
            rtt_memory: 16,
            reliable_resend_threshold: 1.25,
            unreliable_drop_threshhold: 4.,
            reliable_message_blacklist_memory: 8.,
            timeout_delay: std::time::Duration::from_millis(10_000),
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// an io error occurred
    IoError(std::io::Error),
    /// the mtu in the config made it impossible to complete a task
    MtuTooSmall,
    /// some part of a packet from `add` was malformed
    MalformedPacket {
        addr: std::net::SocketAddr,
    },
}

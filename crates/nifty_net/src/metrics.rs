use std::time::Duration;

#[derive(Clone, Default)]
pub struct ConnectionMetrics {
    /// how many UDP packets have been sent from this connection
    pub sent_packets: u64,
    /// total number of bytes that have been sent from this connection
    pub sent_bytes: u64,
    /// the estimated round trip time (ping) of this connection
    ///
    /// is `None` if there have been zero samples to estimate from
    pub rtt: Option<Duration>,
    /// how many total unreliable messages have been sent
    pub unreliable_message_count: u64,
    /// how many total reliable messages have been sent
    pub reliable_message_count: u64,
    /// how many in transit reliable messages have not been acknowledged as received yet
    pub messages_in_transit: usize,
}

use std::time::Duration;

#[derive(Clone, Default)]
pub struct ConnectionMetrics {
    pub sent_packets: u64,
    pub sent_bytes: u64,
    pub rtt: Option<Duration>,
    pub unreliable_message_count: u64,
    pub reliable_message_count: u64,
}

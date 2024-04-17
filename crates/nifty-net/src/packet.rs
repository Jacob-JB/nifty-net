use std::{mem::size_of, net::{SocketAddr, UdpSocket}, time::Duration};

/// a collection of data [Blob]s
///
/// # Serialization scheme
/// - the first two bytes gives the length of the next blob
/// - the following bytes are that blob
/// - repeat, starting with the length of the next blob
pub struct Packet {
    blobs: Vec<Blob>,
}

/// a blob is a piece of data
///
/// more than one blob can be put in one packet
///
/// serialization layout:
/// - first byte: blob type, commented on each variant
/// - remaining bytes: fragment type layout
pub enum Blob {
    /// `0`
    Fragment(Fragment),
    /// `1`
    Heartbeat(Heartbeat),
    /// `2`
    HeartbeatResponse(Heartbeat),
    /// `3`
    Acknowledgement(Acknowledgement),
}

/// used as heartbeat and it's response
///
/// contains the time stamp of when the server sent the heartbeat,
/// when returned it can be used to calculate round trip time
///
/// serialization layout:
/// - 1 bit: send ack
/// - 15 bits: fragmentation_id
/// - 4 bytes: total size of all fragments
/// - 4 bytes: start index of data
/// - remaining bytes: data
pub struct Fragment {
    pub send_ack: bool,
    pub fragmentation_id: u16,
    pub total_size: u32,
    pub start: u32,
    pub data: Box<[u8]>,
}

/// serialization layout:
/// - first 8 bytes: send time
pub struct Heartbeat {
    send_time: u64,
}

/// serialization layout:
/// - 2 bytes: fragmentation id
/// - 4 bytes: acknowledgement range start
/// - 2 bytes: acknowledgement range length
pub struct Acknowledgement {
    pub fragmentation_id: u16,
    pub start: u32,
    pub len: u16,
}


impl Packet {
    pub fn new() -> Self {
        Packet {
            blobs: Vec::new(),
        }
    }

    pub fn push(&mut self, blob: Blob) {
        self.blobs.push(blob);
    }

    pub fn size(&self) -> u16 {
        size_of::<u16>() as u16 * self.blobs.len() as u16 +
        self.blobs.iter().map(Blob::size).sum::<u16>()
    }

    /// returns the size of the largest blob that could be added with a given max size
    pub fn space_left(&self, max_size: u16) -> u16 {
        max_size.saturating_sub(
            // current size + 2 byte header for next blob
            self.size() + 2
        )
    }

    /// returns the number of blobs in the packet
    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }

    pub fn into_iter(self) -> impl Iterator<Item = Blob> {
        self.blobs.into_iter()
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        for blob in self.blobs.iter() {
            bytes.extend_from_slice(&blob.size().to_be_bytes());
            blob.serialize(&mut bytes);
        }

        bytes
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        let mut blobs = Vec::new();

        let mut bytes = bytes;
        loop {
            if bytes.len() == 0 {
                break;
            }

            let blob_size = u16::from_be_bytes(TryFrom::try_from(bytes.get(0..2)?).unwrap()) as usize;
            bytes = bytes.get(2..)?;

            blobs.push(Blob::deserialize(bytes.get(..blob_size)?)?);
            bytes = bytes.get(blob_size..)?;
        }

        Some(Packet {
            blobs,
        })
    }

    pub fn send(&self, addr: SocketAddr, socket: &UdpSocket) -> Result<usize, std::io::Error> {
        socket.send_to(&self.serialize(), addr)
    }
}

impl Blob {
    const HEADER_SIZE: usize = 1;

    /// returns the size of the blob in bytes if it was serialized
    pub fn size(&self) -> u16 {
        (
            Self::HEADER_SIZE as u16 +
            match self {
                Blob::Fragment(fragment) => fragment.size(),
                Blob::Heartbeat(heartbeat) => heartbeat.size(),
                Blob::HeartbeatResponse(heartbeat) => heartbeat.size(),
                Blob::Acknowledgement(acknowledgement) => acknowledgement.size(),
            }
        ) as u16
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        match self {
            Blob::Fragment(fragment) => {
                buffer.push(0);
                fragment.serialize(buffer);
            },
            Blob::Heartbeat(heartbeat) => {
                buffer.push(1);
                heartbeat.serialize(buffer);
            },
            Blob::HeartbeatResponse(heartbeat) => {
                buffer.push(2);
                heartbeat.serialize(buffer);
            },
            Blob::Acknowledgement(acknowledgement) => {
                buffer.push(3);
                acknowledgement.serialize(buffer);
            }
        }
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        let blob_type = bytes.get(0)?;
        let bytes = bytes.get(1..)?;

        Some(match blob_type {
            0 => Blob::Fragment(Fragment::deserialize(bytes)?),
            1 => Blob::Heartbeat(Heartbeat::deserialize(bytes)?),
            2 => Blob::HeartbeatResponse(Heartbeat::deserialize(bytes)?),
            3 => Blob::Acknowledgement(Acknowledgement::deserialize(bytes)?),
            _ => return None,
        })
    }
}

impl Fragment {
    pub const HEADER_SIZE: usize = 10;

    /// if the packet requires sending an acknowledgement, create one
    pub fn acknowledgement(&self) -> Option<Acknowledgement> {
        if self.send_ack {
            Some(Acknowledgement {
                fragmentation_id: self.fragmentation_id,
                start: self.start,
                len: self.data.len() as u16,
            })
        } else {
            None
        }
    }

    /// returns the size of the fragment in bytes if it was serialized
    pub fn size(&self) -> u16 {
        (
            Self::HEADER_SIZE + self.data.len()
        ) as u16
    }

    pub fn serialize(&self, buffer: &mut Vec<u8>) {
        // `fragmentation_id` should only use the 15 least significant bit
        // put the `send_ack` bit as the most significant bit
        // amazing we saved a whole byte
        let first_16_bits = ((self.send_ack as u16) << 15) | self.fragmentation_id;

        buffer.extend_from_slice(&first_16_bits.to_be_bytes());
        buffer.extend_from_slice(&self.total_size.to_be_bytes());
        buffer.extend_from_slice(&self.start.to_be_bytes());
        buffer.extend_from_slice(&self.data);
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {

        let first_16_bits = u16::from_be_bytes(TryFrom::try_from(bytes.get(0..2)?).unwrap());

        let send_ack = (first_16_bits & (1 << 15)) != 0;
        let fragmentation_id = first_16_bits & !(1 << 15);

        let total_size = u32::from_be_bytes(TryFrom::try_from(bytes.get(2..6)?).unwrap());
        let start = u32::from_be_bytes(TryFrom::try_from(bytes.get(6..10)?).unwrap());

        let data = bytes.get(10..)?.into();

        Some(Fragment {
            send_ack,
            fragmentation_id,
            total_size,
            start,
            data,
        })
    }
}

impl Heartbeat {
    pub fn new(time: Duration) -> Self {
        Heartbeat {
            send_time: time.as_millis() as u64,
        }
    }

    pub fn time(&self) -> Duration {
        Duration::from_millis(self.send_time)
    }

    pub fn size(&self) -> u16 {
        size_of::<u64>() as u16
    }

    fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.send_time.to_be_bytes());
    }

    fn deserialize(bytes: &[u8]) -> Option<Self> {
        Some(Heartbeat {
            send_time: u64::from_be_bytes(TryFrom::try_from(bytes).ok()?),
        })
    }
}

impl Acknowledgement {
    pub fn size(&self) -> u16 {
        (
            size_of::<u16>() +
            size_of::<u32>() +
            size_of::<u16>()
        ) as u16
    }

    fn serialize(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.fragmentation_id.to_be_bytes());
        buffer.extend_from_slice(&self.start.to_be_bytes());
        buffer.extend_from_slice(&self.len.to_be_bytes());
    }

    fn deserialize(bytes: &[u8]) -> Option<Self> {
        Some(Acknowledgement {
            fragmentation_id: u16::from_be_bytes(TryFrom::try_from(bytes.get(0..2)?).unwrap()),
            start: u32::from_be_bytes(TryFrom::try_from(bytes.get(2..6)?).unwrap()),
            len: u16::from_be_bytes(TryFrom::try_from(bytes.get(6..8)?).unwrap()),
        })
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_fragment_size() {
        let fragment = Fragment {
            send_ack: true,
            fragmentation_id: 10,
            total_size: 15,
            start: 8,
            data: [1, 2, 3, 4, 5].into(),
        };

        let mut buffer = Vec::new();
        fragment.serialize(&mut buffer);

        assert_eq!(fragment.size(), buffer.len() as u16);
    }

    #[test]
    fn split_fragment_size() {
        let fragment = Fragment {
            send_ack: false,
            fragmentation_id: 50,
            total_size: 10,
            start: 5,
            data: [1, 2, 3, 4, 5].into(),
        };

        let mut buffer = Vec::new();
        fragment.serialize(&mut buffer);

        assert_eq!(fragment.size(), buffer.len() as u16);
    }

    #[test]
    fn blob_size() {
        let blob = Blob::Fragment(Fragment {
            send_ack: true,
            fragmentation_id: 80,
            total_size: 10,
            start: 5,
            data: [1, 2, 3, 4, 5].into(),
        });

        let mut buffer = Vec::new();
        blob.serialize(&mut buffer);

        assert_eq!(blob.size(), buffer.len() as u16);
    }

    #[test]
    fn fragment_serialization() {
        let fragment = Fragment {
            send_ack: true,
            fragmentation_id: 80,
            total_size: 10,
            start: 5,
            data: [1, 2, 3, 4, 5].into(),
        };

        let mut buffer = Vec::new();
        fragment.serialize(&mut buffer);

        let deserialized = Fragment::deserialize(&buffer).unwrap();

        assert_eq!(fragment.send_ack, deserialized.send_ack);
        assert_eq!(fragment.fragmentation_id, deserialized.fragmentation_id);
        assert_eq!(fragment.total_size, deserialized.total_size);
        assert_eq!(fragment.start, deserialized.start);
        assert_eq!(fragment.data, deserialized.data);
    }

    #[test]
    fn blob_serialization() {
        let blob = Blob::Fragment(Fragment {
            send_ack: true,
            fragmentation_id: 80,
            total_size: 10,
            start: 5,
            data: [1, 2, 3, 4, 5].into(),
        });

        let mut buffer = Vec::new();
        blob.serialize(&mut buffer);

        let deserialized = Blob::deserialize(&buffer).unwrap();

        match deserialized {
            Blob::Fragment(_) => (),
            _ => panic!(),
        }
    }

    #[test]
    fn packet_size() {
        let packet = Packet {
            blobs: vec![
                Blob::Fragment(Fragment {
                    send_ack: true,
                    fragmentation_id: 80,
                    total_size: 10,
                    start: 5,
                    data: [1, 2, 3, 4, 5].into(),
                }),
                Blob::Fragment(Fragment {
                    send_ack: true,
                    fragmentation_id: 80,
                    total_size: 10,
                    start: 5,
                    data: [1, 2, 3, 4, 5].into(),
                }),
            ]
        };

        let bytes = packet.serialize();

        assert_eq!(packet.size(), bytes.len() as u16);
    }

    #[test]
    fn packet_serialization() {
        let packet = Packet {
            blobs: vec![
                Blob::Fragment(Fragment {
                    send_ack: true,
                    fragmentation_id: 80,
                    total_size: 10,
                    start: 5,
                    data: [1, 2, 3, 4, 5].into(),
                }),
                Blob::Fragment(Fragment {
                    send_ack: true,
                    fragmentation_id: 80,
                    total_size: 10,
                    start: 5,
                    data: [1, 2, 3, 4, 5].into(),
                }),
            ]
        };

        let bytes = packet.serialize();
        let deserialized = Packet::deserialize(&bytes).unwrap();

        assert_eq!(packet.blobs.len(), deserialized.blobs.len());
    }
}

use std::{ops::Range, time::Duration};

use crate::packet::{Blob, Fragment};



/// a message that a connection is trying to deliver
pub struct SendMessage {
    data: Box<[u8]>,
    /// if an ack is required
    ///
    /// if `Some` contains the last time data was sent/resent,
    /// the inner option being `None` if data was never sent
    reliable: Option<Option<Duration>>,
    fragmentation_id: u16,
    /// how much of the message has been delivered
    delivered: DeliveredIntervals,
}

pub struct ReceiveMessage {
    data: Box<[u8]>,
    reliable: bool,
    fragmentation_id: u16,
    delivered: DeliveredIntervals,
    last_received_time: Duration,
}

/// what portion of a message is delivered
#[derive(Clone)]
pub struct DeliveredIntervals {
    size: usize,
    intervals: Vec<Range<usize>>,
}

pub struct DeliveredIntervalsGaps<'a> {
    next: usize,
    intervals: &'a DeliveredIntervals,
}


impl SendMessage {
    pub fn new(reliable: bool, fragmentation_id: u16, data: Box<[u8]>) -> Self {
        SendMessage {
            delivered: DeliveredIntervals::new(data.len()),
            data,
            reliable: if reliable { Some(None) } else { None },
            fragmentation_id,
        }
    }

    /// gets the fragmentation id of the message
    pub fn fragmentation_id(&self) -> u16 {
        self.fragmentation_id
    }

    /// returns `None` if the message is unreliable
    ///
    /// returns `Some` with the last time data was sent/resent, if at all
    pub fn reliable(&mut self) -> Option<&mut Option<Duration>> {
        self.reliable.as_mut()
    }

    /// gets this messages [DeliveredIntervals]
    pub fn get_deliverd_intervals(&self) -> DeliveredIntervals {
        self.delivered.clone()
    }

    /// sets the [DeliveredIntervals]
    ///
    /// to hold invariants the given intervals must have come from this message to start with
    pub fn set_delivered_intervals(&mut self, delivered: DeliveredIntervals) {
        self.delivered = delivered;
    }

    /// sets that a range of data has been delivered, typically from an acknowledgement
    ///
    /// fails if range was outside the message
    pub fn set_delivered(&mut self, range: Range<usize>) -> Result<(), ()> {
        if range.end > self.data.len() {
            return Err(());
        }

        self.delivered.set_delivered(range);

        Ok(())
    }

    /// tries to create a blob to deliver, using the [DeliveredIntervals] supplied.
    /// if you are delivering messages unrealiably you can immediately reapply the given [DeliveredIntervals],
    /// otherwise don't and just use it within one resend wave
    ///
    /// the outer option will return `None` if no blob is required
    ///
    /// the inner option wil return `None` if the given space is not enough
    pub fn create_blob(&mut self, delivered: &mut DeliveredIntervals, available_space: u16) -> Option<Option<Blob>> {
        let mut gap = delivered.gaps().next()?;

        let Some(available_space) = available_space.checked_sub(Fragment::HEADER_SIZE as u16) else {
            return Some(None);
        };

        if available_space == 0 {
            return Some(None);
        };

        gap.end = gap.end.min(gap.start + available_space as usize);

        delivered.set_delivered(gap.clone());

        Some(Some(Blob::Fragment(Fragment {
            send_ack: self.reliable.is_some(),
            fragmentation_id: self.fragmentation_id,
            total_size: self.data.len() as u32,
            start: gap.start as u32,
            data: self.data.get(gap).unwrap().into(),
        })))
    }

    pub fn delivered(&self) -> bool {
        self.delivered.finished()
    }
}

impl ReceiveMessage {
    pub fn new(time: Duration, fragment: Fragment) -> Result<Self, ()> {
        let mut message = ReceiveMessage {
            data: vec![0; fragment.total_size as usize].into_boxed_slice(),
            reliable: fragment.send_ack,
            fragmentation_id: fragment.fragmentation_id,
            delivered: DeliveredIntervals::new(fragment.total_size as usize),
            last_received_time: Duration::ZERO,
        };

        message.add_fragment(time, fragment)?;

        Ok(message)
    }

    pub fn add_fragment(&mut self, time: Duration, fragment: Fragment) -> Result<(), ()> {
        let target_range = (fragment.start as usize)..(fragment.start as usize + fragment.data.len());

        let Some(target_bytes) = self.data.get_mut(target_range.clone()) else {
            return Err(());
        };

        target_bytes.copy_from_slice(&fragment.data);
        self.delivered.set_delivered(target_range);
        self.last_received_time = time;

        Ok(())
    }

    pub fn fragmentation_id(&self) -> u16 {
        self.fragmentation_id
    }

    pub fn complete(&self) -> bool {
        self.delivered.finished()
    }

    pub fn data(self) -> Box<[u8]> {
        self.data
    }

    pub fn is_reliable(&self) -> bool {
        self.reliable
    }

    pub fn last_received_time(&self) -> Duration {
        self.last_received_time
    }
}

impl DeliveredIntervals {
    fn new(size: usize) -> Self {
        DeliveredIntervals {
            size,
            intervals: Vec::new(),
        }
    }

    fn set_delivered(&mut self, range: Range<usize>) {
        if range.start == range.end {
            return;
        }

        // insert the range whilst preserving order of start values
        let (Ok(index) | Err(index)) = self.intervals.binary_search_by(
            |element| element.start.cmp(&range.start)
        );

        self.intervals.insert(index, range);

        // merge intersecting ranges
        let mut pointer = 0usize;
        loop {
            let Some(higher,) = self.intervals.get(pointer + 1).cloned() else {
                break;
            };

            let Some(lower,) = self.intervals.get_mut(pointer) else {
                break;
            };

            if higher.start <= lower.end {
                lower.end = lower.end.max(higher.end);
                self.intervals.remove(pointer + 1);
            } else {
                pointer += 1;
            }
        }
    }

    fn finished(&self) -> bool {
        let Some(range) = self.intervals.first() else {
            return false;
        };

        return range.start == 0 && range.end == self.size;
    }

    fn gaps(&self) -> DeliveredIntervalsGaps {
        DeliveredIntervalsGaps {
            next: 0,
            intervals: self,
        }
    }
}

impl<'a> Iterator for DeliveredIntervalsGaps<'a> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Self::Item> {
        let lower = if let Some(lower_index) = self.next.checked_sub(1) {
            let Some(lower) = self.intervals.intervals.get(lower_index) else {
                return None;
            };

            lower.end
        } else {
            // points to index -1
            0
        };

        let upper = if let Some(upper) = self.intervals.intervals.get(self.next) {
            upper.start
        } else {
            // points past all the intervals
            self.intervals.size
        };

        self.next += 1;

        if lower == upper {
            // skip zero sized gaps
            self.next()
        } else {
            Some(lower..upper)
        }

    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivered_intervals_finished() {
        let mut delivered = DeliveredIntervals::new(10);
        assert!(!delivered.finished());

        delivered.set_delivered(1..3);
        assert!(!delivered.finished());

        delivered.set_delivered(3..10);
        assert!(!delivered.finished());

        delivered.set_delivered(0..1);
        assert!(delivered.finished());
    }

    #[test]
    fn deliverd_intervals_gaps() {
        let mut delivered = DeliveredIntervals::new(10);

        delivered.set_delivered(1..2);
        delivered.set_delivered(5..6);
        delivered.set_delivered(6..8);

        let mut gaps = delivered.gaps();

        assert_eq!(gaps.next(), Some(0..1));
        assert_eq!(gaps.next(), Some(2..5));
        assert_eq!(gaps.next(), Some(8..10));
        assert_eq!(gaps.next(), None);
    }
}

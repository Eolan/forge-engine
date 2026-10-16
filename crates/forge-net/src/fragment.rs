//! Unreliable messages longer than a datagram, cut into fragments and joined again.
//!
//! A QUIC datagram is never fragmented (RFC 9221): it must fit one packet, about 1200 bytes
//! before path MTU discovery. Every unreliable datagram starts with one byte: the channel in
//! the low six bits, and the high bit set when the datagram is a fragment. A fragment then
//! carries the message's number on its channel (two bytes), its own index and the count (a
//! byte each), and its share of the message. The receiver keeps the fragments of a message
//! until it has all of them, and drops a message whose fragments have not all come within
//! [`Reassembler::new`]'s timeout: one lost fragment loses the message, as for the datagram
//! of a message that fits.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bytes::{BufMut, Bytes, BytesMut};

use crate::channel::ChannelId;

/// The first byte's flag for a fragment.
const FRAGMENT: u8 = 0x80;
/// The channel's bits in the first byte.
const CHANNEL_MASK: u8 = 0x3F;
/// A whole message's header: the first byte.
pub const WHOLE_HEADER: usize = 1;
/// A fragment's header: the first byte, the message's number, the index and the count.
pub const FRAGMENT_HEADER: usize = 5;
/// Fragments a message at most.
pub const MAX_FRAGMENTS: usize = 255;

/// A message too long for [`MAX_FRAGMENTS`] fragments of the datagram size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{len} bytes do not fit {MAX_FRAGMENTS} fragments of {datagram} bytes")]
pub struct TooLarge {
    /// The message's length.
    pub len: usize,
    /// The datagram size it was cut for.
    pub datagram: usize,
}

/// The datagrams that carry `payload` on `channel`, each at most `datagram` bytes: one if it
/// fits, else fragments numbered `message`.
pub fn split(
    channel: ChannelId,
    message: u16,
    payload: &[u8],
    datagram: usize,
) -> Result<Vec<Bytes>, TooLarge> {
    let first = channel.0 & CHANNEL_MASK;
    if WHOLE_HEADER + payload.len() <= datagram {
        let mut b = BytesMut::with_capacity(WHOLE_HEADER + payload.len());
        b.put_u8(first);
        b.put_slice(payload);
        return Ok(vec![b.freeze()]);
    }
    let share = datagram.saturating_sub(FRAGMENT_HEADER);
    let too_large = TooLarge {
        len: payload.len(),
        datagram,
    };
    if share == 0 {
        return Err(too_large);
    }
    let count = payload.len().div_ceil(share);
    if count > MAX_FRAGMENTS {
        return Err(too_large);
    }
    Ok(payload
        .chunks(share)
        .enumerate()
        .map(|(index, chunk)| {
            let mut b = BytesMut::with_capacity(FRAGMENT_HEADER + chunk.len());
            b.put_u8(first | FRAGMENT);
            b.put_u16_le(message);
            b.put_u8(index as u8);
            b.put_u8(count as u8);
            b.put_slice(chunk);
            b.freeze()
        })
        .collect())
}

/// A datagram read: a whole message, or a fragment of one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// A message that fitted one datagram.
    Whole(ChannelId, Bytes),
    /// A fragment.
    Fragment {
        /// The channel.
        channel: ChannelId,
        /// The message's number on its channel.
        message: u16,
        /// The fragment's index.
        index: u8,
        /// The message's fragments.
        count: u8,
        /// Its share of the message.
        data: Bytes,
    },
}

/// Reads a datagram's header; `None` if it is too short to have one.
pub fn parse(datagram: Bytes) -> Option<Piece> {
    let first = *datagram.first()?;
    let channel = ChannelId(first & CHANNEL_MASK);
    if first & FRAGMENT == 0 {
        return Some(Piece::Whole(channel, datagram.slice(WHOLE_HEADER..)));
    }
    if datagram.len() < FRAGMENT_HEADER {
        return None;
    }
    let (index, count) = (datagram[3], datagram[4]);
    if count == 0 || index >= count {
        return None;
    }
    Some(Piece::Fragment {
        channel,
        message: u16::from_le_bytes([datagram[1], datagram[2]]),
        index,
        count,
        data: datagram.slice(FRAGMENT_HEADER..),
    })
}

struct Partial {
    started: Instant,
    parts: Vec<Option<Bytes>>,
    missing: usize,
}

/// Joins fragments into messages (see the module's documentation).
pub struct Reassembler {
    partial: HashMap<(ChannelId, u16), Partial>,
    timeout: Duration,
    /// Messages at most in the making; past it the oldest is dropped.
    capacity: usize,
    /// Messages dropped unfinished.
    pub dropped: u64,
}

impl Reassembler {
    /// A reassembler that drops a message whose fragments have not all come within `timeout`,
    /// and keeps at most `capacity` messages in the making.
    pub fn new(timeout: Duration, capacity: usize) -> Self {
        Self {
            partial: HashMap::new(),
            timeout,
            capacity: capacity.max(1),
            dropped: 0,
        }
    }

    /// Messages in the making.
    pub fn pending(&self) -> usize {
        self.partial.len()
    }

    /// Takes a fragment that came at `now`: the message, once its last fragment is in.
    pub fn insert(
        &mut self,
        now: Instant,
        channel: ChannelId,
        message: u16,
        index: u8,
        count: u8,
        data: Bytes,
    ) -> Option<Bytes> {
        let timeout = self.timeout;
        let before = self.partial.len();
        self.partial
            .retain(|_, p| now.saturating_duration_since(p.started) < timeout);
        self.dropped += (before - self.partial.len()) as u64;
        let key = (channel, message);
        if !self.partial.contains_key(&key) && self.partial.len() >= self.capacity {
            let oldest = self
                .partial
                .iter()
                .min_by_key(|(_, p)| p.started)
                .map(|(k, _)| *k)
                .expect("full, so not empty");
            self.partial.remove(&oldest);
            self.dropped += 1;
        }
        let entry = self.partial.entry(key).or_insert_with(|| Partial {
            started: now,
            parts: vec![None; count as usize],
            missing: count as usize,
        });
        if entry.parts.len() != count as usize {
            // A different message reusing the number (the counter wrapped): start again.
            *entry = Partial {
                started: now,
                parts: vec![None; count as usize],
                missing: count as usize,
            };
        }
        let slot = &mut entry.parts[index as usize];
        if slot.is_none() {
            *slot = Some(data);
            entry.missing -= 1;
        }
        if entry.missing > 0 {
            return None;
        }
        let parts = self.partial.remove(&key).expect("present").parts;
        let len = parts.iter().flatten().map(Bytes::len).sum();
        let mut whole = BytesMut::with_capacity(len);
        for part in parts.into_iter().flatten() {
            whole.put_slice(&part);
        }
        Some(whole.freeze())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(r: &mut Reassembler, now: Instant, datagram: Bytes) -> Option<Bytes> {
        match parse(datagram)? {
            Piece::Whole(_, b) => Some(b),
            Piece::Fragment {
                channel,
                message,
                index,
                count,
                data,
            } => r.insert(now, channel, message, index, count, data),
        }
    }

    #[test]
    fn a_short_message_is_one_datagram() {
        let d = split(ChannelId(5), 0, b"hello", 1200).unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(
            parse(d[0].clone()),
            Some(Piece::Whole(ChannelId(5), "hello".into()))
        );
    }

    #[test]
    fn fragments_join_in_any_order_and_duplicates_change_nothing() {
        let payload: Vec<u8> = (0..5000u32).map(|k| (k * 7) as u8).collect();
        let mut d = split(ChannelId(2), 9, &payload, 1200).unwrap();
        assert_eq!(d.len(), 5000usize.div_ceil(1200 - FRAGMENT_HEADER));
        assert!(d.iter().all(|x| x.len() <= 1200));
        d.reverse();
        d.insert(1, d[0].clone());
        let mut r = Reassembler::new(Duration::from_secs(1), 8);
        let now = Instant::now();
        let out: Vec<Bytes> = d.into_iter().filter_map(|x| feed(&mut r, now, x)).collect();
        assert_eq!(out, vec![Bytes::from(payload)]);
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn a_message_missing_a_fragment_is_dropped_after_the_timeout() {
        let payload = vec![1u8; 3000];
        let d = split(ChannelId(0), 1, &payload, 1200).unwrap();
        let mut r = Reassembler::new(Duration::from_millis(500), 8);
        let t0 = Instant::now();
        assert!(feed(&mut r, t0, d[0].clone()).is_none());
        assert_eq!(r.pending(), 1);
        let later = t0 + Duration::from_secs(1);
        let other = split(ChannelId(0), 2, &payload, 1200).unwrap();
        assert!(feed(&mut r, later, other[0].clone()).is_none());
        assert_eq!((r.pending(), r.dropped), (1, 1));
    }

    #[test]
    fn too_long_a_message_is_refused() {
        let payload = vec![0u8; 256 * 1200];
        assert!(split(ChannelId(0), 0, &payload, 1200).is_err());
    }
}

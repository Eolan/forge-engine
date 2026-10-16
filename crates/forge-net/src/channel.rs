//! Channels: numbered lanes of messages, each with its delivery.
//!
//! Both ends declare the same list of channels ([`Channels`]); the client's hello carries it
//! and the server refuses a client whose list differs. A channel's number is its place in the
//! list. On QUIC (RFC 9000, RFC 9221):
//! - [`Delivery::ReliableOrdered`]: one unidirectional stream per channel and direction, opened
//!   at the first message; each message is framed by its length. A lost packet holds back the
//!   messages after it on that channel only.
//! - [`Delivery::ReliableUnordered`]: one stream per message, so a lost packet holds back only
//!   its own message.
//! - [`Delivery::Unreliable`]: QUIC datagrams, never resent; a message longer than a datagram
//!   is cut into fragments ([`crate::fragment`]) and lost whole if any fragment is.

/// How a channel's messages travel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Delivery {
    /// Every message arrives, in the order sent.
    ReliableOrdered = 0,
    /// Every message arrives, in any order.
    ReliableUnordered = 1,
    /// A message arrives once or not at all, in any order.
    Unreliable = 2,
}

impl Delivery {
    /// The delivery of a byte, if it names one.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::ReliableOrdered),
            1 => Some(Self::ReliableUnordered),
            2 => Some(Self::Unreliable),
            _ => None,
        }
    }

    /// Whether every message arrives.
    pub fn is_reliable(self) -> bool {
        self != Self::Unreliable
    }
}

/// A channel's number: its place in [`Channels`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId(pub u8);

/// Channels at most: the number shares a datagram's first byte with the fragment flag.
pub const MAX_CHANNELS: usize = 64;

/// The channels both ends declare.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channels {
    kinds: Vec<Delivery>,
}

impl Channels {
    /// Channels numbered in the order given.
    ///
    /// # Panics
    /// With no channel, or more than [`MAX_CHANNELS`].
    pub fn new(kinds: &[Delivery]) -> Self {
        assert!(
            !kinds.is_empty() && kinds.len() <= MAX_CHANNELS,
            "between 1 and {MAX_CHANNELS} channels"
        );
        Self {
            kinds: kinds.to_vec(),
        }
    }

    /// One channel of each delivery: 0 reliable ordered, 1 reliable unordered, 2 unreliable.
    pub fn standard() -> Self {
        Self::new(&[
            Delivery::ReliableOrdered,
            Delivery::ReliableUnordered,
            Delivery::Unreliable,
        ])
    }

    /// The channel's delivery, if it exists.
    pub fn delivery(&self, channel: ChannelId) -> Option<Delivery> {
        self.kinds.get(channel.0 as usize).copied()
    }

    /// The deliveries in channel order.
    pub fn kinds(&self) -> &[Delivery] {
        &self.kinds
    }

    /// How many channels.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// Never: there is at least one channel.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }
}

impl Default for Channels {
    fn default() -> Self {
        Self::standard()
    }
}

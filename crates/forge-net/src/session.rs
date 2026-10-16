//! A session: one connection after its handshake, with its channels.
//!
//! [`Session::send`] does not wait: a reliable ordered message joins its channel's queue, which
//! a task writes to the channel's stream; a reliable unordered message gets a task and a stream
//! of its own; an unreliable one leaves as datagrams at once. Tasks read the peer's streams and
//! datagrams into an inbox that [`Session::recv`] empties. The tasks run on the tokio runtime
//! the session was made on, so `send` may be called from any thread.
//!
//! The inbox holds [`INBOX`] messages. When it is full, the stream readers wait (QUIC's flow
//! control then slows the peer down) and new unreliable messages are dropped and counted.

use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use quinn::{Connection, ConnectionError, RecvStream, SendDatagramError};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::channel::{ChannelId, Channels, Delivery};
use crate::fragment::{self, Piece, Reassembler};
use crate::protocol::{CLOSE_NORMAL, Refusal};
use crate::socket::Conditioner;
use crate::wire::{varint, varint_len};

/// Messages the inbox holds before the peer is slowed down or unreliable messages are dropped.
pub const INBOX: usize = 8192;

/// A message received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The channel it came on.
    pub channel: ChannelId,
    /// Its bytes.
    pub payload: Bytes,
}

/// Why a message could not be sent.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    /// No such channel.
    #[error("no channel {0:?}")]
    UnknownChannel(ChannelId),
    /// The message is longer than the session allows.
    #[error("{len} bytes are more than the {max} a message may have")]
    TooLarge {
        /// The message's length.
        len: usize,
        /// The most a message may have.
        max: usize,
    },
    /// The connection is closed.
    #[error("the connection is closed: {0}")]
    Closed(ConnectionError),
    /// The peer takes no datagrams.
    #[error("the peer takes no datagrams")]
    NoDatagrams,
}

/// What a session's settings are, after the handshake.
#[derive(Clone, Debug)]
pub(crate) struct SessionSettings {
    pub channels: Channels,
    /// The most bytes a message may have.
    pub max_message: usize,
    /// The largest datagram sent, at most what the path allows.
    pub datagram_limit: usize,
    /// How long the fragments of an unreliable message are kept waiting for the rest.
    pub fragment_timeout: Duration,
}

#[derive(Debug, Default)]
struct Counters {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    payload_sent: AtomicU64,
    payload_received: AtomicU64,
    /// Unreliable messages dropped on arrival: the inbox was full, or a fragment never came.
    dropped: AtomicU64,
}

/// What a session did so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SessionStats {
    /// The smoothed round-trip time.
    pub rtt: Duration,
    /// UDP datagrams sent and their bytes (QUIC's headers, acks and encryption included).
    pub udp_sent: (u64, u64),
    /// UDP datagrams received and their bytes.
    pub udp_received: (u64, u64),
    /// Packets QUIC counted lost.
    pub lost_packets: u64,
    /// Messages sent and their payload bytes.
    pub messages_sent: (u64, u64),
    /// Messages received and their payload bytes.
    pub messages_received: (u64, u64),
    /// Unreliable messages dropped on arrival.
    pub dropped: u64,
}

struct Inner {
    connection: Connection,
    settings: SessionSettings,
    runtime: Handle,
    /// Each reliable ordered channel's queue, made at its first message.
    ordered: Mutex<Vec<Option<mpsc::UnboundedSender<Bytes>>>>,
    /// The next unreliable message's number, for its fragments.
    next_message: AtomicU16,
    counters: Counters,
}

/// One connection after its handshake (see the module's documentation).
pub struct Session {
    inner: Arc<Inner>,
    inbox: mpsc::Receiver<Message>,
    client_id: u64,
    server_id: u64,
    conditioner: Option<Conditioner>,
    /// A client's endpoint, kept as long as its session.
    _endpoint: Option<quinn::Endpoint>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("client_id", &self.client_id)
            .field("server_id", &self.server_id)
            .field("remote", &self.inner.connection.remote_address())
            .finish()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.inner
            .connection
            .close(CLOSE_NORMAL.into(), b"session dropped");
    }
}

impl Session {
    /// A session on a connection whose handshake is done; starts its readers.
    pub(crate) fn start(
        connection: Connection,
        settings: SessionSettings,
        client_id: u64,
        server_id: u64,
        conditioner: Option<Conditioner>,
        endpoint: Option<quinn::Endpoint>,
    ) -> Self {
        let ordered = vec![None; settings.channels.len()];
        let inner = Arc::new(Inner {
            connection,
            settings,
            runtime: Handle::current(),
            ordered: Mutex::new(ordered),
            next_message: AtomicU16::new(0),
            counters: Counters::default(),
        });
        let (tx, inbox) = mpsc::channel(INBOX);
        tokio::spawn(accept_streams(inner.clone(), tx.clone()));
        tokio::spawn(read_datagrams(inner.clone(), tx));
        Self {
            inner,
            inbox,
            client_id,
            server_id,
            conditioner,
            _endpoint: endpoint,
        }
    }

    /// The client's id, from its token.
    pub fn client_id(&self) -> u64 {
        self.client_id
    }

    /// The server's id.
    pub fn server_id(&self) -> u64 {
        self.server_id
    }

    /// The peer's address.
    pub fn remote_address(&self) -> std::net::SocketAddr {
        self.inner.connection.remote_address()
    }

    /// The conditioner on this end's sending side, if the endpoint has one (a client's own; a
    /// server's sessions share the server's).
    pub fn conditioner(&self) -> Option<&Conditioner> {
        self.conditioner.as_ref()
    }

    /// The largest unreliable message that fits one datagram now.
    pub fn datagram_payload(&self) -> usize {
        self.datagram_size() - fragment::WHOLE_HEADER
    }

    fn datagram_size(&self) -> usize {
        self.inner
            .connection
            .max_datagram_size()
            .unwrap_or(0)
            .min(self.inner.settings.datagram_limit)
    }

    /// Sends `payload` on `channel` without waiting (see the module's documentation).
    pub fn send(&self, channel: ChannelId, payload: impl Into<Bytes>) -> Result<(), SendError> {
        let payload = payload.into();
        let inner = &self.inner;
        let delivery = inner
            .settings
            .channels
            .delivery(channel)
            .ok_or(SendError::UnknownChannel(channel))?;
        let max = inner.settings.max_message;
        if payload.len() > max {
            return Err(SendError::TooLarge {
                len: payload.len(),
                max,
            });
        }
        if let Some(e) = inner.connection.close_reason() {
            return Err(SendError::Closed(e));
        }
        let len = payload.len() as u64;
        match delivery {
            Delivery::ReliableOrdered => {
                let mut queues = inner.ordered.lock().unwrap_or_else(|e| e.into_inner());
                let queue = queues[channel.0 as usize].get_or_insert_with(|| {
                    let (tx, rx) = mpsc::unbounded_channel();
                    inner
                        .runtime
                        .spawn(write_ordered(inner.connection.clone(), channel, rx));
                    tx
                });
                // A closed queue means the writer saw the connection end.
                if queue.send(payload).is_err() {
                    let e = inner
                        .connection
                        .close_reason()
                        .unwrap_or(ConnectionError::LocallyClosed);
                    return Err(SendError::Closed(e));
                }
            }
            Delivery::ReliableUnordered => {
                inner
                    .runtime
                    .spawn(write_one(inner.connection.clone(), channel, payload));
            }
            Delivery::Unreliable => {
                let number = inner.next_message.fetch_add(1, Ordering::Relaxed);
                let size = self.datagram_size();
                let datagrams = fragment::split(channel, number, &payload, size).map_err(|e| {
                    SendError::TooLarge {
                        len: e.len,
                        max: size.saturating_sub(fragment::FRAGMENT_HEADER)
                            * fragment::MAX_FRAGMENTS,
                    }
                })?;
                for d in datagrams {
                    match inner.connection.send_datagram(d) {
                        Ok(()) => {}
                        Err(SendDatagramError::ConnectionLost(e)) => {
                            return Err(SendError::Closed(e));
                        }
                        Err(SendDatagramError::TooLarge) => {
                            // The path shrank under the size just read; the message is lost,
                            // as it would be on the way.
                            break;
                        }
                        Err(_) => return Err(SendError::NoDatagrams),
                    }
                }
            }
        }
        inner.counters.messages_sent.fetch_add(1, Ordering::Relaxed);
        inner
            .counters
            .payload_sent
            .fetch_add(len, Ordering::Relaxed);
        Ok(())
    }

    /// The next message; `None` once the connection is closed and the inbox empty.
    pub async fn recv(&mut self) -> Option<Message> {
        let m = self.inbox.recv().await?;
        self.count_received(&m);
        Some(m)
    }

    /// The next message if one is waiting.
    pub fn try_recv(&mut self) -> Option<Message> {
        let m = self.inbox.try_recv().ok()?;
        self.count_received(&m);
        Some(m)
    }

    fn count_received(&self, m: &Message) {
        let c = &self.inner.counters;
        c.messages_received.fetch_add(1, Ordering::Relaxed);
        c.payload_received
            .fetch_add(m.payload.len() as u64, Ordering::Relaxed);
    }

    /// What the session did so far.
    pub fn stats(&self) -> SessionStats {
        let q = self.inner.connection.stats();
        let c = &self.inner.counters;
        SessionStats {
            rtt: q.path.rtt,
            udp_sent: (q.udp_tx.datagrams, q.udp_tx.bytes),
            udp_received: (q.udp_rx.datagrams, q.udp_rx.bytes),
            lost_packets: q.path.lost_packets,
            messages_sent: (
                c.messages_sent.load(Ordering::Relaxed),
                c.payload_sent.load(Ordering::Relaxed),
            ),
            messages_received: (
                c.messages_received.load(Ordering::Relaxed),
                c.payload_received.load(Ordering::Relaxed),
            ),
            dropped: c.dropped.load(Ordering::Relaxed),
        }
    }

    /// Why the connection closed, if it has.
    pub fn close_reason(&self) -> Option<ConnectionError> {
        self.inner.connection.close_reason()
    }

    /// Waits until the connection closes, and says why.
    pub async fn closed(&self) -> ConnectionError {
        self.inner.connection.closed().await
    }

    /// Closes the connection at once; the peer sees code [`CLOSE_NORMAL`].
    pub fn close(&self) {
        self.inner.connection.close(CLOSE_NORMAL.into(), b"closed");
    }

    /// Closes the connection with a refusal's code (the server's side of a refused hello).
    pub(crate) fn refuse(connection: &Connection, refusal: Refusal) {
        connection.close(refusal.code().into(), &refusal.reason());
    }
}

/// Writes a reliable ordered channel's queue to one stream, each message after its length.
async fn write_ordered(
    connection: Connection,
    channel: ChannelId,
    mut queue: mpsc::UnboundedReceiver<Bytes>,
) {
    let Ok(mut stream) = connection.open_uni().await else {
        return;
    };
    if stream.write_all(&[channel.0]).await.is_err() {
        return;
    }
    let mut frame = Vec::with_capacity(16 * 1024);
    while let Some(first) = queue.recv().await {
        // Whatever else is queued goes in the same write.
        frame.clear();
        let mut next = Some(first);
        while let Some(m) = next {
            varint(&mut frame, m.len() as u64);
            frame.extend_from_slice(&m);
            next = if frame.len() < 64 * 1024 {
                queue.try_recv().ok()
            } else {
                None
            };
        }
        if stream.write_all(&frame).await.is_err() {
            return;
        }
    }
    let _ = stream.finish();
}

/// Writes one reliable unordered message on a stream of its own.
async fn write_one(connection: Connection, channel: ChannelId, payload: Bytes) {
    let Ok(mut stream) = connection.open_uni().await else {
        return;
    };
    if stream.write_all(&[channel.0]).await.is_ok() && stream.write_chunk(payload).await.is_ok() {
        let _ = stream.finish();
    }
}

/// Takes the peer's streams as they open.
async fn accept_streams(inner: Arc<Inner>, inbox: mpsc::Sender<Message>) {
    while let Ok(stream) = inner.connection.accept_uni().await {
        tokio::spawn(read_stream(inner.clone(), stream, inbox.clone()));
    }
}

/// Reads one of the peer's streams into the inbox; closes the connection on a protocol error.
async fn read_stream(inner: Arc<Inner>, mut stream: RecvStream, inbox: mpsc::Sender<Message>) {
    let violation = |inner: &Inner| {
        inner
            .connection
            .close(Refusal::Malformed.code().into(), b"bad stream")
    };
    let mut first = [0u8; 1];
    if stream.read_exact(&mut first).await.is_err() {
        return;
    }
    let channel = ChannelId(first[0]);
    let max = inner.settings.max_message;
    match inner.settings.channels.delivery(channel) {
        Some(Delivery::ReliableOrdered) => loop {
            let mut head = [0u8; 8];
            if stream.read_exact(&mut head[..1]).await.is_err() {
                return; // finished or reset
            }
            let n = varint_len(head[0]);
            if stream.read_exact(&mut head[1..n]).await.is_err() {
                return violation(&inner);
            }
            let mut len = u64::from(head[0] & 0x3F);
            for b in &head[1..n] {
                len = len << 8 | u64::from(*b);
            }
            if len as usize > max {
                return violation(&inner);
            }
            let mut payload = vec![0u8; len as usize];
            if stream.read_exact(&mut payload).await.is_err() {
                return violation(&inner);
            }
            let message = Message {
                channel,
                payload: payload.into(),
            };
            if inbox.send(message).await.is_err() {
                return;
            }
        },
        Some(Delivery::ReliableUnordered) => {
            let Ok(payload) = stream.read_to_end(max).await else {
                return;
            };
            let _ = inbox
                .send(Message {
                    channel,
                    payload: payload.into(),
                })
                .await;
        }
        _ => violation(&inner),
    }
}

/// Reads the peer's datagrams, joins fragments, and drops what the inbox cannot hold.
async fn read_datagrams(inner: Arc<Inner>, inbox: mpsc::Sender<Message>) {
    let mut reassembler = Reassembler::new(inner.settings.fragment_timeout, 64);
    let mut dropped_before = 0;
    while let Ok(datagram) = inner.connection.read_datagram().await {
        let whole = match fragment::parse(datagram) {
            Some(Piece::Whole(channel, payload)) => Some((channel, payload)),
            Some(Piece::Fragment {
                channel,
                message,
                index,
                count,
                data,
            }) => reassembler
                .insert(Instant::now(), channel, message, index, count, data)
                .map(|p| (channel, p)),
            None => None,
        };
        let counters = &inner.counters;
        if reassembler.dropped != dropped_before {
            counters
                .dropped
                .fetch_add(reassembler.dropped - dropped_before, Ordering::Relaxed);
            dropped_before = reassembler.dropped;
        }
        let Some((channel, payload)) = whole else {
            continue;
        };
        if inner.settings.channels.delivery(channel) != Some(Delivery::Unreliable) {
            inner
                .connection
                .close(Refusal::Malformed.code().into(), b"bad datagram");
            return;
        }
        match inbox.try_send(Message { channel, payload }) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                counters.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
        }
    }
}

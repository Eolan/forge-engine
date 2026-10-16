//! A UDP socket under quinn whose sending side goes through a [`LinkConditioner`].
//!
//! quinn sends through an [`AsyncUdpSocket`]; [`ConditionedSocket`] wraps the runtime's own and
//! asks the conditioner, packet by packet, when each one arrives. A packet due at once goes
//! straight out; the others wait in a queue that a task on the runtime empties when they are
//! due. Each endpoint conditions what it sends, so a link conditioned both ways has a
//! conditioner at each end. Receiving is the inner socket's.
//!
//! The conditioner's clock is the runtime's (`tokio::time`), so its delays are the timer's: on
//! Windows a sleep ends on the system timer's tick, by default every 15.6 ms, which rounds a
//! short latency up. The fates themselves (lost, held back, duplicated) depend only on the seed
//! and the order of the packets.

use std::collections::BinaryHeap;
use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll};
use std::time::Duration;

use quinn::udp::{EcnCodepoint, RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use tokio::sync::Notify;
use tokio::time::Instant;

use crate::conditioner::{ConditionerConfig, ConditionerStats, LinkConditioner};

/// A packet waiting for its time.
struct Pending {
    due: Duration,
    order: u64,
    destination: SocketAddr,
    ecn: Option<EcnCodepoint>,
    src_ip: Option<IpAddr>,
    contents: Vec<u8>,
}

impl PartialEq for Pending {
    fn eq(&self, other: &Self) -> bool {
        (self.due, self.order) == (other.due, other.order)
    }
}

impl Eq for Pending {}

impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pending {
    /// Reversed, so that the heap's top is the earliest.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (other.due, other.order).cmp(&(self.due, self.order))
    }
}

struct State {
    conditioner: LinkConditioner,
    queue: BinaryHeap<Pending>,
    order: u64,
    /// Packets the inner socket refused (it would have blocked).
    refused: u64,
}

struct Shared {
    inner: Arc<dyn AsyncUdpSocket>,
    start: Instant,
    state: Mutex<State>,
    /// Wakes the queue's task: a packet came in, or the socket is gone.
    wake: Arc<Notify>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves a queue of packets, still usable.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send_now(&self, state: &mut State, pending: &Pending) {
        let transmit = Transmit {
            destination: pending.destination,
            ecn: pending.ecn,
            contents: &pending.contents,
            segment_size: None,
            src_ip: pending.src_ip,
        };
        if self.inner.try_send(&transmit).is_err() {
            state.refused += 1;
        }
    }

    /// Sends the packets that are due; the time of the next one.
    fn flush(&self) -> Option<Duration> {
        let now = self.start.elapsed();
        let mut state = self.lock();
        while state.queue.peek().is_some_and(|p| p.due <= now) {
            let pending = state.queue.pop().expect("peeked");
            self.send_now(&mut state, &pending);
        }
        state.queue.peek().map(|p| p.due)
    }
}

/// A handle on a socket's conditioner, to read what it did and change the link while it runs.
#[derive(Clone)]
pub struct Conditioner {
    shared: Arc<Shared>,
}

impl fmt::Debug for Conditioner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Conditioner")
            .field("config", &self.config())
            .field("stats", &self.stats())
            .finish()
    }
}

impl Conditioner {
    /// The link's settings.
    pub fn config(&self) -> ConditionerConfig {
        self.shared.lock().conditioner.config()
    }

    /// Changes the link from the next packet on (see [`LinkConditioner::set_config`]).
    pub fn set_config(&self, config: ConditionerConfig) {
        self.shared.lock().conditioner.set_config(config);
    }

    /// Cuts the link (every packet lost) or mends it (back to `config`'s loss).
    pub fn cut(&self, cut: bool, config: ConditionerConfig) {
        let loss = if cut { 1.0 } else { config.loss };
        self.set_config(ConditionerConfig { loss, ..config });
    }

    /// What the conditioner did so far.
    pub fn stats(&self) -> ConditionerStats {
        self.shared.lock().conditioner.stats()
    }

    /// Packets waiting for their time.
    pub fn queued(&self) -> usize {
        self.shared.lock().queue.len()
    }

    /// Packets the inner socket refused to send (it would have blocked).
    pub fn refused(&self) -> u64 {
        self.shared.lock().refused
    }
}

/// quinn's socket with a conditioner on its sending side (see the module's documentation).
pub struct ConditionedSocket {
    shared: Arc<Shared>,
}

impl fmt::Debug for ConditionedSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConditionedSocket")
            .field("inner", &self.shared.inner)
            .finish()
    }
}

impl Drop for ConditionedSocket {
    fn drop(&mut self) {
        self.shared.wake.notify_one();
    }
}

impl ConditionedSocket {
    /// Wraps `inner` and starts the task that sends the delayed packets. Call it inside a tokio
    /// runtime.
    pub fn wrap(
        inner: Arc<dyn AsyncUdpSocket>,
        config: ConditionerConfig,
    ) -> (Arc<Self>, Conditioner) {
        let wake = Arc::new(Notify::new());
        let shared = Arc::new(Shared {
            inner,
            start: Instant::now(),
            state: Mutex::new(State {
                conditioner: LinkConditioner::new(config),
                queue: BinaryHeap::new(),
                order: 0,
                refused: 0,
            }),
            wake: wake.clone(),
        });
        tokio::spawn(send_when_due(Arc::downgrade(&shared), wake));
        let socket = Arc::new(Self {
            shared: shared.clone(),
        });
        (socket, Conditioner { shared })
    }
}

/// The queue's task: sleeps until the next packet is due or a new one comes in, sends what is
/// due, and stops once the socket is gone.
async fn send_when_due(shared: Weak<Shared>, wake: Arc<Notify>) {
    loop {
        let (next, start) = {
            let Some(shared) = shared.upgrade() else {
                return;
            };
            (shared.flush(), shared.start)
        };
        match next {
            Some(due) => {
                tokio::select! {
                    _ = tokio::time::sleep_until(start + due) => {}
                    _ = wake.notified() => {}
                }
            }
            None => wake.notified().await,
        }
    }
}

impl AsyncUdpSocket for ConditionedSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.shared.inner.clone().create_io_poller()
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        let shared = &self.shared;
        let now = shared.start.elapsed();
        let segment = transmit.segment_size.unwrap_or(transmit.contents.len());
        let mut state = shared.lock();
        let mut woke = false;
        for contents in transmit.contents.chunks(segment.max(1)) {
            for due in state
                .conditioner
                .submit(now, contents.len())
                .into_iter()
                .flatten()
            {
                let order = state.order;
                state.order += 1;
                let pending = Pending {
                    due,
                    order,
                    destination: transmit.destination,
                    ecn: transmit.ecn,
                    src_ip: transmit.src_ip,
                    contents: contents.to_vec(),
                };
                if due <= now && state.queue.is_empty() {
                    shared.send_now(&mut state, &pending);
                } else {
                    state.queue.push(pending);
                    woke = true;
                }
            }
        }
        drop(state);
        if woke {
            shared.wake.notify_one();
        }
        // What the link lost is lost on the way, not refused here: quinn sees it sent.
        Ok(())
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        self.shared.inner.poll_recv(cx, bufs, meta)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.shared.inner.local_addr()
    }

    fn max_receive_segments(&self) -> usize {
        self.shared.inner.max_receive_segments()
    }

    fn may_fragment(&self) -> bool {
        self.shared.inner.may_fragment()
    }
}

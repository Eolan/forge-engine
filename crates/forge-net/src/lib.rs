//! `forge-net`: the networking (Phase 5, issue #268; D-010).
//!
//! The transport is QUIC (RFC 9000) through `quinn`, with TLS 1.3 from `rustls` on `ring`'s
//! cryptography. On it:
//!
//! - **a versioned protocol** ([`protocol`]): ALPN names it, the client's hello carries its
//!   version, its channels and a **connection token** ([`token`]) that the login service signs
//!   and the server checks offline; a server refuses a client of another version, with other
//!   channels or a bad token, and says why;
//! - **channels** ([`channel`]): reliable ordered (a stream per channel), reliable unordered (a
//!   stream per message) and unreliable (QUIC datagrams, RFC 9221), with **fragmentation**
//!   ([`fragment`]) for unreliable messages longer than a datagram;
//! - **keep-alives and timeouts**: QUIC's pings and idle timeout ([`TransportSettings`]);
//! - **a link conditioner** ([`conditioner`], [`socket`]): latency, jitter, loss, duplication,
//!   reordering and a rate, decided from a seed, under the UDP socket quinn sends through, so
//!   tests run the real protocol over a bad link on loopback.
//!
//! [`Server`] and [`Client`] make [`Session`]s; a session sends and receives [`Message`]s on
//! its channels. Everything runs on a tokio runtime, which in the engine is the network thread's
//! (ARCHITECTURE.md, "Threads"); the simulation hands messages over and never waits on I/O.
//!
//! On the channels, **replication** (no I/O of its own, so it is tested on a virtual clock):
//! - [`quantize`]: an entity's state in integers both ends share; [`bits`]: the bit writer;
//! - [`snapshot`]: the packet, deltas against what the client acknowledged, and the client's
//!   [`SnapshotReceiver`];
//! - [`interest`] and [`replicate`]: the server's [`Replicator`], interest by cells and a
//!   priority accumulator under each client's byte budget;
//! - [`interpolate`]: the client's picture of the others, a fixed delay behind the server's clock,
//!   and the moment a shot carries for the authority's rewind;
//! - [`crowd`]: a stand-in crowd for the tests and the measures.

#![forbid(unsafe_code)]

pub mod bits;
pub mod channel;
pub mod conditioner;
pub mod crowd;
pub mod endpoint;
pub mod fragment;
pub mod interest;
pub mod interpolate;
pub mod protocol;
pub mod quantize;
pub mod replicate;
pub mod session;
pub mod snapshot;
pub mod socket;
pub mod token;
pub mod wire;

pub use channel::{ChannelId, Channels, Delivery};
pub use conditioner::{ConditionerConfig, ConditionerStats, LinkConditioner};
pub use endpoint::{
    Client, ClientConfig, ConnectError, Server, ServerConfig, ServerEvent, ServerIdentity,
    SetupError, TransportSettings, loopback,
};
pub use interest::InterestGrid;
pub use interpolate::{Interpolator, Moment, PlayoutClock};
pub use protocol::{ALPN, PROTOCOL_VERSION, Refusal};
pub use quantize::{EntityState, QState, Quantization};
pub use quinn::ConnectionError;
pub use replicate::{ReplicationConfig, Replicator, SnapshotStats};
pub use session::{Message, SendError, Session, SessionStats};
pub use snapshot::{Ack, EntityId, SnapshotReceiver, Taken};
pub use socket::Conditioner;
pub use token::{TokenClaims, TokenError, TokenIssuer, TokenVerifier};

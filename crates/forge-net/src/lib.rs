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
//! Replication (snapshots, deltas, priorities, interest) comes next on top of these channels.

#![forbid(unsafe_code)]

pub mod channel;
pub mod conditioner;
pub mod endpoint;
pub mod fragment;
pub mod protocol;
pub mod session;
pub mod socket;
pub mod token;
pub mod wire;

pub use channel::{ChannelId, Channels, Delivery};
pub use conditioner::{ConditionerConfig, ConditionerStats, LinkConditioner};
pub use endpoint::{
    Client, ClientConfig, ConnectError, Server, ServerConfig, ServerEvent, ServerIdentity,
    SetupError, TransportSettings, loopback,
};
pub use protocol::{ALPN, PROTOCOL_VERSION, Refusal};
pub use quinn::ConnectionError;
pub use session::{Message, SendError, Session, SessionStats};
pub use socket::Conditioner;
pub use token::{TokenClaims, TokenError, TokenIssuer, TokenVerifier};

//! The protocol's identity and its refusals.
//!
//! Two layers name the protocol. TLS's ALPN carries [`ALPN`], so that a client of something
//! else fails the TLS handshake. The client's [`crate::wire::Hello`] then carries
//! [`PROTOCOL_VERSION`]: a server of another version closes the connection with
//! [`Refusal::Version`], whose reason names both versions, so the client can say which update
//! it needs. Every refusal is a QUIC application close (RFC 9000, 10.2) with a code from the
//! table below and a few bytes of reason.
//!
//! | code | refusal | reason bytes |
//! |---|---|---|
//! | `0x101` | the version | the server's and the client's, `u16` each |
//! | `0x102` | the token | the [`TokenError`]'s byte |
//! | `0x103` | the channels differ | none |
//! | `0x104` | a malformed hello | none |
//! | `0x105` | no hello in time | none |
//!
//! Code 0 is a session closed normally.

use crate::token::TokenError;

/// The protocol's version. A change in any byte layout raises it.
pub const PROTOCOL_VERSION: u16 = 1;

/// The protocol's name in TLS's application-layer protocol negotiation.
pub const ALPN: &[u8] = b"forge-net";

/// The application close code of a session ended normally.
pub const CLOSE_NORMAL: u32 = 0;

/// Why a server refused a client.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// The two speak different versions.
    #[error("the server speaks version {server}, the client {client}")]
    Version {
        /// The server's version.
        server: u16,
        /// The client's version.
        client: u16,
    },
    /// The token was refused.
    #[error("token refused: {0}")]
    Token(TokenError),
    /// The two declare different channels.
    #[error("the client's channels differ from the server's")]
    Channels,
    /// The hello did not parse.
    #[error("the hello did not parse")]
    Malformed,
    /// No hello came in time.
    #[error("no hello came in time")]
    Timeout,
}

impl Refusal {
    /// Its application close code.
    pub fn code(&self) -> u32 {
        match self {
            Self::Version { .. } => 0x101,
            Self::Token(_) => 0x102,
            Self::Channels => 0x103,
            Self::Malformed => 0x104,
            Self::Timeout => 0x105,
        }
    }

    /// Its reason bytes.
    pub fn reason(&self) -> Vec<u8> {
        match self {
            Self::Version { server, client } => {
                let mut r = server.to_le_bytes().to_vec();
                r.extend_from_slice(&client.to_le_bytes());
                r
            }
            Self::Token(e) => vec![*e as u8],
            _ => Vec::new(),
        }
    }

    /// The refusal a close's code and reason name, if they name one.
    pub fn from_close(code: u64, reason: &[u8]) -> Option<Self> {
        match code {
            0x101 if reason.len() == 4 => Some(Self::Version {
                server: u16::from_le_bytes([reason[0], reason[1]]),
                client: u16::from_le_bytes([reason[2], reason[3]]),
            }),
            0x102 => Some(Self::Token(TokenError::from_u8(*reason.first()?)?)),
            0x103 => Some(Self::Channels),
            0x104 => Some(Self::Malformed),
            0x105 => Some(Self::Timeout),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_round_trip_through_a_close() {
        for r in [
            Refusal::Version {
                server: 1,
                client: 9,
            },
            Refusal::Token(TokenError::Expired),
            Refusal::Channels,
            Refusal::Malformed,
            Refusal::Timeout,
        ] {
            assert_eq!(Refusal::from_close(r.code().into(), &r.reason()), Some(r));
        }
        assert_eq!(Refusal::from_close(0, &[]), None);
    }
}
